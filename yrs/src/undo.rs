use crate::transaction::TransactionOrigin;
use crate::types::{Branch, BranchPtr};
use crate::{
    AfterTransactionSubscription, DeleteSet, DestroySubscription, Doc, Observer, Subscription,
    SubscriptionId, TransactionMut,
};
use atomic_refcell::BorrowMutError;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub struct UndoManager {
    scope: HashSet<BranchPtr>,
    options: Options,
    undo_stack: Vec<StackItem>,
    redo_stack: Vec<StackItem>,
    undoing: bool,
    redoing: bool,
    last_change: usize,
    on_after_transaction: AfterTransactionSubscription,
    on_destroy: DestroySubscription,
    observer_added: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
    observer_popped: Observer<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>,
}

impl UndoManager {
    pub fn new<T>(doc: &Doc, scope: &T) -> Self
    where
        T: AsRef<Branch>,
    {
        Self::with_options(doc, scope, Options::default())
    }

    pub fn with_options<T>(doc: &Doc, scope: &T, options: Options) -> Self
    where
        T: AsRef<Branch>,
    {
        let scope = BranchPtr::from(scope.as_ref());
        let on_after_transaction = doc
            .observe_transaction_cleanup(move |txn, e| todo!())
            .unwrap();
        let on_destroy = doc.observe_destroy(move |txn, e| todo!()).unwrap();
        UndoManager {
            scope: HashSet::from([scope]),
            options,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undoing: false,
            redoing: false,
            last_change: 0,
            on_after_transaction,
            on_destroy,
            observer_added: Observer::new(),
            observer_popped: Observer::new(),
        }
    }

    pub fn observe_item_added<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> (),
    {
        self.observer_added.subscribe(Arc::new(f))
    }

    pub fn unobserve_item_added(&self, subscription_id: SubscriptionId) {
        self.observer_added.unsubscribe(subscription_id)
    }

    pub fn observe_item_popped<F>(&self, f: F) -> UndoEventSubscription
    where
        F: Fn(&TransactionMut, &Event) -> (),
    {
        self.observer_popped.subscribe(Arc::new(f))
    }

    pub fn unobserve_item_popped(&self, subscription_id: SubscriptionId) {
        self.observer_popped.unsubscribe(subscription_id)
    }

    pub fn expand_scope<T>(&mut self, scope: &T)
    where
        T: AsRef<Branch>,
    {
        /*
        ytypes = array.isArray(ytypes) ? ytypes : [ytypes]
        ytypes.forEach(ytype => {
          if (this.scope.every(yt => yt !== ytype)) {
            this.scope.push(ytype)
          }
        })
             */
        todo!()
    }

    pub fn include_origin<O>(&mut self, origin: O) {
        self.options.tracked_origins.insert(Rc::new(origin));
    }

    pub fn exclude_origin<O>(&mut self, origin: &O) {
        self.options.tracked_origins.remove(origin);
    }

