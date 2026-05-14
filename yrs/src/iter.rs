use crate::block::{ItemContent, ItemPtr};
use crate::slice::ItemSlice;
use crate::{Assoc, Out, ReadTxn, StickyIndex};
use smallvec::{smallvec, SmallVec};
use std::ops::Deref;

pub(crate) trait BlockIterator: Iterator<Item = ItemPtr> + Sized {
    #[inline]
    fn within_range(self, from: StickyIndex, to: StickyIndex) -> RangeIter<Self> {
        RangeIter::new(self, from, to)
    }
}

impl<T> BlockIterator for T where T: Iterator<Item = ItemPtr> + Sized {}

pub(crate) trait BlockSliceIterator: Iterator<Item = ItemSlice> + Sized {
    #[inline]
    fn values(self) -> Values<Self> {
        Values::new(self)
    }
}

impl<T> BlockSliceIterator for T where T: Iterator<Item = ItemSlice> + Sized {}

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
    pub fn new(ptr: Option<ItemPtr>) -> Self {
        BlockIter(ptr)
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
    I: Iterator<Item = ItemPtr>,
{
    fn new(iter: I, start: StickyIndex, end: StickyIndex) -> Self {
        RangeIter {
            iter,
            start,
            end,
            state: RangeIterState::Opened,
        }
    }

    fn begin(&mut self, start_offset: &mut u32) -> Option<ItemPtr> {
        let mut offset = 0;
        let mut curr = self.iter.next();
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
                            curr = self.iter.next();
                        }
                    }
                    break;
                }
                _ => curr = self.iter.next(),
            }
        }
        *start_offset = offset;
        curr
    }
}

impl<I> Iterator for RangeIter<I>
where
    I: Iterator<Item = ItemPtr>,
{
    type Item = ItemSlice;

    fn next(&mut self) -> Option<Self::Item> {
        let mut start_offset = 0;
        let ptr = match self.state {
            RangeIterState::Opened => {
                // we just opened an iterator, we might not yet be in range
                self.begin(&mut start_offset)
            }
            RangeIterState::InRange => self.iter.next(),
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

impl<I> DoubleEndedIterator for RangeIter<I>
where
    I: DoubleEndedIterator<Item = ItemPtr>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        let mut end_offset = 0;
        let ptr = match self.state {
            RangeIterState::InRange => self.iter.next_back(),
            RangeIterState::Opened => {
                let mut curr = self.iter.next_back();
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
                        _ => curr = self.iter.next_back(),
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

impl<I> Iterator for Values<I>
where
    I: Iterator<Item = ItemSlice>,
{
    type Item = Out;

    fn next(&mut self) -> Option<Self::Item> {
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
            self.current = Some(self.iter.next()?);
        }
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
    I: Iterator,
{
    pub fn new(iter: I, txn: &'a T) -> Self {
        AsIter { txn, iter }
    }
}

impl<'a, T, I> Iterator for AsIter<'a, T, I>
where
    T: ReadTxn,
    I: Iterator,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

#[cfg(test)]
mod test {
    use crate::block::ClientID;
    use crate::iter::{BlockIterator, BlockSliceIterator, IntoBlockIter, TxnIterator};
    use crate::test_utils::exchange_updates;
    use crate::{Array, Assoc, Doc, StickyIndex, Transact, ID};

    #[test]
    fn range_bounded() {
        let doc = Doc::with_client_id(1);
        let array = doc.get_or_insert_array("array");

        array.insert_range(&mut doc.transact_mut(), 0, [2, 3, 4]);
        array.insert_range(&mut doc.transact_mut(), 0, [1]);
        array.insert_range(&mut doc.transact_mut(), 4, [5, 6]);

        let txn = doc.transact();
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 4), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
            .values()
            .collect();
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
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 1), Assoc::After);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 4), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
            .values()
            .collect();
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
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 2), Assoc::After);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 4), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
            .values()
            .collect();
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
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 5), Assoc::Before);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
            .values()
            .collect();
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
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 4), Assoc::Before);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
            .values()
            .collect();
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
            .within_range(from, to)
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
        let from = StickyIndex::from_type(&txn, &array, Assoc::Before);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 2), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
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
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 2), Assoc::Before);
        let to = StickyIndex::from_type(&txn, &array, Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
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
        let from = StickyIndex::from_id(ID::new(ClientID::new(1), 1), Assoc::Before);
        let to = StickyIndex::from_id(ID::new(ClientID::new(1), 1), Assoc::After);
        let res: Vec<_> = array
            .as_ref()
            .start
            .to_iter()
            .within_range(from, to)
            .values()
            .collect();
        assert_eq!(res, vec![3.into()])
    }
}
