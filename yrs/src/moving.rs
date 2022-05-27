use crate::block::{Block, BlockPtr, ItemContent, Prelim};
use crate::block_iter::BlockIter;
use crate::types::BranchPtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{Transaction, ID};
use std::collections::HashSet;
use std::ops::DerefMut;
use std::rc::Rc;

/// Association type. If true, associate with right block. Otherwise with the left one.
pub type Assoc = bool;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Move {
    pub start: RelativePosition,
    pub end: RelativePosition,
    pub priority: i32,

    /// We store which Items+ContentMove we override. Once we delete
    /// this ContentMove, we need to re-integrate the overridden items.
    ///
    /// This representation can be improved if we ever run into memory issues because of too many overrides.
    /// Ideally, we should probably just re-iterate the document and re-integrate all moved items.
    /// This is fast enough and reduces memory footprint significantly.
    pub(crate) overrides: Option<HashSet<BlockPtr>>,
}

impl Move {
    pub fn new(start: RelativePosition, end: RelativePosition, priority: i32) -> Self {
        Move {
            start,
            end,
            priority,
            overrides: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        match (&self.start.kind, &self.end.kind) {
            (RelativePositionKind::Item(a), RelativePositionKind::Item(b)) => a == b,
            _ => false,
        }
    }

    pub(crate) fn get_moved_coords(
        &self,
        txn: &mut Transaction,
    ) -> (Option<BlockPtr>, Option<BlockPtr>) {
        let start = match &self.start.kind {
            RelativePositionKind::Item(id) => Self::get_item_ptr(txn, id, self.start.assoc),
            RelativePositionKind::Type(id) => {
                if let Some(Block::Item(item)) = txn.store().blocks.get_block(id).as_deref() {
                    if let ItemContent::Type(branch) = &item.content {
                        branch.start
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            RelativePositionKind::TypeName(name) => {
                if let Some(branch) = txn.store().types.get(name) {
                    branch.start
                } else {
                    None
                }
            }
        };

        let end = if let RelativePositionKind::Item(id) = &self.end.kind {
            Self::get_item_ptr(txn, id, self.end.assoc)
        } else {
            None
        };
        (start, end)
    }

    fn get_item_ptr(txn: &mut Transaction, id: &ID, assoc: Assoc) -> Option<BlockPtr> {
        if assoc {
            txn.store_mut().blocks.get_item_clean_start(id)
        } else if let Some(Block::Item(item)) =
            txn.store_mut().blocks.get_item_clean_end(id).as_deref()
        {
            item.right
        } else {
            None
        }
    }

    pub(crate) fn find_move_loop(
        &self,
        txn: &mut Transaction,
        moved: BlockPtr,
        tracked_moved_items: &mut HashSet<BlockPtr>,
    ) -> bool {
        if tracked_moved_items.contains(&moved) {
            true
        } else {
            tracked_moved_items.insert(moved.clone());
            let (mut start, end) = self.get_moved_coords(txn);
            while let Some(Block::Item(item)) = start.as_deref() {
                if start == end {
                    break;
                }

                if !item.is_deleted() && item.moved == Some(moved) {
                    if let ItemContent::Move(m) = &item.content {
                        if m.find_move_loop(txn, start.unwrap(), tracked_moved_items) {
                            return true;
                        }
                    }
                }

                start = item.right;
            }

            false
        }
    }

    fn push_override(&mut self, ptr: BlockPtr) {
        let e = self.overrides.get_or_insert_with(HashSet::default);
        e.insert(ptr);
    }

    pub(crate) fn integrate_block(&mut self, txn: &mut Transaction, item: BlockPtr) {
        let (mut start, end) = self.get_moved_coords(txn);
        let mut max_priority = 0i32;
        let adapt_priority = self.priority < 0;
        while start != end && start.is_some() {
            let start_ptr = start.unwrap().clone();
            if let Some(Block::Item(start_item)) = start.as_deref_mut() {
                let mut curr_moved = start_item.moved;
                let next_prio = if let Some(Block::Item(m)) = curr_moved.as_deref() {
                    if let ItemContent::Move(next) = &m.content {
                        next.priority
                    } else {
                        -1
                    }
                } else {
                    -1
                };

                if adapt_priority
                    || next_prio < self.priority
                    || (curr_moved.is_some()
                        && next_prio == self.priority
                        && (*curr_moved.unwrap().id() < *item.id()))
                {
                    if let Some(moved_ptr) = curr_moved.clone() {
                        self.push_override(moved_ptr);
                    }
                    max_priority = max_priority.max(next_prio);
                    // was already moved
                    let prev_move = start_item.moved;
                    if let Some(prev_move) = prev_move {
                        if !txn.prev_moved.contains_key(&prev_move)
                            && prev_move.id().clock < txn.before_state.get(&prev_move.id().client)
                        {
                            // only override prevMoved if the prevMoved item is not new
                            // we need to know which item previously moved an item
                            txn.prev_moved.insert(start_ptr, prev_move);
                        }
                    }
                    start_item.moved = Some(item);
                    if !start_item.is_deleted() {
                        if let ItemContent::Move(m) = &start_item.content {
                            if m.find_move_loop(txn, start_ptr, &mut HashSet::from([item])) {
                                item.delete_as_cleanup(txn);
                                return;
                            }
                        }
                    }
                } else if let Some(Block::Item(moved_item)) = curr_moved.as_deref_mut() {
                    if let ItemContent::Move(m) = &mut moved_item.content {
                        m.push_override(item);
                    }
                }
                start = start_item.right;
            } else {
                break;
            }
        }

        if adapt_priority {
            self.priority = max_priority + 1;
        }
    }

    pub(crate) fn delete(&self, txn: &mut Transaction, item: BlockPtr) {
        let (mut start, end) = self.get_moved_coords(txn);
        while start != end && start.is_some() {
            if let Some(Block::Item(i)) = start.as_deref_mut() {
                if i.moved == Some(item) {
                    i.moved = None;
                }
                start = i.right;
            } else {
                break;
            }
        }

        fn reintegrate(mut ptr: BlockPtr, txn: &mut Transaction) {
            let ptr_copy = ptr.clone();
            if let Block::Item(item) = ptr.deref_mut() {
                let deleted = item.is_deleted();
                if let ItemContent::Move(content) = &mut item.content {
                    if deleted {
                        // potentially we can integrate the items that reIntegrateItem overrides
                        if let Some(overrides) = &content.overrides {
                            for &inner in overrides.iter() {
                                reintegrate(inner, txn);
                            }
                        }
                    } else {
                        content.integrate_block(txn, ptr_copy)
                    }
                }
            }
        }

        if let Some(overrides) = &self.overrides {
            for &ptr in overrides {
                reintegrate(ptr, txn);
            }
        }
    }
}

impl Encode for Move {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        let is_collapsed = self.is_collapsed();
        encoder.write_u8(if is_collapsed { 1 } else { 0 });
        self.start.encode(encoder);
        if !is_collapsed {
            self.end.encode(encoder);
        }
        encoder.write_uvar(self.priority as u32);
    }
}

impl Decode for Move {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let is_collapsed = if decoder.read_u8() == 0 { false } else { true };
        let start = RelativePosition::decode(decoder);
        let end = if is_collapsed {
            let mut end = start.clone();
            end.assoc = false;
            end
        } else {
            RelativePosition::decode(decoder)
        };
        let priority: u32 = decoder.read_uvar();
        Move::new(start, end, priority as i32)
    }
}

impl Prelim for Move {
    #[inline]
    fn into_content(self, _: &mut Transaction) -> (ItemContent, Option<Self>) {
        (ItemContent::Move(Box::new(self)), None)
    }

    #[inline]
    fn integrate(self, _: &mut Transaction, _inner_ref: BranchPtr) {}
}

impl std::fmt::Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "move(")?;
        write!(f, "{}", self.start)?;
        if self.start != self.end {
            write!(f, "..{}", self.end)?;
        }
        if self.priority != 0 {
            write!(f, ", prio: {}", self.priority)?;
        }
        if let Some(overrides) = self.overrides.as_ref() {
            write!(f, ", overrides: [")?;
            let mut i = overrides.iter();
            if let Some(b) = i.next() {
                write!(f, "{}", b.id())?;
            }
            while let Some(b) = i.next() {
                write!(f, ", {}", b.id())?;
            }
            write!(f, "]")?;
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RelativePosition {
    pub kind: RelativePositionKind,
    /// If true - associate to the right block. Otherwise associate to the left one.
    pub assoc: Assoc,
}

impl RelativePosition {
    fn create(txn: &Transaction, branch: BranchPtr, item: Option<ID>, assoc: Assoc) -> Self {
        let kind = if let Some(id) = item {
            RelativePositionKind::Item(id)
        } else if let Some(item) = branch.item {
            RelativePositionKind::Type(*item.id())
        } else {
            let name = txn.store().get_type_key(branch).unwrap();
            RelativePositionKind::TypeName(name.clone())
        };
        RelativePosition { assoc, kind }
    }

    pub(crate) fn from_type_index(
        txn: &mut Transaction,
        branch: BranchPtr,
        mut index: u32,
        assoc: Assoc,
    ) -> Self {
        if !assoc {
            if index == 0 {
                return Self::create(txn, branch, None, assoc);
            }
            index -= 1;
        }

        let mut walker = BlockIter::new(branch);
        if !walker.try_forward(txn, index) {
            panic!("Block iter couldn't move forward");
        }
        if walker.finished() {
            let id = if !assoc {
                walker.next_item().map(|ptr| ptr.last_id())
            } else {
                None
            };
            Self::create(txn, branch, id, assoc)
        } else {
            let id = walker.next_item().map(|ptr| {
                let mut id = ptr.id().clone();
                id.clock += walker.rel();
                id
            });
            Self::create(txn, branch, id, assoc)
        }
    }

    pub(crate) fn within_range(&self, ptr: Option<BlockPtr>) -> bool {
        if !self.assoc {
            return false;
        } else if let Some(Block::Item(item)) = ptr.as_deref() {
            match (item.left, &self.kind) {
                (Some(ptr), RelativePositionKind::Item(id)) => ptr.last_id() != *id,
                _ => false,
            }
        } else if let RelativePositionKind::Item(_) = self.kind {
            true
        } else {
            false
        }
    }
}

impl std::fmt::Display for RelativePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.assoc {
            write!(f, "<")?;
        }
        write!(f, "{}", self.kind)?;
        if self.assoc {
            write!(f, ">")?;
        }
        Ok(())
    }
}
const RELATIVE_POSITION_TAG_ITEM: u8 = 0;
const RELATIVE_POSITION_TAG_TYPE_NAME: u8 = 1;
const RELATIVE_POSITION_TAG_TYPE: u8 = 2;

