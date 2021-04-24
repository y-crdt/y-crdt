use crate::*;

use updates::encoder::*;
use std::cell::RefMut;
use crate::block::{Item, ID, BlockPtr, Block};

impl <'a> Transaction <'a> {
    pub fn new (store: RefMut<'a, Store>) -> Transaction {
        let start_state_vector = store.blocks.get_state_vector();
        Transaction {
            store,
            start_state_vector,
            merge_blocks: Vec::new(),
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
            let blocks = &self.store.blocks.clients.get(client).unwrap().list;
            let mut block = &blocks[index];
            let mut blocks_len = blocks.len();

            while index < blocks_len && block.id().clock < clock_end {
                if clock_end < block.id().clock + block.len() {
                    self.find_index_clean_start(client, clock_start);
                    let blocks = &self.store.blocks.clients.get(client).unwrap().list;
                    blocks_len = blocks.len();
                    block = &blocks[index];
                }

                f(block);
                index += 1;
                
                let blocks = &self.store.blocks.clients.get(client).unwrap().list;
                block = &blocks[index];
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
        //
        if let Some((right_ptr, id)) = id_ptr {
            let right = {
                let blocks = self.store.blocks.clients.get_mut(&right_ptr.id.client).unwrap();
                &mut blocks.list[right_ptr.pivot as usize]
            };
            if let Some(right_item) = right.as_item_mut() {
                right_item.left = Some(BlockPtr::from(id))
            }
        }

        Some(index)
    }
}
