use crate::block::{Block, BlockPtr, BlockSlice, ItemContent};
use crate::{ReadTxn, Value, ID};
use std::ops::Deref;

pub(crate) trait BlockIterator: Iterator<Item = BlockPtr> + Sized {
    #[inline]
    fn slices(self) -> BlockSlices<Self> {
        BlockSlices(self)
    }

    #[inline]
    fn within_range(self, from: Option<ID>, to: Option<ID>) -> RangeIter<Self> {
        RangeIter::new(self, from, to)
    }
}

impl<T> BlockIterator for T where T: Iterator<Item = BlockPtr> + Sized {}

pub(crate) trait BlockSliceIterator: Iterator<Item = BlockSlice> + Sized {
    #[inline]
    fn values(self) -> Values<Self> {
        Values::new(self)
    }
}

impl<T> BlockSliceIterator for T where T: Iterator<Item = BlockSlice> + Sized {}

pub trait IntoBlockIter {
    fn to_iter(self) -> BlockIter;
}

impl IntoBlockIter for Option<BlockPtr> {
    #[inline]
    fn to_iter(self) -> BlockIter {
        BlockIter(self)
    }
}

/// Iterator over [BlockPtr] references.
/// By default it iterates to the right side.
/// When reversed it iterates to the left side.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct BlockIter(Option<BlockPtr>);

impl BlockIter {
    #[inline]
    fn new(ptr: Option<BlockPtr>) -> Self {
        BlockIter(ptr)
    }

    pub fn moved<T: ReadTxn>(self, txn: &T) -> MoveIter<T> {
        MoveIter::new(txn, self)
    }
}

impl Iterator for BlockIter {
    type Item = BlockPtr;

    fn next(&mut self) -> Option<Self::Item> {
        let curr = self.0.take();
        if let Some(Block::Item(item)) = curr.as_deref() {
            self.0 = item.right;
        }
        curr
    }
}

impl DoubleEndedIterator for BlockIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        let curr = self.0.take();
        if let Some(Block::Item(item)) = curr.as_deref() {
            self.0 = item.left;
        }
        curr
    }
}

impl IntoIterator for BlockPtr {
    type Item = BlockPtr;
    type IntoIter = BlockIter;

    fn into_iter(self) -> Self::IntoIter {
        BlockIter::new(Some(self))
    }
}

/// Block iterator which acknowledges context of move operation and iterates
/// over blocks as they appear after move. It skips over the presence of move destination blocks.
#[derive(Debug)]
pub struct MoveIter<'a, T> {
    iter: BlockIter,
    txn: &'a T,
    stack: MoveStack,
}

