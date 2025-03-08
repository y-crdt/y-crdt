use crate::block::{Item, ItemContent, ItemPtr, Prelim};
use crate::branch::BranchPtr;
use crate::moving::{Move, StickyIndex};
use crate::transaction::{ReadTxn, TransactionMut};
use crate::types::TypePtr;
use crate::{Assoc, Out, ID};

/// Struct used for iterating over the sequence of item's values with respect to a potential
/// [Move] markers that may change their order.
#[derive(Debug, Clone)]
pub(crate) struct BlockIter {
    branch: BranchPtr,
    index: u32,
    rel: u32,
    next_item: Option<ItemPtr>,
    curr_move: Option<ItemPtr>,
    curr_move_start: Option<ItemPtr>,
    curr_move_end: Option<ItemPtr>,
    moved_stack: Vec<StackItem>,
    reached_end: bool,
}

impl BlockIter {
    pub fn new(branch: BranchPtr) -> Self {
        let next_item = branch.start;
        let reached_end = branch.start.is_none();
        BlockIter {
            branch,
            next_item,
            reached_end,
            curr_move: None,
            curr_move_start: None,
            curr_move_end: None,
            index: 0,
            rel: 0,
            moved_stack: Vec::default(),
        }
    }

    #[inline]
    pub fn rel(&self) -> u32 {
        self.rel
    }

    #[inline]
    pub fn finished(&self) -> bool {
        (self.reached_end && self.curr_move.is_none()) || self.index == self.branch.content_len
    }

    #[inline]
    pub fn next_item(&self) -> Option<ItemPtr> {
        self.next_item
    }

    pub fn left(&self) -> Option<ItemPtr> {
        if self.reached_end {
            self.next_item
        } else if let Some(item) = self.next_item.as_deref() {
            item.left
        } else {
            None
        }
    }

    pub fn right(&self) -> Option<ItemPtr> {
        if self.reached_end {
            None
        } else {
            self.next_item
        }
    }

    pub fn move_to(&mut self, index: u32, txn: &mut TransactionMut) {
        if index > self.index {
            if !self.try_forward(txn, index - self.index) {
                panic!("Block iter couldn't move forward");
            }
        } else if index < self.index {
            self.backward(txn, self.index - index)
        }
    }

    fn can_forward(&self, ptr: Option<ItemPtr>, len: u32) -> bool {
        if !self.reached_end || self.curr_move.is_some() {
            if len > 0 {
                return true;
            } else if let Some(item) = ptr.as_deref() {
                return !item.is_countable()
                    || item.is_deleted()
                    || ptr == self.curr_move_end
                    || (self.reached_end && self.curr_move_end.is_none())
                    || item.moved != self.curr_move;
            }
        }

        false
    }

    pub fn forward<T: ReadTxn>(&mut self, txn: &T, len: u32) {
        if !self.try_forward(txn, len) {
            panic!("Length exceeded")
        }
    }

    pub fn try_forward<T: ReadTxn>(&mut self, txn: &T, mut len: u32) -> bool {
        if len == 0 && self.next_item.is_none() {
            return true;
        }

        if self.index + len > self.branch.content_len() || self.next_item.is_none() {
            return false;
        }

        let mut item = self.next_item;
        self.index += len;
        if self.rel != 0 {
            len += self.rel;
            self.rel = 0;
        }

        let encoding = txn.store().offset_kind;
        while self.can_forward(item, len) {
            if item == self.curr_move_end
                || (self.reached_end && self.curr_move_end.is_none() && self.curr_move.is_some())
            {
                item = self.curr_move; // we iterate to the right after the current condition
                self.pop(txn);
            } else if item.is_none() {
                return false;
            } else if let Some(i) = item.as_deref() {
                if i.is_countable() && !i.is_deleted() && i.moved == self.curr_move && len > 0 {
                    let item_len = i.content_len(encoding);
                    if item_len > len {
                        self.rel = len;
                        len = 0;
                        break;
                    } else {
                        len -= item_len;
                    }
                } else if let ItemContent::Move(m) = &i.content {
                    if i.moved == self.curr_move {
                        if let Some(ptr) = self.curr_move {
                            self.moved_stack.push(StackItem::new(
                                self.curr_move_start,
                                self.curr_move_end,
                                ptr,
                            ));
                        }

                        let (start, end) = m.get_moved_coords(txn);
                        self.curr_move = item;
                        self.curr_move_start = start;
                        self.curr_move_end = end;
                        item = start;
                        continue;
                    }
                }
            }

            if self.reached_end {
                return false;
            }

            match item.as_deref() {
                Some(i) if i.right.is_some() => item = i.right,
                _ => self.reached_end = true, //TODO: we need to ensure to iterate further if this.currMoveEnd === null
            }
        }

        self.index -= len;
        self.next_item = item;
        true
    }

