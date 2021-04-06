
use crate::*;
use crate::updates::decoder::UpdateDecoder;
use crate::updates::encoder::DSEncoder;
use crate::updates::encoder::UpdateEncoder;

use rand::Rng;
use lib0::decoding::Decoder;

const BIT7: u8 = 0b01000000;
const BIT8: u8 = 0b10000000;

/// A Y.Doc instance.

impl Doc {
    pub fn new() -> Doc {
        let client_id: u64 = rand::thread_rng().gen();
        Doc {
            client_id,
            store: RefCell::from(Store {
                client_id,
                type_refs: Default::default(),
                types: Default::default(),
                blocks: BlockStore::new(client_id),
            }),
        }
    }
    pub fn get_type(&self, tr: &Transaction, string: &str) -> types::Text {
        let ptr = types::TypePtr::Named(string.to_owned());
        types::Text::from(ptr)
    }
    /// Creates a transaction. Transaction cleanups & calling event handles
    /// happen when the transaction struct is dropped.
    ///
    /// # Example
    ///
    /// ```
    /// use std::rc::Rc;
    ///
    /// struct MyObserver {}
    ///
    /// impl yrs::Subscriber<yrs::events::UpdateEvent> for MyObserver {
    ///   fn on_change (&self, event: yrs::events::UpdateEvent) {
    ///     println!("Observer called!")
    ///   }
    /// }
    ///
    /// let doc = yrs::Doc::new();
    ///
    /// // register update observer
    /// let provider = Rc::from(MyObserver {});
    /// doc.on_update(Rc::downgrade(&provider));
    /// {
    ///   let tr = doc.transact();
    ///   doc.get_type("my type").insert(&tr, 0, 'a');
    ///   doc.get_type("my type").insert(&tr, 0, 'a');
    ///   // the block ends and `tr` is going to be dropped
    /// } // => "Observer called!"
    ///
    /// ```
    pub fn transact(&self) -> Transaction {
        Transaction::new(self.store.borrow_mut())
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
    pub fn encode_state_as_update(&self, tr: &Transaction) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        tr.store.write_structs(&mut update_encoder, &StateVector::empty());
        // @todo this is not satisfactory. We would copy the complete buffer every time this method is called.
        // Instead we should implement `write_state_as_update` and fill an existing object that implements the Write trait.
        update_encoder.to_buffer()
    }
    /// Compute a diff to sync with another client.
    ///
    /// This is the most efficient method to sync with another client by only
    /// syncing the differences.
    ///
    /// The sync protocol in Yrs/js is:
    /// * Send StateVector to the other client.
    /// * The other client comutes a minimal diff to sync by using the StateVector.
    ///
    /// ```
    /// let doc1 = yrs::Doc::new();
    /// let doc2 = yrs::Doc::new();
    ///
    /// let state_vector = doc1.get_state_vector();
    /// // encode state vector to a binary format that you can send to other peers.
    /// let state_vector_encoded: Vec<u8> = state_vector.encode();
    ///
    /// let diff = doc2.encode_diff_as_update(&yrs::StateVector::decode(&state_vector_encoded));
    ///
    /// // apply all missing changes from doc2 to doc1.
    /// doc1.apply_update(&diff);
    /// ```
    pub fn encode_diff_as_update(&self, tr: &Transaction, sv: &StateVector) -> Vec<u8> {
        let mut update_encoder = updates::encoder::EncoderV1::new();
        tr.store.write_structs(&mut update_encoder, sv);
        update_encoder.to_buffer()
    }
    /// Apply a document update.
    pub fn apply_update(&self, tr: &mut Transaction, update: &[u8]) {
        let decoder = &mut Decoder::new(update);
        let update_decoder = &mut updates::decoder::DecoderV1::new(decoder);
        tr.store.read_structs(update_decoder)
    }
    // Retrieve document state vector in order to encode the document diff.
    pub fn get_state_vector(&self, tr: &mut Transaction) -> StateVector {
        tr.store.blocks.get_state_vector()
    }
}

