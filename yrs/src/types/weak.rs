use crate::atomic::AtomicRef;
use crate::block::{Block, BlockPtr, BlockSlice, EmbedPrelim, ItemContent, Prelim};
use crate::types::{Branch, BranchPtr, EventHandler, Observers, TypeRef, Value};
use crate::{GetString, Observable, OffsetKind, ReadTxn, TransactionMut, XmlTextRef, ID};
use std::convert::TryFrom;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

#[repr(transparent)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WeakRef(BranchPtr);

impl WeakRef {
    pub fn try_deref_raw<T: ReadTxn>(&self, txn: &T) -> Option<Value> {
        self.unquote(txn).next()
    }

    pub fn try_deref<T, V>(&self, txn: &T) -> Result<V, Option<V::Error>>
    where
        T: ReadTxn,
        V: TryFrom<Value>,
    {
        if let Some(value) = self.try_deref_raw(txn) {
            match V::try_from(value) {
                Ok(value) => Ok(value),
                Err(value) => Err(Some(value)),
            }
        } else {
            Err(None)
        }
    }

    /// Returns an iterator over [Value]s existing in a scope of the current [WeakRef] quotation
    /// range.  
    pub fn unquote<'txn, T: ReadTxn>(&self, txn: &'txn T) -> Unquote<'txn, T> {
        if let TypeRef::WeakLink(source) = &self.0.type_ref {
            source.unquote(txn)
        } else {
            Unquote::empty(txn)
        }
    }
}

impl AsRef<Branch> for WeakRef {
    fn as_ref(&self) -> &Branch {
        self.0.deref()
    }
}

impl From<BranchPtr> for WeakRef {
    fn from(inner: BranchPtr) -> Self {
        WeakRef(inner)
    }
}

impl TryFrom<BlockPtr> for WeakRef {
    type Error = BlockPtr;

    fn try_from(value: BlockPtr) -> Result<Self, Self::Error> {
        if let Block::Item(item) = value.deref() {
            if let ItemContent::Type(branch) = &item.content {
                let branch = BranchPtr::from(branch.deref());
                return Ok(Self::from(branch));
            }
        }
        Err(value)
    }
}

impl AsMut<Branch> for WeakRef {
    fn as_mut(&mut self) -> &mut Branch {
        self.0.deref_mut()
    }
}

impl Observable for WeakRef {
    type Event = WeakEvent;

    fn try_observer(&self) -> Option<&EventHandler<Self::Event>> {
        if let Some(Observers::Weak(eh)) = self.0.observers.as_ref() {
            Some(eh)
        } else {
            None
        }
    }

    fn try_observer_mut(&mut self) -> Option<&mut EventHandler<Self::Event>> {
        if let Observers::Weak(eh) = self.0.observers.get_or_insert_with(Observers::weak) {
            Some(eh)
        } else {
            None
        }
    }
}

impl GetString for WeakRef {
    fn get_string<T: ReadTxn>(&self, txn: &T) -> String {
        if let TypeRef::WeakLink(source) = &self.0.type_ref {
            let mut curr = source.first_item.get_owned();
            if let Some(Block::Item(item)) = curr.as_deref() {
                if let Some(branch) = item.parent.as_branch() {
                    match &branch.type_ref {
                        TypeRef::Text => {
                            let mut result = String::new();
                            while let Some(Block::Item(item)) = curr.as_deref() {
                                if !item.is_deleted() {
                                    if let ItemContent::String(s) = &item.content {
                                        result.push_str(s.as_str());
                                    }
                                }
                                if item.last_id() == source.quote_end {
                                    break;
                                }
                                curr = item.right;
                            }
                            return result;
                        }
                        TypeRef::XmlText => {
                            return XmlTextRef::get_string_fragment(
                                branch.start,
                                Some(&source.quote_start),
                                Some(&source.quote_end),
                            );
                        }
                        _ => {}
                    }
                }
            }
            "".to_string()
        } else {
            panic!("Defect: called WeakRef::get_string on non WeakRef shared type")
        }
    }
}

/// A preliminary type for [WeakRef]. Once inserted into document it can be used as a weak reference
/// link to another value living inside of the document store.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WeakPrelim(pub(crate) Arc<LinkSource>);

impl WeakPrelim {
    pub(crate) fn new(start: ID, end: ID) -> Self {
        WeakPrelim(Arc::new(LinkSource::new(start, end)))
    }
}

impl From<WeakRef> for WeakPrelim {
    fn from(value: WeakRef) -> Self {
        if let TypeRef::WeakLink(source) = &value.0.type_ref {
            WeakPrelim(source.clone())
        } else {
            panic!("Defect: WeakRef's underlying branch is not matching expected weak ref.")
        }
    }
}