    fn reduce_moves(&mut self, txn: &mut TransactionMut) {
        let mut item = self.next_item;
        if item.is_some() {
            while item == self.curr_move_start {
                item = self.curr_move;
                self.pop(txn);
            }
            self.next_item = item;
        }
    }

    pub fn backward<T: ReadTxn>(&mut self, txn: &mut T, mut len: u32) {
        if self.index < len {
            panic!("Length exceeded");
        }
        self.index -= len;
        let encoding = txn.store().offset_kind;
        if self.reached_end {
            if let Some(next_item) = self.next_item.as_deref() {
                self.rel = if next_item.is_countable() && !next_item.is_deleted() {
                    next_item.content_len(encoding)
                } else {
                    0
                };
            }
        }
        if self.rel >= len {
            self.rel -= len;
            return;
        }
        let mut item = self.next_item;
        if let Some(i) = item.as_deref() {
            if let ItemContent::Move(_) = &i.content {
                item = i.left;
            } else {
                len += if i.is_countable() && !i.is_deleted() && i.moved == self.curr_move {
                    i.content_len(encoding)
                } else {
                    0
                };
                len -= self.rel;
            }
        }
        self.rel = 0;
        while let Some(i) = item.as_deref() {
            if len == 0 {
                break;
            }

            if i.is_countable() && !i.is_deleted() && i.moved == self.curr_move {
                let item_len = i.content_len(encoding);
                if len < item_len {
                    self.rel = item_len - len;
                    len = 0;
                } else {
                    len -= item_len;
                }
                if len == 0 {
                    break;
                }
            } else if let ItemContent::Move(m) = &i.content {
                if i.moved == self.curr_move {
                    if let Some(curr_move) = self.curr_move {
                        self.moved_stack.push(StackItem::new(
                            self.curr_move_start,
                            self.curr_move_end,
                            curr_move,
                        ));
                    }
                    let (start, end) = m.get_moved_coords(txn);
                    self.curr_move = item;
                    self.curr_move_start = start;
                    self.curr_move_end = end;
                    item = start;
                    continue;
                }
            }

            if item == self.curr_move_start {
                item = self.curr_move; // we iterate to the left after the current condition
                self.pop(txn);
            }

            item = if let Some(i) = item.as_deref() {
                i.left
            } else {
                None
            };
        }
        self.next_item = item;
    }

    /// We keep the moved-stack across several transactions. Local or remote changes can invalidate
    /// "moved coords" on the moved-stack.
    ///
    /// The reason for this is that if assoc < 0, then getMovedCoords will return the target.right
    /// item. While the computed item is on the stack, it is possible that a user inserts something
    /// between target and the item on the stack. Then we expect that the newly inserted item
    /// is supposed to be on the new computed item.
    fn pop<T: ReadTxn>(&mut self, txn: &T) {
        let mut start = None;
        let mut end = None;
        let mut moved = None;
        if let Some(stack_item) = self.moved_stack.pop() {
            moved = Some(stack_item.moved_to);
            start = stack_item.start;
            end = stack_item.end;

            let moved_item = stack_item.moved_to;
            if let ItemContent::Move(m) = &moved_item.content {
                if m.start.assoc == Assoc::Before && (m.start.within_range(start))
                    || (m.end.within_range(end))
                {
                    let (s, e) = m.get_moved_coords(txn);
                    start = s;
                    end = e;
                }
            }
        }
        self.curr_move = moved;
        self.curr_move_start = start;
        self.curr_move_end = end;
        self.reached_end = false;
    }