impl<'a, T> MoveIter<'a, T>
where
    T: ReadTxn,
{
    pub(crate) fn new(txn: &'a T, iter: BlockIter) -> Self {
        MoveIter {
            iter,
            txn,
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

impl<'a, T> Iterator for MoveIter<'a, T>
where
    T: ReadTxn,
{
    type Item = BlockPtr;

    fn next(&mut self) -> Option<Self::Item> {
        while {
            if let Some(curr) = self.iter.next() {
                let ptr = curr.clone();
                if let Block::Item(item) = ptr.deref() {
                    let scope = self.stack.current_scope();
                    let ctx = scope.map(|s| s.dest);
                    if item.moved == ctx {
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
                        if let ItemContent::Move(m) = &item.content {
                            // we need to move to a new scope and reposition iterator at the start of it
                            let (start, end) = m.get_moved_coords(self.txn);
                            self.stack.push(MoveScope::new(start, end, curr));
                            self.iter = BlockIter(start);
                        } else {
                            return Some(curr);
                        }
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

impl<'a, T> DoubleEndedIterator for MoveIter<'a, T>
where
    T: ReadTxn,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        while {
            if let Some(curr) = self.iter.next_back() {
                let ptr = curr.clone();
                if let Block::Item(item) = ptr.deref() {
                    let scope = self.stack.current_scope();
                    if item.moved == scope.map(|s| s.dest) {
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
                        if let ItemContent::Move(m) = &item.content {
                            // we need to move to a new scope and reposition iterator at the end of it
                            let (start, end) = m.get_moved_coords(self.txn);
                            self.stack.push(MoveScope::new(start, end, curr));
                            self.iter = BlockIter(end);
                        } else {
                            return Some(curr);
                        }
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
    start: Option<BlockPtr>,
    /// Last block moved in this scope.
    end: Option<BlockPtr>,
    /// A.k.a. return address for the move range. Block pointer where the (start, end)
    /// range has been moved. It always contains an item with move content.
    dest: BlockPtr,
}

impl MoveScope {
    fn new(start: Option<BlockPtr>, end: Option<BlockPtr>, dest: BlockPtr) -> Self {
        MoveScope { start, end, dest }
    }
}

#[derive(Debug)]
pub(crate) struct BlockSlices<I>(I)
where
    I: Iterator<Item = BlockPtr> + Sized;

impl<I> Iterator for BlockSlices<I>
where
    I: Iterator<Item = BlockPtr> + Sized,
{
    type Item = BlockSlice;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = self.0.next()?;
        Some(ptr.into())
    }
}

impl<I> DoubleEndedIterator for BlockSlices<I>
where
    I: DoubleEndedIterator<Item = BlockPtr>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        let ptr = self.0.next_back()?;
        Some(ptr.into())
    }
}

/// Iterator over a slice of a continuous sequence of blocks.
#[derive(Debug)]
pub(crate) struct RangeIter<I> {
    iter: I,
    start: Option<ID>,
    end: Option<ID>,
    state: RangeIterState,
}

impl<I> RangeIter<I>
where
    I: Iterator<Item = BlockPtr>,
{
    fn new(iter: I, start: Option<ID>, end: Option<ID>) -> Self {
        RangeIter {
            iter,
            start,
            end,
            state: RangeIterState::Opened,
        }
    }
}

impl<I> Iterator for RangeIter<I>
where
    I: Iterator<Item = BlockPtr>,
{
    type Item = BlockSlice;

    fn next(&mut self) -> Option<Self::Item> {
        let mut start_offset = 0;
        let ptr = match self.state {
            RangeIterState::InRange => self.iter.next(),
            RangeIterState::Opened => {
                let mut curr = self.iter.next();
                while let Some(ptr) = curr {
                    match &self.start {
                        None => {
                            self.state = RangeIterState::InRange;
                            break;
                        }
                        Some(start) if ptr.contains(start) => {
                            self.state = RangeIterState::InRange;
                            start_offset = start.clock - ptr.id().clock;
                            break;
                        }
                        _ => curr = self.iter.next(),
                    }
                }
                curr
            }
            RangeIterState::Closed => return None,
        }?;
        let end_offset = match &self.end {
            Some(end) if ptr.contains(end) => {
                self.state = RangeIterState::Closed;
                end.clock - ptr.id().clock
            }
            _ => ptr.len() - 1,
        };
        Some(BlockSlice::new(ptr, start_offset, end_offset))
    }
}

impl<I> DoubleEndedIterator for RangeIter<I>
where
    I: DoubleEndedIterator<Item = BlockPtr>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        let mut end_offset = 0;
        let ptr = match self.state {
            RangeIterState::InRange => self.iter.next_back(),
            RangeIterState::Opened => {
                let mut curr = self.iter.next_back();
                while let Some(ptr) = curr {
                    end_offset = ptr.len();
                    match &self.end {
                        None => {
                            self.state = RangeIterState::InRange;
                            break;
                        }
                        Some(end) if ptr.contains(end) => {
                            self.state = RangeIterState::InRange;
                            end_offset = end.clock - ptr.id().clock;
                            break;
                        }
                        _ => curr = self.iter.next_back(),
                    }
                }
                curr
            }
            RangeIterState::Closed => return None,
        }?;
        let start_offset = match &self.start {
            Some(start) if ptr.contains(start) => {
                self.state = RangeIterState::Closed;
                start.clock - ptr.id().clock
            }
            _ => 0,
        };
        Some(BlockSlice::new(ptr, start_offset, end_offset))
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
    current: Option<BlockSlice>,
}

impl<I> Values<I>
where
    I: Iterator<Item = BlockSlice>,
{
    fn new(iter: I) -> Self {
        Values {
            iter,
            current: None,
        }
    }
}

impl<I> Iterator for Values<I>
where
    I: Iterator<Item = BlockSlice>,
{
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(slice) = &mut self.current {
                if slice.start <= slice.end {
                    if let Block::Item(item) = slice.ptr.deref() {
                        if !item.is_deleted() {
                            let mut buf = [Value::default()];
                            let read = item.content.read(slice.start as usize, &mut buf);
                            if read != 0 {
                                slice.start += read as u32;
                                return Some(std::mem::take(&mut buf[0]));
                            }
                        }
                    }
                }
            }
            self.current = Some(self.iter.next()?);
        }
    }
}

#[cfg(test)]
mod test {
    use crate::iter::{BlockIterator, BlockSliceIterator, IntoBlockIter};
    use crate::test_utils::exchange_updates;
    use crate::{Array, Assoc, Doc, Transact, ID};

    #[test]
    fn move_last_elem_iter() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");
        let mut txn = array.transact_mut();
        array.insert_range(&mut txn, 0, [1, 2, 3]);
        drop(txn);

        let mut txn = array.transact_mut();
        array.move_to(&mut txn, 2, 0);

        let start = array.as_ref().start;
        let mut i = start.to_iter().moved(&txn).slices().values();
        assert_eq!(i.next(), Some(3.into()));
        assert_eq!(i.next(), Some(1.into()));
        assert_eq!(i.next(), Some(2.into()));
        assert_eq!(i.next(), None);
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
            let mut i = a1.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), Some(3.into()));
            assert_eq!(i.next(), None);
        }

        exchange_updates(&[&d1, &d2]);
        {
            let txn = d2.transact();
            let mut i = a2.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), Some(3.into()));
            assert_eq!(i.next(), None);
        }

        a1.move_to(&mut d1.transact_mut(), 0, 2);

        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), Some(3.into()));
            assert_eq!(i.next(), None);
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
            let mut i = a1.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), None);
        }

        exchange_updates(&[&d1, &d2]);

        {
            let txn = d2.transact();
            let mut i = a2.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), None);
        }

        a1.move_to(&mut d1.transact_mut(), 0, 2);
        {
            let txn = d1.transact();
            let mut i = a1.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), None);
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
            let mut i = a1.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(3.into()));
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), Some(4.into()));
            assert_eq!(i.next(), None);
        }

        a2.move_range_to(&mut d2.transact_mut(), 2, Assoc::After, 3, Assoc::Before, 1);
        {
            let txn = d2.transact();
            let mut i = a2.as_ref().start.to_iter().moved(&txn).slices().values();
            assert_eq!(i.next(), Some(1.into()));
            assert_eq!(i.next(), Some(3.into()));
            assert_eq!(i.next(), Some(4.into()));
            assert_eq!(i.next(), Some(2.into()));
            assert_eq!(i.next(), None);
        }

        exchange_updates(&[&d1, &d2]);
        exchange_updates(&[&d1, &d2]); // move cycles may not be detected within a single update exchange

        let t1 = d1.transact();
        let v1: Vec<_> = a1
            .as_ref()
            .start
            .to_iter()
            .moved(&t1)
            .slices()
            .values()
            .collect();
        let t2 = d2.transact();
        let v2: Vec<_> = a2
            .as_ref()
            .start
            .to_iter()
            .moved(&t2)
            .slices()
            .values()
            .collect();

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
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved(&txn)
            .within_range(Some(ID::new(1, 1)), Some(ID::new(1, 4)))
            .values()
            .collect();
        assert_eq!(res, vec![3.into(), 4.into(), 5.into()])
    }

    #[test]
    fn range_unbounded() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved(&txn)
            .within_range(None, None)
            .values()
            .collect();
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
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved(&txn)
            .within_range(None, Some(ID::new(1, 2)))
            .values()
            .collect();
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
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved(&txn)
            .within_range(Some(ID::new(1, 2)), None)
            .values()
            .collect();
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
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .moved(&txn)
            .within_range(Some(ID::new(1, 1)), Some(ID::new(1, 1)))
            .values()
            .collect();
        assert_eq!(res, vec![3.into()])
    }
}
