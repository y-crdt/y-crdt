use crate::block::{ItemContent, ItemPtr};
use crate::slice::ItemSlice;
use crate::{Assoc, Out, ReadTxn, StickyIndex};
use smallvec::{smallvec, SmallVec};
use std::ops::Deref;

pub(crate) trait BlockIterator: TxnIterator<Item = ItemPtr> + Sized {
    #[inline]
    fn slices(self) -> BlockSlices<Self> {
        BlockSlices(self)
    }

    #[inline]
    fn within_range(self, from: StickyIndex, to: StickyIndex) -> RangeIter<Self> {
        RangeIter::new(self, from, to)
    }
}

impl<T> BlockIterator for T where T: TxnIterator<Item = ItemPtr> + Sized {}

pub(crate) trait BlockSliceIterator: TxnIterator<Item = ItemSlice> + Sized {
    #[inline]
    fn values(self) -> Values<Self> {
        Values::new(self)
    }
}

impl<T> BlockSliceIterator for T where T: TxnIterator<Item = ItemSlice> + Sized {}

pub trait IntoBlockIter {
    fn to_iter(self) -> BlockIter;
}

impl IntoBlockIter for Option<ItemPtr> {
    #[inline]
    fn to_iter(self) -> BlockIter {
        BlockIter(self)
    }
}

/// Iterator over [ItemPtr] references.
/// By default it iterates to the right side.
/// When reversed it iterates to the left side.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct BlockIter(Option<ItemPtr>);

impl BlockIter {
    #[inline]
    fn new(ptr: Option<ItemPtr>) -> Self {
        BlockIter(ptr)
    }

    pub fn moved(self) -> MoveIter {
        MoveIter::new(self)
    }
}

impl Iterator for BlockIter {
    type Item = ItemPtr;

    fn next(&mut self) -> Option<Self::Item> {
        let curr = self.0.take();
        if let Some(item) = curr.as_deref() {
            self.0 = item.right;
        }
        curr
    }
}

impl DoubleEndedIterator for BlockIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        let curr = self.0.take();
        if let Some(item) = curr.as_deref() {
            self.0 = item.left;
        }
        curr
    }
}

impl IntoIterator for ItemPtr {
    type Item = ItemPtr;
    type IntoIter = BlockIter;

    fn into_iter(self) -> Self::IntoIter {
        BlockIter::new(Some(self))
    }
}

/// Iterator equivalent that can be supplied with transaction when iteration step may need it.
pub trait TxnIterator {
    type Item;
    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item>;

    fn collect<T, B>(&mut self, txn: &T) -> B
    where
        T: ReadTxn,
        B: Default + Extend<Self::Item>,
    {
        let mut buf = B::default();
        while let Some(item) = self.next(txn) {
            buf.extend([item]);
        }
        buf
    }
}

/// DoubleEndedIterator equivalent that can be supplied with transaction when iteration step may need it.
pub trait TxnDoubleEndedIterator: TxnIterator {
    fn next_back<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item>;
}

/// Block iterator which acknowledges context of move operation and iterates
/// over blocks as they appear after move. It skips over the presence of move destination blocks.
#[derive(Debug)]
pub struct MoveIter {
    iter: BlockIter,
    stack: MoveStack,
}

impl MoveIter {
    pub(crate) fn new(iter: BlockIter) -> Self {
        MoveIter {
            iter,
            stack: MoveStack::default(),
        }
    }

    /// Try to remove the latest move scope from the move stack.
    /// If there was any active scope removed this way, we reset a current iterator position
    /// to a scope destination (return address).
    /// Returns `true` if this method call had any effect.
    fn escape_current_scope(&mut self) -> bool {
        if let Some(scope) = self.stack.pop() {
            self.iter = scope.dest.into_iter();
            true
        } else {
            false
        }
    }
}

