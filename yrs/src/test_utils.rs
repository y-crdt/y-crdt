use crate::block::ClientID;
use crate::transaction::ReadTxn;
use crate::updates::decoder::{Decode, Decoder, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use crate::{Doc, StateVector, Transact, Update};
use lib0::decoding::{Cursor, Read};
use rand::distributions::Alphanumeric;
use rand::prelude::{SliceRandom, StdRng};
use rand::{random, Rng, RngCore, SeedableRng};
use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

pub const EXCHANGE_UPDATES_ORIGIN: &str = "exchange_updates";

pub fn exchange_updates(docs: &[&Doc]) {
    for i in 0..docs.len() {
        for j in 0..docs.len() {
            if i != j {
                let a = docs[i];
                let ta = a.transact();
                let b = docs[j];
                let mut tb = b.transact_mut_with(EXCHANGE_UPDATES_ORIGIN);

                let sv = tb.state_vector().encode_v1();
                let update = ta.encode_diff_v1(&StateVector::decode_v1(sv.as_slice()).unwrap());
                tb.apply_update(Update::decode_v1(update.as_slice()).unwrap());
            }
        }
    }
}

const MSG_SYNC_STEP_1: usize = 0;
const MSG_SYNC_STEP_2: usize = 1;
const MSG_SYNC_UPDATE: usize = 2;

pub fn run_scenario<F>(mut seed: u64, mods: &[F], users: usize, iterations: usize)
where
    F: Fn(&mut Doc, &mut StdRng),
{
    if seed == 0 {
        seed = random();
        println!("run scenario with seed: {}", seed);
    }

    let rng = StdRng::seed_from_u64(seed);
    let tc = TestConnector::with_peer_num(rng, users as u64);
    for _ in 0..iterations {
        if tc.0.borrow_mut().rng.gen_range(0, 100) <= 2 {
            // 2% chance to disconnect/reconnect a random user
            if tc.0.borrow_mut().rng.gen_bool(0.5) {
                tc.disconnect_random();
            } else {
                tc.reconnect_random();
            }
        } else if tc.0.borrow_mut().rng.gen_range(0, 100) <= 1 {
            // 1% chance to flush all
            tc.flush_all();
        } else if tc.0.borrow_mut().rng.gen_range(0, 100) <= 50 {
            tc.flush_random();
        }

        {
            let inner = &mut *tc.0.borrow_mut();
            let rng = &mut inner.rng;
            let idx = rng.gen_range(0, inner.peers.len());
            let peer = &mut inner.peers[idx];
            let test = mods.choose(rng).unwrap();
            test(&mut peer.doc, rng);
        };
    }

    tc.assert_final_state();
}

pub struct TestConnector(Rc<RefCell<Inner>>);

struct Inner {
    rng: StdRng,
    peers: Vec<TestPeer>,
    /// Maps all Client IDs to indexes in the `docs` vector.
    all: HashMap<ClientID, usize>,
    /// Maps online Client IDs to indexes in the `docs` vector.
    online: HashMap<ClientID, usize>,
}

impl TestConnector {
    /// Create new [TestConnector] with provided randomizer.
    pub fn with_rng(rng: StdRng) -> Self {
        TestConnector(Rc::new(RefCell::new(Inner {
            rng,
            peers: Vec::new(),
            all: HashMap::new(),
            online: HashMap::new(),
        })))
    }

    /// Create a new [TestConnector] with pre-initialized number of peers.
    pub fn with_peer_num(rng: StdRng, peer_num: u64) -> Self {
        let mut tc = Self::with_rng(rng);
        for client_id in 0..peer_num {
            let peer = tc.create_peer(client_id as ClientID);
            peer.doc.get_or_insert_text("text");
            peer.doc.get_or_insert_map("map");
        }
        tc.sync_all();
        tc
    }

    /// Returns random number generator attached to current [TestConnector].
    pub fn rng(&self) -> RefMut<StdRng> {
        let inner = self.0.borrow_mut();
        RefMut::map(inner, |i| &mut i.rng)
    }

    /// Create a new [TestPeer] with provided `client_id` or return one, if such `client_id`
    /// was already created before.
    pub fn create_peer(&self, client_id: ClientID) -> &mut TestPeer {
        if let Some(peer) = self.get_mut(&client_id) {
            peer
        } else {
            let rc = self.0.clone();
            let inner = unsafe { self.0.as_ptr().as_mut().unwrap() };
            let instance = TestPeer::new(client_id);
            let _sub = instance
                .doc
                .observe_update_v1(move |_, e| {
                    let mut inner = rc.borrow_mut();
                    Self::broadcast(&mut inner, client_id, &e.update);
                })
                .unwrap();
            let idx = inner.peers.len();
            inner.peers.push(instance);
            inner.all.insert(client_id, idx);
            inner.online.insert(client_id, idx);
            &mut inner.peers[idx]
        }
    }

    fn broadcast(inner: &mut RefMut<Inner>, sender: ClientID, payload: &Vec<u8>) {
        let online: Vec<_> = inner
            .online
            .iter()
            .filter_map(|(&id, &idx)| if id != sender { Some(idx) } else { None })
            .collect();
        for idx in online {
            let peer = &mut inner.peers[idx];
            peer.receive(sender, payload.clone());
        }
    }

    /// Try to retrieve a reference to [TestPeer] for a given `client_id`, if such node was created.
    pub fn get(&self, client_id: &ClientID) -> Option<&TestPeer> {
        let inner = unsafe { self.0.as_ptr().as_ref().unwrap() };
        let idx = inner.all.get(client_id)?;
        Some(&inner.peers[*idx])
    }

    /// Try to retrieve a mutable reference to [TestPeer] for a given `client_id`,
    /// if such node was created.
    pub fn get_mut(&self, client_id: &ClientID) -> Option<&mut TestPeer> {
        let inner = self.0.borrow_mut();
        let idx = *inner.all.get(client_id)?;
        unsafe {
            let peers = inner.peers.as_ptr() as *mut TestPeer;
            let peer = peers.offset(idx as isize);
            peer.as_mut()
        }
    }

    /// Disconnects test node with given `client_id` from the rest of known nodes.
    pub fn disconnect(&self, client_id: ClientID) {
        if let Some(peer) = self.get_mut(&client_id) {
            peer.receiving.clear();
        }
        let mut inner = self.0.borrow_mut();
        inner.online.remove(&client_id);
    }

    /// Append `client_id` to the list of known Y instances in [TestConnector].
    /// Also initiate sync with all clients.
    pub fn connect(&self, client_id: ClientID) {
        let mut inner = self.0.borrow_mut();
        Self::connect_inner(&mut inner, client_id);
    }

    fn connect_inner(inner: &mut RefMut<Inner>, client_id: ClientID) {
        if !inner.online.contains_key(&client_id) {
            let idx = *inner.all.get(&client_id).expect("unknown client_id");
            inner.online.insert(client_id, idx);
        }

        let client_idx = *inner.all.get(&client_id).unwrap();
        let payload = {
            let sender = &mut inner.peers[client_idx];
            let mut encoder = EncoderV1::new();
            Self::write_step1(sender, &mut encoder);
            encoder.to_vec()
        };
        Self::broadcast(inner, client_id, &payload);

        let online: Vec<_> = inner
            .online
            .iter()
            .filter_map(|(&id, &idx)| {
                if id != client_id {
                    Some((id, idx))
                } else {
                    None
                }
            })
            .collect();
        for (remote_id, idx) in online {
            let payload = {
                let peer = &inner.peers[idx];
                let mut encoder = EncoderV1::new();
                Self::write_step1(peer, &mut encoder);
                encoder.to_vec()
            };

            let sender = &mut inner.peers[client_idx];
            sender.receive(remote_id, payload);
        }
    }

    /// Reconnects back all known peers.
    pub fn reconnect_all(&mut self) {
        let mut inner = self.0.borrow_mut();
        let all_ids: Vec<_> = inner.all.keys().cloned().collect();
        for client_id in all_ids {
            Self::connect_inner(&mut inner, client_id);
        }
    }

    /// Disconnects all known peers from each other.
    pub fn disconnect_all(&mut self) {
        let mut inner = self.0.borrow_mut();
        let all_ids: Vec<_> = inner.all.keys().cloned().collect();
        for client_id in all_ids {
            Self::connect_inner(&mut inner, client_id);
        }
    }

    /// Reconnects all known peers and processes their pending messages.
    pub fn sync_all(&mut self) {
        self.reconnect_all();
        self.flush_all();
    }

    /// Processes all pending messages of connected peers in random order.
    pub fn flush_all(&self) -> bool {
        let mut did_something = false;
        while self.flush_random() {
            did_something = true;
        }
        did_something
    }

    /// Choose random connection and flush a random message from a random sender.
    /// If this function was unable to flush a message, because there are no more messages to flush,
    /// it returns false. true otherwise.
    pub fn flush_random(&self) -> bool {
        let mut inner = self.0.borrow_mut();
        Self::flush_random_inner(&mut inner)
    }

    fn flush_random_inner(inner: &mut RefMut<Inner>) -> bool {
        if let Some((receiver, sender)) = Self::pick_random_pair(inner) {
            if let Some(m) = receiver
                .receiving
                .get_mut(&sender.client_id())
                .unwrap()
                .pop_front()
            {
                let mut encoder = EncoderV1::new();
                let mut decoder = DecoderV1::new(Cursor::new(m.as_slice()));
                Self::read_sync_message(receiver, &mut decoder, &mut encoder);
                let payload = encoder.to_vec();
                if !payload.is_empty() {
                    sender.receive(receiver.client_id(), payload); // send reply message
                }

                // If update message, add the received message to the list of received messages
                {
                    let mut decoder = DecoderV1::new(Cursor::new(m.as_slice()));
                    let msg_type: usize = decoder.read_var().unwrap();
                    if msg_type == MSG_SYNC_STEP_2 || msg_type == MSG_SYNC_UPDATE {
                        receiver
                            .updates
                            .push_back(decoder.read_buf().unwrap().to_vec())
                    }
                }
                true
            } else {
                receiver.receiving.remove(&sender.client_id());
                Self::flush_random_inner(inner)
            }
        } else {
            false
        }
    }

    fn pick_random_pair<'a>(
        inner: &'a mut RefMut<Inner>,
    ) -> Option<(&'a mut TestPeer, &'a mut TestPeer)> {
        let pairs: Vec<_> = inner
            .peers
            .iter()
            .enumerate()
            .flat_map(|(receiver_idx, conn)| {
                if conn.receiving.is_empty() {
                    vec![]
                } else {
                    conn.receiving
                        .keys()
                        .map(|id| (receiver_idx, *inner.all.get(id).unwrap()))
                        .collect()
                }
            })
            .collect();
        let (receiver_idx, sender_idx) = pairs.choose(&mut inner.rng)?;
        unsafe {
            let ptr = inner.peers.as_ptr() as *mut TestPeer;
            let receiver = ptr.offset(*receiver_idx as isize);
            let sender = ptr.offset(*sender_idx as isize);
            Some((receiver.as_mut().unwrap(), sender.as_mut().unwrap()))
        }
    }

    /// Disconnects one peer at random.
    pub fn disconnect_random(&self) -> bool {
        let id = {
            let mut inner = self.0.borrow_mut();
            let keys: Vec<_> = inner.online.keys().cloned().collect();
            let rng = &mut inner.rng;
            keys.choose(rng).cloned()
        };
        if let Some(id) = id {
            self.disconnect(id);
            true
        } else {
            false
        }
    }

    /// Reconnects one previously disconnected peer at random.
    pub fn reconnect_random(&self) -> bool {
        let mut inner = self.0.borrow_mut();
        let reconnectable: Vec<_> = inner
            .all
            .keys()
            .filter(|&id| !inner.online.contains_key(id))
            .cloned()
            .collect();
        if let Some(&id) = reconnectable.choose(&mut inner.rng) {
            Self::connect_inner(&mut inner, id);
            true
        } else {
            false
        }
    }

    fn read_sync_message<D: Decoder, E: Encoder>(
        peer: &TestPeer,
        decoder: &mut D,
        encoder: &mut E,
    ) -> usize {
        let msg_type = decoder.read_var().unwrap();
        match msg_type {
            MSG_SYNC_STEP_1 => Self::read_sync_step1(peer, decoder, encoder),
            MSG_SYNC_STEP_2 => Self::read_sync_step2(peer, decoder),
            MSG_SYNC_UPDATE => Self::read_update(peer, decoder),
            other => panic!(
                "Unknown message type: {} to {}",
                other,
                peer.doc().client_id()
            ),
        }
        msg_type
    }

    fn read_sync_step1<D: Decoder, E: Encoder>(peer: &TestPeer, decoder: &mut D, encoder: &mut E) {
        Self::write_step2(peer, decoder.read_buf().unwrap(), encoder)
    }

    fn read_sync_step2<D: Decoder>(peer: &TestPeer, decoder: &mut D) {
        let mut txn = peer.doc.transact_mut();

        let update = Update::decode_v1(decoder.read_buf().unwrap()).unwrap();
        txn.apply_update(update);
    }

    fn read_update<D: Decoder>(peer: &TestPeer, decoder: &mut D) {
        Self::read_sync_step2(peer, decoder)
    }

    /// Create a sync step 1 message based on the state of the current shared document.
    fn write_step1<E: Encoder>(peer: &TestPeer, encoder: &mut E) {
        let txn = peer.doc.transact_mut();

        encoder.write_var(MSG_SYNC_STEP_1);
        encoder.write_buf(txn.state_vector().encode_v1());
    }

    fn write_step2<E: Encoder>(peer: &TestPeer, sv: &[u8], encoder: &mut E) {
        let txn = peer.doc.transact_mut();
        let remote_sv = StateVector::decode_v1(sv).unwrap();

        encoder.write_var(MSG_SYNC_STEP_2);
        encoder.write_buf(txn.encode_diff_v1(&remote_sv));
    }

    pub fn assert_final_state(mut self) {
        self.reconnect_all();
        while self.flush_all() { /* do nothing */ }
        // For each document, merge all received document updates with Y.mergeUpdates
        // and create a new document which will be added to the list of "users"
        // This ensures that mergeUpdates works correctly
        /*
        const mergedDocs = users.map(user => {
          const ydoc = new Y.Doc()
          enc.applyUpdate(ydoc, enc.mergeUpdates(user.updates))
          return ydoc
        })
        users.push(.../** @type {any} */(mergedDocs))
        */
        let inner = self.0.borrow();
        for i in 0..(inner.peers.len() - 1) {
            let a = inner.peers[i].doc.transact_mut();
            let b = inner.peers[i + 1].doc.transact_mut();

            let astore = a.store();
            let bstore = b.store();
            assert_eq!(astore.blocks, bstore.blocks);
            assert_eq!(astore.pending, bstore.pending);
            assert_eq!(astore.pending_ds, bstore.pending_ds);
        }
    }

    pub fn peers(&self) -> Peers {
        let inner = unsafe { self.0.as_ptr().as_ref().unwrap() };
        let iter = inner.peers.iter();
        Peers(iter)
    }
}

