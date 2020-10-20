/**
 * Terminology:
 *   Struct: The Y CRDT is modeled using Structs. Unfortunately, the terminology struct is very confusing so I try to avoid it..
 *   Item: A Struct that is used to model sequence data. It is connected through a linked list.
 *   ItemUnintegrated: A Struct that is an unintegrated item. it is not yet connected through the linked list.
 *   GC: A Struct that only marks that content existed but doesn't anymore. It contains all necessary metadata information.
 *
 */
mod encoding;
mod client_hasher;
mod block;
mod transaction;
mod block_store;

use wasm_bindgen::prelude::*;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::rc::Rc;
use std::cell::{RefCell,Cell};
use rand::Rng;
use encoding::*;
use client_hasher::ClientHasher;
use block::*;
use transaction::*;
use block_store::*;

#[test]
fn it_works() {
    assert_eq!(2 + 2, 4);
}

/// A Y.Doc instance.
///
/// ```
/// let doc = yrs::Doc::new();
/// let t = doc.get_type("type_name");
/// ```
#[wasm_bindgen]
pub struct Doc {
    pub client_id: u32,
    inner: Rc<RefCell<DocInner>>,
}

pub struct UpdateEvent {
    pub update: Vec<u8>
}

pub trait Observable <EventType> {
    fn on_change(&self, event: EventType);
}

impl <'a> Doc {
    pub fn on_update (&'a self, observer: std::rc::Weak<impl Observable<UpdateEvent> + 'static>) {
        self.inner.borrow_mut().update_handlers.push(observer);
    }
}

#[wasm_bindgen]
impl Doc {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Doc {
        let client_id: u32 = rand::thread_rng().gen();
        Doc {
            client_id,
            inner: Rc::from(RefCell::from(DocInner {
                client_id,
                type_refs: Default::default(),
                types: Default::default(),
                ss: StructStore::new(client_id),
                update_handlers: Default::default(),
                active_transaction: Default::default()
            }))
        }
    }
    #[wasm_bindgen(js_name = encodeStateVector)]
    pub fn encode_state_vector (&self) -> Vec<u8> {
        let sv = self.inner.borrow().ss.get_state_vector();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = encoding::Encoder::with_capacity(sv.len() * 8);
        for (client_id, clock) in sv.iter() {
            encoder.write_var_u32(*client_id);
            encoder.write_var_u32(*clock);
        }
        encoder.buf
    }
    #[wasm_bindgen(js_name = getType)]
    pub fn get_type(&self, string: &str) -> Type {
        let inner = &mut self.inner.borrow_mut();
        let type_ref = inner.get_type_ref(string);
        let inner = inner.types[type_ref].0.clone();
        Type {
            doc: self.inner.clone(),
            inner
        }
    }
    #[wasm_bindgen(js_name = encodeStateAsUpdate)]
    pub fn encode_state_as_update (&self) -> Vec<u8> {
        let update_encoder = &mut encoding::UpdateEncoder::new();
        self.inner.borrow().write_structs(update_encoder, &StateVector::empty());
        update_encoder.buffer().to_owned()
    }
    #[wasm_bindgen(js_name = applyUpdate)]
    pub fn apply_update (&self, update: &[u8]) {
        let tr = self.transact();
        let update_decoder = &mut encoding::UpdateDecoder::new(update);
        self.inner.borrow_mut().read_structs(update_decoder);
        tr.end();
    }
}

impl Doc {
    pub fn transact (&self) -> Rc<Transaction> {
        self.inner.borrow_mut().transact(&self.inner)
    }
}

pub struct DocInner {
    client_id: u32,
    type_refs: HashMap<String, usize>,
    types: Vec<(Rc<TypeInner>,String)>,
    ss: StructStore,
    update_handlers: Vec<std::rc::Weak<dyn Observable<UpdateEvent>>>,
    active_transaction: std::rc::Weak<Transaction>,
}

