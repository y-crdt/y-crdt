use crate::block::{
    Block, BlockPtr, Item, ItemContent, Skip, BLOCK_GC_REF_NUMBER, BLOCK_SKIP_REF_NUMBER, GC,
    HAS_ORIGIN, HAS_PARENT_SUB, HAS_RIGHT_ORIGIN,
};
use crate::id_set::DeleteSet;
#[cfg(test)]
use crate::store::Store;
use crate::types::TypePtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::{StateVector, Transaction, ID};
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::rc::Rc;

#[derive(Debug, PartialEq, Default)]
pub(crate) struct UpdateBlocks {
    clients: HashMap<u64, VecDeque<Block>, BuildHasherDefault<ClientHasher>>,
}

impl UpdateBlocks {
    /**
    @todo this should be refactored.
    I'm currently using this to add blocks to the Update
    */
    pub(crate) fn add_block(&mut self, block: Block) {
        let e = self.clients.entry(block.id().client).or_default();
        e.push_back(block);
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Returns an iterator that allows a traversal of all of the blocks
    /// which consist into this [Update].
    pub(crate) fn blocks(&self) -> Blocks<'_> {
        Blocks::new(self)
    }

    /// Returns an iterator that allows a traversal of all of the blocks
    /// which consist into this [Update].
    pub(crate) fn into_blocks(self) -> IntoBlocks {
        IntoBlocks::new(self)
    }

    fn split_item(&mut self, client: u64, mut index: usize, diff: u32) {
        let mut blocks = self.clients.get_mut(&client).unwrap();
        if let Block::Item(item) = &mut blocks[index] {
            index += 1;
            let right_split = item.split(diff);
            let right_ptr = right_split.right.clone();
            if let Some(right_ptr) = right_ptr {
                blocks = if right_ptr.id.client == client {
                    blocks
                } else {
                    self.clients.get_mut(&right_ptr.id.client).unwrap()
                };
                let right = &mut blocks[right_ptr.pivot()];
                if let Some(right_item) = right.as_item_mut() {
                    right_item.left = Some(BlockPtr::new(right_split.id.clone(), index as u32));
                }
            }
            blocks.insert(index, Block::Item(right_split));
        };
    }
}

impl std::fmt::Display for UpdateBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{{")?;
        for (client, blocks) in self.clients.iter() {
            writeln!(f, "\t{} -> [", client)?;
            for block in blocks {
                writeln!(f, "\t\t{}", block)?;
            }
            write!(f, "\t]")?;
        }
        writeln!(f, "}}")
    }
}

/// Update type which contains an information about all decoded blocks which are incoming from a
/// remote peer. Since these blocks are not yet integrated into current document's block store,
/// they still may require repairing before doing so as they don't contain full data about their
/// relations.
///
/// Update is conceptually similar to a block store itself, however the work patters are different.
#[derive(Debug, PartialEq, Default)]
pub struct Update {
    pub(crate) blocks: UpdateBlocks,
    pub(crate) delete_set: DeleteSet,
}

