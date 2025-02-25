use crate::block::{ItemPtr, BLOCK_GC_REF_NUMBER, GC, HAS_ORIGIN, HAS_RIGHT_ORIGIN};
use crate::types::TypePtr;
use crate::updates::encoder::Encoder;
use crate::ID;
use std::ops::Deref;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum BlockSlice {
    Item(ItemSlice),
    GC(GCSlice),
}

impl BlockSlice {
    pub fn clock_start(&self) -> u32 {
        match self {
            BlockSlice::Item(s) => s.clock_start(),
            BlockSlice::GC(s) => s.clock_start(),
        }
    }

    pub fn clock_end(&self) -> u32 {
        match self {
            BlockSlice::Item(s) => s.clock_end(),
            BlockSlice::GC(s) => s.clock_end(),
        }
    }

    pub(crate) fn as_item(&self) -> Option<ItemPtr> {
        if let BlockSlice::Item(s) = self {
            Some(s.ptr)
        } else {
            None
        }
    }

    pub fn len(&self) -> u32 {
        match self {
            BlockSlice::Item(s) => s.len(),
            BlockSlice::GC(s) => s.len(),
        }
    }

    #[allow(dead_code)]
    pub fn is_deleted(&self) -> bool {
        match self {
            BlockSlice::Item(s) => s.is_deleted(),
            BlockSlice::GC(_) => true,
        }
    }

    /// Trim a number of countable elements from the beginning of a current slice.
    pub fn trim_start(&mut self, count: u32) {
        match self {
            BlockSlice::Item(s) => s.trim_start(count),
            BlockSlice::GC(s) => s.trim_start(count),
        }
    }

    /// Trim a number of countable elements from the end of a current slice.
    pub fn trim_end(&mut self, count: u32) {
        match self {
            BlockSlice::Item(s) => s.trim_end(count),
            BlockSlice::GC(s) => s.trim_end(count),
        }
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            BlockSlice::Item(s) => s.encode(encoder),
            BlockSlice::GC(s) => s.encode(encoder),
        }
    }
}

impl From<ItemSlice> for BlockSlice {
    fn from(slice: ItemSlice) -> Self {
        BlockSlice::Item(slice)
    }
}

impl From<GCSlice> for BlockSlice {
    fn from(slice: GCSlice) -> Self {
        BlockSlice::GC(slice)
    }
}

/// Defines a logical slice of an underlying [Item]. Yrs blocks define a series of sequential
/// updates performed by a single peer, while [ItemSlice]s enable to refer to a sub-range of these
/// blocks without need to splice them.
///
/// If an underlying [Item] needs to be spliced to fit the boundaries defined by a corresponding
/// [ItemSlice], this can be done with help of transaction (see: [Store::materialize]).
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ItemSlice {
    pub ptr: ItemPtr,
    pub start: u32,
    pub end: u32,
}

impl ItemSlice {
    pub fn new(ptr: ItemPtr, start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        ItemSlice { ptr, start, end }
    }
    pub fn clock_start(&self) -> u32 {
        self.ptr.id.clock + self.start
    }

    pub fn clock_end(&self) -> u32 {
        self.ptr.id.clock + self.end
    }

    /// Returns the number of elements (counted as Yjs ID clock length) of the block range described
    /// by current [ItemSlice].
    pub fn len(&self) -> u32 {
        self.end - self.start + 1
    }

    /// Returns the first [ID] covered by this slice (inclusive).
    pub fn id(&self) -> ID {
        let mut id = self.ptr.id;
        id.clock += self.start;
        id
    }

    /// Returns the last [ID] covered by this slice (inclusive).
    pub fn last_id(&self) -> ID {
        let mut id = self.ptr.id;
        id.clock += self.end;
        id
    }

    /// Trim a number of countable elements from the beginning of a current slice.
    pub(crate) fn trim_start(&mut self, count: u32) {
        debug_assert!(count <= self.len());
        self.start += count;
    }

    /// Trim a number of countable elements from the end of a current slice.
    pub(crate) fn trim_end(&mut self, count: u32) {
        debug_assert!(count <= self.len());
        self.end -= count;
    }

    /// Attempts to trim current block slice to provided range of IDs.
    /// Returns true if current block slice range has been modified as a result of this action.
    pub fn try_trim(&mut self, from: &ID, to: &ID) -> bool {
        let id = self.ptr.id;
        let mut changed = false;
        let start_clock = id.clock + self.start;
        let end_clock = id.clock + self.end;
        if id.client == from.client && from.clock > start_clock && from.clock <= end_clock {
            self.start = from.clock - start_clock;
            changed = true;
        }
        if id.client == to.client && to.clock >= start_clock && to.clock < end_clock {
            self.end = end_clock - to.clock;
            changed = true;
        }
        changed
    }

    /// Returns true when current [ItemSlice] left boundary is equal to the boundary of the
    /// [ItemPtr] this slice wraps.
    pub fn adjacent_left(&self) -> bool {
        self.start == 0
    }

    /// Returns true when current [ItemSlice] right boundary is equal to the boundary of the
    /// [ItemPtr] this slice wraps.
    pub fn adjacent_right(&self) -> bool {
        self.end == self.ptr.len() - 1
    }