impl<'a> Store {
    #[inline]
    pub fn create_item(&mut self, pos: &block::ItemPosition, content: block::ItemContent) {
        let left = pos.after;
        let right = match pos.after.as_ref() {
            Some(left_id) => self.blocks.get_item(left_id).right,
            None => pos.parent.start.get(),
        };
        let id = block::ID {
            client: self.client_id,
            clock: self.blocks.get_local_state(),
        };
        let pivot = self.blocks.local_block_list.integrated_len as u32;
        let item = block::Item {
            id,
            content,
            left,
            right,
            origin: pos.after.as_ref().map(|l| l.id),
            right_origin: right.map(|r| r.id),
            parent: pos.parent.ptr.clone(),
            deleted: false,
            parent_sub: None,
        };
        item.integrate(self, pivot as u32);
        self.blocks.local_block_list.list.push(item);
        self.blocks.local_block_list.integrated_len += 1;
    }
    pub fn read_structs(&mut self, update_decoder: &mut updates::decoder::DecoderV1) {
        let number_of_clients: u32 = update_decoder.rest_decoder.read_var_uint();
        for _ in 0..number_of_clients {
            let client = update_decoder.read_client();
            let number_of_structs: u32 = update_decoder.rest_decoder.read_var_uint();
            let mut clock = update_decoder.rest_decoder.read_var_uint();
            for _ in 0..number_of_structs {
                let info = update_decoder.read_info();
                // we will get parent from either left, right. Otherwise, we
                // read it from update_decoder.
                let mut parent: Option<types::TypePtr> = None;
                let (origin, left) = if info & BIT8 == BIT8 {
                    let id = update_decoder.read_left_id();
                    let ptr = self.blocks.find_item_ptr(&id);
                    parent = Some(self.blocks.get_item(&ptr).parent.clone());
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                let (right_origin, right) = if info & BIT7 == BIT7 {
                    let id = update_decoder.read_right_id();
                    let ptr = self.blocks.find_item_ptr(&id);
                    if info & BIT8 != BIT8 {
                        // only set parent if not already done so above
                        parent = Some(self.blocks.get_item(&ptr).parent.clone());
                    }
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                if info & (BIT7 | BIT8) == 0 {
                    // neither origin nor right_origin is defined
                    let parent_info = update_decoder.read_parent_info();
                    if parent_info {
                        let type_name = update_decoder.read_string();
                        let type_name_ref = self.init_type_ref(type_name);
                        parent = Some(types::TypePtr::NamedRef(type_name_ref as u32));
                    } else {
                        let id = update_decoder.read_left_id();
                        parent = Some(types::TypePtr::Id(block::BlockPtr::from(id)));
                    }
                };
                let stringContent = update_decoder.read_string();
                let content = block::ItemContent::String(stringContent.to_owned());
                let item = block::Item {
                    id: block::ID { client, clock },
                    left,
                    right,
                    origin,
                    right_origin,
                    content,
                    parent: parent.unwrap(),
                    deleted: false,
                    parent_sub: None
                };
                item.integrate(self, clock); // todo compute pivot beforehand
                                             // add item to struct list
                                             // @todo try borow of index and generalize in ss
                let client_struct_list = self
                    .blocks
                    .get_client_structs_list_with_capacity(client, number_of_structs as usize);
                client_struct_list.list.push(item);
                client_struct_list.integrated_len += 1;

                // struct integration done. Now increase clock
                clock += 1;
            }
        }
    }

    pub fn write_structs(&self, update_encoder: &mut updates::encoder::EncoderV1, sv: &StateVector) {
        // turns this into a vector because at some point we want to sort this
        // @todo Sort for better perf!
        let mut structs: Vec<(&u64, &ClientBlockList)> = self
            .blocks
            .clients
            .iter()
            // @todo this could be optimized
            .filter(|(client_id, sl)| sv.get_state(**client_id) < sl.get_state())
            .collect();
        if self.blocks.local_block_list.integrated_len > sv.get_state(self.client_id) as usize {
            structs.push((&self.client_id, &self.blocks.local_block_list));
        }
        update_encoder.rest_encoder.write_var_uint(structs.len());

        for (client_id, client_structs) in structs.iter() {
            let start_clock = sv.get_state(**client_id);
            let start_pivot = client_structs.find_pivot(start_clock);
            update_encoder.write_client(**client_id);
            update_encoder
                .rest_encoder
                .write_var_uint(client_structs.integrated_len as u32 - start_pivot);
            update_encoder.rest_encoder.write_var_uint(start_clock); // initial clock
            for i in (start_pivot as usize)..(client_structs.integrated_len) {
                let item = &client_structs.list[i];
                let info = if item.origin.is_some() { BIT8 } else { 0 } // is left null
                    | if item.right_origin.is_some() { BIT7 } else { 0 }; // is right null
                update_encoder.write_info(info);
                if let Some(origin_id) = item.origin.as_ref() {
                    update_encoder.write_left_id(origin_id);
                }
                if let Some(right_origin_id) = item.right_origin.as_ref() {
                    update_encoder.write_right_id(right_origin_id);
                }
                if item.origin.is_none() && item.right_origin.is_none() {
                    match &item.parent {
                        types::TypePtr::NamedRef(type_name_ref) => {
                            let type_name = &self.types[*type_name_ref as usize].1;
                            update_encoder.write_parent_info(true);
                            update_encoder.write_string(type_name);
                        }
                        types::TypePtr::Id(id) => {
                            update_encoder.write_parent_info(false);
                            update_encoder.write_left_id(&id.id);
                        }
                        types::TypePtr::Named(name) => {
                            update_encoder.write_parent_info(true);
                            update_encoder.write_string(name)
                        }
                    }
                }
            }
        }
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}
