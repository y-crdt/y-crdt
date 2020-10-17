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

use wasm_bindgen::prelude::*;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::rc::Rc;
use std::cell::{RefCell,Cell};
use rand::Rng;
use encoding::*;
use client_hasher::ClientHasher;

/// A Y.Doc instance.
///
/// ```
/// let doc = yrs::Doc::new();
/// let t = doc.get_type("type_name");
/// ```
#[wasm_bindgen]
pub struct Doc {
    pub client_id: u32,
    inner: Rc<DocInner>,
}

pub struct UpdateEvent {
    pub update: Vec<u8>
}

pub trait Observable <EventType> {
    fn on_change(&self, event: EventType);
}

impl <'a> Doc {
    pub fn on_update (&'a self, observer: std::rc::Weak<impl Observable<UpdateEvent> + 'static>) {
        self.inner.update_handlers.borrow_mut().push(observer);
    }
}

#[wasm_bindgen]
impl Doc {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Doc {
        let client_id: u32 = rand::thread_rng().gen();
        Doc {
            client_id,
            inner: Rc::from(DocInner {
                client_id,
                type_refs: Default::default(),
                types: Default::default(),
                ss: RefCell::from(StructStore::new(client_id)),
                update_handlers: Default::default()
            })
        }
    }
    pub fn transact (&self) -> Transaction {
        Transaction::new(&self.inner)
    }
    #[wasm_bindgen(js_name = encodeStateVector)]
    pub fn encode_state_vector (&self) -> Vec<u8> {
        let sv = self.inner.ss.borrow().get_state_vector();
        // expecting to write at most two u32 values (using variable encoding
        // we will probably write less)
        let mut encoder = encoding::Encoder::with_capacity(sv.len() * 8);
        for (client_id, clock) in sv.iter() {
            encoder.write_var_u32(*client_id);
            encoder.write_var_u32(*clock);
        }
        encoder.buf
    }
    fn get_type_ref (&self, string: &str) -> usize {
        *self.inner.type_refs.borrow_mut().entry(string.to_owned()).or_insert_with(|| {
            let mut names = self.inner.types.borrow_mut();
            let type_ref = names.len();
            names.push((Rc::from(TypeInner { start: Cell::new(None), ptr: TypePtr::Named(type_ref as u32) }), string.to_owned()));
            type_ref
        })
    }
    #[wasm_bindgen(js_name = getType)]
    pub fn get_type(&self, string: &str) -> Type {
        let type_ref = self.get_type_ref(string);
        let inner = self.inner.types.borrow()[type_ref].0.clone();
        Type {
            doc: self.inner.clone(),
            inner
        }
    }
    #[wasm_bindgen(js_name = encodeStateAsUpdate)]
    pub fn encode_state_as_update (&self) -> Vec<u8> {
        let update_encoder = &mut encoding::UpdateEncoder::new();
        self.inner.write_structs(update_encoder, &StateVector::empty());
        update_encoder.buffer().to_owned()
    }
    #[wasm_bindgen(js_name = applyUpdate)]
    pub fn apply_update (&self, update: &[u8]) {
        let update_decoder = &mut encoding::UpdateDecoder::new(update);
        self.read_structs(update_decoder);
    }
    fn read_structs (&self, update_decoder: &mut encoding::UpdateDecoder) {
        let tr = self.transact();
        let mut ss = tr.doc.ss.borrow_mut();
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
                    let ptr = ss.find_item_ptr(&id);
                    parent = Some(ss.get_item(&ptr).parent.clone());
                    (Some(id), Some(ptr))
                } else {
                    (None, None)
                };
                let (right_origin, right) = if info & BIT7 == BIT7{
                    let id = update_decoder.read_right_id();
                    let ptr = ss.find_item_ptr(&id);
                    if info & BIT8 != BIT8 {
                        // only set parent if not already done so above
                        parent = Some(ss.get_item(&ptr).parent.clone());
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
                // integrate
                item.integrate(&tr, &mut ss, StructPtr { pivot: clock as usize, id: ID { client, clock } });
                // add item to struct list
                // @todo try borow of index and generalize in ss
                let client_struct_list = ss.structs.entry(client).or_insert_with(||UserStructList::with_capacity(number_of_structs as usize));
                client_struct_list.list.push(item);
                client_struct_list.integrated_len += 1;

                // struct integration done. Now increase clock
                clock += 1;
            }
        }
    }
}

struct DocInner {
    client_id: u32,
    type_refs: RefCell<HashMap<String, usize>>,
    types: RefCell<Vec<(Rc<TypeInner>,String)>>,
    ss: RefCell<StructStore>,
    update_handlers: RefCell<Vec<std::rc::Weak<dyn Observable<UpdateEvent>>>>
}

