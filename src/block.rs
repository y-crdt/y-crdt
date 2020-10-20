use crate::*;

#[derive(Copy, Clone)]
pub struct ID {
    pub client: u32,
    pub clock: u32
}

#[derive(Copy, Clone)]
pub struct BlockPtr {
    pub id: ID,
    pub pivot: u32
}

pub struct Item {
    pub id: ID,
    pub left: Option<BlockPtr>,
    pub right: Option<BlockPtr>,
    pub origin: Option<ID>,
    pub right_origin: Option<ID>,
    pub content: char,
    pub parent: TypePtr
}

impl Item {
    #[inline(always)]
    pub fn integrate (&self, doc: &mut DocInner, pivot: u32) {
        // No conflict resolution yet..
        // We only implement the reconnection part:
        if let Some(right_id) = self.right {
            let right = doc.ss.get_item_mut(&right_id);
            right.left = Some(BlockPtr {
                pivot,
                id: self.id
            });
        }
        match self.left {
            Some(left_id) => {
                let left = doc.ss.get_item_mut(&left_id);
                left.right = Some(BlockPtr {
                    pivot,
                    id: self.id
                });
            }
            None => {
                let parent_type = doc.get_type_from_ptr(&self.parent);
                parent_type.start.set(Some(BlockPtr {
                    pivot,
                    id: self.id
                }));
            }
        }
    }
}

#[derive(Copy, Clone)]
pub struct ItemPosition <'a> {
    pub parent: &'a Type,
    pub after: Option<BlockPtr>
}
