use crate::block::{BlockCell, ClientID, GC};
use crate::transaction::TransactionState;
use crate::{Store, TransactionMut, ID};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct GCCollector {
    items: HashMap<ClientID, Vec<u32>>,
}

impl GCCollector {
    /// Garbage collect all blocks deleted within current transaction scope.
    pub fn collect(store: &mut Store, state: &TransactionState) {
        let mut gc = Self::default();
        gc.mark_in_scope(store, state);
        gc.collect_all_marked(store);
    }

    /// Garbage collect all deleted blocks from current transaction's document store.
    pub fn collect_all(txn: &mut TransactionMut) {
        let mut gc = Self::default();
        let (store, state) = txn.split_mut();
        gc.mark_all(store, state);
        gc.collect_all_marked(store);
    }

    /// Mark deleted items based on a current transaction delete set.
    fn mark_in_scope(&mut self, store: &mut Store, state: &TransactionState) {
        for (client, range) in state.delete_set.iter() {
            if let Some(blocks) = store.blocks.get_client_mut(client) {
                for delete_item in range.iter().rev() {
                    let mut start = delete_item.start;
                    if let Some(mut i) = blocks.find_pivot(start) {
                        while i < blocks.len() {
                            let block = &mut blocks[i];
                            let len = block.len();
                            start += len;
                            if start > delete_item.end {
                                break;
                            } else {
                                if let BlockCell::Block(item) = block {
                                    item.gc(self, false);
                                }
                                i += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    fn mark_all(&mut self, store: &mut Store, state: &mut TransactionState) {
        for (_, client_blocks) in store.blocks.iter_mut() {
            for block in client_blocks.iter_mut() {
                if let BlockCell::Block(item) = block {
                    if item.is_deleted() {
                        item.gc(self, false);
                        state.merge_blocks.push(item.id);
                    }
                }
            }
        }
    }

    /// Marks item with a given [ID] as a candidate for being GCed.
    pub(crate) fn mark(&mut self, id: &ID) {
        let client = self.items.entry(id.client).or_default();
        client.push(id.clock);
    }

    /// Garbage collects all items marked for GC.
    fn collect_all_marked(self, store: &mut Store) {
        for (client_id, clocks) in self.items.into_iter() {
            let client = store.blocks.get_client_blocks_mut(client_id);
            for clock in clocks {
                if let Some(index) = client.find_pivot(clock) {
                    let block = &mut client[index];
                    if let BlockCell::Block(item) = block {
                        if item.is_deleted() && !item.info.is_keep() {
                            let (start, end) = item.clock_range();
                            let gc = BlockCell::GC(GC::new(start, end));
                            *block = gc;
                        }
                    }
                }
            }
        }
    }
}