impl TxnIterator for MoveIter {
    type Item = ItemPtr;

    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        while {
            if let Some(curr) = self.iter.next() {
                let scope = self.stack.current_scope();
                let ctx = scope.map(|s| s.dest);
                if curr.moved == ctx {
                    // current item exists in the right (possibly none) move scope
                    if let Some(scope) = scope {
                        if scope.end == Some(curr) {
                            // we're at the end of the current scope
                            // while we still need to return current block ptr
                            // we can already descent in the move stack
                            self.escape_current_scope();
                            // we just returned to last active moved block ptr, we need to
                            // skip over it
                            self.iter.next();
                        }
                    }
                    if let ItemContent::Move(m) = &curr.content {
                        // we need to move to a new scope and reposition iterator at the start of it
                        let (start, end) = m.get_moved_coords(txn);
                        self.stack.push(MoveScope::new(start, end, curr));
                        self.iter = BlockIter(start);
                    } else {
                        return Some(curr);
                    }
                }

                true // continue to the next loop iteration
            } else {
                // if current iterator reached the end of the block sequence,
                // check if there are any remaining scopes on move stack and
                // if so, reposition iterator to their return address and retry
                let escaped = self.escape_current_scope();
                self.iter.next();
                escaped
            }
        } {}
        None
    }
}

impl TxnDoubleEndedIterator for MoveIter {
    fn next_back<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        while {
            if let Some(curr) = self.iter.next_back() {
                let scope = self.stack.current_scope();
                if curr.moved == scope.map(|s| s.dest) {
                    // current item exists in the right (possibly none) move scope
                    if let Some(scope) = scope {
                        if scope.start == Some(curr) {
                            // we're at the start of the current scope
                            // while we still need to return current block ptr
                            // we can already descent in the move stack
                            self.escape_current_scope();
                            // we just returned to last active moved block ptr, we need to
                            // skip over it
                            self.iter.next_back();
                        }
                    }
                    if let ItemContent::Move(m) = &curr.content {
                        // we need to move to a new scope and reposition iterator at the end of it
                        let (start, end) = m.get_moved_coords(txn);
                        self.stack.push(MoveScope::new(start, end, curr));
                        self.iter = BlockIter(end);
                    } else {
                        return Some(curr);
                    }
                }

                true // continue to the next loop iteration
            } else {
                // if current iterator reached the end of the block sequence,
                // check if there are any remaining scopes on move stack and
                // if so, reposition iterator to their return address and retry
                let escaped = self.escape_current_scope();
                self.iter.next_back();
                escaped
            }
        } {}
        None
    }
}

#[derive(Debug, Default)]
struct MoveStack(Option<Vec<MoveScope>>);

impl MoveStack {
    /// Returns a current scope of move operation.
    /// If `None`, it means that currently iterated elements were not moved anywhere.
    /// Otherwise, we are iterating over consecutive range of elements that have been
    /// relocated.
    fn current_scope(&self) -> Option<&MoveScope> {
        if let Some(stack) = &self.0 {
            stack.last()
        } else {
            None
        }
    }

    /// Pushes a new scope on top of current move stack. This happens when we touched
    /// a new block that contains a move content.
    fn push(&mut self, scope: MoveScope) {
        let stack = self.0.get_or_insert_with(Vec::default);
        stack.push(scope);
    }