    pub fn delete(&mut self, txn: &mut TransactionMut, mut len: u32) {
        let mut item = self.next_item;
        if self.index + len > self.branch.content_len() {
            panic!("Length exceeded");
        }

        let encoding = txn.store().offset_kind;
        let mut i: &Item;
        while len > 0 {
            while let Some(block) = item.as_deref() {
                i = block;
                if !i.is_deleted()
                    && i.is_countable()
                    && !self.reached_end
                    && len > 0
                    && i.moved == self.curr_move
                    && item != self.curr_move_end
                {
                    if self.rel > 0 {
                        let mut id = i.id.clone();
                        id.clock += self.rel;
                        let store = txn.store_mut();
                        item = store
                            .blocks
                            .get_item_clean_start(&id)
                            .map(|s| store.materialize(s));
                        i = item.as_deref().unwrap();
                        self.rel = 0;
                    }
                    if len < i.content_len(encoding) {
                        let mut id = i.id.clone();
                        id.clock += len;
                        let store = txn.store_mut();
                        store
                            .blocks
                            .get_item_clean_start(&id)
                            .map(|s| store.materialize(s));
                    }
                    len -= i.content_len(encoding);
                    txn.delete(item.unwrap());
                    if i.right.is_some() {
                        item = i.right;
                    } else {
                        self.reached_end = true;
                    }
                } else {
                    break;
                }
            }
            if len > 0 {
                self.next_item = item;
                if self.try_forward(txn, 0) {
                    item = self.next_item;
                } else {
                    panic!("Block iter couldn't move forward");
                }
            }
        }
        self.next_item = item;
    }

    pub(crate) fn slice<T: ReadTxn>(&mut self, txn: &T, buf: &mut [Out]) -> u32 {
        let mut len = buf.len() as u32;
        if self.index + len > self.branch.content_len() {
            return 0;
        }
        self.index += len;
        let mut next_item = self.next_item;
        let encoding = txn.store().offset_kind;
        let mut read = 0u32;
        while len > 0 {
            if !self.reached_end {
                while let Some(item) = next_item {
                    if Some(item) != self.curr_move_end
                        && item.is_countable()
                        && !self.reached_end
                        && len > 0
                    {
                        if !item.is_deleted() && item.moved == self.curr_move {
                            // we're iterating inside of a block
                            let r = item
                                .content
                                .read(self.rel as usize, &mut buf[read as usize..])
                                as u32;
                            read += r;
                            len -= r;
                            if self.rel + r == item.content_len(encoding) {
                                self.rel = 0;
                            } else {
                                self.rel += r;
                                continue; // do not iterate to item.right
                            }
                        }

                        if item.right.is_some() {
                            next_item = item.right;
                        } else {
                            self.reached_end = true;
                        }
                    } else {
                        break;
                    }
                }
                if (!self.reached_end || self.curr_move.is_some()) && len > 0 {
                    // always set nextItem before any method call
                    self.next_item = next_item;
                    if !self.try_forward(txn, 0) || self.next_item.is_none() {
                        return read;
                    }
                    next_item = self.next_item;
                }
            } else if self.curr_move.is_some() {
                // reached end but move stack still has some items,
                // so we try to pop move frames and move on the
                // first non-null right neighbor of the popped move block
                while let Some(mov) = self.curr_move.as_deref() {
                    next_item = mov.right;
                    self.pop(txn);
                    if next_item.is_some() {
                        self.reached_end = false;
                        break;
                    }
                }
            } else {
                // reached end and move stack is empty
                next_item = None;
                break;
            }
        }
        self.next_item = next_item;
        self.index -= len;
        read
    }

