use crate::atomic::AtomicRef;
use crate::block::{Block, BlockPtr, BlockSlice, ItemContent, Prelim};
use crate::types::{Branch, BranchPtr, EventHandler, Observers, TypeRef, Value};
use crate::{Observable, TransactionMut, ID};
use std::convert::{TryFrom, TryInto};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

#[repr(transparent)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WeakRef(BranchPtr);

impl WeakRef {
    pub fn try_deref_raw(&self) -> Option<Value> {
        if let TypeRef::WeakLink(source) = &self.0.type_ref {
            source.deref_raw()
        } else {
            None
        }
    }

    pub fn try_deref<T>(&self) -> Option<T>
    where
        T: TryFrom<Value>,
    {
        let value = self.try_deref_raw()?;
        value.try_into().ok()
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

/// A preliminary type for [WeakRef]. Once inserted into document it can be used as a weak reference
/// link to another value living inside of the document store.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WeakPrelim(Arc<LinkSource>);

impl WeakPrelim {
    pub fn new(id: ID) -> Self {
        WeakPrelim(Arc::new(LinkSource::new(id)))
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

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        if let TypeRef::WeakLink(source) = &inner_ref.type_ref {
            source.materialize(txn, inner_ref)
        }
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
}

#[derive(Debug, Eq, PartialEq)]
pub struct LinkSource {
    id: ID,
    pub(crate) linked_item: AtomicRef<BlockPtr>,
}

impl LinkSource {
    pub fn new(id: ID) -> Self {
        LinkSource {
            id,
            linked_item: AtomicRef::default(),
        }
    }

    pub fn id(&self) -> &ID {
        &self.id
    }

    pub(crate) fn deref_raw(&self) -> Option<Value> {
        let mut block_ref = *self.linked_item.get()?;
        if Self::try_right_most(&mut block_ref) {
            self.linked_item.swap(block_ref); // update to latest block
        }

        if let Block::Item(item) = block_ref.deref() {
            if !item.is_deleted() {
                return item.content.get_first();
            }
        }
        None
    }

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
        if self.linked_item.get().is_none() {
            let source_clock = self.id().clock;
            if let Some(mut ptr) = txn.store.blocks.get_block(self.id()) {
                if let Block::Item(item) = ptr.deref() {
                    if item.parent_sub.is_some() {
                        // for map types go to the right-most item
                        while let Some(right) = ptr.as_item().and_then(|i| i.right) {
                            ptr = right;
                        }
                    } else {
                        // for list types make sure that source block is not concatenation
                        // of ranges outside of the linked item
                        if ptr.len() > 1 {
                            let offset = source_clock - ptr.id().clock;
                            let slice = BlockSlice::new(ptr, offset, offset);
                            ptr = txn.store.materialize(slice);
                        }
                    }
                    self.linked_item.swap(ptr);
                }
            }
        }
        if let Some(ptr) = self.linked_item.get().as_deref() {
            let mut ptr = *ptr;
            if let Block::Item(item) = ptr.deref_mut() {
                let linked_by = item.linked_by.get_or_insert_with(Default::default);
                linked_by.insert(inner_ref.clone());
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::exchange_updates;
    use crate::types::{EntryChange, Event, ToJson, Value};
    use crate::{Array, DeepObservable, Doc, Map, MapPrelim, MapRef, Observable, Transact};
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
        let deref = link.try_deref::<MapRef>().unwrap();
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
            let link = a1.link(&txn, 1).unwrap();
            a1.insert(&mut txn, 3, link);

            assert_eq!(a1.get(&txn, 0), Some(1.into()));
            assert_eq!(a1.get(&txn, 1), Some(2.into()));
            assert_eq!(a1.get(&txn, 2), Some(3.into()));
            assert_eq!(
                a1.get(&txn, 3)
                    .unwrap()
                    .to_weak()
                    .unwrap()
                    .try_deref::<Any>(),
                Some(2.into())
            );
        }

        let d2 = Doc::new();
        let a2 = d2.get_or_insert_array("array");

        exchange_updates(&[&d1, &d2]);
        let mut txn = d2.transact_mut();

        assert_eq!(a2.get(&txn, 0), Some(1.into()));
        assert_eq!(a2.get(&txn, 1), Some(2.into()));
        assert_eq!(a2.get(&txn, 2), Some(3.into()));
        assert_eq!(
            a2.get(&txn, 3)
                .unwrap()
                .to_weak()
                .unwrap()
                .try_deref::<Any>(),
            Some(2.into())
        );
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
        let l1: MapRef = link1.try_deref().unwrap();
        let l2: MapRef = link2.try_deref().unwrap();
        assert_eq!(l1.get(&d1.transact(), "a1"), l2.get(&d2.transact(), "a1"));

        m2.insert(&mut d2.transact_mut(), "a2", "world");

        exchange_updates(&[&d1, &d2]);

        let l1: MapRef = link1.try_deref().unwrap();
        let l2: MapRef = link2.try_deref().unwrap();
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
        let l1: MapRef = link1.try_deref().unwrap();
        let l2: MapRef = link2.try_deref().unwrap();
        assert_eq!(l1.get(&d1.transact(), "a1"), l2.get(&d2.transact(), "a1"));

        m2.remove(&mut d2.transact_mut(), "b"); // delete links

        exchange_updates(&[&d1, &d2]);

        // since links have been deleted, they no longer refer to any content
        assert_eq!(link1.try_deref_raw(), None);
        assert_eq!(link2.try_deref_raw(), None);
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
        let l1: MapRef = link1.try_deref().unwrap();
        let l2: MapRef = link2.try_deref().unwrap();
        assert_eq!(l1.get(&d1.transact(), "a1"), l2.get(&d2.transact(), "a1"));

        m2.remove(&mut d2.transact_mut(), "a"); // delete source of the link

        exchange_updates(&[&d1, &d2]);

        // since links have been deleted, they no longer refer to any content
        assert_eq!(link1.try_deref_raw(), None);
        assert_eq!(link2.try_deref_raw(), None);
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
        assert_eq!(link2.try_deref_raw(), Some("value".into()));

        let target2 = Rc::new(RefCell::new(None));
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        m1.insert(&mut d1.transact_mut(), "a", "value2");
        assert_eq!(link1.try_deref_raw(), Some("value2".into()));

        exchange_updates(&[&d1, &d2]);
        assert_eq!(link2.try_deref_raw(), Some("value2".into()));
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
        assert_eq!(link2.try_deref_raw(), Some("value".into()));

        let target2 = Rc::new(RefCell::new(None));
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        m1.remove(&mut d1.transact_mut(), "a");
        assert_eq!(link1.try_deref_raw(), Some("value2".into()));

        exchange_updates(&[&d1, &d2]);
        assert_eq!(link2.try_deref_raw(), Some("value2".into()));
    }

    #[test]
    fn observe_array() {
        let d1 = Doc::new();
        let a1 = d1.get_or_insert_array("array");
        let d2 = Doc::new();
        let a2 = d2.get_or_insert_array("array");

        let mut link1 = {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, ["A", "B", "C"]);
            let link1 = a1.link(&txn, 1).unwrap();
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
        assert_eq!(link2.try_deref_raw(), Some("B".into()));

        let target2 = Rc::new(RefCell::new(None));
        let _sub2 = {
            let target = target2.clone();
            link2.observe(move |txn, e| {
                target.replace(Some(e.target.clone()));
            })
        };

        a1.remove(&mut d1.transact_mut(), 2);
        assert_eq!(link1.try_deref_raw(), None);

        exchange_updates(&[&d1, &d2]);
        assert_eq!(link2.try_deref_raw(), None);
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
            .flat_map(|v| v.to_weak().unwrap().try_deref_raw())
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
            .borrow()
            .iter()
            .flat_map(|v| v.to_weak().unwrap().try_deref_raw())
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
        let doc = Doc::new();
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
                            let value = Value::from(e.target().clone());
                            rs.push((value, Some(e.keys(txn).clone())));
                        }
                        Event::Weak(e) => {
                            let value = Value::from(e.target().clone());
                            rs.push((value, None));
                        }
                        _ => {}
                    }
                }
            })
        };

        let mut txn = doc.transact_mut();
        let nested = array.insert(&mut txn, 0, MapPrelim::new());
        let link = array.link(&txn, 0).unwrap();
        let link = map.insert(&mut txn, "link", link);
        drop(txn);

        // update entry in linked map
        events.borrow_mut().clear();
        nested.insert(&mut doc.transact_mut(), "key", "value");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                Value::from(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Inserted(Any::Undefined.into())
                )]))
            )]
        );

        // delete entry in linked map
        nested.remove(&mut doc.transact_mut(), "key");
        let actual = events.take();
        assert_eq!(
            actual,
            vec![(
                Value::from(nested.clone()),
                Some(HashMap::from([(
                    Arc::from("key"),
                    EntryChange::Removed(Any::Undefined.into())
                )]))
            )]
        );

        // delete linked map
        array.remove(&mut doc.transact_mut(), 0);
        assert_eq!(actual, vec![(Value::from(link.clone()), None)]);
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

        let mut m0 = root.insert(&mut txn, 0, MapPrelim::new());
        let m1 = root.insert(&mut txn, 1, MapPrelim::new());
        let m2 = root.insert(&mut txn, 2, MapPrelim::new());

        let l0 = root.link(&txn, 0).unwrap();
        let l1 = root.link(&txn, 1).unwrap();
        let l2 = root.link(&txn, 2).unwrap();

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
                    EntryChange::Inserted(Any::Undefined.into())
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
                    EntryChange::Inserted(Any::Undefined.into())
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
                    EntryChange::Removed(Any::Undefined.into())
                )])
            )]
        );
    }

    #[test]
    fn remote_map_update() {
        let d1 = Doc::new();
        let m1 = d1.get_or_insert_map("map");
        let d2 = Doc::new();
        let m2 = d2.get_or_insert_map("map");
        let d3 = Doc::new();
        let m3 = d3.get_or_insert_map("map");

        m1.insert(&mut d1.transact_mut(), "key", 1);

        exchange_updates(&[&d1, &d2, &d3]);

        let l2 = m2.link("key").unwrap();
        let l2 = m2.insert(&mut d2.transact_mut(), "link", l2);
        m1.insert(&mut d1.transact_mut(), "key", 2);
        m1.insert(&mut d1.transact_mut(), "key", 3);

        // apply updated content first, link second
        exchange_updates(&[&d3, &d1]);
        exchange_updates(&[&d3, &d2]);

        // make sure that link can find the most recent block
        let l3 = m3.get("link").unwrap().to_weak().unwrap();
        assert_eq!(l3.try_deref_raw(), Some(3.into()));

        exchange_updates(&[&d1, &d2, &d3]);

        let l1 = m1.get("link").unwrap().to_weak().unwrap();
        let l2 = m2.get("link").unwrap().to_weak().unwrap();

        assert_eq!(l1.try_deref_raw(), Some(3.into()));
        assert_eq!(l2.try_deref_raw(), Some(3.into()));
        assert_eq!(l3.try_deref_raw(), Some(3.into()));
    }
}
