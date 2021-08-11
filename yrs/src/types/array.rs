use crate::block::{BlockPtr, ItemContent, ItemPosition};
use crate::types::{Inner, TypePtr};
use crate::{Transaction, ID};
use lib0::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub struct Array(Rc<RefCell<Inner>>);

impl Array {
    pub fn new(inner: Rc<RefCell<Inner>>) -> Self {
        Array(inner)
    }

    pub fn len(&self) -> u32 {
        let inner = self.0.borrow();
        inner.len()
    }

    pub fn insert<V: Into<ItemContent>>(&self, txn: &mut Transaction, index: u32, value: V) {
        let (start, parent) = {
            let parent = self.0.borrow();
            if index <= parent.len() {
                (parent.start.get(), parent.ptr.clone())
            } else {
                panic!("Cannot insert item at index over the length of an array")
            }
        };
        let (left, right) = if index == 0 {
            (None, None)
        } else {
            Self::index_to_ptr(txn, start, index)
        };
        let content = value.into();
        let pos = ItemPosition {
            parent,
            left,
            right,
            index: 0,
        };
        txn.create_item(&pos, content, None);
    }

    fn index_to_ptr(
        txn: &mut Transaction,
        mut ptr: Option<BlockPtr>,
        mut index: u32,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        while let Some(p) = ptr {
            let item = txn
                .store
                .blocks
                .get_item(&p)
                .expect("No item for a given pointer was found.");
            let len = item.len();
            if !item.is_deleted() && item.is_countable() {
                if index == len {
                    let left = Some(p.clone());
                    let right = item.right.clone();
                    return (left, right);
                } else if index < len {
                    let split_point = ID::new(item.id.client, item.id.clock + index);
                    let ptr = BlockPtr::new(split_point, p.pivot() as u32);
                    let (left, mut right) = txn.store.blocks.split_block(&ptr);
                    if right.is_none() {
                        if let Some(left_ptr) = left.as_ref() {
                            if let Some(left) = txn.store.blocks.get_item(left_ptr) {
                                right = left.right.clone();
                            }
                        }
                    }
                    return (left, right);
                }
                index -= len;
            }
            ptr = item.right.clone();
        }
        (None, None)
    }

    pub fn push_back<V: Into<ItemContent>>(&self, txn: &mut Transaction, content: V) {
        let len = self.len();
        self.insert(txn, len, content)
    }

    pub fn push_front<V: Into<ItemContent>>(&self, txn: &mut Transaction, content: V) {
        self.insert(txn, 0, content)
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, mut len: u32) {
        let start = {
            let parent = self.0.borrow();
            parent.start.get()
        };
        let (_, mut ptr) = if index == 0 {
            (None, start)
        } else {
            Self::index_to_ptr(txn, start, index)
        };
        while len > 0 {
            if let Some(mut p) = ptr {
                if let Some(item) = txn.store.blocks.get_item(&p) {
                    if !item.is_deleted() {
                        let item_len = item.len();
                        let (l, r) = if len < item_len {
                            p.id.clock += len;
                            len = 0;
                            txn.store.blocks.split_block(&p)
                        } else {
                            len -= item_len;
                            (ptr, item.right.clone())
                        };
                        txn.delete(&l.unwrap());
                        ptr = r;
                    } else {
                        ptr = item.right.clone();
                    }
                }
            } else {
                break;
            }
        }

        if len > 0 {
            panic!("Array length exceeded");
        }
    }

    pub fn get(&self, txn: &Transaction, mut index: u32) -> Option<Any> {
        let inner = self.0.borrow();
        let mut ptr = inner.start.get();
        while let Some(p) = ptr {
            let item = txn.store.blocks.get_item(&p)?;
            let len = item.len();
            if !item.is_deleted() && item.is_countable() {
                if index < len {
                    let mut value = item.content.get_content(txn);
                    return Some(value.remove(index as usize));
                }
            }
            index -= len;
            ptr = item.right.clone();
        }

        None
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Iter<'b, 'txn> {
        Iter::new(self, txn)
    }

    pub fn to_json(&self, txn: &Transaction<'_>) -> Any {
        let res = self.iter(txn).collect();
        Any::Array(res)
    }
}

pub struct Iter<'b, 'txn> {
    content: VecDeque<Any>,
    ptr: Option<BlockPtr>,
    txn: &'b Transaction<'txn>,
}

impl<'b, 'txn> Iter<'b, 'txn> {
    fn new(array: &Array, txn: &'b Transaction<'txn>) -> Self {
        let inner = array.0.borrow();
        let ptr = inner.start.get();
        Iter {
            ptr,
            txn,
            content: VecDeque::default(),
        }
    }
}

impl<'b, 'txn> Iterator for Iter<'b, 'txn> {
    type Item = Any;