    /// Removes the latest scope from the move stack. Usually done when we detected that
    /// iterator reached the boundary of a move scope and we need to go back to the
    /// original destination.
    fn pop(&mut self) -> Option<MoveScope> {
        if let Some(stack) = &mut self.0 {
            stack.pop()
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct MoveScope {
    /// First block moved in this scope.
    start: Option<ItemPtr>,
    /// Last block moved in this scope.
    end: Option<ItemPtr>,
    /// A.k.a. return address for the move range. Block pointer where the (start, end)
    /// range has been moved. It always contains an item with move content.
    dest: ItemPtr,
}

impl MoveScope {
    fn new(start: Option<ItemPtr>, end: Option<ItemPtr>, dest: ItemPtr) -> Self {
        MoveScope { start, end, dest }
    }
}

#[derive(Debug)]
pub(crate) struct BlockSlices<I>(I)
where
    I: TxnIterator<Item = ItemPtr> + Sized;

impl<I> TxnIterator for BlockSlices<I>
where
    I: TxnIterator<Item = ItemPtr> + Sized,
{
    type Item = ItemSlice;

    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        let ptr = self.0.next(txn)?;
        Some(ptr.into())
    }
}

impl<I> TxnDoubleEndedIterator for BlockSlices<I>
where
    I: TxnDoubleEndedIterator<Item = ItemPtr>,
{
    fn next_back<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        let ptr = self.0.next_back(txn)?;
        Some(ptr.into())
    }
}

/// Iterator over a slice of a continuous sequence of blocks.
#[derive(Debug)]
pub(crate) struct RangeIter<I> {
    iter: I,
    start: StickyIndex,
    end: StickyIndex,
    state: RangeIterState,
}

impl<I> RangeIter<I>
where
    I: TxnIterator<Item = ItemPtr>,
{
    fn new(iter: I, start: StickyIndex, end: StickyIndex) -> Self {
        RangeIter {
            iter,
            start,
            end,
            state: RangeIterState::Opened,
        }
    }

    fn begin<T: ReadTxn>(&mut self, txn: &T, start_offset: &mut u32) -> Option<ItemPtr> {
        let mut offset = 0;
        let mut curr = self.iter.next(txn);
        while let Some(ptr) = curr {
            match self.start.id() {
                None => {
                    self.state = RangeIterState::InRange;
                    break;
                }
                Some(start) if ptr.contains(start) => {
                    self.state = RangeIterState::InRange;
                    offset = start.clock - ptr.id().clock;
                    if self.start.assoc == Assoc::After {
                        // we need to skip first element in a block
                        offset += 1;
                        if offset == ptr.len() {
                            // we're at the last element in a block, move to next block
                            offset = 0;
                            curr = self.iter.next(txn);
                        }
                    }
                    break;
                }
                _ => curr = self.iter.next(txn),
            }
        }
        *start_offset = offset;
        curr
    }
}

impl<I> TxnIterator for RangeIter<I>
where
    I: TxnIterator<Item = ItemPtr>,
{
    type Item = ItemSlice;

    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        let mut start_offset = 0;
        let ptr = match self.state {
            RangeIterState::Opened => {
                // we just opened an iterator, we might not yet be in range
                self.begin(txn, &mut start_offset)
            }
            RangeIterState::InRange => self.iter.next(txn),
            RangeIterState::Closed => return None,
        }?;
        let end_offset = match self.end.id() {
            Some(end) if ptr.contains(end) => {
                self.state = RangeIterState::Closed;
                let mut offset = end.clock - ptr.id().clock;
                if self.end.assoc == Assoc::Before {
                    // we need to exclude last element
                    if offset == start_offset {
                        // last slice is empty - has no elements
                        return None;
                    }
                    offset -= 1;
                }
                offset
            }
            _ => ptr.len() - 1,
        };
        Some(ItemSlice::new(ptr, start_offset, end_offset))
    }
}

