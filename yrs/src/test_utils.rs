#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use fastrand::Rng;

use crate::block::ClientID;
use crate::encoding::read::{Cursor, Read};
use crate::transaction::ReadTxn;
use crate::updates::decoder::{Decode, Decoder, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use crate::{Doc, StateVector, Transact, Update};

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
                let update = Update::decode_v1(update.as_slice()).unwrap();
                tb.apply_update(update).unwrap();
            }
        }
    }
}

const MSG_SYNC_STEP_1: usize = 0;
const MSG_SYNC_STEP_2: usize = 1;
const MSG_SYNC_UPDATE: usize = 2;

pub fn run_scenario<F>(mut seed: u64, mods: &[F], users: usize, iterations: usize)
where
    F: Fn(&mut Doc, &mut Rng),
{
    if seed == 0 {
        seed = fastrand::get_seed();
        println!("run scenario with seed: {}", seed);
    }

    let rng = Rng::with_seed(seed);
    let tc = TestConnector::with_peer_num(rng, users as u64);
    for _ in 0..iterations {
        if tc.0.lock().unwrap().rng.u32(0..100) <= 2 {
            // 2% chance to disconnect/reconnect a random user
            if tc.0.lock().unwrap().rng.bool() {
                tc.disconnect_random();
            } else {
                tc.reconnect_random();
            }
        } else if tc.0.lock().unwrap().rng.u32(0..100) <= 1 {
            // 1% chance to flush all
            tc.flush_all();
        } else if tc.0.lock().unwrap().rng.u32(0..100) <= 50 {
            tc.flush_random();
        }

        {
            let mut inner = tc.0.lock().unwrap();
            let inner = &mut *inner;
            let rng = &mut inner.rng;
            let idx = rng.usize(0..inner.peers.len());
            let peer = &mut inner.peers[idx];
            let test = rng.choice(mods).unwrap();
            let mut peer_state = peer.state();
            test(&mut peer_state.doc, rng);
        };
    }

    tc.assert_final_state();
}

pub struct TestConnector(Arc<Mutex<Inner>>);

struct Inner {
    rng: Rng,
    peers: Vec<TestPeer>,
    /// Maps all Client IDs to indexes in the `docs` vector.
    all: HashMap<ClientID, usize>,
    /// Maps online Client IDs to indexes in the `docs` vector.
    online: HashMap<ClientID, usize>,
}

impl TestConnector {
    /// Create new [TestConnector] with provided randomizer.
    pub fn with_rng(rng: Rng) -> Self {
        TestConnector(Arc::new(Mutex::new(Inner {
            rng,
            peers: Vec::new(),
            all: HashMap::new(),
            online: HashMap::new(),
        })))
    }

    /// Create a new [TestConnector] with pre-initialized number of peers.
    pub fn with_peer_num(rng: Rng, peer_num: u64) -> Self {
        let mut tc = Self::with_rng(rng);
        for client_id in 0..peer_num {
            let peer = tc.create_peer(client_id as ClientID);
            let peer_state = peer.state();
            peer_state.doc.get_or_insert_text("text");
            peer_state.doc.get_or_insert_map("map");
        }
        tc.sync_all();
        tc
    }

    /// Create a new [TestPeer] with provided `client_id` or return one, if such `client_id`
    /// was already created before.
    pub fn create_peer(&self, client_id: ClientID) -> TestPeer {
        if let Some(peer) = self.get(&client_id) {
            peer
        } else {
            let rc = self.0.clone();
            let instance = TestPeer::new(client_id);
            let _sub = {
                let rc = rc.clone();
                let peer_state = instance.state();
                peer_state
                    .doc
                    .observe_update_v1(move |_, e| {
                        let mut inner = rc.lock().unwrap();
                        Self::broadcast(&mut inner, client_id, &e.update);
                    })
                    .unwrap()
            };
            let mut inner = rc.lock().unwrap();
            let idx = inner.peers.len();
            inner.peers.push(instance);
            inner.all.insert(client_id, idx);
            inner.online.insert(client_id, idx);
            inner.peers[idx].clone()
        }
    }