    /// Returns true when boundaries marked by the current [ItemSlice] match the boundaries
    /// of the underlying [ItemPtr].
    pub fn adjacent(&self) -> bool {
        self.adjacent_left() && self.adjacent_right()
    }

    /// Checks if an underlying [Item] has been marked as deleted.
    pub fn is_deleted(&self) -> bool {
        self.ptr.is_deleted()
    }

    /// Checks if an underlying [Item] has been marked as countable.
    pub fn is_countable(&self) -> bool {
        self.ptr.is_countable()
    }

    /// Checks if provided `id` exists within the bounds described by current [ItemSlice].
    pub fn contains_id(&self, id: &ID) -> bool {
        let myself = self.ptr.id();
        myself.client == id.client
            && id.clock >= myself.clock + self.start
            && id.clock <= myself.clock + self.end
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        let item = self.ptr.deref();
        let mut info = item.info();
        let origin = if self.adjacent_left() {
            item.origin
        } else {
            Some(ID::new(item.id.client, item.id.clock + self.start - 1))
        };
        if origin.is_some() {
            info |= HAS_ORIGIN;
        }
        let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
        encoder.write_info(info);
        if let Some(origin_id) = origin {
            encoder.write_left_id(&origin_id);
        }
        if self.adjacent_right() {
            if let Some(right_origin_id) = item.right_origin.as_ref() {
                encoder.write_right_id(right_origin_id);
            }
        }
        if cant_copy_parent_info {
            match &item.parent {
                TypePtr::Branch(branch) => {
                    if let Some(block) = branch.item {
                        encoder.write_parent_info(false);
                        encoder.write_left_id(block.id());
                    } else if let Some(name) = branch.name.as_deref() {
                        encoder.write_parent_info(true);
                        encoder.write_string(name);
                    } else {
                        unreachable!("Could not get parent branch info for item")
                    }
                }
                TypePtr::Named(name) => {
                    encoder.write_parent_info(true);
                    encoder.write_string(name);
                }
                TypePtr::ID(id) => {
                    encoder.write_parent_info(false);
                    encoder.write_left_id(id);
                }
                TypePtr::Unknown => {
                    panic!("Couldn't get item's parent")
                }
            }

            if let Some(parent_sub) = item.parent_sub.as_ref() {
                encoder.write_string(parent_sub.as_ref());
            }
        }
        item.content.encode_slice(encoder, self.start, self.end);
    }

    /// Returns a [ItemSlice] wrapper for a [Item] identified as a right neighbor of this slice.
    /// This method doesn't have to be equivalent of [Item::right]: if current slices's end range
    /// is not an equivalent to the end of underlying [Item], a returned slice will contain the
    /// same block that starts when the current ends.
    ///
    /// # Example
    ///
    /// If an underlying block is responsible for clock range of [0..10) and current slice is [2..8)
    /// then right slice returned by this method will be [8..10).
    pub fn right(&self) -> Option<ItemSlice> {
        let last_clock = self.ptr.len() - 1;
        if self.end == last_clock {
            let item = self.ptr.deref();
            let right_ptr = item.right?;
            Some(ItemSlice::from(right_ptr))
        } else {
            Some(ItemSlice::new(self.ptr, self.end + 1, last_clock))
        }
    }

    /// Returns a [ItemSlice] wrapper for a [Item] identified as a left neighbor of this slice.
    /// This method doesn't have to be equivalent of [Item::right]: if current slices's start range
    /// is not an equivalent to the start of underlying [Item], a returned slice will contain the
    /// range that starts with current underlying block start and end where this slice starts.
    ///
    /// # Example
    ///
    /// If an underlying block is responsible for clock range of [0..10) and current slice is [2..8)
    /// then right slice returned by this method will be [0..2).
    pub fn left(&self) -> Option<ItemSlice> {
        if self.start == 0 {
            let item = self.ptr.deref();
            let left_ptr = item.left?;
            Some(ItemSlice::from(left_ptr))
        } else {
            Some(ItemSlice::new(self.ptr, 0, self.start - 1))
        }
    }
}

impl From<ItemPtr> for ItemSlice {
    fn from(ptr: ItemPtr) -> Self {
        ItemSlice::new(ptr, 0, ptr.len() - 1)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct GCSlice {
    pub start: u32,
    pub end: u32,
}

impl GCSlice {
    pub fn clock_start(&self) -> u32 {
        self.start
    }

    pub fn clock_end(&self) -> u32 {
        self.end
    }

    /// Trim a number of countable elements from the beginning of a current slice.
    pub(crate) fn trim_start(&mut self, count: u32) {
        debug_assert!(count <= self.len());
        self.start += count;
    }

    /// Trim a number of countable elements from the end of a current slice.
    pub(crate) fn trim_end(&mut self, count: u32) {
        debug_assert!(count <= self.len());
        self.end -= count;
    }

    pub fn len(&self) -> u32 {
        self.end - self.start + 1
    }

    pub fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_info(BLOCK_GC_REF_NUMBER);
        encoder.write_len(self.len());
    }
}

impl From<GC> for GCSlice {
    fn from(gc: GC) -> Self {
        GCSlice {
            start: gc.start,
            end: gc.end,
        }
    }
}
