use crate::*;

use updates::encoder::*;
use std::cell::RefMut;
use crate::block::{Item, ID, BlockPtr, Block, ItemContent};
use crate::id_set::{IdSet, IdRange};
use crate::types::TypePtr;

impl <'a> Transaction <'a> {
    pub fn new (store: RefMut<'a, Store>) -> Transaction {
        let start_state_vector = store.blocks.get_state_vector();
        Transaction {
            store,
            start_state_vector,
            merge_blocks: Vec::new(),
            delete_set: IdSet::new(),
            changed: HashMap::new(),
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
    pub fn encode_update (&self) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        self.store.write_structs(&mut update_encoder, &self.start_state_vector);
        update_encoder.to_buffer()
    }

    pub fn iterate_structs<F>(&mut self, client: &u64, clock_start: u32, len: u32, f: &F)
        where F: Fn(&Block) -> () {

        if len == 0 {
            return
        }

        let clock_end = clock_start + len;
        if let Some(mut index) = self.find_index_clean_start(client, clock_start) {
            let mut blocks = self.store.blocks.clients.get(client).unwrap();
            let mut block = &blocks.list[index];

            while index < blocks.list.len() && block.id().clock < clock_end {
                if clock_end < block.clock_end() {
                    self.find_index_clean_start(client, clock_start);
                    blocks = self.store.blocks.clients.get(client).unwrap();
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
            let blocks = self.store.blocks.clients.get_mut(client)?;
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

        let blocks = self.store.blocks.clients.get_mut(&right_ptr.id.client).unwrap();
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
            let mut blocks = self.store.blocks.clients.get_mut(client).unwrap();
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
                                    blocks = self.store.blocks.clients.get_mut(client).unwrap(); // just to make the borrow checker happy
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
                                            blocks = self.store.blocks.clients.get_mut(client).unwrap(); // just to make the borrow checker happy
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
            if item.id.clock < self.start_state_vector.get_state(item.id.client) {
                let set = self.changed.entry(item.parent.clone()).or_default();
                set.insert(item.parent_sub.clone());
            }
            // item.content.delete(transaction)
            match &mut item.content {
                ItemContent::Doc(s, value) => {
                    todo!()
                },
                ItemContent::Type(t) => {
                    todo!()
                },
                _ => {}, // do nothing
            }
        }
    }
}
