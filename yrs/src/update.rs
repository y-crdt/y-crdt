use crate::block::{
    Block, BlockRange, ClientID, Item, ItemContent, BLOCK_GC_REF_NUMBER, BLOCK_SKIP_REF_NUMBER,
    HAS_ORIGIN, HAS_PARENT_SUB, HAS_RIGHT_ORIGIN,
};
use crate::branch::BranchPtr;
use crate::encoding::read::Error;
use crate::error::UpdateError;
use crate::id_set::IdSet;
use crate::store::Store;
use crate::transaction::TransactionMut;
use crate::types::{TypePtr, TypeRef};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::{StateVector, ID};
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::sync::Arc;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct BlockSet {
    pub(crate) clients: HashMap<ClientID, VecDeque<Block>, BuildHasherDefault<ClientHasher>>,
}

impl BlockSet {
    /**
    @todo this should be refactored.
    I'm currently using this to add blocks to the Update
    */
    pub(crate) fn add_block(&mut self, block: Block) {
        let e = self.clients.entry(*block.client()).or_default();
        e.push_back(block);
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Returns an iterator that allows a traversal of all the blocks
    /// which consist into this [Update].
    pub(crate) fn into_blocks(self, ignore_skip: bool) -> IntoBlocks {
        IntoBlocks::new(self, ignore_skip)
    }

    pub fn as_id_set(&self) -> IdSet {
        let mut inserts = IdSet::default();

        for (client, blocks) in self.clients.iter() {
            let mut last_clock = 0;
            let mut last_len = 0;
            for block in blocks {
                if block.is_skip() {
                    continue;
                }
                let clock = block.clock_start();
                if last_clock + last_len == clock {
                    // default case: extend prev entry
                    last_len += block.len();
                } else {
                    if last_len > 0 {
                        inserts.insert(ID::new(*client, last_clock), last_len);
                    }
                    last_clock = clock;
                    last_len = block.len();
                }
            }
            inserts.insert(ID::new(*client, last_clock), last_len);
        }

        inserts
    }

    pub fn exclude(&mut self, exclude: &IdSet) {
        let client_ids: Vec<ClientID> = if self.clients.len() < exclude.len() {
            self.clients.keys().copied().collect()
        } else {
            exclude.client_ids().collect()
        };
        for client_id in client_ids {
            if let Some(id_range) = exclude.get(&client_id) {
                if let Some(structs) = self.clients.get_mut(&client_id) {
                    let clock_start = structs.front().unwrap().clock_start();
                    let clock_end = structs.back().unwrap().next_clock();
                    for (range, _) in id_range.iter() {
                        let mut start_index = 0;
                        if range.start >= clock_end {
                            continue;
                        }
                        if range.start > clock_start {
                            start_index = Self::split_at(structs, range.start);
                        }
                        let mut end_index = structs.len(); // must be set here, after structs is modified
                        if range.end <= clock_start {
                            continue;
                        }
                        if range.end < clock_end {
                            end_index = Self::split_at(structs, range.end);
                        }
                        if start_index < end_index {
                            structs[start_index] = Block::Skip(BlockRange::new(
                                ID::new(client_id, range.start),
                                range.len() as u32,
                            ));
                            if end_index - start_index > 1 {
                                structs.drain((start_index + 1)..end_index);
                            }
                        }
                    }
                }
            }
        }
    }

    fn split_at(blocks: &mut VecDeque<Block>, clock: u32) -> usize {
        let mut index = Self::find_index(blocks, clock);
        let block = &mut blocks[index];
        let block_clock = block.clock_start();
        if let Some(right) = block.splice(clock - block_clock) {
            index += 1;
            blocks.insert(index, right);
        }
        index
    }

    fn find_index(blocks: &VecDeque<Block>, clock: u32) -> usize {
        let mut left = 0;
        let mut right = blocks.len() - 1;
        let mut block = &blocks[right];
        let (mut start, mut end) = block.clock_range();
        if start == clock {
            // a common case is to just append a block at the end, so check first if we can do that
            right
        } else {
            let mut mid = ((clock / end) * right as u32) as usize;
            while left <= right {
                block = &blocks[mid];
                (start, end) = block.clock_range();
                if start <= clock {
                    if clock <= end {
                        return mid;
                    }
                    left = mid + 1;
                } else {
                    right = mid - 1;
                }
                mid = (left + right) / 2;
            }

            unreachable!()
        }
    }
}

/// Update type which contains an information about all decoded blocks which are incoming from a
/// remote peer. Since these blocks are not yet integrated into current document's block store,
/// they still may require repairing before doing so as they don't contain full data about their
/// relations.
///
/// Update is conceptually similar to a block store itself, however the work patters are different.
#[derive(Default, PartialEq)]
pub struct Update {
    pub(crate) blocks: BlockSet,
    pub(crate) delete_set: IdSet,
}

impl Update {
    /// Binary containing an empty update, serialized via lib0 version 1 encoder.
    pub const EMPTY_V1: &'static [u8] = &[0, 0];

    /// Binary containing an empty update, serialized via lib0 version 2 encoder.
    pub const EMPTY_V2: &'static [u8] = &[0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];

    pub fn new() -> Self {
        Self::default()
    }

    /// Check if current update is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.delete_set.is_empty()
    }