    fn next(&mut self) -> Option<Self::Item> {
        match self.content.pop_front() {
            None => {
                if let Some(ptr) = self.ptr.take() {
                    let item = self.txn.store.blocks.get_item(&ptr)?;
                    self.ptr = item.right.clone();
                    if !item.is_deleted() && item.is_countable() {
                        self.content = item.content.get_content(self.txn).into();
                    }
                    self.next()
                } else {
                    None // end of iterator
                }
            }
            value => value,
        }
    }
}

impl Into<ItemContent> for Array {
    fn into(self) -> ItemContent {
        ItemContent::Type(self.0.clone())
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::exchange_updates;
    use crate::Doc;
    use lib0::any::Any;
    use std::collections::HashMap;

    #[test]
    fn push_back() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let a = txn.get_array("array");

        a.push_back(&mut txn, "a");
        a.push_back(&mut txn, "b");
        a.push_back(&mut txn, "c");

        let actual: Vec<_> = a.iter(&txn).collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn push_front() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let a = txn.get_array("array");

        a.push_front(&mut txn, "c");
        a.push_front(&mut txn, "b");
        a.push_front(&mut txn, "a");

        let actual: Vec<_> = a.iter(&txn).collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn insert() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let a = txn.get_array("array");

        a.insert(&mut txn, 0, "a");
        a.insert(&mut txn, 1, "c");
        a.insert(&mut txn, 1, "b");

        let actual: Vec<_> = a.iter(&txn).collect();
        assert_eq!(actual, vec!["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn basic() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);

        let mut t1 = d1.transact();
        let a1 = t1.get_array("array");

        a1.insert(&mut t1, 0, "Hi");
        let update = d1.encode_state_as_update(&t1);

        let mut t2 = d2.transact();
        d2.apply_update(&mut t2, update.as_slice());
        let a2 = t2.get_array("array");
        let actual: Vec<_> = a2.iter(&t2).collect();

        assert_eq!(actual, vec![Any::String("Hi".to_owned())]);
    }

    #[test]
    fn len() {
        let d = Doc::with_client_id(1);

        {
            let mut txn = d.transact();
            let a = txn.get_array("array");

            a.push_back(&mut txn, 0); // len: 1
            a.push_back(&mut txn, 1); // len: 2
            a.push_back(&mut txn, 2); // len: 3
            a.push_back(&mut txn, 3); // len: 4

            a.remove(&mut txn, 0, 1); // len: 3
            a.insert(&mut txn, 0, 0); // len: 4

            assert_eq!(a.len(), 4);
        }
        {
            let mut txn = d.transact();
            let a = txn.get_array("array");
            a.remove(&mut txn, 1, 1); // len: 3
            assert_eq!(a.len(), 3);

            a.insert(&mut txn, 1, 1); // len: 4
            assert_eq!(a.len(), 4);

            a.remove(&mut txn, 2, 1); // len: 3
            assert_eq!(a.len(), 3);

            a.insert(&mut txn, 2, 2); // len: 4
            assert_eq!(a.len(), 4);
        }

        let mut txn = d.transact();
        let a = txn.get_array("array");
        assert_eq!(a.len(), 4);

        a.remove(&mut txn, 1, 1);
        assert_eq!(a.len(), 3);

        a.insert(&mut txn, 1, 1);
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn remove_insert() {
        let d1 = Doc::with_client_id(1);

        let mut t1 = d1.transact();
        let a1 = t1.get_array("array");
        a1.insert(&mut t1, 0, "A");
        a1.remove(&mut t1, 1, 0);
    }

    #[test]
    fn insert_3_elements_try_re_get() {
        let d1 = Doc::with_client_id(1);
        let d2 = Doc::with_client_id(2);
        {
            let mut t1 = d1.transact();
            let a1 = t1.get_array("array");

            a1.push_back(&mut t1, 1);
            a1.push_back(&mut t1, true);
            a1.push_back(&mut t1, false);
            let actual: Vec<_> = a1.iter(&t1).collect();
            assert_eq!(
                actual,
                vec![Any::Number(1.0), Any::Bool(true), Any::Bool(false)]
            );
        }

        exchange_updates(&[&d1, &d2]);

        let mut t2 = d2.transact();
        let a2 = t2.get_array("array");
        let actual: Vec<_> = a2.iter(&t2).collect();
        assert_eq!(
            actual,
            vec![Any::Number(1.0), Any::Bool(true), Any::Bool(false)]
        );
    }

    #[test]
    fn concurrent_insert_with_3_conflicts() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert(&mut txn, 0, 0);
        }

        let d2 = Doc::with_client_id(2);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert(&mut txn, 0, 1);
        }

        let d3 = Doc::with_client_id(3);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.insert(&mut txn, 0, 2);
        }