impl<I> TxnDoubleEndedIterator for RangeIter<I>
where
    I: TxnDoubleEndedIterator<Item = ItemPtr>,
{
    fn next_back<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        let mut end_offset = 0;
        let ptr = match self.state {
            RangeIterState::InRange => self.iter.next_back(txn),
            RangeIterState::Opened => {
                let mut curr = self.iter.next_back(txn);
                while let Some(ptr) = curr {
                    end_offset = ptr.len();
                    match self.end.id() {
                        None => {
                            self.state = RangeIterState::InRange;
                            break;
                        }
                        Some(end) if ptr.contains(end) => {
                            self.state = RangeIterState::InRange;
                            end_offset = end.clock - ptr.id().clock;
                            break;
                        }
                        _ => curr = self.iter.next_back(txn),
                    }
                }
                curr
            }
            RangeIterState::Closed => return None,
        }?;
        let start_offset = match self.start.id() {
            Some(start) if ptr.contains(start) => {
                self.state = RangeIterState::Closed;
                start.clock - ptr.id().clock
            }
            _ => 0,
        };
        Some(ItemSlice::new(ptr, start_offset, end_offset))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum RangeIterState {
    /// Ranged iterator position hasn't reach chosen ID range yet.
    Opened,
    /// Ranged iterator position is inside of a chosen ID range.
    InRange,
    /// Ranged iterator position moved beyond chosen ID range.
    Closed,
}

/// Iterator over particular, non-deleted values of a block sequence.
#[derive(Debug)]
pub(crate) struct Values<I> {
    iter: I,
    current: Option<ItemSlice>,
}

impl<I> Values<I> {
    fn new(iter: I) -> Self {
        Values {
            iter,
            current: None,
        }
    }
}

impl<I> TxnIterator for Values<I>
where
    I: TxnIterator<Item = ItemSlice>,
{
    type Item = Out;

    fn next<T: ReadTxn>(&mut self, txn: &T) -> Option<Self::Item> {
        loop {
            if let Some(slice) = &mut self.current {
                if slice.start <= slice.end {
                    let item = slice.ptr.deref();
                    if !item.is_deleted() {
                        let mut buf = [Out::default()];
                        let read = item.content.read(slice.start as usize, &mut buf);
                        if read != 0 {
                            slice.start += read as u32;
                            return Some(std::mem::take(&mut buf[0]));
                        }
                    }
                }
            }
            self.current = Some(self.iter.next(txn)?);
        }
    }

    fn collect<T, B>(&mut self, txn: &T) -> B
    where
        T: ReadTxn,
        B: Default + Extend<Self::Item>,
    {
        fn read_slice<B>(slice: ItemSlice, buf: &mut B)
        where
            B: Extend<Out>,
        {
            let item = slice.ptr.deref();
            if !item.is_deleted() {
                let size = slice.end - slice.start + 1;
                let mut b: SmallVec<[Out; 2]> = smallvec![Out::default(); size as usize];
                let _ = item.content.read(slice.start as usize, b.as_mut_slice());
                buf.extend(b);
            }
        }

        let mut buf = B::default();
        if let Some(slice) = self.current.take() {
            read_slice(slice, &mut buf);
        }
        while let Some(slice) = self.iter.next(txn) {
            read_slice(slice, &mut buf);
        }
        buf
    }
}

#[derive(Debug)]
pub struct AsIter<'a, T, I> {
    txn: &'a T,
    iter: I,
}

impl<'a, T, I> AsIter<'a, T, I>
where
    T: ReadTxn,
    I: TxnIterator,
{
    pub fn new(iter: I, txn: &'a T) -> Self {
        AsIter { txn, iter }
    }
}

impl<'a, T, I> Iterator for AsIter<'a, T, I>
where
    T: ReadTxn,
    I: TxnIterator,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next(self.txn)
    }
}

#[cfg(test)]
mod test {
    use crate::iter::{BlockIterator, BlockSliceIterator, IntoBlockIter, TxnIterator};
    use crate::test_utils::exchange_updates;
    use crate::{Array, Assoc, Doc, StickyIndex, Transact, ID};

    #[test]
    fn move_last_elem_iter() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");
        let mut txn = doc.transact_mut();
        array.insert_range(&mut txn, 0, [1, 2, 3]);
        drop(txn);

        let mut txn = doc.transact_mut();
        array.move_to(&mut txn, 2, 0);