impl <'a> DocInner {
    pub fn transact (&mut self, doc: &Rc<RefCell<DocInner>>) -> Rc<Transaction> {
        match self.active_transaction.upgrade() {
            Some(tr) => tr,
            None => {
                let tr = Rc::from(Transaction {
                    start_state_vector: self.ss.get_state_vector(),
                    doc: doc.clone()
                });
                self.active_transaction = Rc::downgrade(&tr);
                tr
            }
        }
    }

    #[inline]
    pub fn create_item (&mut self, pos: &ItemPosition, content: char) {
        let left = pos.after;
        let right = match pos.after.as_ref() {
            Some(left_id) => self.ss.get_item(left_id).right,
            None => pos.parent.inner.start.get()
        };
        let id = ID {
            client: self.client_id,
            clock: self.ss.get_local_state()
        };
        let pivot = self.ss.local_struct_list.integrated_len as u32;
        let item = Item {
            id,
            content,
            left,
            right,
            origin: pos.after.as_ref().map(|l| l.id),
            right_origin: right.map(|r| r.id),
            parent: pos.parent.inner.ptr.clone()
        };
        item.integrate(self, pivot as u32);
        self.ss.local_struct_list.list.push(item);
        self.ss.local_struct_list.integrated_len += 1;
    }
    pub fn get_type_from_ptr (&self, ptr: &TypePtr) -> Rc<TypeInner> {
        match ptr {
            TypePtr::Named(name_ref) => {
                self.types[*name_ref as usize].0.clone()
            }
        }
    }
    fn get_type_ref (&mut self, string: &str) -> usize {
        let types = &mut self.types;
        *self.type_refs.entry(string.to_owned()).or_insert_with(|| {
            let type_ref = types.len();
            types.push((Rc::from(TypeInner { start: Cell::new(None), ptr: TypePtr::Named(type_ref as u32) }), string.to_owned()));
            type_ref
        })
    }
    pub fn read_structs (&mut self, update_decoder: &mut encoding::UpdateDecoder) {
        let number_of_clients = update_decoder.rest_decoder.read_var_u32();
        for _ in 0..number_of_clients {
            let client = update_decoder.read_client();
            let number_of_structs = update_decoder.rest_decoder.read_var_u32();
            let mut clock = update_decoder.rest_decoder.read_var_u32();
            for _ in 0..number_of_structs {
                let info = update_decoder.read_info();
                // we will get parent from either left, right. Otherwise, we
                // read it from update_decoder.
                let mut parent: Option<TypePtr> = None;
                let (origin, left) = if info & BIT8 == BIT8 {
                    let id = update_decoder.read_left_id();
                    let ptr = self.ss.find_item_ptr(&id);
                    parent = Some(self.ss.get_item(&ptr).parent.clone());
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                let (right_origin, right) = if info & BIT7 == BIT7{
                    let id = update_decoder.read_right_id();
                    let ptr = self.ss.find_item_ptr(&id);
                    if info & BIT8 != BIT8 {
                        // only set parent if not already done so above
                        parent = Some(self.ss.get_item(&ptr).parent.clone());
                    }
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                if info & (BIT7 | BIT8) == 0 {
                    // neither origin nor right_origin is defined
                    let type_name = update_decoder.read_string();
                    let type_name_ref = self.get_type_ref(&type_name);
                    parent = Some(TypePtr::Named(type_name_ref as u32))
                };
                let content = update_decoder.read_char();
                let item = Item {
                    id: ID {client, clock},
                    left,
                    right,
                    origin,
                    right_origin,
                    content,
                    parent: parent.unwrap()
                };
                item.integrate(self, clock); // todo compute pivot beforehand
                // add item to struct list
                // @todo try borow of index and generalize in ss
                let client_struct_list = self.ss.structs.entry(client).or_insert_with(||UserStructList::with_capacity(number_of_structs as usize));
                client_struct_list.list.push(item);
                client_struct_list.integrated_len += 1;

                // struct integration done. Now increase clock
                clock += 1;
            }
        }
    }

    pub fn write_structs (&self, update_encoder: &mut encoding::UpdateEncoder, sv: &StateVector) {
        // turns this into a vector because at some point we want to sort this
        // @todo Sort for better perf!
        let mut structs: Vec<(&u32, &UserStructList)> = self.ss.structs.iter().filter(|(client_id, sl)| sv.get_state(**client_id) < sl.get_state()).collect();
        if self.ss.local_struct_list.integrated_len > sv.get_state(self.ss.client_id) as usize {
            structs.push((&self.client_id, &self.ss.local_struct_list));
        }
        update_encoder.rest_encoder.write_var_u32(structs.len() as u32);
        for (client_id, client_structs) in structs.iter() {
            let start_clock = sv.get_state(**client_id);
            let start_pivot = client_structs.find_pivot(start_clock);
            update_encoder.write_client(**client_id);
            update_encoder.rest_encoder.write_var_u32(client_structs.integrated_len as u32 - start_pivot);
            update_encoder.rest_encoder.write_var_u32(start_clock); // initial clock
            for i in (start_pivot as usize)..(client_structs.integrated_len) {
                let item = &client_structs.list[i];
                let info =
                    if item.origin.is_some() { BIT8 } else { 0 } // is left null
                    | if item.right_origin.is_some() { BIT7 } else { 0 }; // is right null
                update_encoder.write_info(info);
                if let Some(origin_id) = item.origin.as_ref() {
                    update_encoder.write_left_id(origin_id);
                }
                if let Some(right_origin_id) = item.right_origin.as_ref() {
                    update_encoder.write_right_id(right_origin_id);
                }
                if item.origin.is_none() && item.right_origin.is_none() {
                    let TypePtr::Named(type_name_ref) = &item.parent;
                    let type_name = &self.types[*type_name_ref as usize].1;
                    update_encoder.write_string(type_name);
                }
                update_encoder.write_char(item.content);
            }
        }
    }
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

#[wasm_bindgen]
pub struct Type {
    doc: Rc<RefCell<DocInner>>,
    inner: Rc<TypeInner>
}

#[wasm_bindgen]
impl Type {
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string (&self) -> String {
        let ss = &self.doc.borrow().ss;
        let mut start = self.inner.start.get();

        let mut s = String::new();
        while let Some(a) = start.as_ref() {
            let item = ss.get_item(&a);
            s.push(item.content);
            start = item.right
        }
        s
    }
    fn find_list_pos (&self, ss: &StructStore, pos: u32) -> ItemPosition {
        if pos == 0 {
            ItemPosition {
                parent: self,
                after: None
            }
        } else {
            let mut ptr = &self.inner.start.get();
            let mut curr_pos = 1;
            while curr_pos != pos {
                if let Some(a) = ptr.as_ref() {
                    ptr = &ss.get_item(a).right;
                    curr_pos += 1;
                } else {
                    // todo: throw error here
                    break;
                }
            }
            ItemPosition {
                parent: self,
                after: *ptr
            }
        }
    }
}

impl Type {
    pub fn insert (&self, pos: u32, c: char) {
        let doc = &mut self.doc.borrow_mut();
        let pos = self.find_list_pos(&doc.ss, pos);
        doc.create_item(&pos, c);
    }
}

pub struct TypeInner {
    start: Cell<Option<StructPtr>>,
    ptr: TypePtr
}

#[derive(Default)]
pub struct StateVector (HashMap<u32, u32, BuildHasherDefault<ClientHasher>>);

pub enum TypePtr {
    Named(u32),
    // Id(ID)
}

impl <'a> TypePtr {
    fn clone (&self) -> TypePtr {
        match self {
            TypePtr::Named(tname) => TypePtr::Named(*tname)
        }
    }
}