    pub fn clear(&mut self) -> Result<(), BorrowMutError> {
        todo!()
        /*

        this.doc.transact(transaction => {
          /**
           * @param {StackItem} stackItem
           */
          const clearItem = stackItem => {
            iterateDeletedStructs(transaction, stackItem.deletions, item => {
              if (item instanceof Item && this.scope.some(type => isParentOf(type, item))) {
                keepItem(item, false)
              }
            })
          }
          this.undoStack.forEach(clearItem)
          this.redoStack.forEach(clearItem)
        })
        this.undoStack = []
        this.redoStack = []
             */
    }

    pub fn stop(&mut self) {
        /*
         * UndoManager merges Undo-StackItem if they are created within time-gap
         * smaller than `options.captureTimeout`. Call `um.stopCapturing()` so that the next
         * StackItem won't be merged.
         *
         *
         * @example
         *     // without stopCapturing
         *     ytext.insert(0, 'a')
         *     ytext.insert(1, 'b')
         *     um.undo()
         *     ytext.toString() // => '' (note that 'ab' was removed)
         *     // with stopCapturing
         *     ytext.insert(0, 'a')
         *     um.stopCapturing()
         *     ytext.insert(0, 'b')
         *     um.undo()
         *     ytext.toString() // => 'a' (note that only 'b' was removed)
         *
         */
        self.last_change = 0;
    }

    /// Are there any undo steps available?
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Undo last change on type.
    pub fn undo(&mut self) -> Result<bool, BorrowMutError> {
        self.undoing = true;
        let res = Self::pop(&mut self.undo_stack)?;
        self.undoing = false;
        todo!()
        /*
        this.undoing = true
        let res
        try {
          res = popStackItem(this, this.undoStack, 'undo')
        } finally {
          this.undoing = false
        }
             */
    }

    /// Are there any redo steps available?
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Redo last undo operation.
    pub fn redo(&mut self) -> Result<bool, BorrowMutError> {
        self.redoing = true;
        let res = Self::pop(&mut self.redo_stack)?;
        self.redoing = false;
        todo!()
        /*
            this.redoing = true
        let res
        try {
          res = popStackItem(this, this.redoStack, 'redo')
        } finally {
          this.redoing = false
        }
        return res
             */
    }

    fn pop(stack: &mut Vec<StackItem>) -> Result<(), BorrowMutError> {
        todo!()
    }
}

pub type UndoEventSubscription = Subscription<Arc<dyn Fn(&TransactionMut, &Event) -> ()>>;

#[derive(Clone)]
pub struct Options {
    pub capture_timeout: Duration,
    pub tracked_origins: HashSet<Rc<dyn TransactionOrigin>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            capture_timeout: Duration::from_millis(500),
            tracked_origins: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct StackItem {
    deletions: DeleteSet,
    insertions: DeleteSet,
}

#[derive(Debug, Clone)]
pub struct Event {
    item: StackItem,
    origin: Rc<dyn TransactionOrigin>,
    kind: EventKind,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
pub enum EventKind {
    Undo,
    Redo,
}

#[cfg(test)]
mod test {
    use crate::test_utils::exchange_updates;
    use crate::types::text::{Diff, YChange};
    use crate::types::{Attrs, ToJson};
    use crate::undo::UndoManager;
    use crate::{
        Array, Doc, GetString, Map, MapPrelim, Text, TextPrelim, Transact, XmlElementPrelim,
        XmlFragment, XmlTextPrelim,
    };
    use lib0::any::Any;
    use std::cell::Cell;
    use std::collections::HashMap;

    #[test]
    fn undo_text() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&d1, &txt1);

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");

        // items that are added & deleted in the same transaction won't be undo
        txt1.insert(&mut d1.transact_mut(), 0, "test");
        txt1.remove_range(&mut d1.transact_mut(), 0, 4);
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "");

