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

#[derive(Debug, PartialEq, Default, Clone)]
pub(crate) struct UpdateBlocks {
    clients: HashMap<u64, VecDeque<Block>, BuildHasherDefault<ClientHasher>>,
}

impl UpdateBlocks {
    /**
    @todo this should be refactored.
    I'm currently using this to add blocks to the Update
    */
    pub(crate) fn add_block(&mut self, block: &Block, offset: u32) {
        let copy = block.slice(offset);
        match self.clients.entry(copy.id().client) {
            Entry::Occupied(e) => e.into_mut().push_back(copy),
            Entry::Vacant(e) => {
                let mut q = VecDeque::new();
                q.push_back(copy);
                e.insert(q);
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Returns an iterator that allows a traversal of all of the blocks
    /// which consist into this [Update].
    pub(crate) fn blocks(&self) -> Blocks<'_> {
        Blocks::new(self)
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
#[derive(Debug, PartialEq, Default, Clone)]
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
                let parent_sub = if cant_copy_parent_info && (info & HAS_PARENT_SUB != 0) {
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

    pub fn merge_updates<T: std::iter::IntoIterator<Item = Update>>(block_stores: T) -> Update {
        let mut result = Update::new();
        let update_blocks: Vec<UpdateBlocks> = block_stores
            .into_iter()
            .map(|update| {
                result.delete_set.merge(update.delete_set);
                update.blocks
            })
            .collect();

        let mut lazy_struct_decoders: Vec<Blocks> = update_blocks
            .iter()
            .filter(|block_store| !block_store.is_empty())
            .map(|update_blocks| {
                let mut blocks = update_blocks.blocks();
                blocks.next();
                blocks
            })
            .collect();

        let mut curr_write: Option<Block> = None;

        // Note: We need to ensure that all lazyStructDecoders are fully consumed
        // Note: Should merge document updates whenever possible - even from different updates
        // Note: Should handle that some operations cannot be applied yet ()
        loop {
            // Remove the decoder if we consumed all of its blocks.
            if !lazy_struct_decoders.is_empty() && lazy_struct_decoders[0].current.is_none() {
                lazy_struct_decoders.remove(0);
            }
            if lazy_struct_decoders.is_empty() {
                break;
            }
            // Write higher clients first ⇒ sort by clientID & clock and remove decoders without content
            lazy_struct_decoders.sort_by(|dec1, dec2| {
                let left = dec1.current.unwrap();
                let right = dec2.current.unwrap();
                if left.id().client == right.id().client {
                    let clock_diff = left.id().clock as i32 - right.id().clock as i32;
                    if clock_diff == 0 {
                        return if left.same_type(right) {
                            Ordering::Equal
                        } else {
                            if left.is_skip() {
                                Ordering::Greater
                            } else {
                                Ordering::Less
                            }
                        };
                    } else {
                        if clock_diff <= 0 && (!left.is_skip() || right.is_skip()) {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        }
                    }
                } else {
                    right.id().client.cmp(&left.id().client)
                }
            });

            let curr_decoder = &mut lazy_struct_decoders[0];

            // write from currDecoder until the next operation is from another client or if filler-struct
            // then we need to reorder the decoders and find the next operation to write
            let mut curr: Option<&Block> = curr_decoder.current;
            let tmp_curr;
            let first_client = curr.unwrap().id().client;

            if let Some(curr_write_block) = &mut curr_write {
                let mut iterated = false;

                // iterate until we find something that we haven't written already
                // remember: first the high client-ids are written

                loop {
                    if let Some(block) = curr {
                        if block.id().clock + block.len()
                            < curr_write_block.id().clock + curr_write_block.len()
                            && block.id().client >= curr_write_block.id().client
                        {
                            curr = curr_decoder.next();
                            iterated = true;
                            continue;
                        }
                    }
                    break;
                }
                if curr.is_none() {
                    // continue if decoder is empty
                    continue;
                }
                let curr_unwrapped = curr.clone().unwrap();
                if
                // check whether there is another decoder that has has updates from `first_client`
                curr_unwrapped.id().client != first_client ||
                    // the above while loop was used and we are potentially missing updates
                    (iterated && curr_unwrapped.id().clock > curr_write_block.id().clock + curr_write_block.len())
                {
                    continue;
                }

                if first_client != curr_write_block.id().client {
                    result.blocks.add_block(&curr_unwrapped, 0);
                    curr = curr_decoder.next();
                } else {
                    if curr_write_block.id().clock + curr_write_block.len()
                        < curr_unwrapped.id().clock
                    {
                        // @todo write currStruct & set currStruct = Skip(clock = currStruct.id.clock + currStruct.length, length = curr.id.clock - self.clock)
                        if let Block::Skip(curr_write_skip) = curr_write_block {
                            // extend existing skip
                            let len = curr_unwrapped.id().clock + curr_unwrapped.len()
                                - curr_write_skip.id.clock;
                            curr_write = Some(Block::Skip(Skip {
                                id: curr_write_skip.id,
                                len,
                            }));
                        } else {
                            result.blocks.add_block(curr_write_block, 0);
                            let diff = curr_unwrapped.id().clock
                                - curr_write_block.id().clock
                                - curr_write_block.len();

                            let next_id = ID {
                                client: first_client,
                                clock: curr_write_block.id().clock + curr_write_block.len(),
                            };

                            curr_write = Some(Block::Skip(Skip {
                                id: next_id,
                                len: diff,
                            }));
                        }
                    } else {
                        // if (currWrite.struct.id.clock + currWrite.struct.length >= curr.id.clock) {

                        let diff = curr_write_block.id().clock + curr_write_block.len()
                            - curr_unwrapped.id().clock;
                        if diff > 0 {
                            if let Block::Skip(curr_write_skip) = curr_write_block {
                                // prefer to slice Skip because the other struct might contain more information
                                curr_write_skip.len -= diff;
                            } else {
                                tmp_curr = Some(curr_unwrapped.slice(diff));
                                curr = tmp_curr.as_ref();
                            }
                        }
                        if curr_write_block.try_squash(&curr_unwrapped) {
                            result.blocks.add_block(curr_write_block, 0);
                            curr_write = Some(curr_unwrapped.clone());
                            curr = curr_decoder.next();
                        }
                    }
                }
            } else {
                curr_write = curr_decoder.current.cloned();
                curr = curr_decoder.next();
            }
            loop {
                if let Some(next) = curr {
                    let curr_write_block = curr_write.as_ref().unwrap();
                    if next.id().client == first_client
                        && next.id().clock == curr_write_block.id().clock + curr_write_block.len()
                    {
                        result.blocks.add_block(&curr_write.unwrap(), 0);
                        curr_write = Some(next.slice(0));
                        curr_decoder.next();
                        continue;
                    }
                }
                break;
            }
        }
        if let Some(curr_write_block) = &curr_write {
            result.blocks.add_block(&curr_write_block, 0);
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
    pub(crate) current: Option<&'a Block>,
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
            current: None,
        }
    }

    fn _next(&mut self) -> Option<&'a Block> {
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

impl<'a> Iterator for Blocks<'a> {
    type Item = &'a Block;

    fn next(&mut self) -> Option<Self::Item> {
        self.current = self._next();
        self.current
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
        const input: &str = r#"1494,0x01010B00040104626F6479016100
1495,0x01010B02840B01016100
1496,0x01010B01840B00016200
1497,0x01010B03840B02016200
1498,0x01010B04840B03016100
1499,0x01010B05840B04016200
1500,0x01010B06840B05016100
1501,0x01010B07840B06016200
1502,0x01010B08840B07016100
1503,0x01010B09840B08016200
1504,0x01010B0A840B09016100
1505,0x01010B0B840B0A016200
1506,0x01010B0C840B0B016100
1507,0x01010B0D840B0C016200
1508,0x01010B0E840B0D016100
1509,0x01010B0F840B0E016200
1510,0x01010B10840B0F016100
1511,0x01010B11840B10016200
1512,0x01010B12840B11016100
1513,0x01010B13840B12016200
1514,0x01010B14840B13016100
1515,0x01010B15840B14016200
1516,0x01010B16840B15016100
1517,0x01010B17840B16016200
1518,0x01010B18840B17016100
1519,0x01010B19840B18016100
1520,0x01010B1A840B19016200
1521,0x01010B1B840B1A016100
1522,0x01010B1C840B1B016200
1523,0x01010B1D840B1C016100
1524,0x01010B1E840B1D016200
1525,0x01010B1F840B1E016100
1526,0x01010B20840B1F016200
1527,0x01010B21840B20016100
1528,0x01010B22840B21016200
1529,0x01010B23840B22016100
1530,0x01010B24840B23016200
1531,0x01010B25840B24016100
1532,0x01010B26840B25016200
1533,0x01010B27840B26016100
1534,0x01010B28840B27016200
1535,0x01010B29840B28016100
1536,0x01010B2A840B29016200
1537,0x01010B2B840B2A016100
1538,0x01010B2C840B2B016200
1539,0x01010B2D840B2C016100
1540,0x01010B2E840B2D016200
1541,0x01010B2F840B2E016100
1542,0x01010B30840B2F016200
1543,0x01010B31840B30016100
1544,0x01010B32840B31016200
1545,0x01010B33840B32016100
1546,0x01010B34840B33016200
1547,0x01010B35840B34016100
1548,0x01010B36840B35016200
1549,0x01010B37840B36016100
1550,0x01010B38840B37016200
1551,0x01010B39840B38016100
1552,0x01010B3A840B39016200
1553,0x01010B3B840B3A016100
1554,0x01010B3C840B3B016200
1555,0x01010B3D840B3C016100
1556,0x01010B3E840B3D016200
1557,0x01010B3F840B3E016100
1558,0x01010B40840B3F016200
1559,0x01010B41840B40016100
1560,0x01010B42840B41016200
1561,0x01010B43840B42016200
1562,0x01010B44840B43016100
1563,0x01010B45840B44016200
1564,0x01010B46840B45016100
1565,0x01010B47840B46016200
1566,0x01010B48840B47016100
1567,0x01010B49840B48016200
1568,0x01010B4A840B49016100
1569,0x01010B4B840B4A016200
1570,0x01010B4C840B4B016100
1571,0x01010B4D840B4C016200
1572,0x01010B4E840B4D016100
1573,0x01010B4F840B4E016200
1574,0x01010B50840B4F016100
1575,0x01010B51840B50016200
1576,0x01010B52840B51016100
1577,0x01010B53840B52016200
1578,0x01010B54840B53016100
1579,0x01010B55840B54016200
1580,0x01010B56840B55016100
1581,0x01010B57840B56016200
1582,0x01010B58840B57016100
1583,0x01010B59840B58016200
1584,0x01010B5A840B59016100
1585,0x01010B5B840B5A016200
1586,0x01010B5C840B5B016200
1587,0x01010B5D840B5C016100
1588,0x01010B5E840B5D016200
1589,0x01010B5F840B5E016100
1590,0x01010B60840B5F016100
1591,0x01010B61840B60016200
1592,0x01010B62840B61016200
1593,0x01010B63840B62016100
1594,0x01010B64840B63016200
1595,0x01010B65840B64016100
1596,0x01010B66840B65016200
1597,0x01010B67840B66016100
1598,0x01010B68840B67016200
1599,0x01010B69840B68016100
1600,0x01010B6A840B69016200
1601,0x01010B6B840B6A016100
1602,0x01010B6C840B6B016200
1603,0x01010B6D840B6C016100
1604,0x01010B6E840B6D016200
1605,0x01010B6F840B6E016200
1606,0x01010B70840B6F016100
1607,0x01010B71840B70016200
1608,0x01010B72840B71016100
1609,0x01010B73840B72016200
1610,0x01010B74840B73016100
1611,0x01010B75840B74016200
1612,0x01010B76840B75016100
1613,0x01010B77840B76016200
1614,0x01010B78840B77016200
1615,0x01010B79840B78016100
1616,0x01010B7A840B79016200
1617,0x01010B7B840B7A016100
1618,0x01010B7C840B7B016200
1619,0x01010B7D840B7C016100
1620,0x01010B7E840B7D016200
1621,0x01010B7F840B7E016100
1622,0x01010B8001840B7F016200
1623,0x01010B8101840B8001016100
1624,0x01010B8201840B8101016200
1625,0x01010B8301840B8201016200
1626,0x01010B8401840B8301016100
1627,0x01010B8501840B8401016200
1628,0x01010B8601840B8501016100
1629,0x01010B8701840B8601016100
1630,0x01010B8801840B8701016200
1631,0x01010B8901840B8801016100
1632,0x01010B8A01840B8901016200
1633,0x01010B8B01840B8A01016100
1634,0x01010B8C01840B8B01016200
1635,0x01010B8D01840B8C01016100
1636,0x01010B8E01840B8D01016200
1637,0x01010B8F01840B8E01016100
1638,0x01010B9001840B8F01016200
1639,0x01010B9101840B9001016100
1640,0x01010B9201840B9101016200
1641,0x01010B9301840B9201016200
1642,0x01010B9401840B9301016100
1643,0x01010B9501840B9401016200
1644,0x01010B9601840B9501016100
1645,0x01010B9701840B9601016200
1646,0x01010B9801840B9701016100
1647,0x01010B9901840B9801016200
1648,0x01010B9A01840B9901016100
1649,0x01010B9B01840B9A01016200
1650,0x01010B9C01840B9B01016100
1651,0x01010B9D01840B9C01016200
1652,0x01010B9E01840B9D01016200
1653,0x01010B9F01840B9E01016100
1654,0x01010BA001840B9F01016200
1655,0x01010BA101840BA001016100
1656,0x01010BA201840BA101016200
1657,0x01010BA301840BA201016100
1658,0x01010BA401840BA301016200
1659,0x01010BA501840BA401016200
1660,0x01010BA601840BA501016100
1661,0x01010BA701840BA601016200
1662,0x01010BA801840BA701016100
1663,0x01010BA901840BA801016200
1664,0x01010BAA01840BA901016100
1665,0x01010BAB01840BAA01016200
1666,0x01010BAC01840BAB01016100
1667,0x01010BAD01840BAC01016200
1668,0x01010BAE01840BAD01016100
1669,0x01010BAF01840BAE01016200
1670,0x01010BB001840BAF01016100
1671,0x01010BB101840BB001016200
1672,0x01010BB201840BB101016100
1673,0x01010BB401840BB301016200
1674,0x01010BB501840BB401016100
1675,0x01010BB301840BB201016200
1676,0x01010BB601840BB501016200
1677,0x01010BB701840BB601016100
1678,0x01010BB801840BB701016200
1679,0x01010BB901840BB801016100
1680,0x01010BBA01840BB901016100
1681,0x01010BBB01840BBA01016200
1682,0x01010BBC01840BBB01016100
1683,0x01010BBD01840BBC01016200
1684,0x01010BBE01840BBD01016200
1685,0x01010BBF01840BBE01016100
1686,0x01010BC001840BBF01016200
1687,0x01010BC101840BC001016100
1688,0x01010BC201840BC101016200
1689,0x01010BC301840BC201016100
1690,0x01010BC401840BC301016200
1691,0x01010BC501840BC401016100
1692,0x01010BC601840BC501016200
1693,0x01010BC701840BC601016100
1694,0x01010BC801840BC701016200
1695,0x01010BC901840BC801016200
1696,0x01010BCA01840BC901016100
1697,0x01010BCB01840BCA01016200
1698,0x01010BCC01840BCB01016100
1699,0x01010BCD01840BCC01016200
1700,0x01010BCE01840BCD01016100
1701,0x01010BCF01840BCE01016200
1702,0x01010BD001840BCF01016200
1703,0x01010BD101840BD001016100
1704,0x01010BD201840BD101016200
1705,0x01010BD301840BD201016100
1706,0x01010BD401840BD301016200
1707,0x01010BD501840BD401016100
1708,0x01010BD601840BD501016200
1709,0x01010BD701840BD601016200
1710,0x01010BD801840BD701016100
1711,0x01010BD901840BD801016200
1712,0x01010BDA01840BD901016100
1713,0x01010BDB01840BDA01016200
1714,0x01010BDC01840BDB01016100
1715,0x01010BDD01840BDC01016200
1716,0x01010BDE01840BDD01016100
1717,0x01010BDF01840BDE01016200
1718,0x01010BE001840BDF01016100
1719,0x01010BE101840BE001016200
1720,0x01010BE201840BE101016100
1721,0x01010BE301840BE201016200
1722,0x01010BE401840BE301016100
1723,0x01010BE501840BE401016200
1724,0x01010BE601840BE501016100
1725,0x01010BE701840BE601016200
1726,0x01010BE801840BE701016200
1727,0x01010BE901840BE801016100
1728,0x01010BEA01840BE901016200
1729,0x01010BEB01840BEA01016100
1730,0x01010BEC01840BEB01016200
1731,0x01010BED01840BEC01016100
1732,0x01010BEE01840BED01016200
1733,0x01010BEF01840BEE01016200
1734,0x01010BF001840BEF01016100
1735,0x01010BF101840BF001016200
1736,0x01010BF201840BF101016100
1737,0x01010BF301840BF201016200
1738,0x01010BF401840BF301016100
1739,0x01010BF501840BF401016200
1740,0x01010BF601840BF501016100
1741,0x01010BF701840BF601016200
1742,0x01010BF801840BF701016100
1743,0x01010BF901840BF801016200
1744,0x01010BFA01840BF901016100
1745,0x01010BFB01840BFA01016200
1746,0x01010BFC01840BFB01016200
1747,0x01010BFD01840BFC01016100
1748,0x01010BFE01840BFD01016200
1749,0x01010BFF01840BFE01016100
1750,0x01010B8002840BFF01016200
1751,0x01010B8102840B8002016100
1752,0x01010B8202840B8102016200
1753,0x01010B8302840B8202016100
1754,0x01010B8402840B8302016200
1755,0x01010B8502840B8402016200
1756,0x01010B8602840B8502016100
1757,0x01010B8702840B8602016200
1758,0x01010B8802840B8702016100
1759,0x01010B8902840B8802016200
1760,0x01010B8A02840B8902016100
1761,0x01010B8B02840B8A02016200
1762,0x01010B8C02840B8B02016200
1763,0x01010B8D02840B8C02016100
1764,0x01010B8E02840B8D02016200
1765,0x01010B8F02840B8E02016100
1766,0x01010B9002840B8F02016200
1767,0x01010B9102840B9002016100
1768,0x01010B9202840B9102016200
1769,0x01010B9302840B9202016100
1770,0x01010B9402840B9302016200
1771,0x01010B9502840B9402016100
1772,0x01010B9602840B9502016200
1773,0x01010B9702840B9602016100
1774,0x01010B9802840B9702016100
1775,0x01010B9902840B9802016200
1776,0x01010B9A02840B9902016200
1777,0x01010B9B02840B9A02016100
1778,0x01010B9C02840B9B02016200
1779,0x01010B9D02840B9C02016100
1780,0x01010B9E02840B9D02016200
1781,0x01010B9F02840B9E02016100
1782,0x01010BA002840B9F02016200
1783,0x01010BA102840BA002016100
1784,0x01010BA202840BA102016200
1785,0x01010BA302840BA202016200
1786,0x01010BA402840BA302016100
1787,0x01010BA502840BA402016200
1788,0x01010BA602840BA502016100
1789,0x01010BA702840BA602016200
1790,0x01010BA802840BA702016100
1791,0x01010BA902840BA802016200
1792,0x01010BAA02840BA902016100
1793,0x01010BAB02840BAA02016200
1794,0x01010BAC02840BAB02016200
1795,0x01010BAD02840BAC02016100
1796,0x01010BAE02840BAD02016200
1797,0x01010BAF02840BAE02016100
1798,0x01010BB002840BAF02016200
1799,0x01010BB102840BB002016100
1800,0x01010BB202840BB102016200
1801,0x01010BB302840BB202016100
1802,0x01010BB402840BB302016200
1803,0x01010BB502840BB402016100
1804,0x01010BB602840BB502016200
1805,0x01010BB702840BB602016200
1806,0x01010BB802840BB702016100
1807,0x01010BB902840BB802016200
1808,0x01010BBA02840BB902016100
1809,0x01010BBB02840BBA02016200
1810,0x01010BBC02840BBB02016100
1811,0x01010BBD02840BBC02016200
1812,0x01010BBE02840BBD02016100
1813,0x01010BBF02840BBE02016200
1814,0x01010BC002840BBF02016100
1815,0x01010BC102840BC002016200
1816,0x01010BC202840BC102016100
1817,0x01010BC302840BC202016200
1818,0x01010BC402840BC302016100
1819,0x01010BC502840BC402016200
1820,0x01010BC602840BC502016200
1821,0x01010BC702840BC602016100
1822,0x01010BC802840BC702016200
1823,0x01010BC902840BC802016100
1824,0x01010BCA02840BC902016200
1825,0x01010BCB02840BCA02016100
1826,0x01010BCC02840BCB02016200
1827,0x01010BCD02840BCC02016100
1828,0x01010BCE02840BCD02016200
1829,0x01010BCF02840BCE02016100
1830,0x01010BD002840BCF02016200
1831,0x01010BD102840BD002016100
1832,0x01010BD202840BD102016200
1833,0x01010BD302840BD202016200
1834,0x01010BD402840BD302016100
1835,0x01010BD502840BD402016200
1836,0x01010BD602840BD502016100
1837,0x01010BD702840BD602016200
1838,0x01010BD802840BD702016100
1839,0x01010BD902840BD802016200
1840,0x01010BDA02840BD902016100
1841,0x01010BDB02840BDA02016200
1842,0x01010BDC02840BDB02016100
1843,0x01010BDD02840BDC02016200
1844,0x01010BDE02840BDD02016200
1845,0x01010BDF02840BDE02016100
1846,0x01010BE002840BDF02016200
1847,0x01010BE102840BE002016100
1848,0x01010BE202840BE102016200
1849,0x01010BE302840BE202016100
1850,0x01010BE402840BE302016200
1851,0x01010BE502840BE402016100
1852,0x01010BE602840BE502016200
1853,0x01010BE702840BE602016100
1854,0x01010BE802840BE702016200
1855,0x01010BE902840BE802016200
1856,0x01010BEA02840BE902016100
1857,0x01010BEB02840BEA02016200
1858,0x01010BEC02840BEB02016100
1859,0x01010BED02840BEC02016100
1860,0x01010BEE02840BED02016200
1861,0x01010BEF02840BEE02016100
1862,0x01010BF002840BEF02016200
1863,0x01010BF102840BF002016100
1864,0x01010BF202840BF102016200
1865,0x01010BF302840BF202016200
1866,0x01010BF402840BF302016100
1867,0x01010BF502840BF402016200
1868,0x01010BF602840BF502016100
1869,0x01010BF702840BF602016200
1870,0x01010BF802840BF702016100
1871,0x01010BF902840BF802016200
1872,0x01010BFA02840BF902016100
1873,0x01010BFB02840BFA02016200
1874,0x01010BFC02840BFB02016100
1875,0x01010BFD02840BFC02016200
1876,0x01010BFE02840BFD02016100
1877,0x01010BFF02840BFE02016200
1878,0x01010B8003840BFF02016100
1879,0x01010B8103840B8003016200
1880,0x01010B8203840B8103016100
1881,0x01010B8303840B8203016200
1882,0x01010B8403840B8303016100
1883,0x01010B8503840B8403016200
1884,0x01010B8603840B8503016100
1885,0x01010B8703840B8603016200
1886,0x01010B8803840B8703016100
1887,0x01010B8903840B8803016200
1888,0x01010B8A03840B8903016200
1889,0x01010B8B03840B8A03016100
1890,0x01010B8C03840B8B03016200
1891,0x01010B8D03840B8C03016100
1892,0x01010B8E03840B8D03016200
1893,0x01010B8F03840B8E03016100
1894,0x01010B9003840B8F03016200
1895,0x01010B9103840B9003016100
1896,0x01010B9203840B9103016200
1897,0x01010B9303840B9203016200
1898,0x01010B9403840B9303016100
1899,0x01010B9503840B9403016200
1900,0x01010B9603840B9503016100
1901,0x01010B9703840B9603016200
1902,0x01010B9803840B9703016100
1903,0x01010B9903840B9803016200
1904,0x01010B9A03840B9903016200
1905,0x01010B9B03840B9A03016100
1906,0x01010B9C03840B9B03016200
1907,0x01010B9D03840B9C03016100
1908,0x01010B9E03840B9D03016200
1909,0x01010B9F03840B9E03016100
1910,0x01010BA003840B9F03016200
1911,0x01010BA103840BA003016100
1912,0x01010BA203840BA103016200
1913,0x01010BA303840BA203016100
1914,0x01010BA403840BA303016200
1915,0x01010BA503840BA403016200
1916,0x01010BA603840BA503016100
1917,0x01010BA703840BA603016200
1918,0x01010BA803840BA703016100
1919,0x01010BA903840BA803016200
1920,0x01010BAA03840BA903016100
1921,0x01010BAB03840BAA03016200
1922,0x01010BAC03840BAB03016100
1923,0x01010BAD03840BAC03016200
1924,0x01010BAE03840BAD03016100
1925,0x01010BAF03840BAE03016200
1926,0x01010BB003840BAF03016200
1927,0x01010BB103840BB003016100
1928,0x01010BB203840BB103016100
1929,0x01010BB303840BB203016200
1930,0x01010BB403840BB303016100
1931,0x01010BB503840BB403016200
1932,0x01010BB603840BB503016100
1933,0x01010BB703840BB603016200
1934,0x01010BB803840BB703016100
1935,0x01010BB903840BB803016200
1936,0x01010BBA03840BB903016100
1937,0x01010BBB03840BBA03016200
1938,0x01010BBC03840BBB03016100
1939,0x01010BBD03840BBC03016200
1940,0x01010BBE03840BBD03016100
1941,0x01010BBF03840BBE03016200
1942,0x01010BC003840BBF03016200
1943,0x01010BC103840BC003016100
1944,0x01010BC203840BC103016200
1945,0x01010BC303840BC203016100
1946,0x01010BC403840BC303016200
1947,0x01010BC503840BC403016100
1948,0x01010BC603840BC503016200
1949,0x01010BC703840BC603016100
1950,0x01010BC803840BC703016200
1951,0x01010BC903840BC803016100
1952,0x01010BCA03840BC903016200
1953,0x01010BCB03840BCA03016100
1954,0x01010BCC03840BCB03016200
1955,0x01010BCD03840BCC03016200
1956,0x01010BCE03840BCD03016100
1957,0x01010BCF03840BCE03016200
1958,0x01010BD003840BCF03016100
1959,0x01010BD103840BD003016200
1960,0x01010BD203840BD103016100
1961,0x01010BD303840BD203016200
1962,0x01010BD403840BD303016100
1963,0x01010BD503840BD403016200
1964,0x01010BD603840BD503016100
1965,0x01010BD703840BD603016200
1966,0x01010BD803840BD703016100
1967,0x01010BD903840BD803016200
1968,0x01010BDA03840BD903016100
1969,0x01010BDB03840BDA03016200
1970,0x01010BDC03840BDB03016100
1971,0x01010BDD03840BDC03016200
1972,0x01010BDE03840BDD03016200
1973,0x01010BDF03840BDE03016100
1974,0x01010BE003840BDF03016200
1975,0x01010BE103840BE003016100
1976,0x01010BE203840BE103016200
1977,0x01010BE303840BE203016100
1978,0x01010BE403840BE303016200
1979,0x01010BE503840BE403016100
1980,0x01010BE603840BE503016200
1981,0x01010BE703840BE603016100
1982,0x01010BE803840BE703016200
1983,0x01010BE903840BE803016100
1984,0x01010BEA03840BE903016200
1985,0x01010BEB03840BEA03016100
1986,0x01010BEC03840BEB03016200
1987,0x01010BED03840BEC03016100
1988,0x01010BEE03840BED03016200
1989,0x01010BEF03840BEE03016100
1990,0x01010BF003840BEF03016200
1991,0x01010BF103840BF003016100
1992,0x01010BF203840BF103016200
1993,0x01010BF303840BF203016200
1994,0x01010BF403840BF303016100
1995,0x01010BF503840BF403016200
1996,0x01010BF603840BF503016100
1997,0x01010BF703840BF603016200
1998,0x01010BF803840BF703016100
1999,0x01010BF903840BF803016200
2000,0x01010BFA03840BF903016100
2001,0x01010BFB03840BFA03016200
2002,0x01010BFC03840BFB03016100
2003,0x01010BFD03840BFC03016200
2004,0x01010BFE03840BFD03016100
2005,0x01010BFF03840BFE03016200
2006,0x01010B8004840BFF03016100
2007,0x01010B8104840B8004016200
2008,0x01010B8204840B8104016200
2009,0x01010B8304840B8204016100
2010,0x01010B8404840B8304016200
2011,0x01010B8504840B8404016100
2012,0x01010B8604840B8504016200
2013,0x01010B8704840B8604016100
2014,0x01010B8804840B8704016200
2015,0x01010B8904840B8804016200
2016,0x01010B8A04840B8904016100
2017,0x01010B8B04840B8A04016200
2018,0x01010B8C04840B8B04016100
2019,0x01010B8D04840B8C04016200
2020,0x01010B8E04840B8D04016100
2021,0x01010B8F04840B8E04016200
2022,0x01010B9004840B8F04016100
2023,0x01010B9104840B9004016200
2024,0x01010B9204840B9104016100
2025,0x01010B9304840B9204016200
2026,0x01010B9404840B9304016100
2027,0x01010B9504840B9404016200
2028,0x01010B9604840B9504016200
2029,0x01010B9704840B9604016100
2030,0x01010B9804840B9704016200
2031,0x01010B9904840B9804016100
2032,0x01010B9A04840B9904016200
2033,0x01010B9B04840B9A04016100
2034,0x01010B9C04840B9B04016200
2035,0x01010B9D04840B9C04016100
2036,0x01010B9E04840B9D04016100
2037,0x01010B9F04840B9E04016200
2038,0x01010BA004840B9F04016200
2039,0x01010BA104840BA004016100
2040,0x01010BA204840BA104016200
2041,0x01010BA304840BA204016100
2042,0x01010BA404840BA304016200
2043,0x01010BA504840BA404016100
2044,0x01010BA604840BA504016200
2045,0x01010BA704840BA604016100
2046,0x01010BA804840BA704016200
2047,0x01010BA904840BA804016200
2048,0x01010BAA04840BA904016100
2049,0x01010BAB04840BAA04016200
2050,0x01010BAC04840BAB04016100
2051,0x01010BAD04840BAC04016200
2052,0x01010BAE04840BAD04016100
2053,0x01010BAF04840BAE04016200
2054,0x01010BB004840BAF04016100
2055,0x01010BB104840BB004016200
2056,0x01010BB204840BB104016200
2057,0x01010BB304840BB204016100
2058,0x01010BB404840BB304016200
2059,0x01010BB504840BB404016100
2060,0x01010BB604840BB504016200
2061,0x01010BB704840BB604016100
2062,0x01010BB804840BB704016200
2063,0x01010BB904840BB804016200
2064,0x01010BBA04840BB904016100
2065,0x01010BBB04840BBA04016200
2066,0x01010BBC04840BBB04016100
2067,0x01010BBD04840BBC04016200
2068,0x01010BBE04840BBD04012000
2069,0x01010BBF04840BBE04016100
2070,0x01010BC004840BBF04016200
2071,0x01010BC104840BC004016100
2072,0x01010BC204840BC104016200
2073,0x01010BC304840BC204016100
2074,0x01010BC404840BC304016200
2075,0x01010BC504840BC404016100
2076,0x01010BC604840BC504016200
2077,0x01010BC704840BC604016100
2078,0x01010BC804840BC704016200
2079,0x01010BC904840BC804016100
2080,0x01010BCA04840BC904016200
2081,0x01010BCB04840BCA04016200
2082,0x01010BCC04840BCB04016100
2083,0x01010BCD04840BCC04016200
2084,0x01010BCE04840BCD04016100
2085,0x01010BCF04840BCE04016200
2086,0x01010BD004840BCF04016100
2087,0x01010BD104840BD004016200
2088,0x01010BD204840BD104016100
2089,0x01010BD304840BD204016200"#;
        fn from_hex(s: &str) -> Vec<u8> {
            fn val(c: u8, idx: usize) -> u8 {
                match c {
                    b'A'..=b'F' => c - b'A' + 10,
                    b'a'..=b'f' => c - b'a' + 10,
                    b'0'..=b'9' => c - b'0',
                    _ => panic!("not a hex byte string"),
                }
            }
            s[2..]
                .as_bytes()
                .chunks(2)
                .enumerate()
                .map(|(i, pair)| val(pair[0], 2 * i) << 4 | val(pair[1], 2 * i + 1))
                .collect()
        }

        let updates: Vec<_> = input
            .lines()
            .map(|l| {
                let (id, hex) = l.split_once(',').unwrap();
                let hex = from_hex(hex);
                Update::decode_v1(hex.as_slice())
            })
            .collect();

        let combined = Update::merge_updates(updates);
        let doc = Doc::new();
        let mut txn = doc.transact();
        let body = txn.get_text("body");
        txn.apply_update(combined);
        assert_eq!(body.to_string(&txn), "ababababababababababababaabababababababababababababababababababababbababababababababababababbabaabbababababababbababababbabababababbabaababababababbabababababbabababbabababababababbababaababbabababababbabababbabababbababababababababbabababbababababababbababababbabababbabababababaabbababababbababababbabababababbabababababababbababababababbabababababbabababababbabaabababbabababababababababababbababababbabababbabababababbabababababbaabababababababbababababababbababababababababbababababababababababbabababababababbabababbababababababbabababaabbababababbababababbabababbabab ababababababbabababab");
    }
}
