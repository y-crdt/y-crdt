use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::sync::Arc;

use crate::block::{
    BlockRange, ClientID, Item, ItemContent, ItemPtr, BLOCK_GC_REF_NUMBER, BLOCK_SKIP_REF_NUMBER,
    HAS_ORIGIN, HAS_PARENT_SUB, HAS_RIGHT_ORIGIN,
};
use crate::encoding::read::Error;
use crate::error::UpdateError;
use crate::id_set::DeleteSet;
use crate::slice::ItemSlice;
#[cfg(test)]
use crate::store::Store;
use crate::transaction::TransactionMut;
use crate::types::TypePtr;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::{OffsetKind, StateVector, ID};

#[derive(Debug, Default, PartialEq)]
pub(crate) struct UpdateBlocks {
    clients: HashMap<ClientID, VecDeque<BlockCarrier>, BuildHasherDefault<ClientHasher>>,
}

impl UpdateBlocks {
    /**
    @todo this should be refactored.
    I'm currently using this to add blocks to the Update
    */
    pub(crate) fn add_block(&mut self, block: BlockCarrier) {
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
    pub(crate) fn into_blocks(self, ignore_skip: bool) -> IntoBlocks {
        IntoBlocks::new(self, ignore_skip)
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

impl std::fmt::Debug for BlockCarrier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::fmt::Display for BlockCarrier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockCarrier::Item(x) => x.fmt(f),
            BlockCarrier::Skip(x) => write!(f, "Skip{}", x),
            BlockCarrier::GC(x) => write!(f, "GC{}", x),
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
    pub(crate) blocks: UpdateBlocks,
    pub(crate) delete_set: DeleteSet,
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
                if range.id.clock <= clock && range.id.clock + range.len > clock {
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
                    if let BlockCarrier::Skip(_) = block {
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

    /// Returns a state vector representing a lower bound of client clocks included by blocks
    /// stored in current update.
    pub fn state_vector_lower(&self) -> StateVector {
        let mut sv = StateVector::default();
        for (&client, blocks) in self.blocks.clients.iter() {
            let id = blocks[0].id();
            sv.set_max(client, id.clock);
        }
        sv
    }

    /// Returns a delete set associated with current update.
    pub fn delete_set(&self) -> &DeleteSet {
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
                            } else if let BlockCarrier::Item(block) = a {
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
            let mut store = txn.store_mut();
            let mut client_block_ref_ids: Vec<ClientID> =
                self.blocks.clients.keys().cloned().collect();
            client_block_ref_ids.sort();

            let mut current_client_id = client_block_ref_ids.pop().unwrap();
            let mut current_target = self.blocks.clients.get_mut(&current_client_id);
            let mut stack_head = if let Some(v) = current_target.as_mut() {
                v.pop_front()
            } else {
                None
            };

            let mut local_sv = store.blocks.get_state_vector();
            let mut missing_sv = StateVector::default();
            let mut remaining = UpdateBlocks::default();
            let mut stack = Vec::new();

            while let Some(mut block) = stack_head {
                if !block.is_skip() {
                    let id = *block.id();
                    if local_sv.contains(&id) {
                        let offset = local_sv.get(&id.client) as i32 - id.clock as i32;
                        if let Some(dep) = Self::missing(&block, &local_sv) {
                            stack.push(block);
                            // get the struct reader that has the missing struct
                            match self.blocks.clients.get_mut(&dep) {
                                Some(block_refs) if !block_refs.is_empty() => {
                                    stack_head = block_refs.pop_front();
                                    current_target =
                                        self.blocks.clients.get_mut(&current_client_id);
                                    continue;
                                }
                                _ => {
                                    // This update message causally depends on another update message that doesn't exist yet
                                    missing_sv.set_min(dep, local_sv.get(&dep));
                                    Self::return_stack(stack, &mut self.blocks, &mut remaining);
                                    current_target =
                                        self.blocks.clients.get_mut(&current_client_id);
                                    stack = Vec::new();
                                }
                            }
                        } else if offset == 0 || (offset as u32) < block.len() {
                            let offset = offset as u32;
                            let client = id.client;
                            local_sv.set_max(client, id.clock + block.len());
                            if let BlockCarrier::Item(item) = &mut block {
                                item.repair(store)?;
                            }
                            let should_delete = block.integrate(txn, offset);
                            let mut delete_ptr = if should_delete {
                                let ptr = block.as_item_ptr();
                                ptr
                            } else {
                                None
                            };
                            store = txn.store_mut();
                            match block {
                                BlockCarrier::Item(item) => {
                                    if item.parent != TypePtr::Unknown {
                                        store.blocks.push_block(item)
                                    } else {
                                        // parent is not defined. Integrate GC struct instead
                                        store.blocks.push_gc(BlockRange::new(item.id, item.len));
                                        delete_ptr = None;
                                    }
                                }
                                BlockCarrier::GC(gc) => store.blocks.push_gc(gc),
                                BlockCarrier::Skip(_) => { /* do nothing */ }
                            }

                            if let Some(ptr) = delete_ptr {
                                txn.delete(ptr);
                            }
                            store = txn.store_mut();
                        }
                    } else {
                        // update from the same client is missing
                        let id = block.id();
                        missing_sv.set_min(id.client, id.clock - 1);
                        stack.push(block);
                        // hid a dead wall, add all items from stack to restSS
                        Self::return_stack(stack, &mut self.blocks, &mut remaining);
                        current_target = self.blocks.clients.get_mut(&current_client_id);
                        stack = Vec::new();
                    }
                }

                // iterate to next stackHead
                if !stack.is_empty() {
                    stack_head = stack.pop();
                } else {
                    match current_target.take() {
                        Some(v) if !v.is_empty() => {
                            stack_head = v.pop_front();
                            current_target = Some(v);
                        }
                        _ => {
                            if let Some((client_id, target)) =
                                Self::next_target(&mut client_block_ref_ids, &mut self.blocks)
                            {
                                stack_head = target.pop_front();
                                current_client_id = client_id;
                                current_target = Some(target);
                            } else {
                                // we're done
                                break;
                            }
                        }
                    };
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

        Ok((remaining_blocks, remaining_ds))
    }

    fn missing(block: &BlockCarrier, local_sv: &StateVector) -> Option<ClientID> {
        if let BlockCarrier::Item(item) = block {
            if let Some(origin) = &item.origin {
                if origin.client != item.id.client && origin.clock >= local_sv.get(&origin.client) {
                    return Some(origin.client);
                }
            }

            if let Some(right_origin) = &item.right_origin {
                if right_origin.client != item.id.client
                    && right_origin.clock >= local_sv.get(&right_origin.client)
                {
                    return Some(right_origin.client);
                }
            }

            match &item.parent {
                TypePtr::Branch(parent) => {
                    if let Some(block) = &parent.item {
                        let parent_id = block.id();
                        if parent_id.client != item.id.client
                            && parent_id.clock >= local_sv.get(&parent_id.client)
                        {
                            return Some(parent_id.client);
                        }
                    }
                }
                TypePtr::ID(parent_id) => {
                    if parent_id.client != item.id.client
                        && parent_id.clock >= local_sv.get(&parent_id.client)
                    {
                        return Some(parent_id.client);
                    }
                }
                _ => {}
            }

            match &item.content {
                ItemContent::Move(m) => {
                    if let Some(start) = m.start.id() {
                        if start.clock >= local_sv.get(&start.client) {
                            return Some(start.client);
                        }
                    }
                    if !m.is_collapsed() {
                        if let Some(end) = m.end.id() {
                            if end.clock >= local_sv.get(&end.client) {
                                return Some(end.client);
                            }
                        }
                    }
                }
                ItemContent::Type(branch) => {
                    #[cfg(feature = "weak")]
                    if let crate::types::TypeRef::WeakLink(source) = &branch.type_ref {
                        let start = source.quote_start.id();
                        let end = source.quote_end.id();
                        if let Some(start) = start {
                            if start.clock >= local_sv.get(&start.client) {
                                return Some(start.client);
                            }
                        }
                        if start != end {
                            if let Some(end) = &source.quote_end.id() {
                                if end.clock >= local_sv.get(&end.client) {
                                    return Some(end.client);
                                }
                            }
                        }
                    }
                }
                _ => { /* do nothing */ }
            }
        }
        None
    }

    fn next_target<'a, 'b>(
        client_block_ref_ids: &'a mut Vec<ClientID>,
        blocks: &'b mut UpdateBlocks,
    ) -> Option<(ClientID, &'b mut VecDeque<BlockCarrier>)> {
        while let Some(id) = client_block_ref_ids.pop() {
            match blocks.clients.get(&id) {
                Some(client_blocks) if !client_blocks.is_empty() => {
                    // we need to borrow client_blocks in mutable context AND we're
                    // doing so in a loop at the same time - this combination causes
                    // Rust borrow checker go nuts. TODO: remove the unsafe block
                    let client_blocks = unsafe {
                        (client_blocks as *const VecDeque<BlockCarrier>
                            as *mut VecDeque<BlockCarrier>)
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

    fn return_stack(
        stack: Vec<BlockCarrier>,
        refs: &mut UpdateBlocks,
        remaining: &mut UpdateBlocks,
    ) {
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

    fn decode_block<D: Decoder>(id: ID, decoder: &mut D) -> Result<Option<BlockCarrier>, Error> {
        let info = decoder.read_info()?;
        match info {
            BLOCK_SKIP_REF_NUMBER => {
                let len: u32 = decoder.read_var()?;
                Ok(Some(BlockCarrier::Skip(BlockRange { id, len })))
            }
            BLOCK_GC_REF_NUMBER => {
                let len: u32 = decoder.read_len()?;
                Ok(Some(BlockCarrier::GC(BlockRange { id, len })))
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
                    Some(item) => Ok(Some(BlockCarrier::from(item))),
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
                let mut memo = update_blocks.into_blocks(true).memoized();
                memo.move_next();
                memo
            })
            .collect();

        let mut curr_write: Option<BlockCarrier> = None;

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
                        BlockCarrier::Skip(mut skip) => {
                            // extend existing skip
                            skip.len = curr_block.id().clock + curr_block.len() - skip.id.clock;
                            skip
                        }
                        other => {
                            result.blocks.add_block(other);
                            let diff = curr_block.id().clock - curr_write_last;
                            BlockRange::new(ID::new(first_client, curr_write_last), diff)
                        }
                    };
                    curr_write = Some(BlockCarrier::Skip(skip));
                } else {
                    // if (currWrite.struct.id.clock + currWrite.struct.length >= curr.id.clock) {
                    let diff = curr_write_last.saturating_sub(curr_block.id().clock);

                    let mut block_slice = None;
                    if diff > 0 {
                        if let BlockCarrier::Skip(skip) = curr_write_block {
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

        let mut blocks = UpdateBlocks { clients };
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
        let delete_set = DeleteSet::decode(decoder)?;
        Ok(Update { blocks, delete_set })
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

#[derive(PartialEq)]
pub(crate) enum BlockCarrier {
    Item(Box<Item>),
    GC(BlockRange),
    Skip(BlockRange),
}

impl BlockCarrier {
    pub(crate) fn splice(&self, offset: u32) -> Option<Self> {
        match self {
            BlockCarrier::Item(x) => {
                let next = ItemPtr::from(x).splice(offset, OffsetKind::Utf16)?;
                Some(BlockCarrier::Item(next))
            }
            BlockCarrier::Skip(x) => {
                if offset == 0 {
                    None
                } else {
                    Some(BlockCarrier::Skip(x.slice(offset)))
                }
            }
            BlockCarrier::GC(x) => {
                if offset == 0 {
                    None
                } else {
                    Some(BlockCarrier::GC(x.slice(offset)))
                }
            }
        }
    }
    pub(crate) fn same_type(&self, other: &BlockCarrier) -> bool {
        match (self, other) {
            (BlockCarrier::Skip(_), BlockCarrier::Skip(_)) => true,
            (BlockCarrier::Item(_), BlockCarrier::Item(_)) => true,
            (BlockCarrier::GC(_), BlockCarrier::GC(_)) => true,
            (_, _) => false,
        }
    }
    pub(crate) fn id(&self) -> &ID {
        match self {
            BlockCarrier::Item(x) => x.id(),
            BlockCarrier::Skip(x) => &x.id,
            BlockCarrier::GC(x) => &x.id,
        }
    }

    pub(crate) fn len(&self) -> u32 {
        match self {
            BlockCarrier::Item(x) => x.len(),
            BlockCarrier::Skip(x) => x.len,
            BlockCarrier::GC(x) => x.len,
        }
    }

    pub(crate) fn range(&self) -> BlockRange {
        match self {
            BlockCarrier::Item(item) => BlockRange::new(item.id, item.len),
            BlockCarrier::GC(gc) => gc.clone(),
            BlockCarrier::Skip(skip) => skip.clone(),
        }
    }

    pub(crate) fn last_id(&self) -> ID {
        match self {
            BlockCarrier::Item(x) => x.last_id(),
            BlockCarrier::Skip(x) => x.last_id(),
            BlockCarrier::GC(x) => x.last_id(),
        }
    }

    pub(crate) fn try_squash(&mut self, other: &BlockCarrier) -> bool {
        match (self, other) {
            (BlockCarrier::Item(a), BlockCarrier::Item(b)) => {
                ItemPtr::from(a).try_squash(ItemPtr::from(b))
            }
            (BlockCarrier::Skip(a), BlockCarrier::Skip(b)) => {
                a.merge(b);
                true
            }
            _ => false,
        }
    }

    pub fn as_item_ptr(&mut self) -> Option<ItemPtr> {
        if let BlockCarrier::Item(block) = self {
            Some(ItemPtr::from(block))
        } else {
            None
        }
    }

    pub fn into_block(self) -> Option<Box<Item>> {
        if let BlockCarrier::Item(block) = self {
            Some(block)
        } else {
            None
        }
    }

    #[inline]
    pub fn is_skip(&self) -> bool {
        if let BlockCarrier::Skip(_) = self {
            true
        } else {
            false
        }
    }
    pub fn encode_with_offset<E: Encoder>(&self, encoder: &mut E, offset: u32) {
        match self {
            BlockCarrier::Item(x) => {
                let slice = ItemSlice::new(x.into(), offset, x.len() - 1);
                slice.encode(encoder)
            }
            BlockCarrier::Skip(x) => {
                encoder.write_info(BLOCK_SKIP_REF_NUMBER);
                encoder.write_var(x.len - offset);
            }
            BlockCarrier::GC(x) => {
                encoder.write_info(BLOCK_GC_REF_NUMBER);
                encoder.write_len(x.len - offset);
            }
        }
    }

    pub fn integrate(&mut self, txn: &mut TransactionMut, offset: u32) -> bool {
        match self {
            BlockCarrier::Item(x) => ItemPtr::from(x).integrate(txn, offset),
            BlockCarrier::Skip(x) => x.integrate(offset),
            BlockCarrier::GC(x) => x.integrate(offset),
        }
    }
}

impl From<Box<Item>> for BlockCarrier {
    fn from(block: Box<Item>) -> Self {
        BlockCarrier::Item(block)
    }
}

impl Encode for BlockCarrier {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        match self {
            BlockCarrier::Item(block) => block.encode(encoder),
            BlockCarrier::Skip(skip) => {
                encoder.write_info(BLOCK_SKIP_REF_NUMBER);
                encoder.write_len(skip.len)
            }
            BlockCarrier::GC(gc) => {
                encoder.write_info(BLOCK_GC_REF_NUMBER);
                encoder.write_len(gc.len)
            }
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

/// Conversion for tests only
#[cfg(test)]
impl Into<Store> for Update {
    fn into(self) -> Store {
        use crate::doc::Options;

        let mut store = Store::new(&Options::with_client_id(0));
        for (_, vec) in self.blocks.clients {
            for block in vec {
                if let BlockCarrier::Item(block) = block {
                    store.blocks.push_block(block);
                } else {
                    panic!("Cannot convert Update into block store - Skip block detected");
                }
            }
        }
        store
    }
}

pub(crate) struct Blocks<'a> {
    current_client: std::vec::IntoIter<(&'a ClientID, &'a VecDeque<BlockCarrier>)>,
    current_block: Option<std::collections::vec_deque::Iter<'a, BlockCarrier>>,
}

impl<'a> Blocks<'a> {
    fn new(update: &'a UpdateBlocks) -> Self {
        let mut client_blocks: Vec<(&'a ClientID, &'a VecDeque<BlockCarrier>)> =
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
    type Item = &'a BlockCarrier;

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
    current_client: std::vec::IntoIter<(ClientID, VecDeque<BlockCarrier>)>,
    current_block: Option<std::collections::vec_deque::IntoIter<BlockCarrier>>,
    ignore_skip: bool,
}

impl IntoBlocks {
    fn new(update: UpdateBlocks, ignore_skip: bool) -> Self {
        let mut client_blocks: Vec<(ClientID, VecDeque<BlockCarrier>)> =
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
    type Item = BlockCarrier;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(blocks) = self.current_block.as_mut() {
            let block = blocks.next();
            match block {
                Some(BlockCarrier::Skip(_)) if self.ignore_skip => return self.next(),
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

    use crate::block::{BlockRange, ClientID, Item, ItemContent};
    use crate::encoding::read::Cursor;
    use crate::types::{Delta, TypePtr};
    use crate::update::{BlockCarrier, Update, UpdateBlocks};
    use crate::updates::decoder::{Decode, DecoderV1};
    use crate::updates::encoder::Encode;
    use crate::{
        merge_updates_v1, Any, DeleteSet, Doc, GetString, Options, ReadTxn, StateVector, Text,
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

        let id = ID::new(2026372272, 0);
        let block = u.blocks.clients.get(&id.client).unwrap();
        let mut expected: Vec<BlockCarrier> = Vec::new();
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
            client_id: 1,
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
    fn update_state_vector_with_skips() {
        let mut update = Update::new();
        // skip followed by item => not included in state vector as it's not continuous from 0
        update
            .blocks
            .add_block(BlockCarrier::Skip(BlockRange::new(ID::new(1, 0), 1)));
        update.blocks.add_block(test_item(1, 1, 1));
        // item starting from non-0 => not included
        update.blocks.add_block(test_item(2, 1, 1));
        // item => skip => item : second item not included
        update.blocks.add_block(test_item(3, 0, 1));
        update
            .blocks
            .add_block(BlockCarrier::Skip(BlockRange::new(ID::new(3, 1), 1)));
        update.blocks.add_block(test_item(3, 2, 1));

        let sv = update.state_vector();
        assert_eq!(sv, StateVector::from_iter([(3, 1)]));
    }

    #[test]
    fn test_extends() {
        let mut u = Update::new();
        u.blocks.add_block(test_item(1, 0, 2)); // new data with partial duplicate
        assert!(u.extends(&StateVector::from_iter([(1, 1)])));

        let mut u = Update::new();
        u.blocks.add_block(test_item(1, 0, 1)); // duplicate
        assert!(!u.extends(&StateVector::from_iter([(1, 1)])));

        let mut u = Update::new();
        u.blocks
            .add_block(BlockCarrier::Skip(BlockRange::new(ID::new(1, 0), 2)));
        u.blocks.add_block(test_item(1, 2, 1)); // skip cause disjoin in updates
        assert!(!u.extends(&StateVector::from_iter([(1, 1)])));

        let mut u = Update::new();
        u.blocks.add_block(test_item(1, 1, 1)); // adjacent
        assert!(u.extends(&StateVector::from_iter([(1, 1)])));

        let mut u = Update::new();
        u.blocks.add_block(test_item(1, 2, 1)); // disjoint
        assert!(!u.extends(&StateVector::from_iter([(1, 1)])));
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
            blocks: UpdateBlocks {
                clients: HashMap::from_iter([(
                    1,
                    VecDeque::from_iter([
                        BlockCarrier::Item(
                            Item::new(
                                ID::new(1, 0),
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
                        BlockCarrier::Skip(BlockRange::new(ID::new(1, 5), 3)),
                        BlockCarrier::Item(
                            Item::new(
                                ID::new(1, 8),
                                None,
                                Some(ID::new(1, 7)),
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
            delete_set: DeleteSet::default(),
        }
    }

    fn test_item(client_id: ClientID, clock: u32, len: u32) -> BlockCarrier {
        assert!(len > 0);
        let any: Vec<_> = (0..len).into_iter().map(Any::from).collect();
        BlockCarrier::Item(
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
