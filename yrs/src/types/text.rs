use crate::transaction::Transaction;
use crate::*;

pub struct Text {
    ptr: types::TypePtr,
}

impl Text {
    pub fn from(ptr: types::TypePtr) -> Self {
        Text { ptr }
    }
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self, tr: &Transaction) -> String {
        tr.store
            .get_type(&self.ptr)
            .and_then(|inner| {
                let mut start = inner.start.get();
                let mut s = String::new();
                while let Some(a) = start.as_ref() {
                    if let Some(item) = tr.store.blocks.get_item(&a) {
                        if let block::ItemContent::String(item_string) = &item.content {
                            s.push_str(item_string);
                        }
                        start = item.right;
                    } else {
                        break;
                    }
                }
                Some(s)
            })
            .unwrap_or_default()
    }

    fn find_position(&self, tr: &Transaction, index: u32) -> Option<block::ItemPosition> {
        let inner = tr.store.get_type(&self.ptr)?;
        if index == 0 {
            Some(block::ItemPosition {
                parent: inner.ptr.clone(),
                after: None,
                offset: 0,
            })
        } else {
            let mut ptr = inner.start.get();
            let mut prev = ptr.clone();
            let mut remaining = index;
            while let Some(item) = ptr.and_then(|p| tr.store.blocks.get_item(&p)) {
                let len = item.len();
                if remaining < len {
                    // the index we look for is either after or inside of the index
                    break;
                } else {
                    prev = ptr.take();
                    ptr = item.right;
                    remaining -= len;
                }
            }
            Some(block::ItemPosition {
                parent: inner.ptr.clone(),
                after: prev,
                offset: remaining,
            })
        }
    }
    pub fn insert(&self, tr: &mut Transaction, index: u32, content: &str) {
        if let Some(pos) = self.find_position(tr, index) {
            tr.create_item(&pos, block::ItemContent::String(content.to_owned()));
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }
}

#[cfg(test)]
mod test {
    use crate::Doc;

    #[test]
    fn append_single_character_chunks() {
        let mut doc = Doc::new();
        let mut txn = doc.transact();
        let mut txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 1, "b");
        txt.insert(&mut txn, 2, "c");

        assert_eq!(txt.to_string(&txn).as_str(), "abc");
    }

    #[test]
    fn append_mutli_character_chunks() {
        let mut doc = Doc::new();
        let mut txn = doc.transact();
        let mut txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");

        assert_eq!(txt.to_string(&txn).as_str(), "hello world");
    }

    #[test]
    fn prepend_single_character_chunks() {
        let mut doc = Doc::new();
        let mut txn = doc.transact();
        let mut txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "a");
        txt.insert(&mut txn, 0, "b");
        txt.insert(&mut txn, 0, "c");

        assert_eq!(txt.to_string(&txn).as_str(), "cba");
    }

    #[test]
    fn prepend_mutli_character_chunks() {
        let mut doc = Doc::new();
        let mut txn = doc.transact();
        let mut txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 0, " ");
        txt.insert(&mut txn, 0, "world");

        assert_eq!(txt.to_string(&txn).as_str(), "world hello");
    }

    #[test]
    fn insert_after_chunk() {
        let mut doc = Doc::new();
        let mut txn = doc.transact();
        let mut txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "hello");
        txt.insert(&mut txn, 5, " ");
        txt.insert(&mut txn, 6, "world");
        txt.insert(&mut txn, 6, "beautiful ");

        assert_eq!(txt.to_string(&txn).as_str(), "hello beautiful world");
    }

    #[test]
    fn insert_inside_of_chunk() {
        let mut doc = Doc::new();
        let mut txn = doc.transact();
        let mut txt = txn.get_text("test");

        txt.insert(&mut txn, 0, "it was expected");
        txt.insert(&mut txn, 6, " not");

        assert_eq!(txt.to_string(&txn).as_str(), "it was not expected");
    }
}
