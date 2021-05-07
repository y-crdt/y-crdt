use crate::*;

use crate::block::{Block, BlockPtr, ItemContent, ID};
use crate::block_store::{ClientBlockList, StateVector};
use crate::id_set::IdSet;
use crate::store::Store;
use crate::types::{TypePtr, XorHasher};
use crate::updates::decoder::Decoder;
use std::cell::RefMut;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use updates::encoder::*;

pub struct Transaction<'a> {
    /// Store containing the state of the document.
    pub store: RefMut<'a, Store>,
    /// State vector of a current transaction.
    pub timestamp: StateVector,
    /// ID's of the blocks to be merged.
    pub merge_blocks: Vec<ID>,
    /// Describes the set of deleted items by ids.
    delete_set: IdSet,
    /// All types that were directly modified (property added or child inserted/deleted).
    /// New types are not included in this Set.
    changed: HashMap<TypePtr, HashSet<Option<String>>, BuildHasherDefault<XorHasher>>,
}

impl<'a> Transaction<'a> {
    pub fn new(store: RefMut<'a, Store>) -> Transaction {
        let begin_timestamp = store.blocks.get_state_vector();
        Transaction {
            store,
            timestamp: begin_timestamp,
            merge_blocks: Vec::new(),
            delete_set: IdSet::new(),
            changed: HashMap::with_hasher(BuildHasherDefault::default()),
        }
    }

