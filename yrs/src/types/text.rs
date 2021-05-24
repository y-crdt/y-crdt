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
            })
        } else {
            let mut ptr = inner.start.get();
            let mut current = 0;
            while let Some(item) = ptr.and_then(|p| tr.store.blocks.get_item(&p)) {
                let len = item.len();
                current += len;
                if current >= index {
                    // the index we look for is either after or inside of the index
                    let mut ptr = ptr.unwrap().clone();
                    ptr.id.clock += len - 1;
                    return Some(block::ItemPosition {
                        parent: inner.ptr.clone(),
                        after: Some(ptr),
                    });
                } else {
                    ptr = item.right;
                }
            }
            None
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