    /// Check if current update has changes that add new information to a document with given state.
    pub fn extends(&self, state_vector: &StateVector) -> bool {
        for (client_id, blocks) in self.blocks.clients.iter() {
            let clock = state_vector.get(client_id);
            let mut iter = blocks.iter();
            while let Some(block) = iter.next() {
                let range = block.range();
                if range.clock <= clock && range.clock + range.len > clock {
                    // this block overlaps or extends current state. It must NOT be Skip
                    // in order to introduce any new changes
                    if !block.is_skip() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns a state vector representing an upper bound of client clocks included by blocks
    /// stored in current update.
    pub fn state_vector(&self) -> StateVector {
        let mut sv = StateVector::default();
        for (&client, blocks) in self.blocks.clients.iter() {
            let mut last_clock = 0;
            if !blocks.is_empty() && blocks[0].id().clock == 0 {
                // we expect clocks to start from 0, otherwise blocks for this client are not
                // continuous
                for block in blocks.iter() {
                    if let Block::Skip(_) = block {
                        // if we met skip, we stop counting: blocks are not continuous any more
                        break;
                    }
                    last_clock = block.id().clock + block.len();
                }
            }
            if last_clock != 0 {
                sv.set_max(client, last_clock);
            }
        }
        sv
    }

    /// Returns a state vector representing a lower bound of items inserted by this update,
    /// grouped by their respective clients.
    pub fn state_vector_lower(&self) -> StateVector {
        let mut sv = StateVector::default();
        for (&client, blocks) in self.blocks.clients.iter() {
            for block in blocks.iter() {
                if !block.is_skip() {
                    let id = block.id();
                    sv.set_max(client, id.clock);
                    break;
                }
            }
        }
        sv
    }

    /// Returns an insertion set associated with current update.
    /// It contains ids of all blocks inserted by this update.
    /// If `include_deleted` flag is set, result will include GC'ed blocks and ones that were
    /// inserted but softly deleted.
    pub fn insertions(&self, include_deleted: bool) -> IdSet {
        let mut insertions = IdSet::default();
        for blocks in self.blocks.clients.values() {
            for block in blocks.iter() {
                match block {
                    Block::Item(item) if include_deleted || !item.is_deleted() => {
                        insertions.insert(item.id, item.len);
                    }
                    Block::GC(range) if include_deleted => {
                        insertions.insert(range.id(), range.len);
                    }
                    _ => {}
                }
            }
        }
        insertions
    }

    /// Returns a delete set associated with current update.
    pub fn delete_set(&self) -> &IdSet {
        &self.delete_set
    }

    /// Merges another update into current one. Their blocks are deduplicated and reordered.
    pub fn merge(&mut self, other: Self) {
        for (client, other_blocks) in other.blocks.clients {
            match self.blocks.clients.entry(client) {
                Entry::Occupied(e) => {
                    let blocks = e.into_mut();
                    let mut i2 = other_blocks.into_iter();
                    let mut n2 = i2.next();

                    let mut i1 = 0;

                    while i1 < blocks.len() {
                        let a = &mut blocks[i1];
                        if let Some(b) = n2.as_ref() {
                            if a.try_squash(b) {
                                n2 = i2.next();
                                continue;
                            } else if let Block::Item(block) = a {
                                // we only can split Block::Item
                                let diff = (block.id().clock + block.len()) as isize
                                    - b.id().clock as isize;
                                if diff > 0 {
                                    // `b`'s clock position is inside of `a` -> we need to split `a`
                                    if let Some(new) = a.splice(diff as u32) {
                                        blocks.insert(i1 + 1, new);
                                    }
                                    //blocks = self.blocks.clients.get_mut(&client).unwrap();
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
    pub(crate) fn integrate(
        mut self,
        txn: &mut TransactionMut,
    ) -> Result<(Option<PendingUpdate>, Option<Update>), UpdateError> {
        let remaining_blocks = if self.blocks.is_empty() {
            None
        } else {
            let mut picker = BlockPicker::new(&mut self.blocks);
            let mut next = picker.next();
            let mut state = HashMap::new();
            while let Some(mut stack_head) = next {
                if !stack_head.is_skip() {
                    let id = stack_head.id();
                    let len = stack_head.len();
                    let local_clock = state
                        .entry(id.client)
                        .or_insert_with(|| txn.store.blocks.get_clock(&id.client));
                    let offset = (*local_clock as i32) - (id.clock as i32);

                    if let Some(missing) =
                        Self::missing_dependency(&mut stack_head, &mut txn.store)?
                    {
                        next =
                            picker.switch(stack_head, &missing, |c| txn.store.blocks.get_clock(c));
                        continue;
                    } else {
                        // block has no missing dependencies, therefore we can integrate it right away
                        if offset < 0 {
                            // Block was send out of order (previous blocks from the same client are
                            // missing), however since it has no dependency on them, it's still safe
                            // to integrate. For that we prepend the missing hole with a Skip block.
                            txn.integrate_skip(
                                BlockRange::new(
                                    ID::new(id.client, *local_clock),
                                    offset.abs() as u32,
                                ),
                                0,
                            );
                        }
                        txn.integrate(stack_head, 0);
                        *local_clock = (*local_clock).max(id.clock + len);
                    }
                }
                next = picker.next();
            }
            picker.pending()
        };

        let remaining_ds = txn.apply_delete(&self.delete_set).map(|ds| {
            let mut update = Update::new();
            update.delete_set = ds;
            update
        });

        Ok((remaining_blocks, remaining_ds))
    }

    fn missing_dependency(
        block: &mut Block,
        store: &mut Store,
    ) -> Result<Option<ClientID>, UpdateError> {
        if let Block::Item(item) = block {
            if let Some(origin_left) = &item.origin {
                if store.blocks.is_missing(origin_left) {
                    return Ok(Some(origin_left.client));
                }
            }

            if let Some(origin_right) = &item.right_origin {
                if store.blocks.is_missing(origin_right) {
                    return Ok(Some(origin_right.client));
                }
            }

            match &item.parent {
                TypePtr::Branch(parent) => {
                    if let Some(block) = &parent.item {
                        let parent_id = block.id();
                        if store.blocks.is_missing(parent_id) {
                            return Ok(Some(parent_id.client));
                        }
                    }
                }
                TypePtr::ID(parent_id) => {
                    if store.blocks.is_missing(parent_id) {
                        return Ok(Some(parent_id.client));
                    }
                }
                _ => {}
            }

            #[cfg(feature = "weak")]
            match &item.content {
                ItemContent::Type(branch) => {
                    if let crate::types::TypeRef::WeakLink(source) = &branch.type_ref {
                        let start = source.quote_start.id();
                        let end = source.quote_end.id();
                        if let Some(start) = start {
                            if store.blocks.is_missing(start) {
                                return Ok(Some(start.client));
                            }
                        }
                        if start != end {
                            if let Some(end) = &source.quote_end.id() {
                                if store.blocks.is_missing(end) {
                                    return Ok(Some(end.client));
                                }
                            }
                        }
                    }
                }
                _ => { /* do nothing */ }
            }

            // We have all missing ids, now find the items
            if let Some(origin) = item.origin.as_ref() {
                item.left = store
                    .blocks
                    .get_item_clean_end(origin)
                    .map(|slice| store.materialize(slice));
            }

            if let Some(origin) = item.right_origin.as_ref() {
                item.right = store
                    .blocks
                    .get_item_clean_start(origin)
                    .map(|slice| store.materialize(slice));
            }

            // We have all missing ids, now find the items

            // In the original Y.js algorithm we decoded items as we go and attached them to client
            // block list. During that process if we had right origin but no left, we made a lookup for
            // right origin's parent and attach it as a parent of current block.
            //
            // Here since we decode all blocks first, then apply them, we might not find them in
            // the block store during decoding. Therefore, we retroactively reattach it here.

            item.parent = match &item.parent {
                TypePtr::Branch(branch_ptr) => TypePtr::Branch(*branch_ptr),
                TypePtr::Unknown => match (item.left, item.right) {
                    (Some(left), _) if left.parent != TypePtr::Unknown => {
                        item.parent_sub = left.parent_sub.clone();
                        left.parent.clone()
                    }
                    (_, Some(right)) if right.parent != TypePtr::Unknown => {
                        item.parent_sub = right.parent_sub.clone();
                        right.parent.clone()
                    }
                    _ => TypePtr::Unknown,
                },
                TypePtr::Named(name) => {
                    let branch = store.get_or_create_type(name.clone(), TypeRef::Undefined);
                    TypePtr::Branch(branch)
                }
                TypePtr::ID(id) => {
                    let ptr = store.blocks.get_item(id);
                    if let Some(item) = ptr {
                        match &item.content {
                            ItemContent::Type(branch) => {
                                TypePtr::Branch(BranchPtr::from(branch.as_ref()))
                            }
                            ItemContent::Deleted(_) => TypePtr::Unknown,
                            other => {
                                return Err(UpdateError::InvalidParent(
                                    id.clone(),
                                    other.get_ref_number(),
                                ))
                            }
                        }
                    } else {
                        TypePtr::Unknown
                    }
                }
            };
        }
        Ok(None)
    }

    fn next_target<'a, 'b>(
        client_block_ref_ids: &'a mut Vec<ClientID>,
        blocks: &'b mut BlockSet,
    ) -> Option<(ClientID, &'b mut VecDeque<Block>)> {
        while let Some(id) = client_block_ref_ids.pop() {
            match blocks.clients.get(&id) {
                Some(client_blocks) if !client_blocks.is_empty() => {
                    // we need to borrow client_blocks in mutable context AND we're
                    // doing so in a loop at the same time - this combination causes
                    // Rust borrow checker go nuts. TODO: remove the unsafe block
                    let client_blocks = unsafe {
                        (client_blocks as *const VecDeque<Block> as *mut VecDeque<Block>)
                            .as_mut()
                            .unwrap()
                    };
                    return Some((id, client_blocks));
                }
                _ => {}
            }
        }
        None
    }

    fn return_stack(stack: Vec<Block>, refs: &mut BlockSet, remaining: &mut BlockSet) {
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

    fn decode_block<D: Decoder>(id: ID, decoder: &mut D) -> Result<Option<Block>, Error> {
        let info = decoder.read_info()?;
        match info {
            BLOCK_SKIP_REF_NUMBER => {
                let len: u32 = decoder.read_var()?;
                Ok(Some(Block::Skip(BlockRange::new(id, len))))
            }
            BLOCK_GC_REF_NUMBER => {
                let len: u32 = decoder.read_len()?;
                Ok(Some(Block::GC(BlockRange::new(id, len))))
            }
            info => {
                let cant_copy_parent_info = info & (HAS_ORIGIN | HAS_RIGHT_ORIGIN) == 0;
                let origin = if info & HAS_ORIGIN != 0 {
                    Some(decoder.read_left_id()?)
                } else {
                    None
                };
                let right_origin = if info & HAS_RIGHT_ORIGIN != 0 {
                    Some(decoder.read_right_id()?)
                } else {
                    None
                };
                let parent = if cant_copy_parent_info {
                    if decoder.read_parent_info()? {
                        TypePtr::Named(decoder.read_string()?.into())
                    } else {
                        TypePtr::ID(decoder.read_left_id()?)
                    }
                } else {
                    TypePtr::Unknown
                };
                let parent_sub: Option<Arc<str>> =
                    if cant_copy_parent_info && (info & HAS_PARENT_SUB != 0) {
                        Some(decoder.read_string()?.into())
                    } else {
                        None
                    };
                let content = ItemContent::decode(decoder, info)?;
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
                match item {
                    None => Ok(None),
                    Some(item) => Ok(Some(Block::from(item))),
                }
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
                if block.is_skip() {
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
        encoder.write_var(sorted_clients.len());
        for (&client, (offset, blocks)) in sorted_clients {
            encoder.write_var(blocks.len());
            encoder.write_client(client);

            let mut block = blocks[0];
            encoder.write_var(block.id().clock + offset);
            block.encode_with_offset(encoder, *offset);
            for i in 1..blocks.len() {
                block = blocks[i];
                block.encode_with_offset(encoder, 0);
            }
        }
        self.delete_set.encode(encoder)
    }

    pub fn merge_updates<T>(block_stores: T) -> Update
    where
        T: IntoIterator<Item = Update>,
    {
        let mut result = Update::new();
        let update_blocks: Vec<BlockSet> = block_stores
            .into_iter()
            .map(|update| {
                result.delete_set.merge_with(update.delete_set);
                update.blocks
            })
            .collect();

        let mut lazy_struct_decoders: VecDeque<_> = update_blocks
            .into_iter()
            .filter(|block_store| !block_store.is_empty())
            .map(|update_blocks| {
                let mut memo = update_blocks.into_blocks(true).memoized();
                memo.move_next();
                memo
            })
            .collect();

        let mut curr_write: Option<Block> = None;

        // Note: We need to ensure that all lazyStructDecoders are fully consumed
        // Note: Should merge document updates whenever possible - even from different updates
        // Note: Should handle that some operations cannot be applied yet ()
        loop {
            {
                lazy_struct_decoders
                    .retain(|lazy_struct_decoder| lazy_struct_decoder.current().is_some());
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
                            ordering => ordering.reverse(),
                        }
                    });
            }

            let curr_decoder = match lazy_struct_decoders.iter_mut().next() {
                Some(decoder) => decoder,
                None => break,
            };

            let curr_block = match curr_decoder.current() {
                Some(block) => block,
                None => continue,
            };
            // write from currDecoder until the next operation is from another client or if filler-struct
            // then we need to reorder the decoders and find the next operation to write
            let first_client = curr_block.id().client;
            if let Some(curr_write_block) = curr_write.as_mut() {
                let mut iterated = false;

                // iterate until we find something that we haven't written already
                // remember: first the high client-ids are written
                let curr_write_last = curr_write_block.id().clock + curr_write_block.len();
                while match curr_decoder.current() {
                    Some(block) => {
                        let last = block.id().clock + block.len();
                        last <= curr_write_last && block.id().client >= curr_write_block.id().client
                    }
                    None => false,
                } {
                    curr_decoder.move_next();
                    iterated = true;
                }

                let curr_block = match curr_decoder.current() {
                    Some(block) => block,
                    None => continue,
                };
                let cid = curr_block.id();
                if cid.client != first_client || // check whether there is another decoder that has has updates from `firstClient`
                    (iterated && cid.clock > curr_write_last)
                // the above while loop was used and we are potentially missing updates
                {
                    continue;
                }

                if first_client != curr_write_block.id().client {
                    result
                        .blocks
                        .add_block(curr_write.unwrap_or_else(|| unreachable!()));
                    curr_write = curr_decoder.take();
                    curr_decoder.move_next();
                } else if curr_write_last < curr_block.id().clock {
                    //TODO: write currStruct & set currStruct = Skip(clock = currStruct.id.clock + currStruct.length, length = curr.id.clock - self.clock)
                    let skip = match curr_write.unwrap_or_else(|| unreachable!()) {
                        Block::Skip(mut skip) => {
                            // extend existing skip
                            skip.len = curr_block.id().clock + curr_block.len() - skip.clock;
                            skip
                        }
                        other => {
                            result.blocks.add_block(other);
                            let diff = curr_block.id().clock - curr_write_last;
                            BlockRange::new(ID::new(first_client, curr_write_last), diff)
                        }
                    };
                    curr_write = Some(Block::Skip(skip));
                } else {
                    // if (currWrite.struct.id.clock + currWrite.struct.length >= curr.id.clock) {
                    let diff = curr_write_last.saturating_sub(curr_block.id().clock);

                    let mut block_slice = None;
                    if diff > 0 {
                        if let Block::Skip(skip) = curr_write_block {
                            // prefer to slice Skip because the other struct might contain more information
                            skip.len -= diff as u32;
                        } else {
                            block_slice = Some(curr_block.splice(diff as u32).unwrap());
                        }
                    }

                    let curr_block = block_slice
                        .as_ref()
                        .or(curr_decoder.current())
                        .unwrap_or_else(|| unreachable!());
                    if !curr_write_block.try_squash(curr_block) {
                        result
                            .blocks
                            .add_block(curr_write.unwrap_or_else(|| unreachable!()));
                        curr_write = block_slice.or_else(|| curr_decoder.take());
                        curr_decoder.move_next();
                    }
                }
            } else {
                curr_write = curr_decoder.take();
                curr_decoder.move_next();
            }

            while let Some(next) = curr_decoder.current() {
                let block = curr_write.as_ref().unwrap();
                let nid = next.id();
                if nid.client == first_client && nid.clock == block.id().clock + block.len() {
                    result.blocks.add_block(curr_write.unwrap());
                    curr_write = curr_decoder.take();
                    curr_decoder.move_next();
                } else {
                    break;
                }
            }
        }

        if let Some(block) = curr_write.take() {
            result.blocks.add_block(block);
        }

        result
    }
}

impl Encode for Update {
    #[inline]
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.encode_diff(&StateVector::default(), encoder)
    }
}

impl Decode for Update {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        // read blocks
        let clients_len: u32 = decoder.read_var()?;
        let mut clients = HashMap::with_hasher(BuildHasherDefault::default());
        clients.try_reserve(clients_len as usize)?;

        let mut blocks = BlockSet { clients };
        for _ in 0..clients_len {
            let blocks_len = decoder.read_var::<u32>()? as usize;

            let client = decoder.read_client()?;
            let mut clock: u32 = decoder.read_var()?;
            let blocks = blocks
                .clients
                .entry(client)
                .or_insert_with(|| VecDeque::new());
            // Attempt to pre-allocate memory for the blocks. If the capacity overflows and
            // allocation fails, return an error.
            blocks.try_reserve(blocks_len)?;

            for _ in 0..blocks_len {
                let id = ID::new(client, clock);
                if let Some(block) = Self::decode_block(id, decoder)? {
                    // due to bug in the past it was possible for empty bugs to be generated
                    // even though they had no effect on the document store
                    clock += block.len();
                    blocks.push_back(block);
                }
            }
        }
        // read delete set
        let delete_set = IdSet::decode(decoder)?;
        Ok(Update { blocks, delete_set })
    }
}

struct BlockPicker<'a> {
    store: &'a mut BlockSet,
    latest: Option<(ClientID, VecDeque<Block>)>,
    stack: Vec<Block>,
    /// Order in which clients in `store` should be accessed. Ordered by ClientID asc, works as stack.
    clients: SmallVec<[ClientID; 2]>,
    /// State vector for retrieving clock data about missing updates.
    missing: StateVector,
    /// Blocks from the original update that couldn't be applied.
    unapplicable: BlockSet,
}

impl<'a> BlockPicker<'a> {
    fn new(store: &'a mut BlockSet) -> Self {
        let mut clients: SmallVec<[ClientID; 2]> = store.clients.keys().copied().collect();
        clients.sort();
        BlockPicker {
            store,
            clients,
            missing: StateVector::default(),
            latest: None,
            stack: vec![],
            unapplicable: BlockSet::default(),
        }
    }

    fn next(&mut self) -> Option<Block> {
        match self.stack.pop() {
            None => match &mut self.latest {
                Some((_, latest)) if !latest.is_empty() => latest.pop_front(),
                _ => {
                    let next = self.clients.pop()?;
                    self.latest = self.store.clients.remove_entry(&next);
                    self.next()
                }
            },
            block => block,
        }
    }

    /// Switch iterator to blocks belonging to `missing` client.
    fn switch(
        &mut self,
        stack_head: Block,
        missing: &ClientID,
        missing_clock: impl Fn(&ClientID) -> u32,
    ) -> Option<Block> {
        self.stack.push(stack_head);
        match self.store.clients.get_mut(missing) {
            Some(struct_refs)
                if !struct_refs.is_empty() && !self.stack.iter().any(|s| s.client() == missing) =>
            {
                let stack_head = struct_refs.pop_front();
                stack_head
            }
            _ => {
                // This update message causally depends on another update message that doesn't exist yet
                self.missing.set_min(*missing, missing_clock(missing));
                for item in self.stack.drain(..) {
                    let client = *item.client();
                    let mut unapplicable_blocks = match self.store.clients.remove(&client) {
                        Some(blocks) => blocks,
                        None => match &mut self.latest {
                            Some((latest_client, blocks)) if *latest_client == client => {
                                std::mem::take(blocks)
                            }
                            // item was the last item on clientsStructRefs and the field was
                            // already cleared. Add item to restStructs and continue
                            _ => VecDeque::new(),
                        },
                    };
                    unapplicable_blocks.push_front(item);
                    self.unapplicable
                        .clients
                        .insert(client, unapplicable_blocks);
                }
                self.next()
            }
        }
    }

    fn pending(self) -> Option<PendingUpdate> {
        if self.unapplicable.is_empty() {
            None
        } else {
            Some(PendingUpdate {
                update: Update {
                    blocks: self.unapplicable,
                    delete_set: IdSet::new(),
                },
                missing: self.missing,
            })
        }
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

    fn take(&mut self) -> Option<I::Item> {
        self.current.take()
    }

    fn move_next(&mut self) {
        self.current = self.iter.next();
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

impl std::fmt::Debug for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("");
        if !self.blocks.is_empty() {
            s.field("blocks", &self.blocks);
        }
        if !self.delete_set.is_empty() {
            s.field("delete set", &self.delete_set);
        }
        s.finish()
    }
}

pub(crate) struct IntoBlocks {
    current_client: std::vec::IntoIter<(ClientID, VecDeque<Block>)>,
    current_block: Option<std::collections::vec_deque::IntoIter<Block>>,
    ignore_skip: bool,
}

impl IntoBlocks {
    fn new(update: BlockSet, ignore_skip: bool) -> Self {
        let mut client_blocks: Vec<(ClientID, VecDeque<Block>)> =
            update.clients.into_iter().collect();
        // sorting to return higher client ids first
        client_blocks.sort_by(|a, b| b.0.cmp(&a.0));
        let mut current_client = client_blocks.into_iter();

        let current_block = current_client.next().map(|(_, v)| v.into_iter());
        IntoBlocks {
            current_client,
            current_block,
            ignore_skip,
        }
    }
}

impl Iterator for IntoBlocks {
    type Item = Block;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(blocks) = self.current_block.as_mut() {
            let block = blocks.next();
            match block {
                Some(Block::Skip(_)) if self.ignore_skip => return self.next(),
                Some(block) => return Some(block),
                None => {}
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
    use std::collections::{HashMap, VecDeque};
    use std::iter::FromIterator;
    use std::sync::{Arc, Mutex};

    use crate::block::{Block, BlockRange, ClientID, Item, ItemContent};
    use crate::encoding::read::Cursor;
    use crate::types::{Delta, TypePtr};
    use crate::update::{BlockSet, Update};
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::updates::encoder::Encode;
    use crate::{
        merge_updates_v1, Any, Doc, GetString, IdSet, Options, ReadTxn, StateVector, Text,
        Transact, WriteTxn, XmlFragment, XmlOut, ID,
    };

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
        let u = Update::decode(&mut decoder).unwrap();

        let id = ID::new(ClientID::new(2026372272), 0);
        let block = u.blocks.clients.get(&id.client).unwrap();
        let mut expected: Vec<Block> = Vec::new();
        expected.push(
            Item::new(
                id,
                None,
                None,
                None,
                None,
                TypePtr::Named("".into()),
                Some("keyB".into()),
                ItemContent::Any(vec!["valueB".into()]),
            )
            .unwrap()
            .into(),
        );
        assert_eq!(block, &expected);
    }

    #[test]
    fn update_merge() {
        let d1 = Doc::with_client_id(1);
        let txt1 = d1.get_or_insert_text("test");
        let mut t1 = d1.transact_mut();

        let d2 = Doc::with_client_id(2);
        let txt2 = d2.get_or_insert_text("test");
        let mut t2 = d2.transact_mut();

        txt1.insert(&mut t1, 0, "aaa");
        txt1.insert(&mut t1, 0, "aaa");

        txt2.insert(&mut t2, 0, "bbb");
        txt2.insert(&mut t2, 2, "bbb");

        let binary1 = t1.encode_update_v1();
        let binary2 = t2.encode_update_v1();

        t1.apply_update(Update::decode_v1(binary2.as_slice()).unwrap())
            .unwrap();
        t2.apply_update(Update::decode_v1(binary1.as_slice()).unwrap())
            .unwrap();

        let u1 = Update::decode(&mut DecoderV1::new(Cursor::new(binary1.as_slice()))).unwrap();
        let u2 = Update::decode(&mut DecoderV1::new(Cursor::new(binary2.as_slice()))).unwrap();

        // a crux of our test: merged update upon applying should produce
        // the same output as sequence of updates applied individually
        let u12 = Update::merge_updates(vec![u1, u2]);

        let d3 = Doc::with_client_id(3);
        let txt3 = d3.get_or_insert_text("test");
        let mut t3 = d3.transact_mut();
        t3.apply_update(u12).unwrap();

        let str1 = txt1.get_string(&t1);
        let str2 = txt2.get_string(&t2);
        let str3 = txt3.get_string(&t3);

        assert_eq!(str1, str2);
        assert_eq!(str2, str3);
    }

    #[test]
    fn test_duplicate_updates() {
        let doc = Doc::with_client_id(1);
        let txt = doc.get_or_insert_text("test");
        let mut tr = doc.transact_mut();
        txt.insert(&mut tr, 0, "aaa");

        let binary = tr.encode_update_v1();
        let u1 = decode_update(&binary);
        let u2 = decode_update(&binary);
        let u3 = decode_update(&binary);

        let merged_update = Update::merge_updates(vec![u1, u2]);
        assert_eq!(merged_update, u3);
    }

    #[test]
    fn test_multiple_clients_in_one_update() {
        let binary1 = {
            let doc = Doc::with_client_id(1);
            let txt = doc.get_or_insert_text("test");
            let mut tr = doc.transact_mut();
            txt.insert(&mut tr, 0, "aaa");
            tr.encode_update_v1()
        };
        let binary2 = {
            let doc = Doc::with_client_id(2);
            let txt = doc.get_or_insert_text("test");
            let mut tr = doc.transact_mut();
            txt.insert(&mut tr, 0, "bbb");
            tr.encode_update_v1()
        };

        let u12 = Update::merge_updates(vec![decode_update(&binary1), decode_update(&binary2)]);
        let u12_copy =
            Update::merge_updates(vec![decode_update(&binary1), decode_update(&binary2)]);

        assert_eq!(2, u12.blocks.clients.keys().len());

        let merged_update = Update::merge_updates(vec![u12]);
        assert_eq!(merged_update, u12_copy);
    }

    #[test]
    fn test_v2_encoding_of_fragmented_delete_set() {
        let before = vec![
            0, 1, 0, 11, 129, 215, 239, 201, 16, 198, 237, 152, 220, 8, 4, 4, 0, 4, 1, 1, 0, 11,
            40, 3, 39, 0, 4, 0, 7, 0, 40, 3, 8, 163, 1, 142, 1, 110, 111, 116, 101, 46, 103, 117,
            105, 100, 110, 111, 116, 101, 71, 117, 105, 100, 110, 111, 116, 101, 46, 111, 119, 110,
            101, 114, 111, 119, 110, 101, 114, 110, 111, 116, 101, 46, 116, 121, 112, 101, 110,
            111, 116, 101, 84, 121, 112, 101, 110, 111, 116, 101, 46, 99, 114, 101, 97, 116, 101,
            84, 105, 109, 101, 99, 114, 101, 97, 116, 101, 84, 105, 109, 101, 110, 111, 116, 101,
            46, 116, 105, 116, 108, 101, 116, 105, 116, 108, 101, 49, 112, 114, 111, 115, 101, 109,
            105, 114, 114, 111, 114, 108, 105, 110, 107, 110, 111, 116, 101, 103, 117, 105, 100,
            115, 108, 111, 116, 71, 117, 105, 100, 116, 121, 112, 101, 108, 105, 110, 107, 84, 121,
            112, 101, 99, 104, 105, 108, 100, 114, 101, 110, 98, 9, 8, 10, 5, 9, 8, 15, 74, 0, 5,
            1, 11, 8, 4, 8, 4, 72, 0, 1, 9, 1, 4, 0, 0, 1, 0, 0, 3, 1, 2, 2, 3, 2, 65, 8, 2, 4, 0,
            119, 22, 97, 99, 70, 120, 85, 89, 68, 76, 82, 104, 101, 114, 107, 74, 97, 66, 101, 99,
            115, 99, 51, 103, 125, 136, 57, 125, 0, 119, 13, 49, 54, 56, 53, 53, 51, 48, 50, 56,
            54, 54, 53, 54, 9, 0, 119, 22, 66, 67, 100, 81, 112, 112, 119, 69, 84, 48, 105, 82, 86,
            66, 81, 45, 56, 69, 87, 50, 87, 103, 119, 22, 106, 114, 69, 109, 73, 77, 112, 86, 84,
            101, 45, 99, 114, 78, 50, 86, 76, 51, 99, 97, 72, 81, 119, 8, 108, 105, 110, 107, 110,
            111, 116, 101, 119, 1, 49, 118, 2, 4, 103, 117, 105, 100, 119, 22, 66, 67, 100, 81,
            112, 112, 119, 69, 84, 48, 105, 82, 86, 66, 81, 45, 56, 69, 87, 50, 87, 103, 8, 115,
            108, 111, 116, 71, 117, 105, 100, 119, 22, 106, 114, 69, 109, 73, 77, 112, 86, 84, 101,
            45, 99, 114, 78, 50, 86, 76, 51, 99, 97, 72, 81, 119, 22, 66, 67, 100, 81, 112, 112,
            119, 69, 84, 48, 105, 82, 86, 66, 81, 45, 56, 69, 87, 50, 87, 103, 0,
        ];
        let update = vec![
            0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 198, 182, 140, 174, 4, 1, 2, 0, 0, 5,
        ];
        let doc = Doc::with_options(Options {
            skip_gc: true,
            client_id: ClientID::new(1),
            ..Default::default()
        });
        let prosemirror = doc.get_or_insert_xml_fragment("prosemirror");
        {
            let mut txn = doc.transact_mut();
            let u = Update::decode_v2(&before).unwrap();
            txn.apply_update(u).unwrap();
            let linknote = prosemirror.get(&txn, 0);
            let actual = linknote.and_then(|xml| match xml {
                XmlOut::Element(elem) => Some(elem.tag().clone()),
                _ => None,
            });
            assert_eq!(actual, Some("linknote".into()));
        }
        {
            let mut txn = doc.transact_mut();
            let u = Update::decode_v2(&update).unwrap();
            txn.apply_update(u).unwrap();

            // this should not panic
            let binary = txn.encode_update_v2();
            let _ = Update::decode_v2(&binary).unwrap();

            let linknote = prosemirror.get(&txn, 0);
            assert!(linknote.is_none());
        }
    }

    #[test]
    fn merge_pending_updates() {
        let d0 = Doc::with_client_id(0);
        let server_updates = Arc::new(Mutex::new(vec![]));
        let sub = {
            let server_updates = server_updates.clone();
            d0.observe_update_v1(move |_, update| {
                let mut lock = server_updates.lock().unwrap();
                lock.push(update.update.clone());
            })
            .unwrap()
        };
        let txt = d0.get_or_insert_text("textBlock");
        txt.apply_delta(&mut d0.transact_mut(), [Delta::insert("r")]);
        txt.apply_delta(&mut d0.transact_mut(), [Delta::insert("o")]);
        txt.apply_delta(&mut d0.transact_mut(), [Delta::insert("n")]);
        txt.apply_delta(&mut d0.transact_mut(), [Delta::insert("e")]);
        txt.apply_delta(&mut d0.transact_mut(), [Delta::insert("n")]);
        drop(sub);
        drop(d0);

        let updates = Arc::into_inner(server_updates).unwrap();
        let updates = updates.into_inner().unwrap();

        let d1 = Doc::with_client_id(1);
        d1.transact_mut()
            .apply_update(Update::decode_v1(&updates[0]).unwrap())
            .unwrap();
        let u1 = d1
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let d2 = Doc::with_client_id(2);
        d2.transact_mut()
            .apply_update(Update::decode_v1(&u1).unwrap())
            .unwrap();
        d2.transact_mut()
            .apply_update(Update::decode_v1(&updates[1]).unwrap())
            .unwrap();
        let u2 = d2
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let d3 = Doc::with_client_id(3);
        d3.transact_mut()
            .apply_update(Update::decode_v1(&u2).unwrap())
            .unwrap();
        d3.transact_mut()
            .apply_update(Update::decode_v1(&updates[3]).unwrap())
            .unwrap();
        let u3 = d3
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let d4 = Doc::with_client_id(4);
        d4.transact_mut()
            .apply_update(Update::decode_v1(&u3).unwrap())
            .unwrap();
        d4.transact_mut()
            .apply_update(Update::decode_v1(&updates[2]).unwrap())
            .unwrap();
        let u4 = d4
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        let d5 = Doc::with_client_id(5);
        d5.transact_mut()
            .apply_update(Update::decode_v1(&u4).unwrap())
            .unwrap();
        d5.transact_mut()
            .apply_update(Update::decode_v1(&updates[4]).unwrap())
            .unwrap();

        let txt5 = d5.get_or_insert_text("textBlock");
        let str = txt5.get_string(&d5.transact());
        assert_eq!(str, "nenor");
    }

    #[test]
    fn apply_update_filling_partial_skip() {
        // ref: https://github.com/y-crdt/yn/issues/3

        // Sequence of updates that, when applied in order, leaves a client with an integrated
        // Skip spanning two clocks and then delivers a single-clock block landing inside it.
        let updates = [
            vec![1, 1, 182, 144, 197, 137, 4, 0, 4, 1, 1, 116, 1, 109, 0],
            vec![
                1, 1, 152, 176, 234, 156, 14, 3, 132, 152, 176, 234, 156, 14, 0, 1, 99, 0,
            ],
            vec![0, 1, 152, 176, 234, 156, 14, 1, 2, 1],
            vec![1, 1, 152, 176, 234, 156, 14, 0, 4, 1, 1, 116, 1, 112, 0],
            vec![
                1, 1, 152, 176, 234, 156, 14, 1, 68, 152, 176, 234, 156, 14, 0, 1, 100, 0,
            ],
            vec![
                1, 1, 152, 176, 234, 156, 14, 2, 196, 152, 176, 234, 156, 14, 1, 152, 176, 234,
                156, 14, 0, 1, 110, 0,
            ],
            vec![
                1, 1, 182, 144, 197, 137, 4, 1, 132, 182, 144, 197, 137, 4, 0, 1, 100, 0,
            ],
        ];

        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            for u in &updates {
                txn.apply_update(Update::decode_v1(u).unwrap()).unwrap();
            }
        }

        let txt = doc.get_or_insert_text("t");
        assert_eq!(txt.get_string(&doc.transact()), "mddpc");
    }

    #[test]
    fn apply_update_filling_middle_of_skip() {
        // Anchor client D builds "PQ".
        let d = Doc::with_client_id(100);
        {
            let txt = d.get_or_insert_text("t");
            let mut txn = d.transact_mut();
            txt.insert(&mut txn, 0, "P");
            txt.insert(&mut txn, 1, "Q");
        }
        let d_state = d
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        // Client C syncs from D, then performs 4 single-block inserts. The anchoring is chosen so
        // that:
        //   - C:3 anchors only to remote blocks (left D:1, right none) => it can integrate while
        //     C:1/C:2 are missing, creating a Skip over clocks [1,3).
        //   - C:2 anchors to present blocks outside the Skip (left D:0, right C:0) => when it
        //     arrives after C:3 it lands in the MIDDLE of the Skip (diff_start > 0).
        let c = Doc::with_client_id(1);
        c.transact_mut()
            .apply_update(Update::decode_v1(&d_state).unwrap())
            .unwrap();
        let updates = Arc::new(Mutex::new(vec![]));
        let sub = {
            let updates = updates.clone();
            c.observe_update_v1(move |_, e| updates.lock().unwrap().push(e.update.clone()))
                .unwrap()
        };
        let txt = c.get_or_insert_text("t");
        txt.insert(&mut c.transact_mut(), 1, "a"); // C:0  "PaQ"    left D:0, right D:1
        txt.insert(&mut c.transact_mut(), 2, "b"); // C:1  "PabQ"   left C:0, right D:1
        txt.insert(&mut c.transact_mut(), 1, "c"); // C:2  "PcabQ"  left D:0, right C:0
        txt.insert(&mut c.transact_mut(), 5, "d"); // C:3  "PcabQd" left D:1, right none
        drop(sub);

        let mut msgs = vec![d_state];
        msgs.extend(updates.lock().unwrap().iter().cloned());
        // msgs: [D state, C:0, C:1, C:2, C:3]

        let apply = |order: &[usize]| -> String {
            let doc = Doc::new();
            {
                let mut txn = doc.transact_mut();
                for &i in order {
                    txn.apply_update(Update::decode_v1(&msgs[i]).unwrap())
                        .unwrap();
                }
            }
            let txt = doc.get_or_insert_text("t");
            let s = txt.get_string(&doc.transact());
            s
        };

        // ground truth: full causal delivery order
        let causal = apply(&[0, 1, 2, 3, 4]);
        assert_eq!(causal, "PcabQd");

        // skip-inducing order: D, C:0, then C:3 (creates Skip[1,3)), then C:2 (fills the middle),
        // then C:1. Must converge to the same document state.
        let skipped = apply(&[0, 1, 4, 3, 2]);
        assert_eq!(
            skipped, causal,
            "filling the middle of a Skip dropped a block"
        );
    }

    #[test]
    fn apply_update_pending_delete_set_not_lost() {
        fn apply_updates(updates: &[&[u8]]) -> String {
            let opts = Options {
                skip_gc: true,
                ..Default::default()
            };
            let doc = Doc::with_options(opts);
            {
                let mut txn = doc.transact_mut();
                for u in updates {
                    txn.apply_update(Update::decode_v1(u).unwrap()).unwrap();
                }
            }
            let txn = doc.transact();
            let t = txn.get_text("t").unwrap();
            t.get_string(&txn)
        }

        let a = vec![1, 1, 174, 156, 239, 251, 3, 0, 4, 1, 1, 116, 1, 124, 0]; // insert "|" at 0,
        let b = vec![
            1, 1, 174, 156, 239, 251, 3, 1, 68, 174, 156, 239, 251, 3, 0, 1, 71, 0,
        ]; // insert "G" at 1 (origin A)
        let d = vec![0, 1, 174, 156, 239, 251, 3, 1, 1, 1]; // delete  1 (G)
        let e = vec![0, 1, 174, 156, 239, 251, 3, 1, 0, 1]; // delete 0 (|)

        let causal = apply_updates(&[&a, &b, &d, &e]);
        assert_eq!(causal, "");

        // orderings where the dependency (A) arrives last used to resurrect "G"
        assert_eq!(apply_updates(&[&b, &d, &e, &a]), "");
        assert_eq!(apply_updates(&[&d, &b, &e, &a]), "");
    }

    #[test]
    fn update_state_vector_with_skips() {
        let mut update = Update::new();
        // skip followed by item => not included in state vector as it's not continuous from 0
        update.blocks.add_block(Block::Skip(BlockRange::new(
            ID::new(ClientID::new(1), 0),
            1,
        )));
        update.blocks.add_block(test_item(ClientID::new(1), 1, 1));
        // item starting from non-0 => not included
        update.blocks.add_block(test_item(ClientID::new(2), 1, 1));
        // item => skip => item : second item not included
        update.blocks.add_block(test_item(ClientID::new(3), 0, 1));
        update.blocks.add_block(Block::Skip(BlockRange::new(
            ID::new(ClientID::new(3), 1),
            1,
        )));
        update.blocks.add_block(test_item(ClientID::new(3), 2, 1));

        let sv = update.state_vector();
        assert_eq!(sv, StateVector::from_iter([(ClientID::new(3), 1)]));
    }

    #[test]
    fn test_extends() {
        let mut u = Update::new();
        u.blocks.add_block(test_item(ClientID::new(1), 0, 2)); // new data with partial duplicate
        assert!(u.extends(&StateVector::from_iter([(ClientID::new(1), 1)])));

        let mut u = Update::new();
        u.blocks.add_block(test_item(ClientID::new(1), 0, 1)); // duplicate
        assert!(!u.extends(&StateVector::from_iter([(ClientID::new(1), 1)])));

        let mut u = Update::new();
        u.blocks.add_block(Block::Skip(BlockRange::new(
            ID::new(ClientID::new(1), 0),
            2,
        )));
        u.blocks.add_block(test_item(ClientID::new(1), 2, 1)); // skip cause disjoin in updates
        assert!(!u.extends(&StateVector::from_iter([(ClientID::new(1), 1)])));

        let mut u = Update::new();
        u.blocks.add_block(test_item(ClientID::new(1), 1, 1)); // adjacent
        assert!(u.extends(&StateVector::from_iter([(ClientID::new(1), 1)])));

        let mut u = Update::new();
        u.blocks.add_block(test_item(ClientID::new(1), 2, 1)); // disjoint
        assert!(!u.extends(&StateVector::from_iter([(ClientID::new(1), 1)])));
    }

    #[test]
    fn empty_update_v1() {
        let u = Update::new();
        let binary = u.encode_v1();
        assert_eq!(&binary, Update::EMPTY_V1)
    }

    #[test]
    fn empty_update_v2() {
        let u = Update::new();
        let binary = u.encode_v2();
        assert_eq!(&binary, Update::EMPTY_V2)
    }

    #[test]
    fn update_v2_with_skips() {
        let u1 = update_with_skips();
        let encoded = u1.encode_v2();
        let u2 = Update::decode_v2(&encoded).unwrap();
        assert_eq!(u1, u2);
    }

    #[test]
    fn pending_update_check() {
        let update = update_with_skips();
        let expected = update.encode_v1();
        let doc = Doc::with_client_id(2);
        let mut txn = doc.transact_mut();
        let txt = txn.get_or_insert_text("test");
        txn.apply_update(update).unwrap();
        let str = txt.get_string(&txn);
        assert_eq!(str, "hello"); // 'world' is missing because of skip block
        assert!(txn.has_missing_updates());
        let state = txn.encode_state_as_update_v1(&Default::default());
        assert_eq!(state, expected); // we include pending update
        let pending = txn.prune_pending();
        assert!(pending.is_some());
        let state = txn.encode_state_as_update_v1(&Default::default());
        assert_ne!(state, expected); // we pruned pending update
        let joined = merge_updates_v1([state, pending.unwrap().encode_v1()]).unwrap();
        assert_eq!(joined, expected); // we joined current and pending state, they should be equal
    }

    fn update_with_skips() -> Update {
        Update {
            blocks: BlockSet {
                clients: HashMap::from_iter([(
                    ClientID::new(1),
                    VecDeque::from_iter([
                        Block::Item(
                            Item::new(
                                ID::new(ClientID::new(1), 0),
                                None,
                                None,
                                None,
                                None,
                                TypePtr::Named("test".into()),
                                None,
                                ItemContent::String("hello".into()),
                            )
                            .unwrap(),
                        ),
                        Block::Skip(BlockRange::new(ID::new(ClientID::new(1), 5), 3)),
                        Block::Item(
                            Item::new(
                                ID::new(ClientID::new(1), 8),
                                None,
                                Some(ID::new(ClientID::new(1), 7)),
                                None,
                                None,
                                TypePtr::Unknown,
                                None,
                                ItemContent::String("world".into()),
                            )
                            .unwrap(),
                        ),
                    ]),
                )]),
            },
            delete_set: IdSet::default(),
        }
    }

    fn test_item(client_id: ClientID, clock: u32, len: u32) -> Block {
        assert!(len > 0);
        let any: Vec<_> = (0..len).into_iter().map(Any::from).collect();
        Block::Item(
            Item::new(
                ID::new(client_id, clock),
                None,
                None,
                None,
                None,
                TypePtr::Named("test".into()),
                None,
                ItemContent::Any(any),
            )
            .unwrap(),
        )
    }

    fn decode_update(bin: &[u8]) -> Update {
        Update::decode(&mut DecoderV1::new(Cursor::new(bin))).unwrap()
    }
}