impl Prelim for WeakPrelim {
    type Return = WeakRef;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let inner = Branch::new(TypeRef::WeakLink(self.0.clone()));
        (ItemContent::Type(inner), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {}
}

impl Into<EmbedPrelim<WeakPrelim>> for WeakPrelim {
    fn into(self) -> EmbedPrelim<WeakPrelim> {
        EmbedPrelim::Shared(self)
    }
}

pub struct WeakEvent {
    pub(crate) current_target: BranchPtr,
    target: WeakRef,
}

impl WeakEvent {
    pub(crate) fn new(branch_ref: BranchPtr) -> Self {
        let current_target = branch_ref.clone();
        WeakEvent {
            target: WeakRef::from(branch_ref),
            current_target,
        }
    }

    pub fn target(&self) -> &WeakRef {
        &self.target
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct LinkSource {
    pub(crate) quote_start: ID,
    pub(crate) quote_end: ID,
    pub(crate) first_item: AtomicRef<BlockPtr>,
}

impl LinkSource {
    pub fn new(start: ID, end: ID) -> Self {
        LinkSource {
            quote_start: start,
            quote_end: end,
            first_item: AtomicRef::default(),
        }
    }

    pub fn is_single(&self) -> bool {
        self.quote_start == self.quote_end
    }

    pub(crate) fn unquote<'txn, T: ReadTxn>(&self, txn: &'txn T) -> Unquote<'txn, T> {
        let mut current = self.first_item.get_owned();
        if let Some(ptr) = &mut current {
            if Self::try_right_most(ptr) {
                self.first_item.swap(*ptr);
                current = Some(*ptr);
            }
        }
        Unquote::new(txn, current, &self.quote_start, self.quote_end.clone())
    }