    fn split_rel(&mut self, txn: &mut TransactionMut) {
        if self.rel > 0 {
            if let Some(ptr) = self.next_item {
                let mut item_id = ptr.id().clone();
                item_id.clock += self.rel;
                let store = txn.store_mut();
                self.next_item = store
                    .blocks
                    .get_item_clean_start(&item_id)
                    .map(|s| store.materialize(s));
                self.rel = 0;
            }
        }
    }

    pub(crate) fn read_value<T: ReadTxn>(&mut self, txn: &T) -> Option<Out> {
        let mut buf = [Out::default()];
        if self.slice(txn, &mut buf) != 0 {
            Some(std::mem::replace(&mut buf[0], Out::default()))
        } else {
            None
        }
    }

    pub fn insert_contents<V: Prelim>(
        &mut self,
        txn: &mut TransactionMut,
        value: V,
    ) -> Option<ItemPtr> {
        self.reduce_moves(txn);
        self.split_rel(txn);
        let id = {
            let store = txn.store();
            let client_id = store.client_id;
            let clock = store.blocks.get_clock(&client_id);
            ID::new(client_id, clock)
        };
        let parent = TypePtr::Branch(self.branch);
        let right = self.right();
        let left = self.left();
        let (mut content, remainder) = value.into_content(txn);
        let inner_ref = if let ItemContent::Type(inner_ref) = &mut content {
            Some(BranchPtr::from(inner_ref))
        } else {
            None
        };
        let mut block = Item::new(
            id,
            left,
            left.map(|ptr| ptr.last_id()),
            right,
            right.map(|r| *r.id()),
            parent,
            None,
            content,
        )?;
        let mut block_ptr = ItemPtr::from(&mut block);

        block_ptr.integrate(txn, 0);

        txn.store_mut().blocks.push_block(block);

        if let Some(remainder) = remainder {
            remainder.integrate(txn, inner_ref.unwrap().into())
        }

        if let Some(item) = right.as_deref() {
            self.next_item = item.right;
        } else {
            self.next_item = left;
            self.reached_end = true;
        }

        Some(block_ptr)
    }

    pub fn insert_move(&mut self, txn: &mut TransactionMut, start: StickyIndex, end: StickyIndex) {
        self.insert_contents(txn, Move::new(start, end, -1));
    }

    pub fn values<'a, 'txn, T: ReadTxn>(
        &'a mut self,
        txn: &'txn mut TransactionMut<'txn>,
    ) -> Values<'a, 'txn> {
        Values::new(self, txn)
    }
}

pub struct Values<'a, 'txn> {
    iter: &'a mut BlockIter,
    txn: &'txn mut TransactionMut<'txn>,
}

impl<'a, 'txn> Values<'a, 'txn> {
    fn new(iter: &'a mut BlockIter, txn: &'txn mut TransactionMut<'txn>) -> Self {
        Values { iter, txn }
    }
}

impl<'a, 'txn> Iterator for Values<'a, 'txn> {
    type Item = Out;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iter.reached_end || self.iter.index == self.iter.branch.content_len() {
            None
        } else {
            let mut buf = [Out::default()];
            if self.iter.slice(self.txn, &mut buf) != 0 {
                Some(std::mem::replace(&mut buf[0], Out::default()))
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
struct StackItem {
    start: Option<ItemPtr>,
    end: Option<ItemPtr>,
    moved_to: ItemPtr,
}

impl StackItem {
    fn new(start: Option<ItemPtr>, end: Option<ItemPtr>, moved_to: ItemPtr) -> Self {
        StackItem {
            start,
            end,
            moved_to,
        }
    }
}