        exchange_updates(&[&d1, &d2, &d3]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    fn to_array(d: &Doc) -> Vec<Any> {
        let mut txn = d.transact();
        let a = txn.get_array("array");
        a.iter(&txn).collect()
    }

    #[test]
    fn concurrent_insert_remove_with_3_conflicts() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
            a.push_back(&mut txn, "z");
        }
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        exchange_updates(&[&d1, &d2, &d3]);

        {
            // start state: [x,y,z]
            let mut t1 = d1.transact();
            let mut t2 = d2.transact();
            let mut t3 = d3.transact();
            let a1 = t1.get_array("array");
            let a2 = t2.get_array("array");
            let a3 = t3.get_array("array");

            a1.insert(&mut t1, 1, 0); // [x,0,y,z]
            a2.remove(&mut t2, 0, 1); // [y,z]
            a2.remove(&mut t2, 1, 1); // [y]
            a3.insert(&mut t3, 1, 2); // [x,2,y,z]
        }

        exchange_updates(&[&d1, &d2, &d3]);
        // after exchange expected: [0,2,y]

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    #[test]
    fn insertions_in_late_sync() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
        }
        let d2 = Doc::with_client_id(2);
        let d3 = Doc::with_client_id(3);

        exchange_updates(&[&d1, &d2, &d3]);

        {
            let mut t1 = d1.transact();
            let mut t2 = d2.transact();
            let mut t3 = d3.transact();
            let a1 = t1.get_array("array");
            let a2 = t2.get_array("array");
            let a3 = t3.get_array("array");

            a1.insert(&mut t1, 1, "user0");
            a2.insert(&mut t2, 1, "user1");
            a3.insert(&mut t3, 1, "user2");
        }

        exchange_updates(&[&d1, &d2, &d3]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);
        let a3 = to_array(&d3);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
        assert_eq!(a2, a3, "Peer 2 and peer 3 states are different");
    }

    #[test]
    fn removals_in_late_sync() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
        }
        let d2 = Doc::with_client_id(2);

        exchange_updates(&[&d1, &d2]);

        {
            let mut t1 = d1.transact();
            let mut t2 = d2.transact();
            let a1 = t1.get_array("array");
            let a2 = t2.get_array("array");

            a2.remove(&mut t2, 1, 1);
            a1.remove(&mut t1, 0, 2);
        }

        exchange_updates(&[&d1, &d2]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
    }

    #[test]
    fn insert_then_merge_delete_on_sync() {
        let d1 = Doc::with_client_id(1);
        {
            let mut txn = d1.transact();
            let a = txn.get_array("array");
            a.push_back(&mut txn, "x");
            a.push_back(&mut txn, "y");
            a.push_back(&mut txn, "z");
        }
        let d2 = Doc::with_client_id(2);

        exchange_updates(&[&d1, &d2]);

        {
            let mut t2 = d2.transact();
            let a2 = t2.get_array("array");

            a2.remove(&mut t2, 0, 3);
        }

        exchange_updates(&[&d1, &d2]);

        let a1 = to_array(&d1);
        let a2 = to_array(&d2);

        assert_eq!(a1, a2, "Peer 1 and peer 2 states are different");
    }

    #[test]
    fn iter_array_containing_types() {
        let d = Doc::with_client_id(1);
        let mut txn = d.transact();
        let a = txn.get_array("arr");
        for i in 0..10 {
            let name = format!("map{}", i);
            let m = txn.get_map(name.as_str());
            m.insert(&mut txn, "value".to_owned(), i);
            a.push_back(&mut txn, m);
        }

        for (i, value) in a.iter(&txn).enumerate() {
            let mut expected = HashMap::new();
            expected.insert("value".to_owned(), Any::Number(i as f64));
            assert_eq!(value, Any::Map(expected));
        }
    }
}