impl Update {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.delete_set.is_empty()
    }

    /// Returns a state vector representing an upper bound of client clocks included by blocks
    /// stored in current update.
    pub fn state_vector(&self) -> StateVector {
        let mut sv = StateVector::default();
        for (&client, blocks) in self.blocks.clients.iter() {
            let last_id = blocks[blocks.len() - 1].last_id();
            sv.set_max(client, last_id.clock + 1);
        }
        sv
    }

    /// Merges another update into current one. Their blocks are deduplicated and reordered.
    pub fn merge(&mut self, other: Self) {
        for (client, other_blocks) in other.blocks.clients {
            match self.blocks.clients.entry(client) {
                Entry::Occupied(e) => {
                    let mut blocks = e.into_mut();
                    let mut i2 = other_blocks.into_iter();
                    let mut n2 = i2.next();

                    let mut i1 = 0;

                    while i1 < blocks.len() {
                        let a = &mut blocks[i1];
                        if let Some(b) = n2.as_ref() {
                            if a.try_squash(b) {
                                n2 = i2.next();
                                continue;
                            } else if let Block::Item(a) = a {
                                // we only can split Block::Item
                                let diff = (a.id.clock + a.len()) as isize - b.id().clock as isize;
                                if diff > 0 {
                                    // `b`'s clock position is inside of `a` -> we need to split `a`
                                    self.blocks.split_item(client, i1, diff as u32);
                                    blocks = self.blocks.clients.get_mut(&client).unwrap();
                                }
                            }
                            i1 += 1;
                            n2 = i2.next();
                        } else {
                            break;
                        }
                    }

                    while let Some(b) = n2 {
                        blocks.push_back(b);
                        n2 = i2.next();
                    }
                }
                Entry::Vacant(e) => {
                    e.insert(other_blocks);
                }
            }
        }
    }

    /// Integrates current update into a block store referenced by a given transaction.
    /// If entire integration process was successful a `None` value is returned. Otherwise a
    /// pending update object is returned which contains blocks that couldn't be integrated, most
    /// likely because there were missing blocks that are used as a dependencies of other blocks
    /// contained in this update.
    pub fn integrate(mut self, txn: &mut Transaction) -> (Option<PendingUpdate>, Option<Update>) {
        let remaining_blocks = if self.blocks.is_empty() {
            None
        } else {
            let mut store = txn.store_mut();
            let mut client_block_ref_ids: Vec<u64> = self.blocks.clients.keys().cloned().collect();
            client_block_ref_ids.sort_by(|a, b| b.cmp(a));

            let mut current_client_id = client_block_ref_ids.pop();
            let mut current_target =
                current_client_id.and_then(|id| self.blocks.clients.get_mut(&id));
            let mut stack_head = Self::next(&mut current_target);

            let mut local_sv = store.blocks.get_state_vector();
            let mut missing_sv = StateVector::default();
            let mut remaining = UpdateBlocks::default();
            let mut stack = Vec::new();

            while let Some(mut block) = stack_head {
                let id = block.id();
                if local_sv.contains(id) {
                    let offset = local_sv.get(&id.client) as i32 - id.clock as i32;
                    if let Some(dep) = Self::missing(&block, &local_sv) {
                        stack.push(block);
                        // get the struct reader that has the missing struct
                        let block_refs = store.blocks.get_client_blocks_mut(dep);
                        if block_refs.integrated_len() == block_refs.len() {
                            // This update message causally depends on another update message that doesn't exist yet
                            missing_sv.set_min(dep, local_sv.get(&dep));
                            Self::return_stack(stack, &mut self.blocks, &mut remaining);
                            current_target =
                                current_client_id.and_then(|id| self.blocks.clients.get_mut(&id));
                            stack = Vec::new();
                        } else {
                            stack_head = Self::next(&mut current_target);
                            continue;
                        }
                    } else if offset == 0 || (offset as u32) < block.len() {
                        let offset = offset as u32;
                        let client = id.client;
                        local_sv.set_max(client, id.clock + block.len());
                        block.as_item_mut().map(|item| item.repair(store));
                        let should_delete = block.integrate(txn, offset, offset);
                        let delete_ptr = if should_delete {
                            Some(BlockPtr::new(block.id().clone(), offset))
                        } else {
                            None
                        };

                        store = txn.store_mut();
                        let blocks = store.blocks.get_client_blocks_mut(client);
                        blocks.push(block);

                        if let Some(ptr) = delete_ptr {
                            txn.delete(&ptr);
                        }
                        store = txn.store_mut();
                    }
                } else {
                    // update from the same client is missing
                    stack.push(block);
                    // hid a dead wall, add all items from stack to restSS
                    Self::return_stack(stack, &mut self.blocks, &mut remaining);
                    current_target =
                        current_client_id.and_then(|id| self.blocks.clients.get_mut(&id));
                    stack = Vec::new();
                }

                // iterate to next stackHead
                if !stack.is_empty() {
                    stack_head = stack.pop();
                } else {
                    current_target = match current_target.take() {
                        None => None,
                        Some(v) => {
                            if !v.is_empty() {
                                Some(v)
                            } else {
                                if let Some((client_id, target)) =
                                    Self::next_target(&mut client_block_ref_ids, &mut self.blocks)
                                {
                                    current_client_id = Some(client_id);
                                    Some(target)
                                } else {
                                    current_client_id = None;
                                    None
                                }
                            }
                        }
                    };
                    stack_head = Self::next(&mut current_target);
                }
            }
            if remaining.is_empty() {
                None
            } else {
                Some(PendingUpdate {
                    update: Update {
                        blocks: remaining,
                        delete_set: DeleteSet::new(),
                    },
                    missing: missing_sv,
                })
            }
        };

        let remaining_ds = txn.apply_delete(&self.delete_set).map(|ds| {
            let mut update = Update::new();
            update.delete_set = ds;
            update
        });
        return (remaining_blocks, remaining_ds);
    }

    fn next(target: &mut Option<&mut VecDeque<Block>>) -> Option<Block> {
        if let Some(v) = target {
            v.pop_front()
        } else {
            None
        }
    }

    fn missing(block: &Block, local_sv: &StateVector) -> Option<u64> {
        if let Block::Item(item) = block {
            if let Some(origin) = item.origin {
                if origin.client != item.id.client && !local_sv.contains(&item.id) {
                    return Some(origin.client);
                }
            } else if let Some(right_origin) = item.right_origin {
                if right_origin.client != item.id.client && !local_sv.contains(&item.id) {
                    return Some(right_origin.client);
                }
            } else if let TypePtr::Id(parent) = item.parent {
                if parent.id.client != item.id.client && !local_sv.contains(&item.id) {
                    return Some(parent.id.client);
                }
            }
        }
        None
    }

    fn next_target<'a, 'b>(
        client_block_ref_ids: &'a mut Vec<u64>,
        blocks: &'b mut UpdateBlocks,
    ) -> Option<(u64, &'b mut VecDeque<Block>)> {
        loop {
            if let Some((id, Some(client_blocks))) = client_block_ref_ids
                .pop()
                .map(move |id| (id, blocks.clients.get_mut(&id)))
            {
                if !client_blocks.is_empty() {
                    return Some((id, client_blocks));
                }
            }

            break;
        }
        None
    }

    fn return_stack(stack: Vec<Block>, refs: &mut UpdateBlocks, remaining: &mut UpdateBlocks) {
        for item in stack.into_iter() {
            let client = item.id().client;
            // remove client from clientsStructRefsIds to prevent users from applying the same update again
            if let Some(mut unapplicable_items) = refs.clients.remove(&client) {
                // decrement because we weren't able to apply previous operation
                unapplicable_items.push_front(item);
                remaining.clients.insert(client, unapplicable_items);
            } else {
                // item was the last item on clientsStructRefs and the field was already cleared.
                // Add item to restStructs and continue
                let mut blocks = VecDeque::with_capacity(1);
                blocks.push_back(item);
                remaining.clients.insert(client, blocks);
            }
        }
    }

    fn decode_block<D: Decoder>(id: ID, decoder: &mut D) -> Block {
        let info = decoder.read_info();
        match info {
            BLOCK_SKIP_REF_NUMBER => {
                let len: u32 = decoder.read_uvar();
                Block::Skip(Skip { id, len })
            }
            BLOCK_GC_REF_NUMBER => {
                let len: u32 = decoder.read_uvar();
                Block::GC(GC { id, len })
            }
            info => {
                let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                let origin = if info & HAS_ORIGIN != 0 {
                    Some(decoder.read_left_id())
                } else {
                    None
                };
                let right_origin = if info & HAS_RIGHT_ORIGIN != 0 {
                    Some(decoder.read_right_id())
                } else {
                    None
                };
                let parent = if cant_copy_parent_info {
                    if decoder.read_parent_info() {
                        TypePtr::Named(decoder.read_string().into())
                    } else {
                        TypePtr::Id(BlockPtr::from(decoder.read_left_id()))
                    }
                } else {
                    TypePtr::Unknown
                };
                let parent_sub: Option<Rc<str>> =
                    if cant_copy_parent_info && (info & HAS_PARENT_SUB != 0) {
                        Some(decoder.read_string().into())
                    } else {
                        None
                    };
                let content = ItemContent::decode(decoder, info, BlockPtr::from(id.clone()));
                let item = Item::new(
                    id,
                    None,
                    origin,
                    None,
                    right_origin,
                    parent,
                    parent_sub,
                    content,
                );
                Block::Item(item)
            }
        }
    }

    pub(crate) fn encode_diff<E: Encoder>(&self, remote_sv: &StateVector, encoder: &mut E) {
        let mut clients = HashMap::new();
        for (client, blocks) in self.blocks.clients.iter() {
            let remote_clock = remote_sv.get(client);
            let mut iter = blocks.iter();
            let mut curr = iter.next();
            while let Some(block) = curr {
                if let Block::Skip(_) = block {
                    curr = iter.next();
                } else if block.id().clock + block.len() > remote_clock {
                    let e = clients.entry(*client).or_insert_with(|| (0, Vec::new()));
                    e.0 = (remote_clock as i64 - block.id().clock as i64).max(0) as u32;
                    e.1.push(block);
                    curr = iter.next();
                    while let Some(block) = curr {
                        e.1.push(block);
                        curr = iter.next();
                    }
                } else {
                    // read until something new comes up
                    curr = iter.next();
                }
            }
        }

        // Write higher clients first ⇒ sort by clientID & clock and remove decoders without content
        let mut sorted_clients: Vec<_> =
            clients.iter().filter(|(_, (_, q))| !q.is_empty()).collect();
        sorted_clients.sort_by(|&(x_id, _), &(y_id, _)| y_id.cmp(x_id));

        // finish lazy struct writing
        encoder.write_uvar(sorted_clients.len());
        for (&client, (offset, blocks)) in sorted_clients {
            encoder.write_uvar(blocks.len());
            encoder.write_uvar(client);

            let mut block = blocks[0];
            encoder.write_uvar(block.id().clock + offset);
            block.encode_with_offset(encoder, *offset);
            for i in 1..blocks.len() {
                block = blocks[i];
                block.encode_with_offset(encoder, 0);
            }
        }
        self.delete_set.encode(encoder);
    }

    pub fn merge_updates<T>(block_stores: T) -> Update
    where
        T: IntoIterator<Item = Update>,
    {
        let mut result = Update::new();
        let update_blocks: Vec<UpdateBlocks> = block_stores
            .into_iter()
            .map(|update| {
                result.delete_set.merge(update.delete_set);
                update.blocks
            })
            .collect();

        let mut lazy_struct_decoders: VecDeque<_> = update_blocks
            .into_iter()
            .filter(|block_store| !block_store.is_empty())
            .map(|update_blocks| {
                let mut memo = update_blocks.into_blocks().memoized();
                memo.advance();
                memo
            })
            .collect();

        let mut curr_write: Option<Block> = None;

        // Note: We need to ensure that all lazyStructDecoders are fully consumed
        // Note: Should merge document updates whenever possible - even from different updates
        // Note: Should handle that some operations cannot be applied yet ()
        loop {
            {
                // sort
                lazy_struct_decoders
                    .make_contiguous()
                    .sort_by(|dec1, dec2| {
                        // Write higher clients first ⇒ sort by clientID & clock and remove decoders without content
                        let left = dec1.current().unwrap();
                        let right = dec2.current().unwrap();
                        let lid = left.id();
                        let rid = right.id();
                        match lid.client.cmp(&rid.client) {
                            Ordering::Equal => match lid.clock.cmp(&rid.clock) {
                                Ordering::Equal if left.same_type(right) => Ordering::Equal,
                                Ordering::Equal if left.is_skip() => Ordering::Greater,
                                Ordering::Equal => Ordering::Less,
                                Ordering::Less if !left.is_skip() || right.is_skip() => {
                                    Ordering::Less
                                }
                                ordering => ordering,
                            },
                            ordering => ordering,
                        }
                    });
            }

            if let Some(mut curr_decoder) = lazy_struct_decoders.pop_front() {
                let mut curr = curr_decoder.next();
                let mut first_client = 0;
                if let Some(curr_block) = curr.take() {
                    // write from currDecoder until the next operation is from another client or if filler-struct
                    // then we need to reorder the decoders and find the next operation to write
                    first_client = curr_block.id().client;
                    if let Some(mut curr_write_block) = curr_write.take() {
                        let mut iterated = false;

                        // iterate until we find something that we haven't written already
                        // remember: first the high client-ids are written
                        let curr_write_last = curr_write_block.id().clock + curr_write_block.len();
                        let mut forwarder = Some(curr_block);
                        while let Some(block) = forwarder.take() {
                            let last = block.id().clock + block.len();
                            if last < curr_write_last
                                && block.id().client >= curr_write_block.id().client
                            {
                                forwarder = curr_decoder.next();
                                iterated = true;
                            } else {
                                forwarder = Some(block);
                                break;
                            }
                        }

                        if let Some(mut curr_block) = forwarder.take() {
                            let cid = curr_block.id();
                            if cid.client != first_client || // check whether there is another decoder that has has updates from `firstClient`
                                (iterated && cid.clock > curr_write_last)
                            // the above while loop was used and we are potentially missing updates
                            {
                                continue;
                            }

                            if first_client != curr_write_block.id().client {
                                result.blocks.add_block(curr_write_block);
                                curr_write = Some(curr_block);
                                curr = curr_decoder.next();
                            } else {
                                if curr_write_last < curr_block.id().clock {
                                    //TODO: write currStruct & set currStruct = Skip(clock = currStruct.id.clock + currStruct.length, length = curr.id.clock - self.clock)
                                    let skip = if let Block::Skip(mut skip) = curr_write_block {
                                        // extend existing skip
                                        skip.len = curr_block.id().clock + curr_block.len()
                                            - skip.id.clock;
                                        skip
                                    } else {
                                        result.blocks.add_block(curr_write_block);
                                        let diff = curr_block.id().clock - curr_write_last;
                                        Skip::new(ID::new(first_client, curr_write_last), diff)
                                    };
                                    curr_write_block = Block::Skip(skip);
                                } else {
                                    // if (currWrite.struct.id.clock + currWrite.struct.length >= curr.id.clock) {
                                    let diff =
                                        curr_write_last as i32 - curr_block.id().clock as i32;

                                    if diff > 0 {
                                        if let Block::Skip(skip) = &mut curr_write_block {
                                            // prefer to slice Skip because the other struct might contain more information
                                            skip.len -= diff as u32;
                                        } else {
                                            curr_block = curr_block.slice(diff as u32).unwrap();
                                        }
                                    }

                                    if !curr_write_block.try_squash(&curr_block) {
                                        result.blocks.add_block(curr_write_block);
                                        curr_write = Some(curr_block);
                                        curr = curr_decoder.next();
                                    }
                                }
                            }
                        } else {
                            // current decoder is empty
                            continue;
                        }
                    } else {
                        curr_write = Some(curr_block);
                        curr = curr_decoder.next();
                    }
                } else {
                    continue;
                }

                while let Some(next) = curr.take() {
                    let block = curr_write.take().unwrap();
                    let nid = next.id();
                    if nid.client == first_client && nid.clock == block.id().clock + block.len() {
                        result.blocks.add_block(block);
                        curr_write = Some(next);
                        curr = curr_decoder.next();
                    } else {
                        break;
                    }
                }
            } else {
                break;
            }
        }

        if let Some(block) = curr_write.take() {
            result.blocks.add_block(block);
        }

        result
    }
}

impl Encode for Update {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.encode_diff(&StateVector::default(), encoder);
    }
}

impl Decode for Update {
    fn decode<D: Decoder>(decoder: &mut D) -> Self {
        // read blocks
        let clients_len: u32 = decoder.read_uvar();
        let mut blocks = UpdateBlocks {
            clients: HashMap::with_capacity_and_hasher(
                clients_len as usize,
                BuildHasherDefault::default(),
            ),
        };
        for _ in 0..clients_len {
            let blocks_len = decoder.read_uvar::<u32>() as usize;

            let client = decoder.read_client();
            let mut clock: u32 = decoder.read_uvar();
            let blocks = blocks
                .clients
                .entry(client)
                .or_insert_with(|| VecDeque::with_capacity(blocks_len));

            for _ in 0..blocks_len {
                let id = ID::new(client, clock);
                let block = Self::decode_block(id, decoder);
                clock += block.len();
                blocks.push_back(block);
            }
        }
        // read delete set
        let delete_set = DeleteSet::decode(decoder);
        Update { blocks, delete_set }
    }
}

/// Similar to [Peekable], but can be used in situation when [Peekable::peek] is not allowed
/// due to a lack of of `&mut self` reference. [Memo] can be proactively advanced using
/// [Memo::advance] which works similar to [Peekable::peek], but later peeked element can still be
/// accessed using [Memo::current] which doesn't require mutable reference.
struct Memo<I: Iterator + ?Sized> {
    current: Option<I::Item>,
    iter: I,
}

