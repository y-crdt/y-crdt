use crate::id_set::DeleteSet;
use crate::update::Update;
use crate::updates::decoder::{Decode, Decoder, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use crate::{Doc, StateVector};
use lib0::decoding::{Cursor, Read};
use lib0::encoding::Write;
use rand::prelude::SliceRandom;
use rand::rngs::ThreadRng;
use rand::seq::IteratorRandom;
use rand::{thread_rng, Rng};
use std::collections::{HashMap, VecDeque};

const MSG_SYNC_STEP_1: usize = 0;
const MSG_SYNC_STEP_2: usize = 1;
const MSG_SYNC_UPDATE: usize = 2;

pub struct TestConnector {
    rng: ThreadRng,
    clients: Vec<TestPeer>,
    /// Maps all Client IDs to indexes in the `docs` vector.
    all: HashMap<u64, usize>,
    /// Maps online Client IDs to indexes in the `docs` vector.
    online: HashMap<u64, usize>,
}

impl TestConnector {
    /// Create new [TestConnector] with random seed.
    pub fn new() -> Self {
        Self::with_rng(thread_rng())
    }

    /// Create new [TestConnector] with provided randomizer.
    pub fn with_rng(rng: ThreadRng) -> Self {
        TestConnector {
            rng,
            clients: Vec::new(),
            all: HashMap::new(),
            online: HashMap::new(),
        }
    }

    /// Create a new [TestConnector] with pre-initialized number of peers.
    pub fn with_peer_num(rng: ThreadRng, peer_num: u64) -> Self {
        let mut tc = Self::with_rng(rng);
        for client_id in 0..peer_num {
            let peer = tc.create_peer(client_id);
            let mut txn = peer.doc.transact();
            txn.get_text("text");
        }
        tc.sync_all();
        tc
    }

    /// Returns random number generator attached to current [TestConnector].
    pub fn rng(&mut self) -> &mut ThreadRng {
        &mut self.rng
    }

    /// Create a new [TestPeer] with provided `client_id` or return one, if such `client_id`
    /// was already created before.
    pub fn create_peer(&mut self, client_id: u64) -> &mut TestPeer {
        if !self.all.contains_key(&client_id) {
            let instance = TestPeer::new(client_id);
            let idx = self.clients.len();
            self.clients.push(instance);
            self.all.insert(client_id, idx);
            self.online.insert(client_id, idx);
            &mut self.clients[idx]
        } else {
            self.get_mut(&client_id).unwrap()
        }
    }

    /// Try to retrieve a reference to [TestPeer] for a given `client_id`, if such node was created.
    pub fn get(&self, client_id: &u64) -> Option<&TestPeer> {
        let idx = self.all.get(client_id)?;
        Some(&self.clients[*idx])
    }

    /// Try to retrieve a mutable reference to [TestPeer] for a given `client_id`,
    /// if such node was created.
    pub fn get_mut(&mut self, client_id: &u64) -> Option<&mut TestPeer> {
        let idx = self.all.get(client_id)?;
        Some(&mut self.clients[*idx])
    }

    /// Disconnects test node with given `client_id` from the rest of known nodes.
    pub fn disconnect(&mut self, client_id: u64) {
        if let Some(peer) = self.get_mut(&client_id) {
            peer.receiving.clear();
        }
        self.online.remove(&client_id);
    }

    /// Append `client_id` to the list of known Y instances in [TestConnector].
    /// Also initiate sync with all clients.
    pub fn connect(&mut self, client_id: u64) {
        if !self.online.contains_key(&client_id) {
            let idx = *self.all.get(&client_id).expect("unknown client_id");
            self.online.insert(client_id, idx);
        }
    }

    /// Reconnects back all known peers.
    pub fn reconnect_all(&mut self) {
        let all_ids: Vec<_> = self.all.keys().cloned().collect();
        for id in all_ids {
            self.connect(id);
        }
    }

    /// Disconnects all known peers from each other.
    pub fn disconnect_all(&mut self) {
        let all_ids: Vec<_> = self.all.keys().cloned().collect();
        for id in all_ids {
            self.connect(id);
        }
    }

    /// Reconnects all known peers and processes their pending messages.
    pub fn sync_all(&mut self) {
        self.reconnect_all();
        self.flush_all();
    }

    /// Processes all pending messages of connected peers in random order.
    pub fn flush_all(&mut self) -> bool {
        let mut did_something = false;
        while self.flush_random() {
            did_something = true;
        }
        did_something
    }

    /// Choose random connection and flush a random message from a random sender.
    /// If this function was unable to flush a message, because there are no more messages to flush,
    /// it returns false. true otherwise.
    pub fn flush_random(&mut self) -> bool {
        if let Some((receiver, sender)) = self.pick_random_pair() {
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
                    let msg_type: usize = decoder.read_uvar();
                    if msg_type == MSG_SYNC_STEP_2 || msg_type == MSG_SYNC_UPDATE {
                        receiver.updates.push_back(decoder.read_buf().to_vec())
                    }
                }
                true
            } else {
                receiver.receiving.remove(&sender.client_id());
                self.flush_random()
            }
        } else {
            false
        }
    }

    fn pick_random_pair(&mut self) -> Option<(&mut TestPeer, &mut TestPeer)> {
        let pairs: Vec<_> = self
            .clients
            .iter()
            .enumerate()
            .flat_map(|(receiver_idx, conn)| {
                if conn.receiving.is_empty() {
                    vec![]
                } else {
                    conn.receiving
                        .keys()
                        .map(|id| (receiver_idx, *self.all.get(id).unwrap()))
                        .collect()
                }
            })
            .collect();
        let (receiver_idx, sender_idx) = pairs.choose(&mut self.rng)?;
        unsafe {
            let ptr = self.clients.as_mut_ptr();
            let receiver = ptr.offset(*receiver_idx as isize);
            let sender = ptr.offset(*sender_idx as isize);
            Some((receiver.as_mut().unwrap(), sender.as_mut().unwrap()))
        }
    }

    /// Disconnects one peer at random.
    pub fn disconnect_random(&mut self) -> bool {
        if let Some(id) = self.online.keys().choose(&mut self.rng).cloned() {
            self.disconnect(id);
            true
        } else {
            false
        }
    }

    /// Reconnects one previously disconnected peer at random.
    pub fn reconnect_random(&mut self) -> bool {
        let reconnectable: Vec<_> = self
            .all
            .keys()
            .filter(|&id| !self.online.contains_key(id))
            .cloned()
            .collect();
        if let Some(&id) = reconnectable.choose(&mut self.rng) {
            self.connect(id);
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
        let msg_type = decoder.read_uvar();
        match msg_type {
            MSG_SYNC_STEP_1 => Self::read_sync_step1(peer, decoder, encoder),
            MSG_SYNC_STEP_2 => Self::read_sync_step2(peer, decoder),
            MSG_UPDATE => Self::read_update(peer, decoder),
            other => panic!(
                "Unknown message type: {} to {}",
                other,
                peer.doc().client_id
            ),
        }
        msg_type
    }

    fn read_sync_step1<D: Decoder, E: Encoder>(peer: &TestPeer, decoder: &mut D, encoder: &mut E) {
        Self::write_step2(peer, decoder.read_buf(), encoder)
    }

    fn read_sync_step2<D: Decoder>(peer: &TestPeer, decoder: &mut D) {
        let mut txn = peer.doc.transact();

        peer.doc.apply_update(&mut txn, decoder.read_buf());
    }

    fn read_update<D: Decoder>(peer: &TestPeer, decoder: &mut D) {
        Self::read_sync_step2(peer, decoder)
    }

    /// Create a sync step 1 message based on the state of the current shared document.
    fn write_step1<E: Encoder>(peer: &TestPeer, encoder: &mut E) {
        let txn = peer.doc.transact();

        encoder.write_uvar(MSG_SYNC_STEP_1);
        encoder.write_buf(peer.doc.encode_state_vector(&txn));
    }

    fn write_step2<E: Encoder>(peer: &TestPeer, sv: &[u8], encoder: &mut E) {
        let txn = peer.doc.transact();
        let remote_sv = StateVector::decode_v1(sv);

        encoder.write_uvar(MSG_SYNC_STEP_2);
        encoder.write_buf(peer.doc.encode_delta_as_update(&remote_sv, &txn));
    }
}

pub struct TestPeer {
    doc: Doc,
    receiving: HashMap<u64, VecDeque<Vec<u8>>>,
    updates: VecDeque<Vec<u8>>,
}

impl TestPeer {
    pub fn new(client_id: u64) -> Self {
        TestPeer {
            doc: Doc::with_client_id(client_id),
            receiving: HashMap::new(),
            updates: VecDeque::new(),
        }
    }

    pub fn client_id(&self) -> u64 {
        self.doc.client_id
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    /// Receive a message from another client. This message is only appended to the list of
    /// receiving messages. TestConnector decides when this client actually reads this message.
    fn receive(&mut self, from: u64, message: Vec<u8>) {
        let messages = self.receiving.entry(from).or_default();
        messages.push_back(message);
    }
}