    fn broadcast(inner: &mut Inner, sender: ClientID, payload: &Vec<u8>) {
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
    pub fn get(&self, client_id: &ClientID) -> Option<TestPeer> {
        let inner = self.0.lock().unwrap();
        let idx = inner.all.get(client_id)?;
        Some(inner.peers[*idx].clone())
    }

    /// Disconnects test node with given `client_id` from the rest of known nodes.
    pub fn disconnect(&self, client_id: ClientID) {
        if let Some(peer) = self.get(&client_id) {
            peer.clear();
        }
        let mut inner = self.0.lock().unwrap();
        inner.online.remove(&client_id);
    }

    /// Append `client_id` to the list of known Y instances in [TestConnector].
    /// Also initiate sync with all clients.
    pub fn connect(&self, client_id: ClientID) {
        let mut inner = self.0.lock().unwrap();
        Self::connect_inner(&mut *inner, client_id);
    }

    fn connect_inner(inner: &mut Inner, client_id: ClientID) {
        if !inner.online.contains_key(&client_id) {
            let idx = *inner.all.get(&client_id).expect("unknown client_id");
            inner.online.insert(client_id, idx);
        }

        let client_idx = *inner.all.get(&client_id).unwrap();
        let payload = {
            let sender = &inner.peers[client_idx];
            let mut sender = sender.state();
            let mut encoder = EncoderV1::new();
            Self::write_step1(&mut *sender, &mut encoder);
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
                let mut peer = peer.state();
                let mut encoder = EncoderV1::new();
                Self::write_step1(&mut *peer, &mut encoder);
                encoder.to_vec()
            };

            let sender = &mut inner.peers[client_idx];
            sender.receive(remote_id, payload);
        }
    }

    /// Reconnects back all known peers.
    pub fn reconnect_all(&mut self) {
        let mut inner = self.0.lock().unwrap();
        let all_ids: Vec<_> = inner.all.keys().cloned().collect();
        for client_id in all_ids {
            Self::connect_inner(&mut inner, client_id);
        }
    }

    /// Disconnects all known peers from each other.
    pub fn disconnect_all(&mut self) {
        let mut inner = self.0.lock().unwrap();
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
        let mut inner = self.0.lock().unwrap();
        Self::flush_random_inner(&mut *inner)
    }

    fn flush_random_inner(inner: &mut Inner) -> bool {
        if let Some((receiver, sender)) = Self::pick_random_pair(inner) {
            if let Some(m) = receiver.try_recv(&sender.client_id()) {
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
                        receiver.send(decoder.read_buf().unwrap().to_vec())
                    }
                }
                true
            } else {
                receiver.remove(&sender.client_id());
                Self::flush_random_inner(inner)
            }
        } else {
            false
        }
    }

    fn pick_random_pair(inner: &mut Inner) -> Option<(&mut TestPeer, &mut TestPeer)> {
        let pairs: Vec<_> = inner
            .peers
            .iter()
            .enumerate()
            .flat_map(|(receiver_idx, conn)| {
                let conn = conn.state();
                if conn.receiving.is_empty() {
                    vec![]
                } else {
                    conn.receiving
                        .iter()
                        .map(|(key, _)| (receiver_idx, *inner.all.get(key).unwrap()))
                        .collect()
                }
            })
            .collect();
        let (receiver_idx, sender_idx) = inner.rng.choice(pairs)?;
        unsafe {
            let ptr = inner.peers.as_ptr() as *mut TestPeer;
            let receiver = ptr.offset(receiver_idx as isize);
            let sender = ptr.offset(sender_idx as isize);
            Some((receiver.as_mut().unwrap(), sender.as_mut().unwrap()))
        }
    }

    /// Disconnects one peer at random.
    pub fn disconnect_random(&self) -> bool {
        let id = {
            let mut inner = self.0.lock().unwrap();
            let keys: Vec<_> = inner.online.keys().cloned().collect();
            let rng = &mut inner.rng;
            rng.choice(keys).clone()
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
        let mut inner = self.0.lock().unwrap();
        let reconnectable: Vec<_> = inner
            .all
            .keys()
            .filter(|&id| !inner.online.contains_key(id))
            .cloned()
            .collect();
        if let Some(id) = inner.rng.choice(reconnectable) {
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
        let mut peer = peer.state();
        match msg_type {
            MSG_SYNC_STEP_1 => Self::read_sync_step1(&mut *peer, decoder, encoder),
            MSG_SYNC_STEP_2 => Self::read_sync_step2(&mut *peer, decoder),
            MSG_SYNC_UPDATE => Self::read_update(&mut *peer, decoder),
            other => panic!(
                "Unknown message type: {} to {}",
                other,
                peer.doc.client_id()
            ),
        }
        msg_type
    }

    fn read_sync_step1<D: Decoder, E: Encoder>(
        peer: &mut TestPeerState,
        decoder: &mut D,
        encoder: &mut E,
    ) {
        Self::write_step2(peer, decoder.read_buf().unwrap(), encoder)
    }

    fn read_sync_step2<D: Decoder>(peer: &mut TestPeerState, decoder: &mut D) {
        let mut txn = peer.doc.transact_mut();

        let update = Update::decode_v1(decoder.read_buf().unwrap()).unwrap();
        txn.apply_update(update).unwrap();
    }

    fn read_update<D: Decoder>(peer: &mut TestPeerState, decoder: &mut D) {
        Self::read_sync_step2(peer, decoder)
    }

    /// Create a sync step 1 message based on the state of the current shared document.
    fn write_step1<E: Encoder>(peer: &mut TestPeerState, encoder: &mut E) {
        let txn = peer.doc.transact_mut();

        encoder.write_var(MSG_SYNC_STEP_1);
        encoder.write_buf(txn.state_vector().encode_v1());
    }

    fn write_step2<E: Encoder>(peer: &mut TestPeerState, sv: &[u8], encoder: &mut E) {
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
        let inner = self.0.lock().unwrap();
        for i in 0..(inner.peers.len() - 1) {
            let p1 = inner.peers[i].state();
            let p2 = inner.peers[i + 1].state();
            let a = p1.doc.transact_mut();
            let b = p2.doc.transact_mut();

            let astore = a.store();
            let bstore = b.store();
            assert_eq!(astore.blocks, bstore.blocks);
            assert_eq!(astore.pending, bstore.pending);
            assert_eq!(astore.pending_ds, bstore.pending_ds);
        }
    }

    pub fn peers(&self) -> Peers {
        let inner = self.0.lock().unwrap();
        Peers::new(inner)
    }
}

pub struct Peers<'a> {
    inner: MutexGuard<'a, Inner>,
    i: usize,
}