        // follow redone items
        txt1.insert(&mut d1.transact_mut(), 0, "a");
        mgr.stop();
        txt1.remove_range(&mut d1.transact_mut(), 0, 1);
        mgr.stop();
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "a");
        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "");

        txt1.insert(&mut d1.transact_mut(), 0, "abc");
        txt2.insert(&mut d2.transact_mut(), 0, "xyz");

        exchange_updates(&[&d1, &d2]);

        mgr.undo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "xyz");
        mgr.redo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "abcxyz");

        exchange_updates(&[&d1, &d2]);

        txt2.remove_range(&mut d2.transact_mut(), 0, 1);

        exchange_updates(&[&d1, &d2]);

        assert_eq!(txt1.get_string(&d1.transact()), "xyz");
        mgr.redo().unwrap();
        assert_eq!(txt1.get_string(&d1.transact()), "bcxyz");

        // test marks
        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt1.format(&mut d1.transact_mut(), 1, 3, attrs.clone());
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(
            diff,
            vec![
                Diff::new("b".into(), None),
                Diff::new("cxy".into(), Some(Box::new(attrs.clone()))),
                Diff::new("z".into(), None),
            ]
        );

        mgr.undo().unwrap();
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(diff, vec![Diff::new("bcxyz".into(), None)]);

        mgr.redo().unwrap();
        let diff = txt1.diff(&d1.transact(), YChange::identity);
        assert_eq!(
            diff,
            vec![
                Diff::new("b".into(), None),
                Diff::new("cxy".into(), Some(Box::new(attrs.clone()))),
                Diff::new("z".into(), None),
            ]
        );
    }

    #[test]
    fn double_undo() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        txt.insert(&mut doc.transact_mut(), 0, "1221");

        let mut mgr = UndoManager::new(&doc, &txt);
        txt.insert(&mut doc.transact_mut(), 2, "3");
        txt.insert(&mut doc.transact_mut(), 3, "3");

        mgr.undo().unwrap();
        mgr.undo().unwrap();

        txt.insert(&mut doc.transact_mut(), 2, "3");
        assert_eq!(txt.get_string(&doc.transact()), "12321");
    }

    #[test]
    fn undo_map() {
        let d1 = Doc::with_client_id(1);
        let map1 = d1.get_or_insert_map("test");

        let d2 = Doc::with_client_id(2);
        let map2 = d2.get_or_insert_map("test");

        map1.insert(&mut d1.transact_mut(), "a", 0);
        let mut mgr = UndoManager::new(&d1, &map1);
        map1.insert(&mut d1.transact_mut(), "a", 1);
        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 0.into());
        mgr.redo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 1.into());

        // testing sub-types and if it can restore a whole type
        let sub_type = MapPrelim::from(HashMap::from([("x".to_owned(), 42)]));
        map1.insert(&mut d1.transact_mut(), "a", MapPrelim::<u32>::new());
        let sub_type = map1.get(&d1.transact(), "a").unwrap().to_ymap().unwrap();
        sub_type.insert(&mut d1.transact_mut(), "x", 42);
        let actual = map1.to_json(&d1.transact());
        let expected = Any::from_json(r#"{ "a": { "x": 42 } }"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 1.into());
        mgr.redo().unwrap();
        let actual = map1.to_json(&d1.transact());
        let expected = Any::from_json(r#"{ "a": { "x": 42 } }"#).unwrap();
        assert_eq!(actual, expected);

        exchange_updates(&[&d1, &d2]);

        // if content is overwritten by another user, undo operations should be skipped
        map2.insert(&mut d1.transact_mut(), "a", 44);

        exchange_updates(&[&d1, &d2]);

        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 44.into());
        mgr.redo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "a").unwrap(), 44.into());

        // test setting value multiple times
        map1.insert(&mut d1.transact_mut(), "b", "initial");
        mgr.stop();
        map1.insert(&mut d1.transact_mut(), "b", "val1");
        map1.insert(&mut d1.transact_mut(), "b", "val2");
        mgr.stop();
        mgr.undo().unwrap();
        assert_eq!(map1.get(&d1.transact(), "b").unwrap(), "initial".into());
    }

    #[test]
    fn undo_array() {
        let d1 = Doc::with_client_id(1);
        let array1 = d1.get_or_insert_array("test");

        let d2 = Doc::with_client_id(2);
        let array2 = d2.get_or_insert_array("test");

        let mut mgr = UndoManager::new(&d1, &array1);
        array1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3]);
        array2.insert_range(&mut d2.transact_mut(), 0, [4, 5, 6]);

        exchange_updates(&[&d1, &d2]);

        assert_eq!(
            array1.to_json(&d1.transact()),
            vec![1, 2, 3, 4, 5, 6].into()
        );
        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![4, 5, 6].into());
        mgr.redo().unwrap();
        assert_eq!(
            array1.to_json(&d1.transact()),
            vec![1, 2, 3, 4, 5, 6].into()
        );

        exchange_updates(&[&d1, &d2]);

        array2.remove_range(&mut d2.transact_mut(), 0, 1); // user2 deletes [1]

        exchange_updates(&[&d1, &d2]);

        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![4, 5, 6].into());
        mgr.redo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());
        array1.remove_range(&mut d1.transact_mut(), 0, 5);

        // test nested structure
        array1.insert(&mut d1.transact_mut(), 0, MapPrelim::<u32>::new());
        let map = array1.get(&d1.transact(), 0).unwrap().to_ymap().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);
        mgr.stop();
        map.insert(&mut d1.transact_mut(), "a", 1);
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1}]"#).unwrap();
        assert_eq!(actual, expected);

        exchange_updates(&[&d1, &d2]);

        let map2 = array2.get(&d2.transact(), 0).unwrap().to_ymap().unwrap();
        map.insert(&mut d2.transact_mut(), "b", 2);

        exchange_updates(&[&d1, &d2]);

        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1,"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.undo().unwrap();
        assert_eq!(array1.to_json(&d1.transact()), vec![2, 3, 4, 5, 6].into());

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"b":2}]"#).unwrap();
        assert_eq!(actual, expected);

        mgr.redo().unwrap();
        let actual = array1.to_json(&d1.transact());
        let expected = Any::from_json(r#"[{"a":1,"b":2}]"#).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn undo_xml() {
        let d1 = Doc::with_client_id(1);
        let xml1 = d1.get_or_insert_xml_element("undefined");

        let mut mgr = UndoManager::new(&d1, &xml1);
        let child = xml1.insert(&mut d1.transact_mut(), 0, XmlElementPrelim::empty("p"));
        let text_child = child.insert(&mut d1.transact_mut(), 0, XmlTextPrelim("content"));

        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>content</p></undefined>"
        );
        // format textchild and revert that change
        mgr.stop();
        text_child.format(
            &mut d1.transact_mut(),
            3,
            4,
            Attrs::from([("bold".into(), Any::Map(Box::new(HashMap::new())))]),
        );
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
        mgr.undo().unwrap();
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>content</p></undefined>"
        );
        mgr.redo().unwrap();
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
        xml1.remove_range(&mut d1.transact_mut(), 0, 1);
        assert_eq!(xml1.get_string(&d1.transact()), "<undefined></undefined>");
        mgr.undo().unwrap();
        assert_eq!(
            xml1.get_string(&d1.transact()),
            "<undefined><p>con<bold>tent</bold></p></undefined>"
        );
    }

    #[test]
    fn undo_events() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&doc, &txt);
        let mut counter = Cell::new(0);
        /*

        const { text0 } = init(tc, { users: 3 })
        const undoManager = new Y.UndoManager(text0)
        let counter = 0
        let receivedMetadata = -1
        undoManager.on('stack-item-added', /** @param {any} event */ event => {
          t.assert(event.type != null)
          t.assert(event.changedParentTypes != null && event.changedParentTypes.has(text0))
          event.stackItem.meta.set('test', counter++)
        })
        undoManager.on('stack-item-popped', /** @param {any} event */ event => {
          t.assert(event.type != null)
          t.assert(event.changedParentTypes != null && event.changedParentTypes.has(text0))
          receivedMetadata = event.stackItem.meta.get('test')
        })
        text0.insert(0, 'abc')
        undoManager.undo()
        t.assert(receivedMetadata === 0)
        undoManager.redo()
        t.assert(receivedMetadata === 1)
               */
        todo!()
    }

    #[test]
    fn track_class() {
        /*

        const { users, text0 } = init(tc, { users: 3 })
        // only track origins that are numbers
        const undoManager = new Y.UndoManager(text0, { trackedOrigins: new Set([Number]) })
        users[0].transact(() => {
          text0.insert(0, 'abc')
        }, 42)
        t.assert(text0.toString() === 'abc')
        undoManager.undo()
        t.assert(text0.toString() === '')
               */
        todo!()
    }

    #[test]
    fn type_scope() {
        /*

        const { array0 } = init(tc, { users: 3 })
        // only track origins that are numbers
        const text0 = new Y.Text()
        const text1 = new Y.Text()
        array0.insert(0, [text0, text1])
        const undoManager = new Y.UndoManager(text0)
        const undoManagerBoth = new Y.UndoManager([text0, text1])
        text1.insert(0, 'abc')
        t.assert(undoManager.undoStack.length === 0)
        t.assert(undoManagerBoth.undoStack.length === 1)
        t.assert(text1.toString() === 'abc')
        undoManager.undo()
        t.assert(text1.toString() === 'abc')
        undoManagerBoth.undo()
        t.assert(text1.toString() === '')
               */
        todo!()
    }

    #[test]
    fn undo_in_embed() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut mgr = UndoManager::new(&doc, &txt);

        let nested_text = TextPrelim("initial text");
        mgr.stop();
        let attrs = Attrs::from([("bold".into(), true.into())]);
        txt.insert_embed_with_attributes(&mut doc.transact_mut(), 0, nested_text, attrs);

        /*

        t.assert(nestedText.toString() === 'initial text')
        undoManager.stopCapturing()
        nestedText.delete(0, nestedText.length)
        nestedText.insert(0, 'other text')
        t.assert(nestedText.toString() === 'other text')
        undoManager.undo()
        t.assert(nestedText.toString() === 'initial text')
        undoManager.undo()
        t.assert(text0.length === 0)
               */
        todo!()
    }

    #[test]
    fn undo_delete_filter() {
        /*

        const array0 = /** @type {any} */ (init(tc, { users: 3 }).array0)
        const undoManager = new Y.UndoManager(array0, { deleteFilter: item => !(item instanceof Y.Item) || (item.content instanceof Y.ContentType && item.content.type._map.size === 0) })
        const map0 = new Y.Map()
        map0.set('hi', 1)
        const map1 = new Y.Map()
        array0.insert(0, [map0, map1])
        undoManager.undo()
        t.assert(array0.length === 1)
        array0.get(0)
        t.assert(Array.from(array0.get(0).keys()).length === 1)
               */
        todo!()
    }

    #[test]
    fn undo_until_change_performed() {
        /*

        const doc = new Y.Doc()
        const doc2 = new Y.Doc()
        doc.on('update', update => Y.applyUpdate(doc2, update))
        doc2.on('update', update => Y.applyUpdate(doc, update))

        const yArray = doc.getArray('array')
        const yArray2 = doc2.getArray('array')
        const yMap = new Y.Map()
        yMap.set('hello', 'world')
        yArray.push([yMap])
        const yMap2 = new Y.Map()
        yMap2.set('key', 'value')
        yArray.push([yMap2])

        const undoManager = new Y.UndoManager([yArray], { trackedOrigins: new Set([doc.clientID]) })
        const undoManager2 = new Y.UndoManager([doc2.get('array')], { trackedOrigins: new Set([doc2.clientID]) })

        Y.transact(doc, () => yMap2.set('key', 'value modified'), doc.clientID)
        undoManager.stopCapturing()
        Y.transact(doc, () => yMap.set('hello', 'world modified'), doc.clientID)
        Y.transact(doc2, () => yArray2.delete(0), doc2.clientID)
        undoManager2.undo()
        undoManager.undo()
        t.compareStrings(yMap2.get('key'), 'value')
               */
        todo!()
    }

    #[test]
    fn nested_undo() {
        // This issue has been reported in https://github.com/yjs/yjs/issues/317
        /*

        const doc = new Y.Doc({ gc: false })
        const design = doc.getMap()
        const undoManager = new Y.UndoManager(design, { captureTimeout: 0 })

        /**
         * @type {Y.Map<any>}
         */
        const text = new Y.Map()

        const blocks1 = new Y.Array()
        const blocks1block = new Y.Map()

        doc.transact(() => {
          blocks1block.set('text', 'Type Something')
          blocks1.push([blocks1block])
          text.set('blocks', blocks1block)
          design.set('text', text)
        })

        const blocks2 = new Y.Array()
        const blocks2block = new Y.Map()
        doc.transact(() => {
          blocks2block.set('text', 'Something')
          blocks2.push([blocks2block])
          text.set('blocks', blocks2block)
        })

        const blocks3 = new Y.Array()
        const blocks3block = new Y.Map()
        doc.transact(() => {
          blocks3block.set('text', 'Something Else')
          blocks3.push([blocks3block])
          text.set('blocks', blocks3block)
        })

        t.compare(design.toJSON(), { text: { blocks: { text: 'Something Else' } } })
        undoManager.undo()
        t.compare(design.toJSON(), { text: { blocks: { text: 'Something' } } })
        undoManager.undo()
        t.compare(design.toJSON(), { text: { blocks: { text: 'Type Something' } } })
        undoManager.undo()
        t.compare(design.toJSON(), { })
        undoManager.redo()
        t.compare(design.toJSON(), { text: { blocks: { text: 'Type Something' } } })
        undoManager.redo()
        t.compare(design.toJSON(), { text: { blocks: { text: 'Something' } } })
        undoManager.redo()
        t.compare(design.toJSON(), { text: { blocks: { text: 'Something Else' } } })
               */
        todo!()
    }

    fn consecutive_redo_bug() {
        // https://github.com/yjs/yjs/issues/355
        todo!()
        /*

        const doc = new Y.Doc()
        const yRoot = doc.getMap()
        const undoMgr = new Y.UndoManager(yRoot)

        let yPoint = new Y.Map()
        yPoint.set('x', 0)
        yPoint.set('y', 0)
        yRoot.set('a', yPoint)
        undoMgr.stopCapturing()

        yPoint.set('x', 100)
        yPoint.set('y', 100)
        undoMgr.stopCapturing()

        yPoint.set('x', 200)
        yPoint.set('y', 200)
        undoMgr.stopCapturing()

        yPoint.set('x', 300)
        yPoint.set('y', 300)
        undoMgr.stopCapturing()

        t.compare(yPoint.toJSON(), { x: 300, y: 300 })

        undoMgr.undo() // x=200, y=200
        t.compare(yPoint.toJSON(), { x: 200, y: 200 })
        undoMgr.undo() // x=100, y=100
        t.compare(yPoint.toJSON(), { x: 100, y: 100 })
        undoMgr.undo() // x=0, y=0
        t.compare(yPoint.toJSON(), { x: 0, y: 0 })
        undoMgr.undo() // nil
        t.compare(yRoot.get('a'), undefined)

        undoMgr.redo() // x=0, y=0
        yPoint = yRoot.get('a')

        t.compare(yPoint.toJSON(), { x: 0, y: 0 })
        undoMgr.redo() // x=100, y=100
        t.compare(yPoint.toJSON(), { x: 100, y: 100 })
        undoMgr.redo() // x=200, y=200
        t.compare(yPoint.toJSON(), { x: 200, y: 200 })
        undoMgr.redo() // expected x=300, y=300, actually nil
        t.compare(yPoint.toJSON(), { x: 300, y: 300 })
               */
    }

    #[test]
    fn undo_xml_bug() {
        // https://github.com/yjs/yjs/issues/304
        todo!()
        /*
        const origin = 'origin'
        const doc = new Y.Doc()
        const fragment = doc.getXmlFragment('t')
        const undoManager = new Y.UndoManager(fragment, {
          captureTimeout: 0,
          trackedOrigins: new Set([origin])
        })

        // create element
        doc.transact(() => {
          const e = new Y.XmlElement('test-node')
          e.setAttribute('a', '100')
          e.setAttribute('b', '0')
          fragment.insert(fragment.length, [e])
        }, origin)

        // change one attribute
        doc.transact(() => {
          const e = fragment.get(0)
          e.setAttribute('a', '200')
        }, origin)

        // change both attributes
        doc.transact(() => {
          const e = fragment.get(0)
          e.setAttribute('a', '180')
          e.setAttribute('b', '50')
        }, origin)

        undoManager.undo()
        undoManager.undo()
        undoManager.undo()

        undoManager.redo()
        undoManager.redo()
        undoManager.redo()
        t.compare(fragment.toString(), '<test-node a="180" b="50"></test-node>')
               */
    }

    #[test]
    fn undo_block_bug() {
        // https://github.com/yjs/yjs/issues/343
        todo!()
        /*

        const doc = new Y.Doc({ gc: false })
        const design = doc.getMap()

        const undoManager = new Y.UndoManager(design, { captureTimeout: 0 })

        const text = new Y.Map()

        const blocks1 = new Y.Array()
        const blocks1block = new Y.Map()
        doc.transact(() => {
          blocks1block.set('text', '1')
          blocks1.push([blocks1block])

          text.set('blocks', blocks1block)
          design.set('text', text)
        })

        const blocks2 = new Y.Array()
        const blocks2block = new Y.Map()
        doc.transact(() => {
          blocks2block.set('text', '2')
          blocks2.push([blocks2block])
          text.set('blocks', blocks2block)
        })

        const blocks3 = new Y.Array()
        const blocks3block = new Y.Map()
        doc.transact(() => {
          blocks3block.set('text', '3')
          blocks3.push([blocks3block])
          text.set('blocks', blocks3block)
        })

        const blocks4 = new Y.Array()
        const blocks4block = new Y.Map()
        doc.transact(() => {
          blocks4block.set('text', '4')
          blocks4.push([blocks4block])
          text.set('blocks', blocks4block)
        })

        // {"text":{"blocks":{"text":"4"}}}
        undoManager.undo() // {"text":{"blocks":{"3"}}}
        undoManager.undo() // {"text":{"blocks":{"text":"2"}}}
        undoManager.undo() // {"text":{"blocks":{"text":"1"}}}
        undoManager.undo() // {}
        undoManager.redo() // {"text":{"blocks":{"text":"1"}}}
        undoManager.redo() // {"text":{"blocks":{"text":"2"}}}
        undoManager.redo() // {"text":{"blocks":{"text":"3"}}}
        undoManager.redo() // {"text":{}}
        t.compare(design.toJSON(), { text: { blocks: { text: '4' } } })
               */
    }

    #[test]
    fn undo_delete_text_format() {
        // https://github.com/yjs/yjs/issues/392
        todo!()
        /*

        const doc = new Y.Doc()
        const text = doc.getText()
        text.insert(0, 'Attack ships on fire off the shoulder of Orion.')
        const doc2 = new Y.Doc()
        const text2 = doc2.getText()
        Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc))
        const undoManager = new Y.UndoManager(text)

        text.format(13, 7, { bold: true })
        undoManager.stopCapturing()
        Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc))

        text.format(16, 4, { bold: null })
        undoManager.stopCapturing()
        Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc))

        undoManager.undo()
        Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc))

        const expect = [
          { insert: 'Attack ships ' },
          {
            insert: 'on fire',
            attributes: { bold: true }
          },
          { insert: ' off the shoulder of Orion.' }
        ]
        t.compare(text.toDelta(), expect)
        t.compare(text2.toDelta(), expect)
               */
    }

    #[test]
    fn special_deletion_case() {
        // https://github.com/yjs/yjs/issues/447
        todo!()
        /*

        const origin = 'undoable'
        const doc = new Y.Doc()
        const fragment = doc.getXmlFragment()
        const undoManager = new Y.UndoManager(fragment, { trackedOrigins: new Set([origin]) })
        doc.transact(() => {
          const e = new Y.XmlElement('test')
          e.setAttribute('a', '1')
          e.setAttribute('b', '2')
          fragment.insert(0, [e])
        })
        t.compareStrings(fragment.toString(), '<test a="1" b="2"></test>')
        doc.transact(() => {
          // change attribute "b" and delete test-node
          const e = fragment.get(0)
          e.setAttribute('b', '3')
          fragment.delete(0)
        }, origin)
        t.compareStrings(fragment.toString(), '')
        undoManager.undo()
        t.compareStrings(fragment.toString(), '<test a="1" b="2"></test>')
               */
    }
}