        let start = array.as_ref().start;
        let mut i = start.to_iter().moved().slices().values();
        assert_eq!(i.next(&txn), Some(3.into()));
        assert_eq!(i.next(&txn), Some(1.into()));
        assert_eq!(i.next(&txn), Some(2.into()));
        assert_eq!(i.next(&txn), None);
    }

    #[test]
    fn move_1() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        {
            let mut txn = d1.transact_mut();
            a1.insert_range(&mut txn, 0, [1, 2, 3]);
            a1.move_to(&mut txn, 1, 0);
        }
        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), Some(3.into()));
            assert_eq!(i.next(&txn), None);
        }

        exchange_updates(&[&d1, &d2]);
        {
            let txn = d2.transact();
            let mut i = a2.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), Some(3.into()));
            assert_eq!(i.next(&txn), None);
        }

        a1.move_to(&mut d1.transact_mut(), 0, 2);

        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), Some(3.into()));
            assert_eq!(i.next(&txn), None);
        }
    }

    #[test]
    fn move_2() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        a1.insert_range(&mut d1.transact_mut(), 0, [1, 2]);
        a1.move_to(&mut d1.transact_mut(), 1, 0);
        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), None);
        }

        exchange_updates(&[&d1, &d2]);

        {
            let txn = d2.transact();
            let mut i = a2.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), None);
        }

        a1.move_to(&mut d1.transact_mut(), 0, 2);
        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), None);
        }
    }

    #[test]
    fn move_cycles() {
        let d1 = Doc::with_client_id(1);
        let a1 = d1.get_or_insert_array("array");

        let d2 = Doc::with_client_id(2);
        let a2 = d2.get_or_insert_array("array");

        a1.insert_range(&mut d1.transact_mut(), 0, [1, 2, 3, 4]);
        exchange_updates(&[&d1, &d2]);

        a1.move_range_to(&mut d1.transact_mut(), 0, Assoc::After, 1, Assoc::Before, 3);
        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(3.into()));
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), Some(4.into()));
            assert_eq!(i.next(&txn), None);
        }

        a2.move_range_to(&mut d2.transact_mut(), 2, Assoc::After, 3, Assoc::Before, 1);
        {
            let txn = d2.transact();
            let mut i = a2.as_ref().start.to_iter().moved().slices().values();
            assert_eq!(i.next(&txn), Some(1.into()));
            assert_eq!(i.next(&txn), Some(3.into()));
            assert_eq!(i.next(&txn), Some(4.into()));
            assert_eq!(i.next(&txn), Some(2.into()));
            assert_eq!(i.next(&txn), None);
        }

        exchange_updates(&[&d1, &d2]);
        exchange_updates(&[&d1, &d2]); // move cycles may not be detected within a single update exchange

        let t1 = d1.transact();
        let v1: Vec<_> = a1
            .as_ref()
            .start
            .to_iter()
            .moved()
            .slices()
            .values()
            .collect(&t1);
        let t2 = d2.transact();
        let v2: Vec<_> = a2
            .as_ref()
            .start
            .to_iter()
            .moved()
            .slices()
            .values()
            .collect(&t2);

        assert_eq!(v1, v2);
    }

    #[test]
    fn range_bounded() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(1, 4), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![3.into(), 4.into(), 5.into()])
    }

    #[test]
    fn range_left_exclusive() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 1), Assoc::After);
        let to = StickyIndex::from_id(ID::new(1, 4), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![4.into(), 5.into()])
    }

    #[test]
    fn range_left_exclusive_2() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 2), Assoc::After);
        let to = StickyIndex::from_id(ID::new(1, 4), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![5.into()])
    }

    #[test]
    fn range_right_exclusive() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(1, 5), Assoc::Before);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![3.into(), 4.into(), 5.into()])
    }

    #[test]
    fn range_right_exclusive_2() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(1, 4), Assoc::Before);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![3.into(), 4.into()])
    }

    #[test]
    fn range_unbounded() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_type(&txn, &array, Assoc::Before);
        let to = StickyIndex::from_type(&txn, &array, Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(
            res,
            vec![1.into(), 2.into(), 3.into(), 4.into(), 5.into(), 6.into()]
        )
    }

    #[test]
    fn range_left_unbounded() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_type(&txn, &array, Assoc::Before);
        let to = StickyIndex::from_id(ID::new(1, 2), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![1.into(), 2.into(), 3.into(), 4.into()])
    }

    #[test]
    fn range_right_unbounded() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 2), Assoc::Before);
        let to = StickyIndex::from_type(&txn, &array, Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![4.into(), 5.into(), 6.into()])
    }

    #[test]
    fn range_single_slice() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(1, 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(1, 1), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved()
            .within_range(from, to)
            .values()
            .collect(&txn);
        assert_eq!(res, vec![3.into()])
    }
}
