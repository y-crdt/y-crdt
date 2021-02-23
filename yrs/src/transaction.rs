use crate::*;

#[wasm_bindgen]
pub struct Transaction {
    #[wasm_bindgen(skip)]
    pub doc: Rc<RefCell<DocInner>>,

    #[wasm_bindgen(skip)]
    pub start_state_vector: StateVector,
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let inner = &mut self.doc.borrow_mut();
        // only compute update if observers exist
        if !inner.update_handlers.is_empty() {
            let update_encoder = &mut encoding::UpdateEncoder::new();
            inner.write_structs(update_encoder, &self.start_state_vector);
            let update = update_encoder.buffer();
            let mut needs_removed = Vec::new();
            for (i, update_handler) in inner.update_handlers.iter().enumerate() {
                let update_event = events::UpdateEvent {
                    update: update.to_vec(),
                };
                match update_handler.upgrade() {
                    Some(handler) => handler.on_change(update_event),
                    None => needs_removed.push(i),
                };
            }
            // delete weak references that don't point to data anymore.
            for handler in needs_removed.iter().rev() {
                inner.update_handlers.remove(*handler);
            }
        }
    }
}
