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
                    let item = tr.store.blocks.get_item(&a);
                    if let block::ItemContent::String(item_string) = &item.content {
                        s.push_str(item_string);
                    }
                    start = item.right
                }
                Some(s)
            })
            .unwrap_or_default()
    }
    fn find_list_pos(&self, tr: &Transaction, pos: u32) -> Option<block::ItemPosition> {
        tr.store.get_type(&self.ptr).and_then(|inner| {
            if pos == 0 {
                Some(block::ItemPosition {
                    parent: inner.ptr.clone(),
                    after: None,
                })
            } else {
                let mut ptr = inner.start.get();
                let mut curr_pos = 1;
                while curr_pos != pos {
                    if let Some(a) = ptr.as_ref() {
                        ptr = tr.store.blocks.get_item(a).right;
                        curr_pos += 1;
                    } else {
                        // todo: throw error here
                        break;
                    }
                }
                Some(block::ItemPosition {
                    parent: inner.ptr.clone(),
                    after: ptr,
                })
            }
        })
    }
    pub fn insert(&self, tr: &mut Transaction, pos: u32, content: &str) {
        if let Some(pos) = self.find_list_pos(tr, pos) {
            tr.store
                .create_item(&pos, block::ItemContent::String(content.to_owned()));
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }
}
