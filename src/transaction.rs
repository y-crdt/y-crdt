use crate::*;

pub struct Transaction {
    pub doc: Rc<RefCell<DocInner>>,
    pub start_state_vector: StateVector
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let inner = &mut self.doc.borrow_mut();
        let update_encoder = &mut encoding::UpdateEncoder::new();
        inner.write_structs(update_encoder, &self.start_state_vector);
        let update = update_encoder.buffer();
        let mut needs_removed = Vec::new();
        for (i, update_handler) in inner.update_handlers.iter().enumerate() {
            let update_event = UpdateEvent {
                update: update.to_vec()
            };
            match update_handler.upgrade() {
                Some(handler) => handler.on_change(update_event),
                None => needs_removed.push(i)
            };
        }
        // delete weak references that don't point to data anymore.
        for handler in needs_removed.iter().rev() {
            inner.update_handlers.remove(*handler);
        }
    }
}

impl Transaction {
    pub fn end (self: Rc<Self>) {
        // .. nop
        // This method is called to end ownership of the transaction.
        // Drop will be automatically called which will handle the transaction cleanup.
    }
}
