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
    pub fn integrate(
        mut self,
        txn: &mut Transaction<'_>,
    ) -> (Option<PendingUpdate>, Option<Update>) {
        let remaining_blocks = if self.blocks.is_empty() {
            None
        } else {
            let mut client_block_ref_ids: Vec<u64> = self.blocks.clients.keys().cloned().collect();
            client_block_ref_ids.sort_by(|a, b| b.cmp(a));

            let mut current_client_id = client_block_ref_ids.pop();
            let mut current_target =
                current_client_id.and_then(|id| self.blocks.clients.get_mut(&id));
            let mut stack_head = Self::next(&mut current_target);

            let mut local_sv = txn.store.blocks.get_state_vector();
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
                        let block_refs = txn.store.blocks.get_client_blocks_mut(dep);
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
                        block.as_item_mut().map(|item| item.repair(txn));
                        let should_delete = block.integrate(txn, offset, offset);
                        let delete_ptr = if should_delete {
                            Some(BlockPtr::new(block.id().clone(), offset))
                        } else {
                            None
                        };

                        let blocks = txn.store.blocks.get_client_blocks_mut(client);
                        blocks.push(block);

                        if let Some(ptr) = delete_ptr {
                            txn.delete(&ptr);
                        }
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
                        TypePtr::Named(Rc::new(decoder.read_string().to_owned()))
                    } else {
                        TypePtr::Id(BlockPtr::from(decoder.read_left_id()))
                    }
                } else {
                    TypePtr::Unknown
                };
                let parent_sub = if cant_copy_parent_info && (info & HAS_PARENT_SUB != 0) {
                    Some(decoder.read_string().to_owned())
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
                    let clock_diff = left.id().clock - right.id().clock;
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
        let mut store = Store::new(0);
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
    use std::rc::Rc;

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
            TypePtr::Named(Rc::new("".to_owned())),
            Some("keyB".to_owned()),
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
}
