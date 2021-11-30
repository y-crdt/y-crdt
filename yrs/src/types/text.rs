use crate::block::{BlockPtr, ItemContent};
use crate::transaction::Transaction;
use crate::types::{Branch, BranchRef, Event, Observer};
use crate::*;
use std::cell::Ref;

/// A shared data type used for collaborative text editing. It enables multiple users to add and
/// remove chunks of text in efficient manner. This type is internally represented as a mutable
/// double-linked list of text chunks - an optimization occurs during [Transaction::commit], which
/// allows to squash multiple consecutively inserted characters together as a single chunk of text
/// even between transaction boundaries in order to preserve more efficient memory model.
///
/// [Text] structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, [Text] is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Text(BranchRef);

impl Text {
    /// Converts context of this text data structure into a single string value.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self, txn: &Transaction) -> String {
        let inner = self.0.as_ref();
        let mut start = inner.start;
        let mut s = String::new();
        let store = txn.store();
        while let Some(a) = start.as_ref() {
            if let Some(item) = store.blocks.get_item(&a) {
                if !item.is_deleted() {
                    if let block::ItemContent::String(item_string) = &item.content {
                        s.push_str(item_string);
                    }
                }
                start = item.right.clone();
            } else {
                break;
            }
        }
        s
    }

    /// Returns a number of characters visible in a current text data structure.
    pub fn len(&self) -> u32 {
        self.0.borrow().len()
    }

    pub(crate) fn inner(&self) -> Ref<Branch> {
        self.0.borrow()
    }

    pub(crate) fn find_position(
        &self,
        txn: &mut Transaction,
        mut count: u32,
    ) -> Option<block::ItemPosition> {
        let mut pos = {
            let inner = self.0.borrow();
            block::ItemPosition {
                parent: inner.ptr.clone(),
                left: None,
                right: inner.start,
                index: 0,
            }
        };

        let store = txn.store_mut();
        while let Some(right_ptr) = pos.right.as_ref() {
            if count == 0 {
                break;
            }

            if let Some(mut right) = store.blocks.get_item(right_ptr) {
                if !right.is_deleted() {
                    let mut right_len = right.len();
                    if count < right_len {
                        // split right item
                        let split_ptr = BlockPtr::new(
                            ID::new(right.id.client, right.id.clock + count),
                            right_ptr.pivot() as u32,
                        );
                        let (_, _) = store.blocks.split_block(&split_ptr);
                        right = store.blocks.get_item(right_ptr).unwrap();
                        right_len = right.len();
                    }
                    pos.index += right_len;
                    count -= right_len;
                }
                pos.left = pos.right.take();
                pos.right = right.right.clone();
            } else {
                return None;
            }
        }

        Some(pos)
    }

    /// Inserts a `chunk` of text at a given `index`.
    /// If `index` is `0`, this `chunk` will be inserted at the beginning of a current text.
    /// If `index` is equal to current data structure length, this `chunk` will be appended at
    /// the end of it.
    /// This method will panic if provided `index` is greater than the length of a current text.
    pub fn insert(&self, tr: &mut Transaction, index: u32, chunk: &str) {
        if let Some(pos) = self.find_position(tr, index) {
            let value = crate::block::PrelimText(chunk.to_owned());
            tr.create_item(&pos, value, None);
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    /// Appends a given `chunk` of text at the end of a current text structure.
    pub fn push(&self, txn: &mut Transaction, chunk: &str) {
        let idx = self.len();
        self.insert(txn, idx, chunk)
    }

    /// Removes up to a `len` characters from a current text structure, starting at given `index`.
    /// This method panics in case when not all expected characters were removed (due to
    /// insufficient number of characters to remove) or `index` is outside of the bounds of text.
    pub fn remove_range(&self, txn: &mut Transaction, index: u32, len: u32) {
        let removed = self.0.remove_at(txn, index, len);
        if removed != len {
            panic!("Couldn't remove {} elements from an array. Only {} of them were successfully removed.", len, removed);
        }
    }

    /// Subscribes a given callback to be triggered whenever current text is changed.
    /// A callback is triggered whenever a transaction gets committed. This function does not
    /// trigger if changes have been observed by nested shared collections.
    ///
    /// All text changes can be tracked by using [Event::delta] method: keep in mind that delta
    /// contains collection of individual characters rather than strings.
    ///
    /// Returns an [Observer] which, when dropped, will unsubscribe current callback.
    pub fn observe<F>(&self, f: F) -> Observer
    where
        F: Fn(&Transaction, &Event) -> () + 'static,
    {
        let mut branch = self.0.borrow_mut();
        branch.observe(f)
    }
}

impl Into<ItemContent> for Text {
    fn into(self) -> ItemContent {
        ItemContent::Type(self.0.clone())
    }
}

impl From<BranchRef> for Text {
    fn from(inner: BranchRef) -> Self {
        Text(inner)
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::{run_scenario, RngExt};
    use crate::types::Change;
    use crate::updates::decoder::Decode;
    use crate::updates::encoder::{Encoder, EncoderV1};
    use crate::{Doc, Update};
    use lib0::any::Any;
    use rand::prelude::StdRng;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn append_single_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        assert_eq!(txt.to_string(&txn).as_str(), "abc");
    }

    #[test]
    fn append_mutli_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");

        assert_eq!(txt.to_string(&txn).as_str(), "hello world");
    }

    #[test]
    fn prepend_single_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 0, "b");
        txt.insert(&mut txn, 0, "c");

        assert_eq!(txt.to_string(&txn).as_str(), "cba");
    }

    #[test]
    fn prepend_mutli_character_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 0, " ");
        txt.insert(&mut txn, 0, "world");

        assert_eq!(txt.to_string(&txn).as_str(), "world hello");
    }

    #[test]
    fn insert_after_block() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");
        txt.insert(&mut txn, 6, "beautiful ");

        assert_eq!(txt.to_string(&txn).as_str(), "hello beautiful world");
    }

    #[test]
    fn insert_inside_of_block() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "it was expected");
        txt.insert(&mut txn, 6, " not");

        assert_eq!(txt.to_string(&txn).as_str(), "it was not expected");
    }

    #[test]
    fn insert_concurrent_root() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "hello ");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();
        let txt2 = t2.get_text("test");

        txt2.insert(&mut t2, 0, "world");

        let d1_sv = d1.get_state_vector(&t1);
        let d2_sv = d2.get_state_vector(&t2);

        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        let u2 = d2.encode_delta_as_update_v1(&t2, &d1_sv);

        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "hello world");
    }

    #[test]
    fn insert_concurrent_in_the_middle() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "I expect that");
        assert_eq!(txt1.to_string(&t1).as_str(), "I expect that");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();

        let d2_sv = d2.get_state_vector(&t2);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let txt2 = t2.get_text("test");
        assert_eq!(txt2.to_string(&t2).as_str(), "I expect that");

        txt2.insert(&mut t2, 1, " have");
        txt2.insert(&mut t2, 13, "ed");
        assert_eq!(txt2.to_string(&t2).as_str(), "I have expected that");

        txt1.insert(&mut t1, 1, " didn't");
        assert_eq!(txt1.to_string(&t1).as_str(), "I didn't expect that");

        let d2_sv = d2.get_state_vector(&t2);
        let d1_sv = d1.get_state_vector(&t1);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        let u2 = d2.encode_delta_as_update_v1(&t2, &d1_sv);
        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a.as_str(), "I didn't have expected that");
    }

    #[test]
    fn append_concurrent() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "aaa");
        assert_eq!(txt1.to_string(&t1).as_str(), "aaa");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();

        let d2_sv = d2.get_state_vector(&t2);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let txt2 = t2.get_text("test");
        assert_eq!(txt2.to_string(&t2).as_str(), "aaa");

        txt2.insert(&mut t2, 3, "bbb");
        txt2.insert(&mut t2, 6, "bbb");
        assert_eq!(txt2.to_string(&t2).as_str(), "aaabbbbbb");

        txt1.insert(&mut t1, 3, "aaa");
        assert_eq!(txt1.to_string(&t1).as_str(), "aaaaaa");

        let d2_sv = d2.get_state_vector(&t2);
        let d1_sv = d1.get_state_vector(&t1);
        let u1 = d1.encode_delta_as_update_v1(&t1, &d2_sv);
        let u2 = d2.encode_delta_as_update_v1(&t2, &d1_sv);

        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a.as_str(), "aaaaaabbbbbb");
        assert_eq!(a, b);
    }

    #[test]
    fn delete_single_block_start() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "bbb");
        txt.insert(&mut txn, 0, "aaa");
        txt.remove_range(&mut txn, 0, 3);

        assert_eq!(txt.len(), 3);
        assert_eq!(txt.to_string(&txn).as_str(), "bbb");
    }

    #[test]
    fn delete_single_block_end() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "bbb");
        txt.insert(&mut txn, 0, "aaa");
        txt.remove_range(&mut txn, 3, 3);

        assert_eq!(txt.to_string(&txn).as_str(), "aaa");
    }

    #[test]
    fn delete_multiple_whole_blocks() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        txt.remove_range(&mut txn, 1, 1);
        assert_eq!(txt.to_string(&txn).as_str(), "ac");

        txt.remove_range(&mut txn, 1, 1);
        assert_eq!(txt.to_string(&txn).as_str(), "a");

        txt.remove_range(&mut txn, 0, 1);
        assert_eq!(txt.to_string(&txn).as_str(), "");
    }

    #[test]
    fn delete_slice_of_block() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "abc");
        txt.remove_range(&mut txn, 1, 1);

        assert_eq!(txt.to_string(&txn).as_str(), "ac");
    }

    #[test]
    fn delete_multiple_blocks_with_slicing() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello ");
        txt.insert(&mut txn, 6, "beautiful");
        txt.insert(&mut txn, 15, " world");

        txt.remove_range(&mut txn, 5, 11);
        assert_eq!(txt.to_string(&txn).as_str(), "helloworld");
    }

    #[test]
    fn insert_after_delete() {
        let doc = Doc::new();
        let mut txn = doc.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello ");
        txt.remove_range(&mut txn, 0, 5);
        txt.insert(&mut txn, 1, "world");

        assert_eq!(txt.to_string(&txn).as_str(), " world");
    }

    #[test]
    fn concurrent_insert_delete() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        txt1.insert(&mut t1, 0, "hello world");
        assert_eq!(txt1.to_string(&t1).as_str(), "hello world");

        let u1 = d1.encode_state_as_update_v1(&t1);

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();
        d2.apply_update_v1(&mut t2, u1.as_slice());
        let txt2 = t2.get_text("test");
        assert_eq!(txt2.to_string(&t2).as_str(), "hello world");

        txt1.insert(&mut t1, 5, " beautiful");
        txt1.insert(&mut t1, 21, "!");
        txt1.remove_range(&mut t1, 0, 5);
        assert_eq!(txt1.to_string(&t1).as_str(), " beautiful world!");

        txt2.remove_range(&mut t2, 5, 5);
        txt2.remove_range(&mut t2, 0, 1);
        txt2.insert(&mut t2, 0, "H");
        assert_eq!(txt2.to_string(&t2).as_str(), "Hellod");

        let sv1 = d1.get_state_vector(&t1);
        let sv2 = d2.get_state_vector(&t2);
        let u1 = d1.encode_delta_as_update_v1(&t1, &sv2);
        let u2 = d2.encode_delta_as_update_v1(&t2, &sv1);

        d1.apply_update_v1(&mut t1, u2.as_slice());
        d2.apply_update_v1(&mut t2, u1.as_slice());

        let a = txt1.to_string(&t1);
        let b = txt2.to_string(&t2);

        assert_eq!(a, b);
        assert_eq!(a, "H beautifuld!".to_owned());
    }

    #[test]
    fn insert_and_remove_event_changes() {
        let d1 = Doc::with_client_id(1);
        let txt = {
            let mut txn = d1.transact();
            txn.get_text("text")
        };
        let mut delta = Rc::new(RefCell::new(None));
        let delta_c = delta.clone();
        let _sub = txt.observe(move |txn, e| {
            delta_c.borrow_mut().insert(e.delta(txn).to_vec());
        });

        // insert initial string
        {
            let mut txn = d1.transact();
            txt.insert(&mut txn, 0, "abcd");
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Change::Added(vec![
                Any::String("a".to_string()).into(),
                Any::String("b".to_string()).into(),
                Any::String("c".to_string()).into(),
                Any::String("d".to_string()).into()
            ])])
        );

        // remove middle
        {
            let mut txn = d1.transact();
            txt.remove_range(&mut txn, 1, 2);
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Change::Retain(1), Change::Removed(2)])
        );

        // insert again
        {
            let mut txn = d1.transact();
            txt.insert(&mut txn, 1, "ef");
        }
        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![
                Change::Retain(1),
                Change::Added(vec![
                    Any::String("e".to_string()).into(),
                    Any::String("f".to_string()).into()
                ])
            ])
        );

        // replicate data to another peer
        let d2 = Doc::with_client_id(2);
        let txt = {
            let mut txn = d2.transact();
            txn.get_text("text")
        };
        let delta_c = delta.clone();
        let _sub = txt.observe(move |txn, e| {
            delta_c.borrow_mut().insert(e.delta(txn).to_vec());
        });

        {
            let t1 = d1.transact();
            let mut t2 = d2.transact();

            let sv = t2.state_vector();
            let mut encoder = EncoderV1::new();
            t1.encode_diff(&sv, &mut encoder);
            t2.apply_update(Update::decode_v1(encoder.to_vec().as_slice()));
        }

        assert_eq!(
            delta.borrow_mut().take(),
            Some(vec![Change::Added(vec![
                Any::String("a".to_string()).into(),
                Any::String("e".to_string()).into(),
                Any::String("f".to_string()).into(),
                Any::String("d".to_string()).into()
            ])])
        );
    }

    #[test]
    fn unicode_support() {
        let mut d = Doc::with_client_id(1);
        let mut txn = d.transact();
        let txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "😀🙄"); // emoji are a 4-byte unicode points
        assert_eq!(txt.to_string(&txn), "😀🙄");
        assert_eq!(txt.len(), 2);
        txt.insert(&mut txn, 1, "🥰");
        assert_eq!(txt.to_string(&txn), "😀🥰🙄");
        assert_eq!(txt.len(), 3);
    }

    fn text_transactions() -> [Box<dyn Fn(&mut Doc, &mut StdRng)>; 2] {
        fn insert_text(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let ytext = txn.get_text("text");
            let pos = rng.between(0, ytext.len());
            let word = rng.random_string();
            ytext.insert(&mut txn, pos, word.as_str());
        }

        fn delete_text(doc: &mut Doc, rng: &mut StdRng) {
            let mut txn = doc.transact();
            let ytext = txn.get_text("text");
            let len = ytext.len();
            if len > 0 {
                let pos = rng.between(0, len - 1);
                let to_delete = rng.between(2, len - pos);
                ytext.remove_range(&mut txn, pos, to_delete);
            }
        }

        [Box::new(insert_text), Box::new(delete_text)]
    }

    fn fuzzy(iterations: usize) {
        run_scenario(0, &text_transactions(), 5, iterations)
    }

    #[test]
    fn fuzzy_test_3() {
        fuzzy(3)
    }
}
