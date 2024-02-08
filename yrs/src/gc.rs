use crate::block::{BlockCell, ClientID, GC};
use crate::{TransactionMut, ID};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct GCCollector {
    items: HashMap<ClientID, Vec<u32>>,
}

impl GCCollector {
    pub fn collect(txn: &mut TransactionMut) {
        let mut gc = Self::default();
        gc.mark_all(txn);
        gc.collect_all_marked(txn);
    }

    fn mark_all(&mut self, txn: &mut TransactionMut) {
        for (client, range) in txn.delete_set.iter() {
            if let Some(blocks) = txn.store.blocks.get_client_mut(client) {
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

    /// Marks item with a given [ID] as a candidate for being GCed.
    pub(crate) fn mark(&mut self, id: &ID) {
        let client = self.items.entry(id.client).or_default();
        client.push(id.clock);
    }

    /// Garbage collects all items marked for GC.
    fn collect_all_marked(self, txn: &mut TransactionMut) {
        for (client_id, clocks) in self.items.into_iter() {
            let client = txn.store.blocks.get_client_blocks_mut(client_id);
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
