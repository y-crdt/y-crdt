use crate::block::{BlockCell, ClientID, GC};
use crate::{DeleteSet, Store, TransactionMut, ID};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct GCCollector {
    marked: HashMap<ClientID, Vec<u32>>,
}

impl GCCollector {
    /// Garbage collect all blocks deleted within current transaction scope.
    pub fn collect(txn: &mut TransactionMut) {
        let mut gc = Self::default();
        gc.mark_in_scope(&mut txn.store, None, &txn.delete_set);
        gc.collect_marked(txn);
    }

    /// Garbage collect all deleted blocks from current transaction's document store.
    pub fn collect_all(txn: &mut TransactionMut, delete_set: Option<&DeleteSet>) {
        let mut gc = Self::default();
        match delete_set {
            None => gc.mark_all(txn),
            Some(ds) => gc.mark_in_scope(&mut txn.store, Some(&mut txn.merge_blocks), ds),
        }
        gc.collect_marked(txn);
    }

    /// Mark deleted items based on a provided delete set.
    fn mark_in_scope(
        &mut self,
        store: &mut Store,
        mut merge_blocks: Option<&mut Vec<ID>>,
        delete_set: &DeleteSet,
    ) {
        for (client, range) in delete_set.iter() {
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
                                    if let Some(merge_blocks) = merge_blocks.as_deref_mut() {
                                        merge_blocks.push(item.id);
                                    }
                                }
                                i += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    fn mark_all(&mut self, txn: &mut TransactionMut) {
        for (_, client_blocks) in txn.store.blocks.iter_mut() {
            for block in client_blocks.iter_mut() {
                if let BlockCell::Block(item) = block {
                    if item.is_deleted() {
                        item.gc(self, false);
                        txn.merge_blocks.push(item.id);
                    }
                }
            }
        }
    }

    /// Marks item with a given [ID] as a candidate for being GCed.
    pub(crate) fn mark(&mut self, id: &ID) {
        let client = self.marked.entry(id.client).or_default();
        client.push(id.clock);
    }

    /// Garbage collects all items marked for GC.
    fn collect_marked(self, txn: &mut TransactionMut) {
        for (client_id, clocks) in self.marked.into_iter() {
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