impl<I: Iterator> Memo<I> {
    fn current(&self) -> Option<&I::Item> {
        self.current.as_ref()
    }

    fn advance(&mut self) -> bool {
        match self.iter.next() {
            None => false,
            other => {
                self.current = other;
                true
            }
        }
    }
}

impl<I: Iterator> Iterator for Memo<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self.current.take() {
            None => self.iter.next(),
            Some(n) => Some(n),
        }
    }
}

trait Memoizable: Iterator {
    fn memoized(self) -> Memo<Self>;
}

impl<T: Iterator> Memoizable for T {
    fn memoized(self) -> Memo<Self> {
        Memo {
            iter: self,
            current: None,
        }
    }
}

/// A pending update which contains unapplied blocks from the update which created it.
#[derive(Debug, PartialEq)]
pub struct PendingUpdate {
    /// Collection of unapplied blocks.
    pub update: Update,
    /// A state vector that informs about minimal client clock values that need to be satisfied
    /// in order to successfully apply corresponding `update`.
    pub missing: StateVector,
}

impl PendingUpdate {}

impl std::fmt::Display for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() && self.delete_set.is_empty() {
            write!(f, "{{}}")
        } else {
            write!(f, "{{")?;
            write!(f, "body: {}", self.blocks)?;
            write!(f, "delete_set: {}", self.delete_set)?;
            write!(f, "}}")
        }
    }
}

/// Conversion for tests only
#[cfg(test)]
impl Into<Store> for Update {
    fn into(self) -> Store {
        use crate::doc::Options;

        let mut store = Store::new(Options::with_client_id(0));
        for (client_id, vec) in self.blocks.clients {
            let blocks = store
                .blocks
                .get_client_blocks_with_capacity_mut(client_id, vec.len());
            for block in vec {
                blocks.push(block);
            }
        }
        store
    }
}

pub(crate) struct Blocks<'a> {
    current_client: std::vec::IntoIter<(&'a u64, &'a VecDeque<Block>)>,
    current_block: Option<std::collections::vec_deque::Iter<'a, Block>>,
}

impl<'a> Blocks<'a> {
    fn new(update: &'a UpdateBlocks) -> Self {
        let mut client_blocks: Vec<(&'a u64, &'a VecDeque<Block>)> =
            update.clients.iter().collect();
        // sorting to return higher client ids first
        client_blocks.sort_by(|a, b| b.0.cmp(a.0));
        let mut current_client = client_blocks.into_iter();

        let current_block = current_client.next().map(|(_, v)| v.iter());
        Blocks {
            current_client,
            current_block,
        }
    }
}

impl<'a> Iterator for Blocks<'a> {
    type Item = &'a Block;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(blocks) = self.current_block.as_mut() {
            let block = blocks.next();
            if block.is_some() {
                return block;
            }
        }

        if let Some(entry) = self.current_client.next() {
            self.current_block = Some(entry.1.iter());
            self.next()
        } else {
            None
        }
    }
}

pub(crate) struct IntoBlocks {
    current_client: std::vec::IntoIter<(u64, VecDeque<Block>)>,
    current_block: Option<std::collections::vec_deque::IntoIter<Block>>,
}

impl IntoBlocks {
    fn new(update: UpdateBlocks) -> Self {
        let mut client_blocks: Vec<(u64, VecDeque<Block>)> = update.clients.into_iter().collect();
        // sorting to return higher client ids first
        client_blocks.sort_by(|a, b| b.0.cmp(&a.0));
        let mut current_client = client_blocks.into_iter();

        let current_block = current_client.next().map(|(_, v)| v.into_iter());
        IntoBlocks {
            current_client,
            current_block,
        }
    }
}

impl Iterator for IntoBlocks {
    type Item = Block;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(blocks) = self.current_block.as_mut() {
            let block = blocks.next();
            if block.is_some() {
                return block;
            }
        }