impl DocInner {
    fn write_structs (&self, update_encoder: &mut encoding::UpdateEncoder, sv: &StateVector) {
        let ss = self.ss.borrow();
        let types = self.types.borrow();
        // turns this into a vector because at some point we want to sort this
        // @todo Sort for better perf!
        let mut structs: Vec<(&u32, &UserStructList)> = ss.structs.iter().filter(|(client_id, sl)| sv.get_state(**client_id) < sl.get_state()).collect();
        if ss.local_struct_list.integrated_len > sv.get_state(self.client_id) as usize {
            structs.push((&ss.client_id, &ss.local_struct_list));
        }
        update_encoder.rest_encoder.write_var_u32(structs.len() as u32);
        for (client_id, client_structs) in structs.iter() {
            update_encoder.write_client(**client_id);
            update_encoder.rest_encoder.write_var_u32(client_structs.integrated_len as u32);
            update_encoder.rest_encoder.write_var_u32(0); // initial clock
            for i in 0..(client_structs.integrated_len) {
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
                    let type_name = &types[*type_name_ref as usize].1;
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
    doc: Rc<DocInner>,
    inner: Rc<TypeInner>
}

#[wasm_bindgen]
impl Type {
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string (&self) -> String {
        let ss = self.doc.ss.borrow();
        let mut start = self.inner.start.get();

        let mut s = String::new();
        while let Some(a) = start.as_ref() {
            let item = ss.get_item(&a);
            s.push(item.content);
            start = item.right
        }
        s.clone()
    }
    fn find_list_pos (&self, tr: &Transaction, pos: u32) -> ItemPosition {
        if pos == 0 {
            ItemPosition {
                parent: self,
                after: None
            }
        } else {
            let mut ptr = &self.inner.start.get();
            let mut curr_pos = 1;
            let ss = tr.doc.ss.borrow();
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
    pub fn insert (&self, tr: &mut Transaction, pos: u32, c: char) {
        let pos = self.find_list_pos(&tr, pos);
        tr.create_item(&pos, c)
    }
}

pub struct TypeInner {
    start: Cell<Option<StructPtr>>,
    ptr: TypePtr
}

#[derive(Default)]
struct StateVector (HashMap<u32, u32, BuildHasherDefault<ClientHasher>>);

impl StateVector {
    fn empty () -> Self {
        StateVector::default()
    }
    fn len (&self) -> usize {
        self.0.len()
    }
    fn from (ss: &StructStore) -> Self {
        let mut sv = StateVector::default();
        sv.0.insert(ss.client_id, ss.local_struct_list.get_state());
        for (client_id, client_struct_list) in ss.structs.iter() {
            sv.0.insert(*client_id, client_struct_list.get_state());
        }
        sv
    }
    fn get_state (&self, client_id: u32) -> u32 {
        match self.0.get(&client_id) {
            Some(state) => *state,
            None => 0
        }
    }
    fn iter (&self) -> std::collections::hash_map::Iter<u32, u32> {
        self.0.iter()
    }
}

#[wasm_bindgen]
pub struct Transaction {
    doc: Rc<DocInner>,
    start_state_vector: StateVector
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let update_encoder = &mut encoding::UpdateEncoder::new();
        self.doc.write_structs(update_encoder, &self.start_state_vector);
        let update = update_encoder.buffer();
        let mut needs_removed = Vec::new();
        for (i, update_handler) in self.doc.update_handlers.borrow().iter().enumerate() {
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
            self.doc.update_handlers.borrow_mut().remove(*handler);
        }
    }
}

impl Transaction {
    fn new (doc: &Rc<DocInner>) -> Transaction {
        Transaction {
            doc: doc.clone(),
            start_state_vector: doc.ss.borrow().get_state_vector()
        }
    }
    fn get_type_from_ptr (&self, ptr: &TypePtr) -> Rc<TypeInner> {
        match ptr {
            TypePtr::Named(name_ref) => {
                self.doc.types.borrow()[*name_ref as usize].0.clone()
            }
        }
    }
    #[inline(always)]
    fn create_item (&mut self, pos: &ItemPosition, content: char) {
        let mut ss = self.doc.ss.borrow_mut();
        let left = pos.after;
        let right = match pos.after.as_ref() {
            Some(left_id) => ss.get_item(left_id).right,
            None => pos.parent.inner.start.get()
        };
        let id = ID {
            client: self.doc.client_id,
            clock: ss.local_struct_list.get_state()
        };
        let item = Item {
            id,
            content,
            left,
            right,
            origin: pos.after.as_ref().map(|l| l.id),
            right_origin: right.map(|r| r.id),
            parent: pos.parent.inner.ptr.clone()
        };
        let pivot = ss.local_struct_list.integrated_len;
        let own_item_ptr = StructPtr {
            pivot,
            id,
        };
        item.integrate(self, &mut ss, own_item_ptr);
        ss.local_struct_list.list.push(item);
        ss.local_struct_list.integrated_len += 1;
    }
}

struct UserStructList {
    list: Vec<Item>,
    integrated_len: usize,
}

impl UserStructList {
    #[inline]
    fn new () -> UserStructList {
        UserStructList {
            list: Vec::new(),
            integrated_len: 0
        }
    }
    #[inline]
    fn with_capacity (capacity: usize) -> UserStructList {
        UserStructList {
            list: Vec::with_capacity(capacity),
            integrated_len: 0
        }
    }
    #[inline]
    fn get_state (&self) -> u32 {
        if self.integrated_len == 0 {
            0
        } else {
            let item = &self.list[self.integrated_len - 1];
            item.id.clock + 1
        }
    }
}

pub struct StructStore {
    structs: HashMap::<u32, UserStructList, BuildHasherDefault<ClientHasher>>,
    client_id: u32,
    local_struct_list: UserStructList,
    // contains structs that can't be integrated because they depend on other structs
    // unintegrated: HashMap::<u32, Vec<Item>, BuildHasherDefault<ClientHasher>>,
}

impl StructStore {
    fn new (client_id: u32) -> StructStore {
        StructStore {
            structs: HashMap::<u32, UserStructList, BuildHasherDefault<ClientHasher>>::default(),
            client_id,
            local_struct_list: UserStructList::new(),
            // unintegrated: HashMap::<u32, Vec<Item>, BuildHasherDefault<ClientHasher>>::default()
        }
    }
    fn get_state_vector (&self) -> StateVector {
        StateVector::from(self)
    }
    #[inline(always)]
    fn find_item_ptr(&self, id: &ID) -> StructPtr {
        StructPtr {
            id: *id,
            pivot: id.clock as usize,
        }
    }
    #[inline(always)]
    fn get_item_mut (&mut self, ptr: &StructPtr) -> &mut Item {
        if ptr.id.client == self.client_id {
            unsafe {
                self.local_struct_list.list.get_unchecked_mut(ptr.pivot)
            }
        } else {
            unsafe {
                // this is not a dangerous expectation because we really checked
                // beforehand that these items existed (once a reference ptr was created we
                // know that the item existed)
                self.structs.get_mut(&ptr.id.client).unwrap().list.get_unchecked_mut(ptr.pivot)
            }
        }
    }
    #[inline(always)]
    fn get_item (&self, ptr: &StructPtr) -> &Item {
        if ptr.id.client == self.client_id {
            &self.local_struct_list.list[ptr.pivot]
        } else {
            // this is not a dangerous expectation because we really checked
            // beforehand that these items existed (once a reference was created we
            // know that the item existed)
            &self.structs[&ptr.id.client].list[ptr.pivot]
        }
    }
    /*
    #[inline(always)]
    fn get_state (&self, client: u32) -> u32 {
        if client == self.client_id {
            self.local_struct_list.get_state()
        } else {
            if let Some(client_structs) = self.structs.get(&client) {
                client_structs.get_state()
            } else {
                0
            }
        }
    }
    fn get_client_structs_list (&mut self, client_id: u32) -> &mut UserStructList {
        if client_id == self.client_id {
            &mut self.local_struct_list
        } else {
            self.structs.entry(client_id).or_insert_with(UserStructList::new)
        }
    }
    */
}

#[derive(Copy, Clone)]
pub struct ID {
    pub client: u32,
    pub clock: u32
}

#[derive(Copy, Clone)]
pub struct StructPtr {
    id: ID,
    pivot: usize
}

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

pub struct Item {
    id: ID,
    left: Option<StructPtr>,
    right: Option<StructPtr>,
    origin: Option<ID>,
    right_origin: Option<ID>,
    content: char,
    parent: TypePtr
}

impl Item {
    #[inline]
    fn integrate (&self, tr: &Transaction, ss: &mut StructStore, own_item_ptr: StructPtr) {
        // No conflict resolution yet..
        // We only implement the reconnection part:
        if let Some(right_id) = self.right.as_ref() {
            let right = ss.get_item_mut(right_id);
            right.left = Some(own_item_ptr);
        }
        match self.left.as_ref() {
            Some(left_id) => {
                let left = ss.get_item_mut(left_id);
                left.right = Some(own_item_ptr);
            }
            None => {
                let parent_type = tr.get_type_from_ptr(&self.parent);
                parent_type.start.set(Some(own_item_ptr));
            }
        }
    }
}

#[derive(Copy, Clone)]
struct ItemPosition <'a> {
    parent: &'a Type,
    after: Option<StructPtr>
}