impl Encode for RelativePosition {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match &self.kind {
            RelativePositionKind::Item(id) => {
                encoder.write_u8(RELATIVE_POSITION_TAG_ITEM);
                encoder.write_uvar(id.client);
                encoder.write_uvar(id.clock);
            }
            RelativePositionKind::TypeName(tname) => {
                encoder.write_u8(RELATIVE_POSITION_TAG_TYPE_NAME);
                encoder.write_string(&tname);
            }
            RelativePositionKind::Type(id) => {
                encoder.write_u8(RELATIVE_POSITION_TAG_TYPE_NAME);
                encoder.write_uvar(id.client);
                encoder.write_uvar(id.clock);
            }
        }
        if self.assoc {
            encoder.write_ivar(1);
        } else {
            encoder.write_ivar(-1);
        }
    }
}

impl Decode for RelativePosition {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        let tag = decoder.read_u8();
        let kind = match tag {
            RELATIVE_POSITION_TAG_ITEM => {
                let client = decoder.read_uvar();
                let clock = decoder.read_uvar();
                RelativePositionKind::Item(ID::new(client, clock))
            }
            RELATIVE_POSITION_TAG_TYPE_NAME => {
                RelativePositionKind::TypeName(decoder.read_string().into())
            }
            RELATIVE_POSITION_TAG_TYPE => {
                let client = decoder.read_uvar();
                let clock = decoder.read_uvar();
                RelativePositionKind::Type(ID::new(client, clock))
            }
            unknown => panic!("RelativePosition decoder met unknown tag {}", unknown),
        };
        let assoc = {
            if decoder.has_content() {
                if decoder.read_ivar() >= 0 {
                    true
                } else {
                    false
                }
            } else {
                true
            }
        };
        RelativePosition { kind, assoc }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RelativePositionKind {
    Item(ID),
    Type(ID),
    TypeName(Rc<str>),
}

impl std::fmt::Display for RelativePositionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelativePositionKind::Item(id) => write!(f, "{}", id),
            RelativePositionKind::Type(id) => write!(f, ":{}", id),
            RelativePositionKind::TypeName(tname) => write!(f, "'{}'", tname),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AbsolutePosition {
    branch: BranchPtr,
    index: u32,
    assoc: Assoc,
}

impl AbsolutePosition {
    fn new(branch: BranchPtr, index: u32, assoc: Assoc) -> Self {
        AbsolutePosition {
            branch,
            index,
            assoc,
        }
    }
}