    /// If provided ref is pointing to map type which has been updated, we may want to invalidate
    /// current pointer to point to its right most neighbor.
    fn try_right_most(block_ref: &mut BlockPtr) -> bool {
        match BlockPtr::deref(block_ref) {
            Block::Item(item) if item.parent_sub.is_some() => {
                // for map types go to the most recent one
                if let Some(mut curr_block) = item.right {
                    while let Block::Item(item) = curr_block.deref() {
                        if let Some(right) = item.right {
                            curr_block = right;
                        } else {
                            break;
                        }
                    }
                    *block_ref = curr_block;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub(crate) fn materialize(&self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let mut curr = self
            .first_item
            .get_owned()
            .or(txn.store.blocks.get_block(&self.quote_start));
        if let Some(mut ptr) = curr {
            let offset = self.quote_start.clock as i32 - ptr.id().clock as i32;
            if offset > 0 {
                let slice = BlockSlice::new(ptr, offset as u32, ptr.len() - 1);
                ptr = txn.store.materialize(slice);
            }
            self.first_item.swap(ptr);
            curr = Some(ptr);
        }
        while let Some(mut ptr) = curr {
            let last = if ptr.contains(&self.quote_end) {
                let offset = self.quote_end.clock - ptr.id().clock;
                let slice = BlockSlice::new(ptr, 0, offset);
                ptr = txn.store.materialize(slice);
                true
            } else {
                false
            };
            if let Block::Item(item) = ptr.clone().deref_mut() {
                item.info.set_linked();
                let linked_by = txn.store.linked_by.entry(ptr).or_default();
                linked_by.insert(inner_ref);
                if last {
                    break;
                } else {
                    curr = item.right;
                }
            } else {
                break;
            }
        }
    }
}

/// Iterator over non-deleted items, bounded by the given ID range.
pub struct Unquote<'txn, T: ReadTxn> {
    txn: &'txn T,
    to: ID,
    current: Option<BlockPtr>,
    encoding: OffsetKind,
    offset: u32,
}

impl<'txn, T: ReadTxn> Unquote<'txn, T> {
    fn new(txn: &'txn T, current: Option<BlockPtr>, from: &ID, to: ID) -> Self {
        let mut offset = 0;
        if let Some(ptr) = current {
            if ptr.contains(&from) {
                offset = from.clock - ptr.id().clock;
            }
        }
        let encoding = txn.store().options.offset_kind;
        Unquote {
            txn,
            to,
            offset,
            encoding,
            current,
        }
    }

    fn empty(txn: &'txn T) -> Self {
        Unquote {
            txn,
            to: ID::new(0, 0), // won't be used anyway
            current: None,
            encoding: OffsetKind::Bytes,
            offset: 0,
        }
    }
}

impl<'txn, T: ReadTxn> Iterator for Unquote<'txn, T> {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        // move to a first non-deleted item
        while let Some(Block::Item(item)) = self.current.as_deref() {
            if !item.is_deleted() && self.offset < item.content_len(self.encoding) {
                break;
            }
            self.current = item.right;
            self.offset = 0;
        }
        let ptr = self.current?;
        if let Block::Item(item) = ptr.deref() {
            let mut result = [Value::default(); 1];
            if item.content.read(self.offset as usize, &mut result) != 0 {
                if item.id.client == self.to.client && item.id.clock + self.offset == self.to.clock
                {
                    self.current = None; // we reached the end of range
                }
                self.offset += 1;
                return Some(std::mem::take(&mut result[0]));
            }
        }
        None
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::exchange_updates;
    use crate::types::text::{Diff, YChange};
    use crate::types::weak::WeakPrelim;
    use crate::types::{Attrs, EntryChange, Event, ToJson, Value};
    use crate::{
        Array, DeepObservable, Doc, GetString, Map, MapPrelim, MapRef, Observable, Text, Transact,
    };
    use lib0::any::Any;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Arc;

    #[test]
    fn basic_map_link() {
        let doc = Doc::new();
        let map = doc.get_or_insert_map("map");
        let mut txn = doc.transact_mut();
        let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
        let nested = map.insert(&mut txn, "a", nested);
        let link = map.link(&txn, "a").unwrap();
        map.insert(&mut txn, "b", link);

        let link = map.get(&txn, "b").unwrap().to_weak().unwrap();
        let expected = nested.to_json(&txn);
        let deref: MapRef = link.try_deref(&txn).unwrap();
        let actual = deref.to_json(&txn);

        assert_eq!(actual, expected);
    }

    #[test]
    fn basic_array_link() {
        let d1 = Doc::new();
        let a1 = d1.get_or_insert_array("array");
        {
            let mut txn = d1.transact_mut();

            a1.insert_range(&mut txn, 0, [1, 2, 3]);
            let link = a1.quote(&txn, 1, 1).unwrap();
            a1.insert(&mut txn, 3, link);

            assert_eq!(a1.get(&txn, 0), Some(1.into()));
            assert_eq!(a1.get(&txn, 1), Some(2.into()));
            assert_eq!(a1.get(&txn, 2), Some(3.into()));
            let actual: Any = a1
                .get(&txn, 3)
                .unwrap()
                .to_weak()
                .unwrap()
                .try_deref(&txn)
                .unwrap();
            assert_eq!(actual, 2.into());
        }

        let d2 = Doc::new();
        let a2 = d2.get_or_insert_array("array");

        exchange_updates(&[&d1, &d2]);
        let txn = d2.transact_mut();

        assert_eq!(a2.get(&txn, 0), Some(1.into()));
        assert_eq!(a2.get(&txn, 1), Some(2.into()));
        assert_eq!(a2.get(&txn, 2), Some(3.into()));
        let actual: Any = a2
            .get(&txn, 3)
            .unwrap()
            .to_weak()
            .unwrap()
            .try_deref(&txn)
            .unwrap();
        assert_eq!(actual, 2.into());
    }

    #[test]
    fn array_quote_multi_elements() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let nested = {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, [1, 2]);
            let nested = a1.push_back(&mut txn, MapPrelim::from([("key", "value")]));
            a1.push_back(&mut txn, 3);
            nested
        };
        let l1 = {
            let mut t1 = d1.transact_mut();
            let prelim = a1.quote(&t1, 1, 3).unwrap();
            a1.insert(&mut t1, 0, prelim)
        };

        let t1 = d1.transact();
        assert_eq!(
            l1.unquote(&t1).collect::<Vec<Value>>(),
            vec![2.into(), Value::YMap(nested.clone()), 3.into()]
        );
        assert_eq!(a1.get(&t1, 1), Some(1.into()));
        assert_eq!(a1.get(&t1, 2), Some(2.into()));
        assert_eq!(a1.get(&t1, 3), Some(Value::YMap(nested.clone())));
        assert_eq!(a1.get(&t1, 4), Some(3.into()));
        drop(t1);

        exchange_updates(&[&d1, &d2]);

        let t2 = d2.transact();
        let l2 = a2.get(&t2, 0).unwrap().to_weak().unwrap();
        let unquoted: Vec<_> = l2.unquote(&t2).map(|v| v.to_string(&t2)).collect();
        assert_eq!(
            unquoted,
            vec![
                "2".to_string(),
                r#"{key: value}"#.to_string(),
                "3".to_string()
            ]
        );
        assert_eq!(a2.get(&t2, 1), Some(1.into()));
        assert_eq!(a2.get(&t2, 2), Some(2.into()));
        assert_eq!(
            a2.get(&t2, 3).map(|v| v.to_string(&t2)),
            Some(r#"{key: value}"#.to_string())
        );
        assert_eq!(a2.get(&t2, 4), Some(3.into()));
        drop(t2);

        a2.insert_range(&mut d2.transact_mut(), 3, ["A", "B"]);

        let t2 = d2.transact();
        let unquoted: Vec<_> = l2.unquote(&t2).map(|v| v.to_string(&t2)).collect();
        assert_eq!(
            unquoted,
            vec![
                "2".to_string(),
                "A".to_string(),
                "B".to_string(),
                r#"{key: value}"#.to_string(),
                "3".to_string()
            ]
        );
        drop(t2);

        exchange_updates(&[&d1, &d2]);

        assert_eq!(
            l1.unquote(&d1.transact()).collect::<Vec<Value>>(),
            vec![
                2.into(),
                "A".into(),
                "B".into(),
                Value::YMap(nested.clone()),
                3.into()
            ]
        );
    }

    #[test]
    fn self_quotation() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        a1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3, 4]);
        let l1 = a1.quote(&d1.transact(), 0, 3).unwrap();
        // link is inserted into its own range
        let l1 = a1.insert(&mut d1.transact_mut(), 1, l1);
        let t1 = d1.transact();
        let unquote: Vec<_> = l1.unquote(&t1).collect();
        assert_eq!(
            unquote,
            vec![1.into(), Value::YWeakLink(l1.clone()), 2.into(), 3.into()]
        );
        assert_eq!(a1.get(&t1, 0), Some(1.into()));
        assert_eq!(a1.get(&t1, 1), Some(Value::YWeakLink(l1.clone())));
        assert_eq!(a1.get(&t1, 2), Some(2.into()));
        assert_eq!(a1.get(&t1, 3), Some(3.into()));
        assert_eq!(a1.get(&t1, 4), Some(4.into()));
        drop(t1);