impl<'a> Peers<'a> {
    fn new(inner: MutexGuard<'a, Inner>) -> Self {
        Self { inner, i: 0 }
    }
}

impl<'a> Iterator for Peers<'a> {
    type Item = TestPeer;

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.inner.peers.len() {
            None
        } else {
            let peer = self.inner.peers[self.i].clone();
            self.i += 1;
            Some(peer)
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct TestPeer {
    state: Arc<Mutex<TestPeerState>>,
}

#[derive(Debug)]
struct TestPeerState {
    doc: Doc,
    receiving: HashMap<ClientID, VecDeque<Vec<u8>>>,
    updates: VecDeque<Vec<u8>>,
}

impl TestPeer {
    pub fn new(client_id: ClientID) -> Self {
        TestPeer {
            state: Arc::new(Mutex::new(TestPeerState {
                doc: Doc::with_client_id(client_id),
                receiving: HashMap::new(),
                updates: VecDeque::new(),
            })),
        }
    }

    fn state(&self) -> MutexGuard<TestPeerState> {
        self.state.lock().unwrap()
    }

    fn remove(&self, client: &ClientID) {
        let mut state = self.state();
        state.receiving.remove(client);
    }

    fn send(&self, data: Vec<u8>) {
        let mut state = self.state();
        state.updates.push_back(data);
    }

    fn try_recv(&self, sender: &ClientID) -> Option<Vec<u8>> {
        let mut state = self.state();
        let client = state.receiving.get_mut(sender)?;
        client.pop_front()
    }

    pub fn client_id(&self) -> ClientID {
        self.state().doc.client_id()
    }

    fn clear(&self) {
        let mut state = self.state();
        state.receiving.clear();
    }

    /// Receive a message from another client. This message is only appended to the list of
    /// receiving messages. TestConnector decides when this client actually reads this message.
    fn receive(&self, from: ClientID, message: Vec<u8>) {
        let mut state = self.state();
        let messages = state.receiving.entry(from).or_default();
        messages.push_back(message);
    }
}

pub(crate) trait RngExt {
    fn between(&mut self, x: u32, y: u32) -> u32;

    fn random_string(&mut self) -> String;
}

impl RngExt for Rng {
    fn between(&mut self, x: u32, y: u32) -> u32 {
        let a = x.min(y);
        let b = x.max(y);
        if a == b {
            a
        } else {
            self.u32(a..b)
        }
    }

    fn random_string(&mut self) -> String {
        let mut res = String::new();
        for _ in 0..self.usize(1..10) {
            res.push(self.alphanumeric());
        }
        res
    }
}