        if let Some(entry) = self.current_client.next() {
            self.current_block = Some(entry.1.into_iter());
            self.next()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use crate::block::{Block, Item, ItemContent};
    use crate::types::TypePtr;
    use crate::update::Update;
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::{Doc, ID};
    use lib0::decoding::Cursor;

    #[test]
    fn update_decode() {
        /* Generated with:

           ```js
           var Y = require('yjs');

           var doc = new Y.Doc()
           var map = doc.getMap()
           map.set('keyB', 'valueB')

           // Merge changes from remote
           var update = Y.encodeStateAsUpdate(doc)
           ```
        */
        let update: &[u8] = &[
            1, 1, 176, 249, 159, 198, 7, 0, 40, 1, 0, 4, 107, 101, 121, 66, 1, 119, 6, 118, 97,
            108, 117, 101, 66, 0,
        ];
        let mut decoder = DecoderV1::from(update);
        let u = Update::decode(&mut decoder);

        let id = ID::new(2026372272, 0);
        let block = u.blocks.clients.get(&id.client).unwrap();
        let mut expected = Vec::new();
        expected.push(Block::Item(Item::new(
            id,
            None,
            None,
            None,
            None,
            TypePtr::Named("".into()),
            Some("keyB".into()),
            ItemContent::Any(vec!["valueB".into()]),
        )));
        assert_eq!(block, &expected);
    }

    #[test]
    fn update_merge() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let txt1 = t1.get_text("test");

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();
        let txt2 = t2.get_text("test");

        txt1.insert(&mut t1, 0, "aaa");
        txt1.insert(&mut t1, 0, "aaa");

        txt1.insert(&mut t1, 0, "bbb");
        txt1.insert(&mut t1, 2, "bbb");

        let binary1 = t1.encode_update_v1();
        let binary2 = t2.encode_update_v1();

        d1.apply_update_v1(&mut t1, binary2.as_slice());
        d2.apply_update_v1(&mut t2, binary1.as_slice());

        let u1 = Update::decode(&mut DecoderV1::new(Cursor::new(binary1.as_slice())));
        let u2 = Update::decode(&mut DecoderV1::new(Cursor::new(binary2.as_slice())));

        // a crux of our test: merged update upon applying should produce
        // the same output as sequence of updates applied individually
        let u12 = Update::merge_updates(vec![u1, u2]);

        let d3 = Doc::with_client_id(3);
        let mut t3 = d3.transact();
        let txt3 = t3.get_text("test");
        t3.apply_update(u12);

        let str1 = txt1.to_string(&t1);
        let str2 = txt2.to_string(&t2);
        let str3 = txt3.to_string(&t3);

        assert_eq!(str1, str2);
        assert_eq!(str2, str3);
    }
    #[test]
    fn update_merge_many() {
        const input: &[&[u8]] = &[
            &[1, 1, 11, 0, 4, 1, 4, 98, 111, 100, 121, 1, 97, 0],
            &[1, 1, 11, 2, 132, 11, 1, 1, 97, 0],
            &[1, 1, 11, 1, 132, 11, 0, 1, 98, 0],
            &[1, 1, 11, 3, 132, 11, 2, 1, 98, 0],
            &[1, 1, 11, 4, 132, 11, 3, 1, 97, 0],
            &[1, 1, 11, 5, 132, 11, 4, 1, 98, 0],
            &[1, 1, 11, 6, 132, 11, 5, 1, 97, 0],
            &[1, 1, 11, 7, 132, 11, 6, 1, 98, 0],
            &[1, 1, 11, 8, 132, 11, 7, 1, 97, 0],
            &[1, 1, 11, 9, 132, 11, 8, 1, 98, 0],
            &[1, 1, 11, 10, 132, 11, 9, 1, 97, 0],
            &[1, 1, 11, 11, 132, 11, 10, 1, 98, 0],
            &[1, 1, 11, 12, 132, 11, 11, 1, 97, 0],
            &[1, 1, 11, 13, 132, 11, 12, 1, 98, 0],
            &[1, 1, 11, 14, 132, 11, 13, 1, 97, 0],
            &[1, 1, 11, 15, 132, 11, 14, 1, 98, 0],
            &[1, 1, 11, 16, 132, 11, 15, 1, 97, 0],
            &[1, 1, 11, 17, 132, 11, 16, 1, 98, 0],
            &[1, 1, 11, 18, 132, 11, 17, 1, 97, 0],
            &[1, 1, 11, 19, 132, 11, 18, 1, 98, 0],
            &[1, 1, 11, 20, 132, 11, 19, 1, 97, 0],
            &[1, 1, 11, 21, 132, 11, 20, 1, 98, 0],
            &[1, 1, 11, 22, 132, 11, 21, 1, 97, 0],
            &[1, 1, 11, 23, 132, 11, 22, 1, 98, 0],
            &[1, 1, 11, 24, 132, 11, 23, 1, 97, 0],
            &[1, 1, 11, 25, 132, 11, 24, 1, 97, 0],
            &[1, 1, 11, 26, 132, 11, 25, 1, 98, 0],
            &[1, 1, 11, 27, 132, 11, 26, 1, 97, 0],
            &[1, 1, 11, 28, 132, 11, 27, 1, 98, 0],
            &[1, 1, 11, 29, 132, 11, 28, 1, 97, 0],
            &[1, 1, 11, 30, 132, 11, 29, 1, 98, 0],
            &[1, 1, 11, 31, 132, 11, 30, 1, 97, 0],
            &[1, 1, 11, 32, 132, 11, 31, 1, 98, 0],
            &[1, 1, 11, 33, 132, 11, 32, 1, 97, 0],
            &[1, 1, 11, 34, 132, 11, 33, 1, 98, 0],
            &[1, 1, 11, 35, 132, 11, 34, 1, 97, 0],
            &[1, 1, 11, 36, 132, 11, 35, 1, 98, 0],
            &[1, 1, 11, 37, 132, 11, 36, 1, 97, 0],
            &[1, 1, 11, 38, 132, 11, 37, 1, 98, 0],
            &[1, 1, 11, 39, 132, 11, 38, 1, 97, 0],
            &[1, 1, 11, 40, 132, 11, 39, 1, 98, 0],
            &[1, 1, 11, 41, 132, 11, 40, 1, 97, 0],
            &[1, 1, 11, 42, 132, 11, 41, 1, 98, 0],
            &[1, 1, 11, 43, 132, 11, 42, 1, 97, 0],
            &[1, 1, 11, 44, 132, 11, 43, 1, 98, 0],
            &[1, 1, 11, 45, 132, 11, 44, 1, 97, 0],
            &[1, 1, 11, 46, 132, 11, 45, 1, 98, 0],
            &[1, 1, 11, 47, 132, 11, 46, 1, 97, 0],
            &[1, 1, 11, 48, 132, 11, 47, 1, 98, 0],
            &[1, 1, 11, 49, 132, 11, 48, 1, 97, 0],
            &[1, 1, 11, 50, 132, 11, 49, 1, 98, 0],
            &[1, 1, 11, 51, 132, 11, 50, 1, 97, 0],
            &[1, 1, 11, 52, 132, 11, 51, 1, 98, 0],
            &[1, 1, 11, 53, 132, 11, 52, 1, 97, 0],
            &[1, 1, 11, 54, 132, 11, 53, 1, 98, 0],
            &[1, 1, 11, 55, 132, 11, 54, 1, 97, 0],
            &[1, 1, 11, 56, 132, 11, 55, 1, 98, 0],
            &[1, 1, 11, 57, 132, 11, 56, 1, 97, 0],
            &[1, 1, 11, 58, 132, 11, 57, 1, 98, 0],
            &[1, 1, 11, 59, 132, 11, 58, 1, 97, 0],
            &[1, 1, 11, 60, 132, 11, 59, 1, 98, 0],
            &[1, 1, 11, 61, 132, 11, 60, 1, 97, 0],
            &[1, 1, 11, 62, 132, 11, 61, 1, 98, 0],
            &[1, 1, 11, 63, 132, 11, 62, 1, 97, 0],
            &[1, 1, 11, 64, 132, 11, 63, 1, 98, 0],
            &[1, 1, 11, 65, 132, 11, 64, 1, 97, 0],
            &[1, 1, 11, 66, 132, 11, 65, 1, 98, 0],
            &[1, 1, 11, 67, 132, 11, 66, 1, 98, 0],
            &[1, 1, 11, 68, 132, 11, 67, 1, 97, 0],
            &[1, 1, 11, 69, 132, 11, 68, 1, 98, 0],
            &[1, 1, 11, 70, 132, 11, 69, 1, 97, 0],
            &[1, 1, 11, 71, 132, 11, 70, 1, 98, 0],
            &[1, 1, 11, 72, 132, 11, 71, 1, 97, 0],
            &[1, 1, 11, 73, 132, 11, 72, 1, 98, 0],
            &[1, 1, 11, 74, 132, 11, 73, 1, 97, 0],
            &[1, 1, 11, 75, 132, 11, 74, 1, 98, 0],
            &[1, 1, 11, 76, 132, 11, 75, 1, 97, 0],
            &[1, 1, 11, 77, 132, 11, 76, 1, 98, 0],
            &[1, 1, 11, 78, 132, 11, 77, 1, 97, 0],
            &[1, 1, 11, 79, 132, 11, 78, 1, 98, 0],
            &[1, 1, 11, 80, 132, 11, 79, 1, 97, 0],
            &[1, 1, 11, 81, 132, 11, 80, 1, 98, 0],
            &[1, 1, 11, 82, 132, 11, 81, 1, 97, 0],
            &[1, 1, 11, 83, 132, 11, 82, 1, 98, 0],
            &[1, 1, 11, 84, 132, 11, 83, 1, 97, 0],
            &[1, 1, 11, 85, 132, 11, 84, 1, 98, 0],
            &[1, 1, 11, 86, 132, 11, 85, 1, 97, 0],
            &[1, 1, 11, 87, 132, 11, 86, 1, 98, 0],
            &[1, 1, 11, 88, 132, 11, 87, 1, 97, 0],
            &[1, 1, 11, 89, 132, 11, 88, 1, 98, 0],
            &[1, 1, 11, 90, 132, 11, 89, 1, 97, 0],
            &[1, 1, 11, 91, 132, 11, 90, 1, 98, 0],
            &[1, 1, 11, 92, 132, 11, 91, 1, 98, 0],
            &[1, 1, 11, 93, 132, 11, 92, 1, 97, 0],
            &[1, 1, 11, 94, 132, 11, 93, 1, 98, 0],
            &[1, 1, 11, 95, 132, 11, 94, 1, 97, 0],
            &[1, 1, 11, 96, 132, 11, 95, 1, 97, 0],
            &[1, 1, 11, 97, 132, 11, 96, 1, 98, 0],
            &[1, 1, 11, 98, 132, 11, 97, 1, 98, 0],
            &[1, 1, 11, 99, 132, 11, 98, 1, 97, 0],
            &[1, 1, 11, 100, 132, 11, 99, 1, 98, 0],
            &[1, 1, 11, 101, 132, 11, 100, 1, 97, 0],
            &[1, 1, 11, 102, 132, 11, 101, 1, 98, 0],
            &[1, 1, 11, 103, 132, 11, 102, 1, 97, 0],
            &[1, 1, 11, 104, 132, 11, 103, 1, 98, 0],
            &[1, 1, 11, 105, 132, 11, 104, 1, 97, 0],
            &[1, 1, 11, 106, 132, 11, 105, 1, 98, 0],
            &[1, 1, 11, 107, 132, 11, 106, 1, 97, 0],
            &[1, 1, 11, 108, 132, 11, 107, 1, 98, 0],
            &[1, 1, 11, 109, 132, 11, 108, 1, 97, 0],
            &[1, 1, 11, 110, 132, 11, 109, 1, 98, 0],
            &[1, 1, 11, 111, 132, 11, 110, 1, 98, 0],
            &[1, 1, 11, 112, 132, 11, 111, 1, 97, 0],
            &[1, 1, 11, 113, 132, 11, 112, 1, 98, 0],
            &[1, 1, 11, 114, 132, 11, 113, 1, 97, 0],
            &[1, 1, 11, 115, 132, 11, 114, 1, 98, 0],
            &[1, 1, 11, 116, 132, 11, 115, 1, 97, 0],
            &[1, 1, 11, 117, 132, 11, 116, 1, 98, 0],
            &[1, 1, 11, 118, 132, 11, 117, 1, 97, 0],
            &[1, 1, 11, 119, 132, 11, 118, 1, 98, 0],
            &[1, 1, 11, 120, 132, 11, 119, 1, 98, 0],
            &[1, 1, 11, 121, 132, 11, 120, 1, 97, 0],
            &[1, 1, 11, 122, 132, 11, 121, 1, 98, 0],
            &[1, 1, 11, 123, 132, 11, 122, 1, 97, 0],
            &[1, 1, 11, 124, 132, 11, 123, 1, 98, 0],
            &[1, 1, 11, 125, 132, 11, 124, 1, 97, 0],
            &[1, 1, 11, 126, 132, 11, 125, 1, 98, 0],
            &[1, 1, 11, 127, 132, 11, 126, 1, 97, 0],
            &[1, 1, 11, 128, 1, 132, 11, 127, 1, 98, 0],
            &[1, 1, 11, 129, 1, 132, 11, 128, 1, 1, 97, 0],
            &[1, 1, 11, 130, 1, 132, 11, 129, 1, 1, 98, 0],
            &[1, 1, 11, 131, 1, 132, 11, 130, 1, 1, 98, 0],
            &[1, 1, 11, 132, 1, 132, 11, 131, 1, 1, 97, 0],
            &[1, 1, 11, 133, 1, 132, 11, 132, 1, 1, 98, 0],
            &[1, 1, 11, 134, 1, 132, 11, 133, 1, 1, 97, 0],
            &[1, 1, 11, 135, 1, 132, 11, 134, 1, 1, 97, 0],
            &[1, 1, 11, 136, 1, 132, 11, 135, 1, 1, 98, 0],
            &[1, 1, 11, 137, 1, 132, 11, 136, 1, 1, 97, 0],
            &[1, 1, 11, 138, 1, 132, 11, 137, 1, 1, 98, 0],
            &[1, 1, 11, 139, 1, 132, 11, 138, 1, 1, 97, 0],
            &[1, 1, 11, 140, 1, 132, 11, 139, 1, 1, 98, 0],
            &[1, 1, 11, 141, 1, 132, 11, 140, 1, 1, 97, 0],
            &[1, 1, 11, 142, 1, 132, 11, 141, 1, 1, 98, 0],
            &[1, 1, 11, 143, 1, 132, 11, 142, 1, 1, 97, 0],
            &[1, 1, 11, 144, 1, 132, 11, 143, 1, 1, 98, 0],
            &[1, 1, 11, 145, 1, 132, 11, 144, 1, 1, 97, 0],
            &[1, 1, 11, 146, 1, 132, 11, 145, 1, 1, 98, 0],
            &[1, 1, 11, 147, 1, 132, 11, 146, 1, 1, 98, 0],
            &[1, 1, 11, 148, 1, 132, 11, 147, 1, 1, 97, 0],
            &[1, 1, 11, 149, 1, 132, 11, 148, 1, 1, 98, 0],
            &[1, 1, 11, 150, 1, 132, 11, 149, 1, 1, 97, 0],
            &[1, 1, 11, 151, 1, 132, 11, 150, 1, 1, 98, 0],
            &[1, 1, 11, 152, 1, 132, 11, 151, 1, 1, 97, 0],
            &[1, 1, 11, 153, 1, 132, 11, 152, 1, 1, 98, 0],
            &[1, 1, 11, 154, 1, 132, 11, 153, 1, 1, 97, 0],
            &[1, 1, 11, 155, 1, 132, 11, 154, 1, 1, 98, 0],
            &[1, 1, 11, 156, 1, 132, 11, 155, 1, 1, 97, 0],
            &[1, 1, 11, 157, 1, 132, 11, 156, 1, 1, 98, 0],
            &[1, 1, 11, 158, 1, 132, 11, 157, 1, 1, 98, 0],
            &[1, 1, 11, 159, 1, 132, 11, 158, 1, 1, 97, 0],
            &[1, 1, 11, 160, 1, 132, 11, 159, 1, 1, 98, 0],
            &[1, 1, 11, 161, 1, 132, 11, 160, 1, 1, 97, 0],
            &[1, 1, 11, 162, 1, 132, 11, 161, 1, 1, 98, 0],
            &[1, 1, 11, 163, 1, 132, 11, 162, 1, 1, 97, 0],
            &[1, 1, 11, 164, 1, 132, 11, 163, 1, 1, 98, 0],
            &[1, 1, 11, 165, 1, 132, 11, 164, 1, 1, 98, 0],
            &[1, 1, 11, 166, 1, 132, 11, 165, 1, 1, 97, 0],
            &[1, 1, 11, 167, 1, 132, 11, 166, 1, 1, 98, 0],
            &[1, 1, 11, 168, 1, 132, 11, 167, 1, 1, 97, 0],
            &[1, 1, 11, 169, 1, 132, 11, 168, 1, 1, 98, 0],
            &[1, 1, 11, 170, 1, 132, 11, 169, 1, 1, 97, 0],
            &[1, 1, 11, 171, 1, 132, 11, 170, 1, 1, 98, 0],
            &[1, 1, 11, 172, 1, 132, 11, 171, 1, 1, 97, 0],
            &[1, 1, 11, 173, 1, 132, 11, 172, 1, 1, 98, 0],
            &[1, 1, 11, 174, 1, 132, 11, 173, 1, 1, 97, 0],
            &[1, 1, 11, 175, 1, 132, 11, 174, 1, 1, 98, 0],
            &[1, 1, 11, 176, 1, 132, 11, 175, 1, 1, 97, 0],
            &[1, 1, 11, 177, 1, 132, 11, 176, 1, 1, 98, 0],
            &[1, 1, 11, 178, 1, 132, 11, 177, 1, 1, 97, 0],
            &[1, 1, 11, 180, 1, 132, 11, 179, 1, 1, 98, 0],
            &[1, 1, 11, 181, 1, 132, 11, 180, 1, 1, 97, 0],
            &[1, 1, 11, 179, 1, 132, 11, 178, 1, 1, 98, 0],
            &[1, 1, 11, 182, 1, 132, 11, 181, 1, 1, 98, 0],
            &[1, 1, 11, 183, 1, 132, 11, 182, 1, 1, 97, 0],
            &[1, 1, 11, 184, 1, 132, 11, 183, 1, 1, 98, 0],
            &[1, 1, 11, 185, 1, 132, 11, 184, 1, 1, 97, 0],
            &[1, 1, 11, 186, 1, 132, 11, 185, 1, 1, 97, 0],
            &[1, 1, 11, 187, 1, 132, 11, 186, 1, 1, 98, 0],
            &[1, 1, 11, 188, 1, 132, 11, 187, 1, 1, 97, 0],
            &[1, 1, 11, 189, 1, 132, 11, 188, 1, 1, 98, 0],
            &[1, 1, 11, 190, 1, 132, 11, 189, 1, 1, 98, 0],
            &[1, 1, 11, 191, 1, 132, 11, 190, 1, 1, 97, 0],
            &[1, 1, 11, 192, 1, 132, 11, 191, 1, 1, 98, 0],
            &[1, 1, 11, 193, 1, 132, 11, 192, 1, 1, 97, 0],
            &[1, 1, 11, 194, 1, 132, 11, 193, 1, 1, 98, 0],
            &[1, 1, 11, 195, 1, 132, 11, 194, 1, 1, 97, 0],
            &[1, 1, 11, 196, 1, 132, 11, 195, 1, 1, 98, 0],
            &[1, 1, 11, 197, 1, 132, 11, 196, 1, 1, 97, 0],
            &[1, 1, 11, 198, 1, 132, 11, 197, 1, 1, 98, 0],
            &[1, 1, 11, 199, 1, 132, 11, 198, 1, 1, 97, 0],
            &[1, 1, 11, 200, 1, 132, 11, 199, 1, 1, 98, 0],
            &[1, 1, 11, 201, 1, 132, 11, 200, 1, 1, 98, 0],
            &[1, 1, 11, 202, 1, 132, 11, 201, 1, 1, 97, 0],
            &[1, 1, 11, 203, 1, 132, 11, 202, 1, 1, 98, 0],
            &[1, 1, 11, 204, 1, 132, 11, 203, 1, 1, 97, 0],
            &[1, 1, 11, 205, 1, 132, 11, 204, 1, 1, 98, 0],
            &[1, 1, 11, 206, 1, 132, 11, 205, 1, 1, 97, 0],
            &[1, 1, 11, 207, 1, 132, 11, 206, 1, 1, 98, 0],
            &[1, 1, 11, 208, 1, 132, 11, 207, 1, 1, 98, 0],
            &[1, 1, 11, 209, 1, 132, 11, 208, 1, 1, 97, 0],
            &[1, 1, 11, 210, 1, 132, 11, 209, 1, 1, 98, 0],
            &[1, 1, 11, 211, 1, 132, 11, 210, 1, 1, 97, 0],
            &[1, 1, 11, 212, 1, 132, 11, 211, 1, 1, 98, 0],
            &[1, 1, 11, 213, 1, 132, 11, 212, 1, 1, 97, 0],
            &[1, 1, 11, 214, 1, 132, 11, 213, 1, 1, 98, 0],
            &[1, 1, 11, 215, 1, 132, 11, 214, 1, 1, 98, 0],
            &[1, 1, 11, 216, 1, 132, 11, 215, 1, 1, 97, 0],
            &[1, 1, 11, 217, 1, 132, 11, 216, 1, 1, 98, 0],
            &[1, 1, 11, 218, 1, 132, 11, 217, 1, 1, 97, 0],
            &[1, 1, 11, 219, 1, 132, 11, 218, 1, 1, 98, 0],
            &[1, 1, 11, 220, 1, 132, 11, 219, 1, 1, 97, 0],
            &[1, 1, 11, 221, 1, 132, 11, 220, 1, 1, 98, 0],
            &[1, 1, 11, 222, 1, 132, 11, 221, 1, 1, 97, 0],
            &[1, 1, 11, 223, 1, 132, 11, 222, 1, 1, 98, 0],
            &[1, 1, 11, 224, 1, 132, 11, 223, 1, 1, 97, 0],
            &[1, 1, 11, 225, 1, 132, 11, 224, 1, 1, 98, 0],
            &[1, 1, 11, 226, 1, 132, 11, 225, 1, 1, 97, 0],
            &[1, 1, 11, 227, 1, 132, 11, 226, 1, 1, 98, 0],
            &[1, 1, 11, 228, 1, 132, 11, 227, 1, 1, 97, 0],
            &[1, 1, 11, 229, 1, 132, 11, 228, 1, 1, 98, 0],
            &[1, 1, 11, 230, 1, 132, 11, 229, 1, 1, 97, 0],
            &[1, 1, 11, 231, 1, 132, 11, 230, 1, 1, 98, 0],
            &[1, 1, 11, 232, 1, 132, 11, 231, 1, 1, 98, 0],
            &[1, 1, 11, 233, 1, 132, 11, 232, 1, 1, 97, 0],
            &[1, 1, 11, 234, 1, 132, 11, 233, 1, 1, 98, 0],
            &[1, 1, 11, 235, 1, 132, 11, 234, 1, 1, 97, 0],
            &[1, 1, 11, 236, 1, 132, 11, 235, 1, 1, 98, 0],
            &[1, 1, 11, 237, 1, 132, 11, 236, 1, 1, 97, 0],
            &[1, 1, 11, 238, 1, 132, 11, 237, 1, 1, 98, 0],
            &[1, 1, 11, 239, 1, 132, 11, 238, 1, 1, 98, 0],
            &[1, 1, 11, 240, 1, 132, 11, 239, 1, 1, 97, 0],
            &[1, 1, 11, 241, 1, 132, 11, 240, 1, 1, 98, 0],
            &[1, 1, 11, 242, 1, 132, 11, 241, 1, 1, 97, 0],
            &[1, 1, 11, 243, 1, 132, 11, 242, 1, 1, 98, 0],
            &[1, 1, 11, 244, 1, 132, 11, 243, 1, 1, 97, 0],
            &[1, 1, 11, 245, 1, 132, 11, 244, 1, 1, 98, 0],
            &[1, 1, 11, 246, 1, 132, 11, 245, 1, 1, 97, 0],
            &[1, 1, 11, 247, 1, 132, 11, 246, 1, 1, 98, 0],
            &[1, 1, 11, 248, 1, 132, 11, 247, 1, 1, 97, 0],
            &[1, 1, 11, 249, 1, 132, 11, 248, 1, 1, 98, 0],
            &[1, 1, 11, 250, 1, 132, 11, 249, 1, 1, 97, 0],
            &[1, 1, 11, 251, 1, 132, 11, 250, 1, 1, 98, 0],
            &[1, 1, 11, 252, 1, 132, 11, 251, 1, 1, 98, 0],
            &[1, 1, 11, 253, 1, 132, 11, 252, 1, 1, 97, 0],
            &[1, 1, 11, 254, 1, 132, 11, 253, 1, 1, 98, 0],
            &[1, 1, 11, 255, 1, 132, 11, 254, 1, 1, 97, 0],
            &[1, 1, 11, 128, 2, 132, 11, 255, 1, 1, 98, 0],
            &[1, 1, 11, 129, 2, 132, 11, 128, 2, 1, 97, 0],
            &[1, 1, 11, 130, 2, 132, 11, 129, 2, 1, 98, 0],
            &[1, 1, 11, 131, 2, 132, 11, 130, 2, 1, 97, 0],
            &[1, 1, 11, 132, 2, 132, 11, 131, 2, 1, 98, 0],
            &[1, 1, 11, 133, 2, 132, 11, 132, 2, 1, 98, 0],
            &[1, 1, 11, 134, 2, 132, 11, 133, 2, 1, 97, 0],
            &[1, 1, 11, 135, 2, 132, 11, 134, 2, 1, 98, 0],
            &[1, 1, 11, 136, 2, 132, 11, 135, 2, 1, 97, 0],
            &[1, 1, 11, 137, 2, 132, 11, 136, 2, 1, 98, 0],
            &[1, 1, 11, 138, 2, 132, 11, 137, 2, 1, 97, 0],
            &[1, 1, 11, 139, 2, 132, 11, 138, 2, 1, 98, 0],
            &[1, 1, 11, 140, 2, 132, 11, 139, 2, 1, 98, 0],
            &[1, 1, 11, 141, 2, 132, 11, 140, 2, 1, 97, 0],
            &[1, 1, 11, 142, 2, 132, 11, 141, 2, 1, 98, 0],
            &[1, 1, 11, 143, 2, 132, 11, 142, 2, 1, 97, 0],
            &[1, 1, 11, 144, 2, 132, 11, 143, 2, 1, 98, 0],
            &[1, 1, 11, 145, 2, 132, 11, 144, 2, 1, 97, 0],
            &[1, 1, 11, 146, 2, 132, 11, 145, 2, 1, 98, 0],
            &[1, 1, 11, 147, 2, 132, 11, 146, 2, 1, 97, 0],
            &[1, 1, 11, 148, 2, 132, 11, 147, 2, 1, 98, 0],
            &[1, 1, 11, 149, 2, 132, 11, 148, 2, 1, 97, 0],
            &[1, 1, 11, 150, 2, 132, 11, 149, 2, 1, 98, 0],
            &[1, 1, 11, 151, 2, 132, 11, 150, 2, 1, 97, 0],
            &[1, 1, 11, 152, 2, 132, 11, 151, 2, 1, 97, 0],
            &[1, 1, 11, 153, 2, 132, 11, 152, 2, 1, 98, 0],
            &[1, 1, 11, 154, 2, 132, 11, 153, 2, 1, 98, 0],
            &[1, 1, 11, 155, 2, 132, 11, 154, 2, 1, 97, 0],
            &[1, 1, 11, 156, 2, 132, 11, 155, 2, 1, 98, 0],
            &[1, 1, 11, 157, 2, 132, 11, 156, 2, 1, 97, 0],
            &[1, 1, 11, 158, 2, 132, 11, 157, 2, 1, 98, 0],
            &[1, 1, 11, 159, 2, 132, 11, 158, 2, 1, 97, 0],
            &[1, 1, 11, 160, 2, 132, 11, 159, 2, 1, 98, 0],
            &[1, 1, 11, 161, 2, 132, 11, 160, 2, 1, 97, 0],
            &[1, 1, 11, 162, 2, 132, 11, 161, 2, 1, 98, 0],
            &[1, 1, 11, 163, 2, 132, 11, 162, 2, 1, 98, 0],
            &[1, 1, 11, 164, 2, 132, 11, 163, 2, 1, 97, 0],
            &[1, 1, 11, 165, 2, 132, 11, 164, 2, 1, 98, 0],
            &[1, 1, 11, 166, 2, 132, 11, 165, 2, 1, 97, 0],
            &[1, 1, 11, 167, 2, 132, 11, 166, 2, 1, 98, 0],
            &[1, 1, 11, 168, 2, 132, 11, 167, 2, 1, 97, 0],
            &[1, 1, 11, 169, 2, 132, 11, 168, 2, 1, 98, 0],
            &[1, 1, 11, 170, 2, 132, 11, 169, 2, 1, 97, 0],
            &[1, 1, 11, 171, 2, 132, 11, 170, 2, 1, 98, 0],
            &[1, 1, 11, 172, 2, 132, 11, 171, 2, 1, 98, 0],
            &[1, 1, 11, 173, 2, 132, 11, 172, 2, 1, 97, 0],
            &[1, 1, 11, 174, 2, 132, 11, 173, 2, 1, 98, 0],
            &[1, 1, 11, 175, 2, 132, 11, 174, 2, 1, 97, 0],
            &[1, 1, 11, 176, 2, 132, 11, 175, 2, 1, 98, 0],
            &[1, 1, 11, 177, 2, 132, 11, 176, 2, 1, 97, 0],
            &[1, 1, 11, 178, 2, 132, 11, 177, 2, 1, 98, 0],
            &[1, 1, 11, 179, 2, 132, 11, 178, 2, 1, 97, 0],
            &[1, 1, 11, 180, 2, 132, 11, 179, 2, 1, 98, 0],
            &[1, 1, 11, 181, 2, 132, 11, 180, 2, 1, 97, 0],
            &[1, 1, 11, 182, 2, 132, 11, 181, 2, 1, 98, 0],
            &[1, 1, 11, 183, 2, 132, 11, 182, 2, 1, 98, 0],
            &[1, 1, 11, 184, 2, 132, 11, 183, 2, 1, 97, 0],
            &[1, 1, 11, 185, 2, 132, 11, 184, 2, 1, 98, 0],
            &[1, 1, 11, 186, 2, 132, 11, 185, 2, 1, 97, 0],
            &[1, 1, 11, 187, 2, 132, 11, 186, 2, 1, 98, 0],
            &[1, 1, 11, 188, 2, 132, 11, 187, 2, 1, 97, 0],
            &[1, 1, 11, 189, 2, 132, 11, 188, 2, 1, 98, 0],
            &[1, 1, 11, 190, 2, 132, 11, 189, 2, 1, 97, 0],
            &[1, 1, 11, 191, 2, 132, 11, 190, 2, 1, 98, 0],
            &[1, 1, 11, 192, 2, 132, 11, 191, 2, 1, 97, 0],
            &[1, 1, 11, 193, 2, 132, 11, 192, 2, 1, 98, 0],
            &[1, 1, 11, 194, 2, 132, 11, 193, 2, 1, 97, 0],
            &[1, 1, 11, 195, 2, 132, 11, 194, 2, 1, 98, 0],
            &[1, 1, 11, 196, 2, 132, 11, 195, 2, 1, 97, 0],
            &[1, 1, 11, 197, 2, 132, 11, 196, 2, 1, 98, 0],
            &[1, 1, 11, 198, 2, 132, 11, 197, 2, 1, 98, 0],
            &[1, 1, 11, 199, 2, 132, 11, 198, 2, 1, 97, 0],
            &[1, 1, 11, 200, 2, 132, 11, 199, 2, 1, 98, 0],
            &[1, 1, 11, 201, 2, 132, 11, 200, 2, 1, 97, 0],
            &[1, 1, 11, 202, 2, 132, 11, 201, 2, 1, 98, 0],
            &[1, 1, 11, 203, 2, 132, 11, 202, 2, 1, 97, 0],
            &[1, 1, 11, 204, 2, 132, 11, 203, 2, 1, 98, 0],
            &[1, 1, 11, 205, 2, 132, 11, 204, 2, 1, 97, 0],
            &[1, 1, 11, 206, 2, 132, 11, 205, 2, 1, 98, 0],
            &[1, 1, 11, 207, 2, 132, 11, 206, 2, 1, 97, 0],
            &[1, 1, 11, 208, 2, 132, 11, 207, 2, 1, 98, 0],
            &[1, 1, 11, 209, 2, 132, 11, 208, 2, 1, 97, 0],
            &[1, 1, 11, 210, 2, 132, 11, 209, 2, 1, 98, 0],
            &[1, 1, 11, 211, 2, 132, 11, 210, 2, 1, 98, 0],
            &[1, 1, 11, 212, 2, 132, 11, 211, 2, 1, 97, 0],
            &[1, 1, 11, 213, 2, 132, 11, 212, 2, 1, 98, 0],
            &[1, 1, 11, 214, 2, 132, 11, 213, 2, 1, 97, 0],
            &[1, 1, 11, 215, 2, 132, 11, 214, 2, 1, 98, 0],
            &[1, 1, 11, 216, 2, 132, 11, 215, 2, 1, 97, 0],
            &[1, 1, 11, 217, 2, 132, 11, 216, 2, 1, 98, 0],
            &[1, 1, 11, 218, 2, 132, 11, 217, 2, 1, 97, 0],
            &[1, 1, 11, 219, 2, 132, 11, 218, 2, 1, 98, 0],
            &[1, 1, 11, 220, 2, 132, 11, 219, 2, 1, 97, 0],
            &[1, 1, 11, 221, 2, 132, 11, 220, 2, 1, 98, 0],
            &[1, 1, 11, 222, 2, 132, 11, 221, 2, 1, 98, 0],
            &[1, 1, 11, 223, 2, 132, 11, 222, 2, 1, 97, 0],
            &[1, 1, 11, 224, 2, 132, 11, 223, 2, 1, 98, 0],
            &[1, 1, 11, 225, 2, 132, 11, 224, 2, 1, 97, 0],
            &[1, 1, 11, 226, 2, 132, 11, 225, 2, 1, 98, 0],
            &[1, 1, 11, 227, 2, 132, 11, 226, 2, 1, 97, 0],
            &[1, 1, 11, 228, 2, 132, 11, 227, 2, 1, 98, 0],
            &[1, 1, 11, 229, 2, 132, 11, 228, 2, 1, 97, 0],
            &[1, 1, 11, 230, 2, 132, 11, 229, 2, 1, 98, 0],
            &[1, 1, 11, 231, 2, 132, 11, 230, 2, 1, 97, 0],
            &[1, 1, 11, 232, 2, 132, 11, 231, 2, 1, 98, 0],
            &[1, 1, 11, 233, 2, 132, 11, 232, 2, 1, 98, 0],
            &[1, 1, 11, 234, 2, 132, 11, 233, 2, 1, 97, 0],
            &[1, 1, 11, 235, 2, 132, 11, 234, 2, 1, 98, 0],
            &[1, 1, 11, 236, 2, 132, 11, 235, 2, 1, 97, 0],
            &[1, 1, 11, 237, 2, 132, 11, 236, 2, 1, 97, 0],
            &[1, 1, 11, 238, 2, 132, 11, 237, 2, 1, 98, 0],
            &[1, 1, 11, 239, 2, 132, 11, 238, 2, 1, 97, 0],
            &[1, 1, 11, 240, 2, 132, 11, 239, 2, 1, 98, 0],
            &[1, 1, 11, 241, 2, 132, 11, 240, 2, 1, 97, 0],
            &[1, 1, 11, 242, 2, 132, 11, 241, 2, 1, 98, 0],
            &[1, 1, 11, 243, 2, 132, 11, 242, 2, 1, 98, 0],
            &[1, 1, 11, 244, 2, 132, 11, 243, 2, 1, 97, 0],
            &[1, 1, 11, 245, 2, 132, 11, 244, 2, 1, 98, 0],
            &[1, 1, 11, 246, 2, 132, 11, 245, 2, 1, 97, 0],
            &[1, 1, 11, 247, 2, 132, 11, 246, 2, 1, 98, 0],
            &[1, 1, 11, 248, 2, 132, 11, 247, 2, 1, 97, 0],
            &[1, 1, 11, 249, 2, 132, 11, 248, 2, 1, 98, 0],
            &[1, 1, 11, 250, 2, 132, 11, 249, 2, 1, 97, 0],
            &[1, 1, 11, 251, 2, 132, 11, 250, 2, 1, 98, 0],
            &[1, 1, 11, 252, 2, 132, 11, 251, 2, 1, 97, 0],
            &[1, 1, 11, 253, 2, 132, 11, 252, 2, 1, 98, 0],
            &[1, 1, 11, 254, 2, 132, 11, 253, 2, 1, 97, 0],
            &[1, 1, 11, 255, 2, 132, 11, 254, 2, 1, 98, 0],
            &[1, 1, 11, 128, 3, 132, 11, 255, 2, 1, 97, 0],
            &[1, 1, 11, 129, 3, 132, 11, 128, 3, 1, 98, 0],
            &[1, 1, 11, 130, 3, 132, 11, 129, 3, 1, 97, 0],
            &[1, 1, 11, 131, 3, 132, 11, 130, 3, 1, 98, 0],
            &[1, 1, 11, 132, 3, 132, 11, 131, 3, 1, 97, 0],
            &[1, 1, 11, 133, 3, 132, 11, 132, 3, 1, 98, 0],
            &[1, 1, 11, 134, 3, 132, 11, 133, 3, 1, 97, 0],
            &[1, 1, 11, 135, 3, 132, 11, 134, 3, 1, 98, 0],
            &[1, 1, 11, 136, 3, 132, 11, 135, 3, 1, 97, 0],
            &[1, 1, 11, 137, 3, 132, 11, 136, 3, 1, 98, 0],
            &[1, 1, 11, 138, 3, 132, 11, 137, 3, 1, 98, 0],
            &[1, 1, 11, 139, 3, 132, 11, 138, 3, 1, 97, 0],
            &[1, 1, 11, 140, 3, 132, 11, 139, 3, 1, 98, 0],
            &[1, 1, 11, 141, 3, 132, 11, 140, 3, 1, 97, 0],
            &[1, 1, 11, 142, 3, 132, 11, 141, 3, 1, 98, 0],
            &[1, 1, 11, 143, 3, 132, 11, 142, 3, 1, 97, 0],
            &[1, 1, 11, 144, 3, 132, 11, 143, 3, 1, 98, 0],
            &[1, 1, 11, 145, 3, 132, 11, 144, 3, 1, 97, 0],
            &[1, 1, 11, 146, 3, 132, 11, 145, 3, 1, 98, 0],
            &[1, 1, 11, 147, 3, 132, 11, 146, 3, 1, 98, 0],
            &[1, 1, 11, 148, 3, 132, 11, 147, 3, 1, 97, 0],
            &[1, 1, 11, 149, 3, 132, 11, 148, 3, 1, 98, 0],
            &[1, 1, 11, 150, 3, 132, 11, 149, 3, 1, 97, 0],
            &[1, 1, 11, 151, 3, 132, 11, 150, 3, 1, 98, 0],
            &[1, 1, 11, 152, 3, 132, 11, 151, 3, 1, 97, 0],
            &[1, 1, 11, 153, 3, 132, 11, 152, 3, 1, 98, 0],
            &[1, 1, 11, 154, 3, 132, 11, 153, 3, 1, 98, 0],
            &[1, 1, 11, 155, 3, 132, 11, 154, 3, 1, 97, 0],
            &[1, 1, 11, 156, 3, 132, 11, 155, 3, 1, 98, 0],
            &[1, 1, 11, 157, 3, 132, 11, 156, 3, 1, 97, 0],
            &[1, 1, 11, 158, 3, 132, 11, 157, 3, 1, 98, 0],
            &[1, 1, 11, 159, 3, 132, 11, 158, 3, 1, 97, 0],
            &[1, 1, 11, 160, 3, 132, 11, 159, 3, 1, 98, 0],
            &[1, 1, 11, 161, 3, 132, 11, 160, 3, 1, 97, 0],
            &[1, 1, 11, 162, 3, 132, 11, 161, 3, 1, 98, 0],
            &[1, 1, 11, 163, 3, 132, 11, 162, 3, 1, 97, 0],
            &[1, 1, 11, 164, 3, 132, 11, 163, 3, 1, 98, 0],
            &[1, 1, 11, 165, 3, 132, 11, 164, 3, 1, 98, 0],
            &[1, 1, 11, 166, 3, 132, 11, 165, 3, 1, 97, 0],
            &[1, 1, 11, 167, 3, 132, 11, 166, 3, 1, 98, 0],
            &[1, 1, 11, 168, 3, 132, 11, 167, 3, 1, 97, 0],
            &[1, 1, 11, 169, 3, 132, 11, 168, 3, 1, 98, 0],
            &[1, 1, 11, 170, 3, 132, 11, 169, 3, 1, 97, 0],
            &[1, 1, 11, 171, 3, 132, 11, 170, 3, 1, 98, 0],
            &[1, 1, 11, 172, 3, 132, 11, 171, 3, 1, 97, 0],
            &[1, 1, 11, 173, 3, 132, 11, 172, 3, 1, 98, 0],
            &[1, 1, 11, 174, 3, 132, 11, 173, 3, 1, 97, 0],
            &[1, 1, 11, 175, 3, 132, 11, 174, 3, 1, 98, 0],
            &[1, 1, 11, 176, 3, 132, 11, 175, 3, 1, 98, 0],
            &[1, 1, 11, 177, 3, 132, 11, 176, 3, 1, 97, 0],
            &[1, 1, 11, 178, 3, 132, 11, 177, 3, 1, 97, 0],
            &[1, 1, 11, 179, 3, 132, 11, 178, 3, 1, 98, 0],
            &[1, 1, 11, 180, 3, 132, 11, 179, 3, 1, 97, 0],
            &[1, 1, 11, 181, 3, 132, 11, 180, 3, 1, 98, 0],
            &[1, 1, 11, 182, 3, 132, 11, 181, 3, 1, 97, 0],
            &[1, 1, 11, 183, 3, 132, 11, 182, 3, 1, 98, 0],
            &[1, 1, 11, 184, 3, 132, 11, 183, 3, 1, 97, 0],
            &[1, 1, 11, 185, 3, 132, 11, 184, 3, 1, 98, 0],
            &[1, 1, 11, 186, 3, 132, 11, 185, 3, 1, 97, 0],
            &[1, 1, 11, 187, 3, 132, 11, 186, 3, 1, 98, 0],
            &[1, 1, 11, 188, 3, 132, 11, 187, 3, 1, 97, 0],
            &[1, 1, 11, 189, 3, 132, 11, 188, 3, 1, 98, 0],
            &[1, 1, 11, 190, 3, 132, 11, 189, 3, 1, 97, 0],
            &[1, 1, 11, 191, 3, 132, 11, 190, 3, 1, 98, 0],
            &[1, 1, 11, 192, 3, 132, 11, 191, 3, 1, 98, 0],
            &[1, 1, 11, 193, 3, 132, 11, 192, 3, 1, 97, 0],
            &[1, 1, 11, 194, 3, 132, 11, 193, 3, 1, 98, 0],
            &[1, 1, 11, 195, 3, 132, 11, 194, 3, 1, 97, 0],
            &[1, 1, 11, 196, 3, 132, 11, 195, 3, 1, 98, 0],
            &[1, 1, 11, 197, 3, 132, 11, 196, 3, 1, 97, 0],
            &[1, 1, 11, 198, 3, 132, 11, 197, 3, 1, 98, 0],
            &[1, 1, 11, 199, 3, 132, 11, 198, 3, 1, 97, 0],
            &[1, 1, 11, 200, 3, 132, 11, 199, 3, 1, 98, 0],
            &[1, 1, 11, 201, 3, 132, 11, 200, 3, 1, 97, 0],
            &[1, 1, 11, 202, 3, 132, 11, 201, 3, 1, 98, 0],
            &[1, 1, 11, 203, 3, 132, 11, 202, 3, 1, 97, 0],
            &[1, 1, 11, 204, 3, 132, 11, 203, 3, 1, 98, 0],
            &[1, 1, 11, 205, 3, 132, 11, 204, 3, 1, 98, 0],
            &[1, 1, 11, 206, 3, 132, 11, 205, 3, 1, 97, 0],
            &[1, 1, 11, 207, 3, 132, 11, 206, 3, 1, 98, 0],
            &[1, 1, 11, 208, 3, 132, 11, 207, 3, 1, 97, 0],
            &[1, 1, 11, 209, 3, 132, 11, 208, 3, 1, 98, 0],
            &[1, 1, 11, 210, 3, 132, 11, 209, 3, 1, 97, 0],
            &[1, 1, 11, 211, 3, 132, 11, 210, 3, 1, 98, 0],
            &[1, 1, 11, 212, 3, 132, 11, 211, 3, 1, 97, 0],
            &[1, 1, 11, 213, 3, 132, 11, 212, 3, 1, 98, 0],
            &[1, 1, 11, 214, 3, 132, 11, 213, 3, 1, 97, 0],
            &[1, 1, 11, 215, 3, 132, 11, 214, 3, 1, 98, 0],
            &[1, 1, 11, 216, 3, 132, 11, 215, 3, 1, 97, 0],
            &[1, 1, 11, 217, 3, 132, 11, 216, 3, 1, 98, 0],
            &[1, 1, 11, 218, 3, 132, 11, 217, 3, 1, 97, 0],
            &[1, 1, 11, 219, 3, 132, 11, 218, 3, 1, 98, 0],
            &[1, 1, 11, 220, 3, 132, 11, 219, 3, 1, 97, 0],
            &[1, 1, 11, 221, 3, 132, 11, 220, 3, 1, 98, 0],
            &[1, 1, 11, 222, 3, 132, 11, 221, 3, 1, 98, 0],
            &[1, 1, 11, 223, 3, 132, 11, 222, 3, 1, 97, 0],
            &[1, 1, 11, 224, 3, 132, 11, 223, 3, 1, 98, 0],
            &[1, 1, 11, 225, 3, 132, 11, 224, 3, 1, 97, 0],
            &[1, 1, 11, 226, 3, 132, 11, 225, 3, 1, 98, 0],
            &[1, 1, 11, 227, 3, 132, 11, 226, 3, 1, 97, 0],
            &[1, 1, 11, 228, 3, 132, 11, 227, 3, 1, 98, 0],
            &[1, 1, 11, 229, 3, 132, 11, 228, 3, 1, 97, 0],
            &[1, 1, 11, 230, 3, 132, 11, 229, 3, 1, 98, 0],
            &[1, 1, 11, 231, 3, 132, 11, 230, 3, 1, 97, 0],
            &[1, 1, 11, 232, 3, 132, 11, 231, 3, 1, 98, 0],
            &[1, 1, 11, 233, 3, 132, 11, 232, 3, 1, 97, 0],
            &[1, 1, 11, 234, 3, 132, 11, 233, 3, 1, 98, 0],
            &[1, 1, 11, 235, 3, 132, 11, 234, 3, 1, 97, 0],
            &[1, 1, 11, 236, 3, 132, 11, 235, 3, 1, 98, 0],
            &[1, 1, 11, 237, 3, 132, 11, 236, 3, 1, 97, 0],
            &[1, 1, 11, 238, 3, 132, 11, 237, 3, 1, 98, 0],
            &[1, 1, 11, 239, 3, 132, 11, 238, 3, 1, 97, 0],
            &[1, 1, 11, 240, 3, 132, 11, 239, 3, 1, 98, 0],
            &[1, 1, 11, 241, 3, 132, 11, 240, 3, 1, 97, 0],
            &[1, 1, 11, 242, 3, 132, 11, 241, 3, 1, 98, 0],
            &[1, 1, 11, 243, 3, 132, 11, 242, 3, 1, 98, 0],
            &[1, 1, 11, 244, 3, 132, 11, 243, 3, 1, 97, 0],
            &[1, 1, 11, 245, 3, 132, 11, 244, 3, 1, 98, 0],
            &[1, 1, 11, 246, 3, 132, 11, 245, 3, 1, 97, 0],
            &[1, 1, 11, 247, 3, 132, 11, 246, 3, 1, 98, 0],
            &[1, 1, 11, 248, 3, 132, 11, 247, 3, 1, 97, 0],
            &[1, 1, 11, 249, 3, 132, 11, 248, 3, 1, 98, 0],
            &[1, 1, 11, 250, 3, 132, 11, 249, 3, 1, 97, 0],
            &[1, 1, 11, 251, 3, 132, 11, 250, 3, 1, 98, 0],
            &[1, 1, 11, 252, 3, 132, 11, 251, 3, 1, 97, 0],
            &[1, 1, 11, 253, 3, 132, 11, 252, 3, 1, 98, 0],
            &[1, 1, 11, 254, 3, 132, 11, 253, 3, 1, 97, 0],
            &[1, 1, 11, 255, 3, 132, 11, 254, 3, 1, 98, 0],
            &[1, 1, 11, 128, 4, 132, 11, 255, 3, 1, 97, 0],
            &[1, 1, 11, 129, 4, 132, 11, 128, 4, 1, 98, 0],
            &[1, 1, 11, 130, 4, 132, 11, 129, 4, 1, 98, 0],
            &[1, 1, 11, 131, 4, 132, 11, 130, 4, 1, 97, 0],
            &[1, 1, 11, 132, 4, 132, 11, 131, 4, 1, 98, 0],
            &[1, 1, 11, 133, 4, 132, 11, 132, 4, 1, 97, 0],
            &[1, 1, 11, 134, 4, 132, 11, 133, 4, 1, 98, 0],
            &[1, 1, 11, 135, 4, 132, 11, 134, 4, 1, 97, 0],
            &[1, 1, 11, 136, 4, 132, 11, 135, 4, 1, 98, 0],
            &[1, 1, 11, 137, 4, 132, 11, 136, 4, 1, 98, 0],
            &[1, 1, 11, 138, 4, 132, 11, 137, 4, 1, 97, 0],
            &[1, 1, 11, 139, 4, 132, 11, 138, 4, 1, 98, 0],
            &[1, 1, 11, 140, 4, 132, 11, 139, 4, 1, 97, 0],
            &[1, 1, 11, 141, 4, 132, 11, 140, 4, 1, 98, 0],
            &[1, 1, 11, 142, 4, 132, 11, 141, 4, 1, 97, 0],
            &[1, 1, 11, 143, 4, 132, 11, 142, 4, 1, 98, 0],
            &[1, 1, 11, 144, 4, 132, 11, 143, 4, 1, 97, 0],
            &[1, 1, 11, 145, 4, 132, 11, 144, 4, 1, 98, 0],
            &[1, 1, 11, 146, 4, 132, 11, 145, 4, 1, 97, 0],
            &[1, 1, 11, 147, 4, 132, 11, 146, 4, 1, 98, 0],
            &[1, 1, 11, 148, 4, 132, 11, 147, 4, 1, 97, 0],
            &[1, 1, 11, 149, 4, 132, 11, 148, 4, 1, 98, 0],
            &[1, 1, 11, 150, 4, 132, 11, 149, 4, 1, 98, 0],
            &[1, 1, 11, 151, 4, 132, 11, 150, 4, 1, 97, 0],
            &[1, 1, 11, 152, 4, 132, 11, 151, 4, 1, 98, 0],
            &[1, 1, 11, 153, 4, 132, 11, 152, 4, 1, 97, 0],
            &[1, 1, 11, 154, 4, 132, 11, 153, 4, 1, 98, 0],
            &[1, 1, 11, 155, 4, 132, 11, 154, 4, 1, 97, 0],
            &[1, 1, 11, 156, 4, 132, 11, 155, 4, 1, 98, 0],
            &[1, 1, 11, 157, 4, 132, 11, 156, 4, 1, 97, 0],
            &[1, 1, 11, 158, 4, 132, 11, 157, 4, 1, 97, 0],
            &[1, 1, 11, 159, 4, 132, 11, 158, 4, 1, 98, 0],
            &[1, 1, 11, 160, 4, 132, 11, 159, 4, 1, 98, 0],
            &[1, 1, 11, 161, 4, 132, 11, 160, 4, 1, 97, 0],
            &[1, 1, 11, 162, 4, 132, 11, 161, 4, 1, 98, 0],
            &[1, 1, 11, 163, 4, 132, 11, 162, 4, 1, 97, 0],
            &[1, 1, 11, 164, 4, 132, 11, 163, 4, 1, 98, 0],
            &[1, 1, 11, 165, 4, 132, 11, 164, 4, 1, 97, 0],
            &[1, 1, 11, 166, 4, 132, 11, 165, 4, 1, 98, 0],
            &[1, 1, 11, 167, 4, 132, 11, 166, 4, 1, 97, 0],
            &[1, 1, 11, 168, 4, 132, 11, 167, 4, 1, 98, 0],
            &[1, 1, 11, 169, 4, 132, 11, 168, 4, 1, 98, 0],
            &[1, 1, 11, 170, 4, 132, 11, 169, 4, 1, 97, 0],
            &[1, 1, 11, 171, 4, 132, 11, 170, 4, 1, 98, 0],
            &[1, 1, 11, 172, 4, 132, 11, 171, 4, 1, 97, 0],
            &[1, 1, 11, 173, 4, 132, 11, 172, 4, 1, 98, 0],
            &[1, 1, 11, 174, 4, 132, 11, 173, 4, 1, 97, 0],
            &[1, 1, 11, 175, 4, 132, 11, 174, 4, 1, 98, 0],
            &[1, 1, 11, 176, 4, 132, 11, 175, 4, 1, 97, 0],
            &[1, 1, 11, 177, 4, 132, 11, 176, 4, 1, 98, 0],
            &[1, 1, 11, 178, 4, 132, 11, 177, 4, 1, 98, 0],
            &[1, 1, 11, 179, 4, 132, 11, 178, 4, 1, 97, 0],
            &[1, 1, 11, 180, 4, 132, 11, 179, 4, 1, 98, 0],
            &[1, 1, 11, 181, 4, 132, 11, 180, 4, 1, 97, 0],
            &[1, 1, 11, 182, 4, 132, 11, 181, 4, 1, 98, 0],
            &[1, 1, 11, 183, 4, 132, 11, 182, 4, 1, 97, 0],
            &[1, 1, 11, 184, 4, 132, 11, 183, 4, 1, 98, 0],
            &[1, 1, 11, 185, 4, 132, 11, 184, 4, 1, 98, 0],
            &[1, 1, 11, 186, 4, 132, 11, 185, 4, 1, 97, 0],
            &[1, 1, 11, 187, 4, 132, 11, 186, 4, 1, 98, 0],
            &[1, 1, 11, 188, 4, 132, 11, 187, 4, 1, 97, 0],
            &[1, 1, 11, 189, 4, 132, 11, 188, 4, 1, 98, 0],
            &[1, 1, 11, 190, 4, 132, 11, 189, 4, 1, 32, 0],
            &[1, 1, 11, 191, 4, 132, 11, 190, 4, 1, 97, 0],
            &[1, 1, 11, 192, 4, 132, 11, 191, 4, 1, 98, 0],
            &[1, 1, 11, 193, 4, 132, 11, 192, 4, 1, 97, 0],
            &[1, 1, 11, 194, 4, 132, 11, 193, 4, 1, 98, 0],
            &[1, 1, 11, 195, 4, 132, 11, 194, 4, 1, 97, 0],
            &[1, 1, 11, 196, 4, 132, 11, 195, 4, 1, 98, 0],
            &[1, 1, 11, 197, 4, 132, 11, 196, 4, 1, 97, 0],
            &[1, 1, 11, 198, 4, 132, 11, 197, 4, 1, 98, 0],
            &[1, 1, 11, 199, 4, 132, 11, 198, 4, 1, 97, 0],
            &[1, 1, 11, 200, 4, 132, 11, 199, 4, 1, 98, 0],
            &[1, 1, 11, 201, 4, 132, 11, 200, 4, 1, 97, 0],
            &[1, 1, 11, 202, 4, 132, 11, 201, 4, 1, 98, 0],
            &[1, 1, 11, 203, 4, 132, 11, 202, 4, 1, 98, 0],
            &[1, 1, 11, 204, 4, 132, 11, 203, 4, 1, 97, 0],
            &[1, 1, 11, 205, 4, 132, 11, 204, 4, 1, 98, 0],
            &[1, 1, 11, 206, 4, 132, 11, 205, 4, 1, 97, 0],
            &[1, 1, 11, 207, 4, 132, 11, 206, 4, 1, 98, 0],
            &[1, 1, 11, 208, 4, 132, 11, 207, 4, 1, 97, 0],
            &[1, 1, 11, 209, 4, 132, 11, 208, 4, 1, 98, 0],
            &[1, 1, 11, 210, 4, 132, 11, 209, 4, 1, 97, 0],
            &[1, 1, 11, 211, 4, 132, 11, 210, 4, 1, 98, 0],
        ];

        let updates: Vec<_> = input.into_iter().map(|&u| Update::decode_v1(u)).collect();

        let combined = Update::merge_updates(updates);
        let doc = Doc::new();
        let mut txn = doc.transact();
        let body = txn.get_text("body");
        txn.apply_update(combined);
        assert_eq!(body.to_string(&txn), "ababababababababababababaabababababababababababababababababababababbababababababababababababbabaabbababababababbababababbabababababbabaababababababbabababababbabababbabababababababbababaababbabababababbabababbabababbababababababababbabababbababababababbababababbabababbabababababaabbababababbababababbabababababbabababababababbababababababbabababababbabababababbabaabababbabababababababababababbababababbabababbabababababbabababababbaabababababababbababababababbababababababababbababababababababababbabababababababbabababbababababababbabababaabbababababbababababbabababbabab ababababababbabababab");
    }
}