    /// Encodes the document state to a binary format.
    ///
    /// Document updates are idempotent and commutative. Caveats:
    /// * It doesn't matter in which order document updates are applied.
    /// * As long as all clients receive the same document updates, all clients
    ///   end up with the same content.
    /// * Even if an update contains known information, the unknown information
    ///   is extracted and integrated into the document structure.
    ///
    /// ```
    /// let doc1 = yrs::Doc::new();
    /// let doc2 = yrs::Doc::new();
    ///
    /// // some content
    /// doc1.get_type("my type").insert(&doc1.transact(), 0, 'a');
    ///
    /// let update = doc1.encode_state_as_update();
    ///
    /// doc2.apply_update(&update);
    ///
    /// assert_eq!(doc1.get_type("my type").to_string(), "a");
    /// ```
    ///
    pub fn encode_update(&self) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        self.store
            .write_blocks(&mut update_encoder, &self.timestamp);
        update_encoder.to_vec()
    }

    pub fn iterate_structs<F>(&mut self, client: &u64, clock_start: u32, len: u32, f: &F)
    where
        F: Fn(&Block) -> (),
    {
        if len == 0 {
            return;
        }

        let clock_end = clock_start + len;
        if let Some(mut index) = self.find_index_clean_start(client, clock_start) {
            let mut blocks = self.store.blocks.get(client).unwrap();
            let mut block = &blocks.list[index];

            while index < blocks.list.len() && block.id().clock < clock_end {
                if clock_end < block.clock_end() {
                    self.find_index_clean_start(client, clock_start);
                    blocks = self.store.blocks.get(client).unwrap();
                    block = &blocks.list[index];
                }

                f(block);
                index += 1;

                block = &blocks.list[index];
            }
        }
    }

    pub fn find_index_clean_start(&mut self, client: &u64, clock: u32) -> Option<usize> {
        let mut id_ptr = None;
        let mut index = 0;

        {
            let blocks = self.store.blocks.get_mut(client)?;
            index = blocks.find_pivot(clock)?;
            let block = &mut blocks.list[index];
            if let Some(item) = block.as_item_mut() {
                if item.id.clock < clock {
                    // if we run over the clock, we need to the split item
                    let half = item.split(clock - item.id.clock);
                    if let Some(ptr) = half.right {
                        id_ptr = Some((ptr.clone(), half.id.clone()))
                    }
                    index += 1;

                    self.merge_blocks.push(half.id.clone());
                    //NOTE: is this right to insert an item right away, or should we always put it
                    // to transaction.merge_blocks? If we do so, we later may not be able to find it
                    // by iterating over the blocks alone?
                    blocks.list.insert(index, Block::Item(half));
                }
            }
        }

        if let Some((right_ptr, id)) = id_ptr {
            self.rewire(&right_ptr, id);
        }

        Some(index)
    }

    fn rewire(&mut self, right_ptr: &BlockPtr, id: ID) {
        // if we had split an item, it was inserted as a new right. We need to rewrite pointers
        // of the old right to point into the new_item on its left:
        //
        // Before:
        //  +------+ --> +------+ --> +-------+
        //  | LEFT |     | ITEM |     | RIGHT |
        //  +------+ <-- +------+     +-------+
        //         ^------------------+
        //
        // After:
        //  +------+ --> +------+ --> +-------+
        //  | LEFT |     | ITEM |     | RIGHT |
        //  +------+ <-- +------+ <-- +-------+

        let blocks = self.store.blocks.get_mut(&right_ptr.id.client).unwrap();
        let right = &mut blocks.list[right_ptr.pivot as usize];
        if let Some(right_item) = right.as_item_mut() {
            right_item.left = Some(BlockPtr::from(id))
        }
    }

    /// Applies given `id_set` onto current transaction to run multi-range deletion.
    /// Returns a remaining of original ID set, that couldn't be applied.
    pub fn apply_delete(&mut self, id_set: &IdSet) -> IdSet {
        let mut unapplied = IdSet::new();
        for (client, ranges) in id_set.iter() {
            let mut blocks = self.store.blocks.get_mut(client).unwrap();
            let state = blocks.get_state();

            for range in ranges.iter() {
                let clock = range.clock;
                let clock_end = clock + range.len;

                if clock < state {
                    if state < clock_end {
                        unapplied.insert(ID::new(*client, clock), clock_end - state);
                    }
                    // We can ignore the case of GC and Delete structs, because we are going to skip them
                    if let Some(mut index) = blocks.find_pivot(clock) {
                        // We can ignore the case of GC and Delete structs, because we are going to skip them
                        if let Some(item) = blocks.list[index].as_item_mut() {
                            // split the first item if necessary
                            if !item.deleted && item.id.clock < clock {
                                index += 1;
                                let right = item.split(clock - item.id.clock);
                                let id = right.id.clone();
                                let right_ptr = right.right.clone();
                                self.merge_blocks.push(id);
                                blocks.list.insert(index, Block::Item(right));
                                if let Some(right_ptr) = right_ptr {
                                    self.rewire(&right_ptr, id);
                                    blocks = self.store.blocks.get_mut(client).unwrap();
                                    // just to make the borrow checker happy
                                }
                            }

                            while index < blocks.list.len() {
                                let block = &mut blocks.list[index];
                                index += 1;
                                if let Some(item) = block.as_item_mut() {
                                    if item.id.clock < clock_end {
                                        if !item.deleted {
                                            let ptr = BlockPtr::from(item.id.clone());
                                            if item.id.clock + item.content.len() > clock_end {
                                                index += 1;
                                                let right = item.split(clock - item.id.clock);
                                                let id = right.id.clone();
                                                let right_ptr = right.right.clone();
                                                self.merge_blocks.push(id);
                                                blocks.list.insert(index, Block::Item(right));
                                                if let Some(right_ptr) = right_ptr {
                                                    self.rewire(&right_ptr, id);
                                                }
                                            }
                                            self.delete(&ptr);
                                            blocks = self.store.blocks.get_mut(client).unwrap();
                                            // just to make the borrow checker happy
                                        }
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    unapplied.insert(ID::new(*client, clock), clock_end - clock);
                }
            }
        }
        unapplied
    }

    fn delete(&mut self, ptr: &BlockPtr) {
        let item = self.store.blocks.get_item_mut(&ptr);
        if !item.deleted {
            //TODO:
            // if let Some(parent) = self.store.get_type(&item.parent) {
            //     // adjust the length of parent
            //     if (this.countable && this.parentSub === null) {
            //         parent._length -= this.length
            //     }
            // }
            item.deleted = true;
            self.delete_set.insert(item.id.clone(), item.len());
            // addChangedTypeToTransaction(transaction, item.type, item.parentSub)
            if item.id.clock < self.timestamp.get(&item.id.client) {
                let set = self.changed.entry(item.parent.clone()).or_default();
                set.insert(item.parent_sub.clone());
            }
            // item.content.delete(transaction)
            match &mut item.content {
                ItemContent::Doc(s, value) => {
                    todo!()
                }
                ItemContent::Type(inner) => {
                    todo!()
                }
                _ => {} // do nothing
            }
        }
    }

    fn update<D: Decoder>(&mut self, decoder: &mut D) {
        let ss = BlockStore::decode(decoder);
        self.integrate_blocks(ss);
    }

    fn add_stack(stack: Vec<Block>, blocks: &mut BlockStore, remaining: &mut BlockStore) {
        for item in stack {
            let id = item.id();
            let to_insert = if let Some(mut unapplicable) = blocks.remove(&id.client) {
                // decrement because we weren't able to apply previous operation
                unapplicable
                    .list
                    .drain(unapplicable.integrated_len - 1..)
                    .collect()
            } else {
                // item was the last item on clientsStructRefs and the field was already cleared.
                // Add item to remaining and continue
                vec![item]
            };
            remaining.insert(id.client, ClientBlockList::from(to_insert));
        }
    }

    fn integrate_blocks(&mut self, mut blocks: BlockStore) -> Option<IntegrationOutput> {
        let mut stack: Vec<Block> = Vec::new();

        // sort them so that we take the higher id first,
        // in case of conflicts the lower id will probably
        // not conflict with the id from the higher user.
        let mut block_ids: Vec<u64> = blocks.keys().cloned().collect();
        block_ids.sort_by(|&a, &b| b.cmp(&a));
        if block_ids.is_empty() {
            None
        } else {
            let mut current_target = blocks.next_missing(&mut block_ids)?;
            if block_ids.is_empty() {
                return None;
            }

            let mut remaining = BlockStore::new();
            let mut missing_vector = StateVector::empty();
            let mut state = StateVector::empty();
            let mut stack_head: Block = current_target.advance();

            loop {
                if let Block::Skip(_) = stack_head {
                    // nothing
                } else {
                    let id = stack_head.id();
                    let client = id.client;
                    let local_clock = self.store.get_state(&client);
                    state.insert(client, local_clock);
                    let offset = local_clock as i32 - item.id.clock as i32;
                    if offset < 0 {
                        stack.push(stack_head);
                        missing_vector.update_missing(client, id.clock);
                        Self::add_stack(stack, &mut blocks, &mut remaining);
                        stack = Vec::new();
                    } else {
                        if let Some(missing) = self.get_missing(&item) {
                            stack.push(item);
                            // get the struct reader that has the missing struct
                            let refs = blocks.get_mut(missing).unwrap_or_else(ClientBlockList::new);
                            if refs.is_integrated() {
                                // This update message causally depends on another update message that doesn't exist yet
                                missing_vector.update_missing(*missing, store.get_state(missing));
                                Self::add_stack(stack, &mut blocks, &mut remaining);
                                stack = Vec::new();
                            } else {
                                stack_head = refs.advance();
                                continue;
                            }
                        } else if offset == 0 || offset < item.len() as i32 {
                            // all fine, apply the stackhead
                            item.integrate(&mut self, offset as usize);
                            state.insert(client, item.id.clock + item.len())
                        }
                    }
                }

                // iterate to next stackHead
                if let Some(next) = stack.pop() {
                    stack_head = next;
                } else if current_target.integrated_len < current_target.list.len() {
                    stack_head = current_target.advance();
                } else {
                    if let Some(t) = blocks.next_missing(&mut block_ids) {
                        current_target = t;
                        stack_head = current_target.advance();
                    } else {
                        break;
                    }
                }
            }

            if !remaining.is_empty() {
                Some(IntegrationOutput {
                    missing: missing_vector,
                    remaining,
                })
            } else {
                None
            }
        }
    }
}

struct IntegrationOutput {
    missing: StateVector,
    remaining: BlockStore,
}