pub struct Peers<'a>(std::slice::Iter<'a, TestPeer>);

impl<'a> Iterator for Peers<'a> {
    type Item = &'a TestPeer;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

pub struct TestPeer {
    doc: Doc,
    receiving: HashMap<ClientID, VecDeque<Vec<u8>>>,
    updates: VecDeque<Vec<u8>>,
}

impl TestPeer {
    pub fn new(client_id: ClientID) -> Self {
        TestPeer {
            doc: Doc::with_client_id(client_id),
            receiving: HashMap::new(),
            updates: VecDeque::new(),
        }
    }

    pub fn client_id(&self) -> ClientID {
        self.doc.client_id()
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    /// Receive a message from another client. This message is only appended to the list of
    /// receiving messages. TestConnector decides when this client actually reads this message.
    fn receive(&mut self, from: ClientID, message: Vec<u8>) {
        let messages = self.receiving.entry(from).or_default();
        messages.push_back(message);
    }
}

pub(crate) trait RngExt: RngCore {
    fn between(&mut self, x: u32, y: u32) -> u32 {
        let a = x.min(y);
        let b = x.max(y);
        if a == b {
            a
        } else {
            self.gen_range(a, b)
        }
    }

    fn random_string(&mut self) -> String {
        let len = self.gen_range(1, 10);
        self.sample_iter(&Alphanumeric)
            .take(len)
            .map(char::from)
            .collect()
    }
}

impl<T> RngExt for T where T: RngCore {}