        exchange_updates(&[&d1, &d2]);

        let t2 = d2.transact();
        let l2 = a2.get(&t2, 1).unwrap().to_weak().unwrap();
        let unquote: Vec<_> = l2.unquote(&t2).collect();
        assert_eq!(
            unquote,
            vec![1.into(), Value::YWeakLink(l2.clone()), 2.into(), 3.into()]
        );
        assert_eq!(a2.get(&t2, 0), Some(1.into()));
        assert_eq!(a2.get(&t2, 1), Some(Value::YWeakLink(l2.clone())));
        assert_eq!(a2.get(&t2, 2), Some(2.into()));
        assert_eq!(a2.get(&t2, 3), Some(3.into()));
        assert_eq!(a2.get(&t2, 4), Some(4.into()));
    }

    #[test]
    fn update() {
        let d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");

        let d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
            m1.insert(&mut txn, "a", nested);
            let link = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link)
        };

        exchange_updates(&[&d1, &d2]);

        let link2 = m2.get(&d2.transact(), "b").unwrap().to_weak().unwrap();
        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(l1.get(&d1.transact(), "a1"), l2.get(&d2.transact(), "a1"));

        m2.insert(&mut d2.transact_mut(), "a2", "world");

        exchange_updates(&[&d1, &d2]);

        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(l1.get(&d1.transact(), "a2"), l2.get(&d2.transact(), "a2"));
    }

    #[test]
    fn delete_weak_link() {
        let d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");

        let d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
            m1.insert(&mut txn, "a", nested);
            let link = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link)
        };

        exchange_updates(&[&d1, &d2]);

        let link2 = m2.get(&d2.transact(), "b").unwrap().to_weak().unwrap();
        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(l1.get(&d1.transact(), "a1"), l2.get(&d2.transact(), "a1"));

        m2.remove(&mut d2.transact_mut(), "b"); // delete links

        exchange_updates(&[&d1, &d2]);

        // since links have been deleted, they no longer refer to any content
        assert_eq!(link1.try_deref_raw(&d1.transact()), None);
        assert_eq!(link2.try_deref_raw(&d2.transact()), None);
    }

    #[test]
    fn delete_source() {
        let d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");

        let d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let link1 = {
            let mut txn = d1.transact_mut();
            let nested = MapPrelim::from([("a1".to_owned(), "hello".to_owned())]);
            m1.insert(&mut txn, "a", nested);
            let link = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link)
        };

        exchange_updates(&[&d1, &d2]);

        let link2 = m2.get(&d2.transact(), "b").unwrap().to_weak().unwrap();
        let l1: MapRef = link1.try_deref(&d1.transact()).unwrap();
        let l2: MapRef = link2.try_deref(&d2.transact()).unwrap();
        assert_eq!(l1.get(&d1.transact(), "a1"), l2.get(&d2.transact(), "a1"));

        m2.remove(&mut d2.transact_mut(), "a"); // delete source of the link

        exchange_updates(&[&d1, &d2]);

        // since links have been deleted, they no longer refer to any content
        assert_eq!(link1.try_deref_raw(&d1.transact()), None);
        assert_eq!(link2.try_deref_raw(&d2.transact()), None);
    }

    #[test]
    fn observe_map_update() {
        let d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");
        let d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let mut link1 = {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", "value");
            let link1 = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link1)
        };

        let target1 = Rc::new(RefCell::new(None));
        let _sub1 = {
            let target = target1.clone();
            link1.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        exchange_updates(&[&d1, &d2]);

        let mut link2 = m2.get(&d2.transact(), "b").unwrap().to_weak().unwrap();
        assert_eq!(link2.try_deref_raw(&d2.transact()), Some("value".into()));

        let target2 = Rc::new(RefCell::new(None));
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        m1.insert(&mut d1.transact_mut(), "a", "value2");
        assert_eq!(link1.try_deref_raw(&d1.transact()), Some("value2".into()));

        exchange_updates(&[&d1, &d2]);
        assert_eq!(link2.try_deref_raw(&d2.transact()), Some("value2".into()));
    }

    #[test]
    fn observe_map_delete() {
        let d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");
        let d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");

        let mut link1 = {
            let mut txn = d1.transact_mut();
            m1.insert(&mut txn, "a", "value");
            let link1 = m1.link(&txn, "a").unwrap();
            m1.insert(&mut txn, "b", link1)
        };

        let target1 = Rc::new(RefCell::new(None));
        let _sub1 = {
            let target = target1.clone();
            link1.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        exchange_updates(&[&d1, &d2]);

        let mut link2 = m2.get(&d2.transact(), "b").unwrap().to_weak().unwrap();
        assert_eq!(link2.try_deref_raw(&d2.transact()), Some("value".into()));

        let target2 = Rc::new(RefCell::new(None));
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        m1.remove(&mut d1.transact_mut(), "a");
        let l1 = (*target1).take().unwrap();
        assert_eq!(l1.try_deref_raw(&d1.transact()), None);

        exchange_updates(&[&d1, &d2]);
        let l2 = (*target2).take().unwrap();
        assert_eq!(l2.try_deref_raw(&d2.transact()), None);
    }

    #[test]
    fn observe_array() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let mut link1 = {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, ["A", "B", "C"]);
            let link1 = a1.quote(&txn, 1, 2).unwrap();
            a1.insert(&mut txn, 0, link1)
        };

        let target1 = Rc::new(RefCell::new(None));
        let _sub1 = {
            let target = target1.clone();
            link1.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        exchange_updates(&[&d1, &d2]);

        let mut link2 = a2.get(&d2.transact(), 0).unwrap().to_weak().unwrap();
        let actual: Vec<_> = link2.unquote(&d2.transact()).collect();
        assert_eq!(actual, vec!["B".into(), "C".into()]);

        let target2 = Rc::new(RefCell::new(None));
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        a1.remove(&mut d1.transact_mut(), 2);
        let actual: Vec<_> = link1.unquote(&d1.transact()).collect();
        assert_eq!(actual, vec!["C".into()]);

        exchange_updates(&[&d1, &d2]);
        let l2 = (*target2).take().unwrap();
        let actual: Vec<_> = l2.unquote(&d2.transact()).collect();
        assert_eq!(actual, vec!["C".into()]);

        a2.remove(&mut d2.transact_mut(), 2);
        let l2 = (*target2).take().unwrap();
        let actual: Vec<_> = l2.unquote(&d2.transact()).collect();
        assert_eq!(actual, vec![]);

        exchange_updates(&[&d1, &d2]);
        let l1 = (*target1).take().unwrap();
        let actual: Vec<_> = l1.unquote(&d1.transact()).collect();
        assert_eq!(actual, vec![]);

        a1.remove(&mut d1.transact_mut(), 1);
        assert_eq!((*target1).take(), None);
    }

    #[test]
    fn deep_observe_transitive() {
        /*
          Structure:
            - map1
              - link-key: <=+-+
            - map2:         | |
              - key: value1-+ |
              - link-link: <--+
        */
        let doc = Doc::new();
        let m1 = doc.get_or_insert_map("map1");
        let m2 = doc.get_or_insert_map("map2");
        let mut txn = doc.transact_mut();

        // test observers in a face of linked chains of values
        m2.insert(&mut txn, "key", "value1");
        let link1 = m2.link(&txn, "key").unwrap();
        let link1 = m1.insert(&mut txn, "link-key", link1);
        let link2 = m1.link(&txn, "link-key").unwrap();
        let mut link2 = m2.insert(&mut txn, "link-link", link2);
        drop(txn);

        let events = Rc::new(RefCell::new(vec![]));
        let _sub1 = {
            let events = events.clone();
            link2.observe_deep(move |txn, evts| {
                let mut er = events.borrow_mut();
                for e in evts.iter() {
                    er.push(e.target());
                }
            })
        };
        m2.insert(&mut doc.transact_mut(), "key", "value2");
        let actual: Vec<_> = events
            .borrow()
            .iter()
            .flat_map(|v| v.clone().to_weak().unwrap().try_deref_raw(&doc.transact()))
            .collect();
        assert_eq!(actual, vec!["value2".into()])
    }

    #[test]
    fn deep_observe_transitive2() {
        /*
          Structure:
            - map1
              - link-key: <=+-+
            - map2:         | |
              - key: value1-+ |
              - link-link: <==+--+
            - map3:              |
              - link-link-link:<-+
        */
        let doc = Doc::new();
        let m1 = doc.get_or_insert_map("map1");
        let m2 = doc.get_or_insert_map("map2");
        let m3 = doc.get_or_insert_map("map3");
        let mut txn = doc.transact_mut();

        // test observers in a face of multi-layer linked chains of values
        m2.insert(&mut txn, "key", "value1");
        let link1 = m2.link(&txn, "key").unwrap();
        let link1 = m1.insert(&mut txn, "link-key", link1);
        let link2 = m1.link(&txn, "link-key").unwrap();
        let link2 = m2.insert(&mut txn, "link-link", link2);
        let link3 = m2.link(&txn, "link-link").unwrap();
        let mut link3 = m3.insert(&mut txn, "link-link-link", link3);
        drop(txn);

        let events = Rc::new(RefCell::new(vec![]));
        let _sub1 = {
            let events = events.clone();
            link3.observe_deep(move |txn, evts| {
                let mut er = events.borrow_mut();
                for e in evts.iter() {
                    er.push(e.target());
                }
            })
        };
        m2.insert(&mut doc.transact_mut(), "key", "value2");
        let actual: Vec<_> = events
            .take()
            .into_iter()
            .flat_map(|v| v.to_weak().unwrap().try_deref_raw(&doc.transact()))
            .collect();
        assert_eq!(actual, vec!["value2".into()])
    }

    #[test]
    fn deep_observe_map() {
        /*
          Structure:
            - map (observed):
              - link:<----+
            - array:      |
               0: nested:-+
                 - key: value
        */
        let doc = Doc::with_client_id(1);
        let mut map = doc.get_or_insert_map("map");
        let array = doc.get_or_insert_array("array");

        let events = Rc::new(RefCell::new(vec![]));
        let _sub = {
            let events = events.clone();
            map.observe_deep(move |txn, e| {
                let mut rs = events.borrow_mut();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => {
                            let value = Value::YMap(e.target().clone());
                            rs.push((value, Some(e.keys(txn).clone())));
                        }
                        Event::Weak(e) => {
                            let value = Value::YWeakLink(e.target().clone());
                            rs.push((value, None));
                        }
                        _ => {}
                    }
                }
            })
        };

        let mut txn = doc.transact_mut();
        let nested = array.insert(&mut txn, 0, MapPrelim::<u32>::new());
        let link = array.quote(&txn, 0, 1).unwrap();
        let link = map.insert(&mut txn, "link", link);
        drop(txn);

        // update entry in linked map
        events.borrow_mut().clear();
        nested.insert(&mut doc.transact_mut(), "key", "value");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                Value::YMap(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );

        // delete entry in linked map
        nested.remove(&mut doc.transact_mut(), "key");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                Value::YMap(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Removed("value".into())
                )]))
            )]
        );

        // delete linked map
        array.remove(&mut doc.transact_mut(), 0);
        let actual = events.take();
        assert_eq!(actual, vec![(Value::YWeakLink(link.clone()), None)]);
    }

    #[test]
    fn deep_observe_array() {
        // test observers in a face of linked chains of values
        /*
          Structure:
            - map:
              - nested: --------+
                - key: value    |
            - array (observed): |
              0: <--------------+
        */
        let doc = Doc::with_client_id(1);
        let map = doc.get_or_insert_map("map");
        let mut array = doc.get_or_insert_array("array");

        let nested = map.insert(
            &mut doc.transact_mut(),
            "nested",
            MapPrelim::<String>::new(),
        );
        let link = map.link(&doc.transact(), "nested").unwrap();
        let link = array.insert(&mut doc.transact_mut(), 0, link);

        let mut events = Rc::new(RefCell::new(vec![]));
        let _sub = {
            let events = events.clone();
            array.observe_deep(move |txn, e| {
                let mut events = events.borrow_mut();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => events
                            .push((Value::YMap(e.target().clone()), Some(e.keys(&txn).clone()))),
                        Event::Weak(e) => events.push((Value::YWeakLink(e.target().clone()), None)),
                        _ => {}
                    }
                }
            })
        };
        nested.insert(&mut doc.transact_mut(), "key", "value");
        assert_eq!(
            events.take(),
            vec![(
                Value::YMap(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );
        // update existing entry
        nested.insert(&mut doc.transact_mut(), "key", "value2");
        assert_eq!(
            events.take(),
            vec![(
                Value::YMap(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Updated("value".into(), "value2".into())
                )]))
            )]
        );

        // delete entry in linked map
        nested.remove(&mut doc.transact_mut(), "key");
        assert_eq!(
            events.take(),
            vec![(
                Value::YMap(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Removed("value2".into())
                )]))
            )]
        );

        // delete linked map
        map.remove(&mut doc.transact_mut(), "nested");
        assert_eq!(events.take(), vec![(Value::YWeakLink(link), None)]);
    }

    #[test]
    fn deep_observe_new_element_within_quoted_range() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");
        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        let (m1, m3) = {
            let mut t1 = d1.transact_mut();
            a1.push_back(&mut t1, 1);
            let m1 = a1.push_back(&mut t1, MapPrelim::<String>::new());
            let m3 = a1.push_back(&mut t1, MapPrelim::<String>::new());
            a1.push_back(&mut t1, 2);
            (m1, m3)
        };
        let mut l1 = {
            let mut t1 = d1.transact_mut();
            let link = a1.quote(&t1, 1, 2).unwrap();
            a1.insert(&mut t1, 0, link)
        };

        exchange_updates(&[&d1, &d2]);

        let mut e1 = Rc::new(RefCell::new(vec![]));
        let _s1 = {
            let events = e1.clone();
            l1.observe_deep(move |txn, e| {
                let mut events = events.borrow_mut();
                events.clear();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => events
                            .push((Value::YMap(e.target().clone()), Some(e.keys(txn).clone()))),
                        Event::Weak(e) => events.push((Value::YWeakLink(e.target().clone()), None)),
                        _ => {}
                    }
                }
            })
        };

        let mut l2 = a2.get(&d2.transact(), 0).unwrap().to_weak().unwrap();
        let mut e2 = Rc::new(RefCell::new(vec![]));
        let _s2 = {
            let events = e2.clone();
            l2.observe_deep(move |txn, e| {
                let mut events = events.borrow_mut();
                events.clear();
                for e in e.iter() {
                    match e {
                        Event::Map(e) => events
                            .push((Value::YMap(e.target().clone()), Some(e.keys(txn).clone()))),
                        Event::Weak(e) => events.push((Value::YWeakLink(e.target().clone()), None)),
                        _ => {}
                    }
                }
            })
        };

        let m20 = a1.insert(&mut d1.transact_mut(), 3, MapPrelim::<String>::new());
        exchange_updates(&[&d1, &d2]);
        m20.insert(&mut d1.transact_mut(), "key", "value");
        assert_eq!(
            e1.take(),
            vec![(
                Value::YMap(m20.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );

        exchange_updates(&[&d1, &d2]);

        let m21 = a2.get(&d2.transact(), 3).unwrap().to_ymap().unwrap();
        assert_eq!(
            e2.take(),
            vec![(
                Value::YMap(m21.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted("value".into())
                )]))
            )]
        );
    }

    #[test]
    fn deep_observe_recursive() {
        // test observers in a face of cycled chains of values
        /*
          Structure:
           array (observed):
             m0:--------+
              - k1:<-+  |
                     |  |
             m1------+  |
              - k2:<-+  |
                     |  |
             m2------+  |
              - k0:<----+
        */
        let doc = Doc::new();
        let root = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();

        let mut m0 = root.insert(&mut txn, 0, MapPrelim::<u32>::new());
        let m1 = root.insert(&mut txn, 1, MapPrelim::<u32>::new());
        let m2 = root.insert(&mut txn, 2, MapPrelim::<u32>::new());

        let l0 = root.quote(&txn, 0, 1).unwrap();
        let l1 = root.quote(&txn, 1, 1).unwrap();
        let l2 = root.quote(&txn, 2, 1).unwrap();

        // create cyclic reference between links
        let l1 = m0.insert(&mut txn, "k1", l1);
        let l2 = m1.insert(&mut txn, "k2", l2);
        let l0 = m2.insert(&mut txn, "k0", l0);
        drop(txn);

        let events = Rc::new(RefCell::new(vec![]));
        let _sub = {
            let events = events.clone();
            m0.observe_deep(move |txn, e| {
                let mut rs = events.borrow_mut();
                for e in e.iter() {
                    if let Event::Map(e) = e {
                        let value = e.target().clone();
                        rs.push((value, e.keys(txn).clone()));
                    }
                }
            })
        };

        m1.insert(&mut doc.transact_mut(), "test-key1", "value1");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                m1.clone(),
                HashMap::from([(
                    Arc::from("test-key1"),
                    EntryChange::Inserted("value1".into())
                )])
            )]
        );

        m2.insert(&mut doc.transact_mut(), "test-key2", "value2");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                m2.clone(),
                HashMap::from([(
                    Arc::from("test-key2"),
                    EntryChange::Inserted("value2".into())
                )])
            )]
        );

        m1.remove(&mut doc.transact_mut(), "test-key1");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                m1.clone(),
                HashMap::from([(
                    Arc::from("test-key1"),
                    EntryChange::Removed("value1".into())
                )])
            )]
        );
    }

    #[test]
    fn remote_map_update() {
        let d1 = Doc::with_client_id(1);
        let m1 = d1.get_or_insert_map("map");
        let d2 = Doc::with_client_id(2);
        let m2 = d2.get_or_insert_map("map");
        let d3 = Doc::with_client_id(3);
        let m3 = d3.get_or_insert_map("map");

        m1.insert(&mut d1.transact_mut(), "key", 1);

        exchange_updates(&[&d1, &d2, &d3]);

        let l2 = m2.link(&d2.transact(), "key").unwrap();
        let l2 = m2.insert(&mut d2.transact_mut(), "link", l2);
        m1.insert(&mut d1.transact_mut(), "key", 2);
        m1.insert(&mut d1.transact_mut(), "key", 3);

        // apply updated content first, link second
        exchange_updates(&[&d3, &d1]);
        exchange_updates(&[&d3, &d2]);

        // make sure that link can find the most recent block
        let l3 = m3.get(&d3.transact(), "link").unwrap().to_weak().unwrap();
        assert_eq!(l3.try_deref_raw(&d3.transact()), Some(3.into()));

        exchange_updates(&[&d1, &d2, &d3]);

        let l1 = m1.get(&d1.transact(), "link").unwrap().to_weak().unwrap();
        let l2 = m2.get(&d2.transact(), "link").unwrap().to_weak().unwrap();

        assert_eq!(l1.try_deref_raw(&d1.transact()), Some(3.into()));
        assert_eq!(l2.try_deref_raw(&d2.transact()), Some(3.into()));
        assert_eq!(l3.try_deref_raw(&d3.transact()), Some(3.into()));
    }

    #[test]
    fn basic_text() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("text");
        let a1 = d1.get_or_insert_array("array");
        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("text");
        let a2 = d2.get_or_insert_array("array");

        txt1.insert(&mut d1.transact_mut(), 0, "abcd"); // 'abcd'
        let l1 = {
            let mut txn = d1.transact_mut();
            let q = txt1.quote(&mut txn, 1, 2); // quote: [bc]
            a1.insert(&mut txn, 0, q.unwrap())
        };
        assert_eq!(l1.get_string(&d1.transact()), "bc".to_string());

        txt1.insert(&mut d1.transact_mut(), 2, "ef"); // 'abefcd', quote: [befc]
        assert_eq!(l1.get_string(&d1.transact()), "befc".to_string());

        txt1.remove_range(&mut d1.transact_mut(), 3, 3); // 'abe', quote: [be]
        assert_eq!(l1.get_string(&d1.transact()), "be".to_string());

        txt1.insert_embed(&mut d1.transact_mut(), 3, WeakPrelim::from(l1.clone())); // 'abe[be]'

        exchange_updates(&[&d1, &d2]);

        let diff = txt2.diff(&d2.transact(), YChange::identity);
        let l2 = diff[1].insert.clone().to_weak().unwrap();
        assert_eq!(l2.get_string(&d2.transact()), "be".to_string());
    }

    #[test]
    fn test_quote_formatted_text() {
        let doc = Doc::with_client_id(1);
        let txt1 = doc.get_or_insert_xml_text("text1");
        let txt2 = doc.get_or_insert_xml_text("text2");
        let array = doc.get_or_insert_array("array");
        txt1.insert(&mut doc.transact_mut(), 0, "abcde");
        let b = Attrs::from([("b".into(), true.into())]);
        let i = Attrs::from([("i".into(), true.into())]);
        txt1.format(&mut doc.transact_mut(), 0, 1, b.clone()); // '<b>a</b>bcde'
        txt1.format(&mut doc.transact_mut(), 1, 3, i.clone()); // '<b>a</b><i>bcd</i>e'
        let l1 = {
            let mut txn = doc.transact_mut();
            let l = txt1.quote(&mut txn, 0, 2).unwrap();
            array.insert(&mut txn, 0, l) // <b>a</b><i>b</i>
        };
        let l2 = {
            let mut txn = doc.transact_mut();
            let l = txt1.quote(&mut txn, 2, 1).unwrap();
            array.insert(&mut txn, 0, l) // <i>c</i>
        };
        let l3 = {
            let mut txn = doc.transact_mut();
            let l = txt1.quote(&mut txn, 3, 2).unwrap();
            array.insert(&mut txn, 0, l) // <i>d</i>e
        };
        assert_eq!(l1.get_string(&doc.transact()), "<b>a</b><i>b</i>");
        assert_eq!(l2.get_string(&doc.transact()), "<i>c</i>");
        assert_eq!(l3.get_string(&doc.transact()), "<i>d</i>e");

        txt2.insert_embed(&mut doc.transact_mut(), 0, WeakPrelim::from(l1.clone()));
        txt2.insert_embed(&mut doc.transact_mut(), 1, WeakPrelim::from(l2.clone()));
        txt2.insert_embed(&mut doc.transact_mut(), 2, WeakPrelim::from(l3.clone()));

        let txn = doc.transact();
        let diff: Vec<_> = txt2
            .diff(&txn, YChange::identity)
            .into_iter()
            .map(|d| d.insert.to_weak().unwrap().get_string(&txn))
            .collect();
        assert_eq!(
            diff,
            vec![
                "<b>a</b><i>b</i>".to_string(),
                "<i>c</i>".to_string(),
                "<i>d</i>e".to_string()
            ]
        );
    }
}
