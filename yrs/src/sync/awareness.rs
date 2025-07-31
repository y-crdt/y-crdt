use dashmap::{DashMap, Entry};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Formatter;
use std::sync::Arc;
use thiserror::Error;

use crate::block::ClientID;
use crate::sync::{Clock, Timestamp};
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::{Doc, Observer, Origin};

const NULL_STR: &str = "null";

#[cfg(feature = "sync")]
type AwarenessUpdateFn = Box<dyn Fn(&Awareness, &Event, Option<&Origin>) + Send + Sync + 'static>;

#[cfg(not(feature = "sync"))]
type AwarenessUpdateFn = Box<dyn Fn(&Awareness, &Event, Option<&Origin>) + 'static>;

/// The Awareness class implements a simple shared state protocol that can be used for non-persistent
/// data like awareness information (cursor, username, status, ..). Each client can update its own
/// local state and listen to state changes of remote clients.
///
/// Each client is identified by a unique client id (something we borrow from `doc.clientID`).
/// A client can override its own state by propagating a message with an increasing timestamp
/// (`clock`). If such a message is received, it is applied if the known state of that client is
/// older than the new state (`clock < new_clock`). If a client thinks that a remote client is
/// offline, it may propagate a message with `{ clock, state: null, client }`. If such a message is
/// received, and the known clock of that client equals the received clock, it will clean the state.
///
/// Before a client disconnects, it should propagate a `null` state with an updated clock.
pub struct Awareness {
    doc: Doc,
    states: DashMap<ClientID, ClientState>,
    clock: Arc<dyn Clock>,
    on_update: Observer<AwarenessUpdateFn>,
    on_change: Observer<AwarenessUpdateFn>,
}

impl Awareness {
    /// Creates a new instance of [Awareness] struct, which operates over a given document.
    /// Awareness instance has full ownership of that document. If necessary it can be accessed
    /// using either [Awareness::doc] or [Awareness::doc_mut] methods.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    pub fn new(doc: Doc) -> Self {
        Self::with_clock(doc, crate::sync::time::SystemClock)
    }

    /// Creates a new instance of [Awareness] struct, which operates over a given document.
    /// Awareness instance has full ownership of that document. If necessary it can be accessed
    /// using either [Awareness::doc] or [Awareness::doc_mut] methods.
    pub fn with_clock<C>(doc: Doc, clock: C) -> Self
    where
        C: Clock + 'static,
    {
        Awareness {
            doc,
            states: DashMap::new(),
            clock: Arc::new(clock),
            on_update: Observer::new(),
            on_change: Observer::new(),
        }
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(feature = "sync")]
    pub fn on_update<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&Awareness, &Event, Option<&Origin>) + Send + Sync + 'static,
    {
        self.on_update.subscribe(Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(not(feature = "sync"))]
    pub fn on_update<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&Awareness, &Event, Option<&Origin>) + 'static,
    {
        self.on_update.subscribe(Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(feature = "sync")]
    pub fn on_update_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&Awareness, &Event, Option<&Origin>) + Send + Sync + 'static,
    {
        self.on_update.subscribe_with(key.into(), Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(not(feature = "sync"))]
    pub fn on_update_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&Awareness, &Event, Option<&Origin>) + 'static,
    {
        self.on_update.subscribe_with(key.into(), Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    pub fn unobserve_update<K>(&self, key: K) -> bool
    where
        K: Into<Origin>,
    {
        self.on_update.unsubscribe(&key.into())
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(feature = "sync")]
    pub fn on_change<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&Awareness, &Event, Option<&Origin>) + Send + Sync + 'static,
    {
        self.on_change.subscribe(Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(not(feature = "sync"))]
    pub fn on_change<F>(&self, f: F) -> crate::Subscription
    where
        F: Fn(&Awareness, &Event, Option<&Origin>) + 'static,
    {
        self.on_change.subscribe(Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(feature = "sync")]
    pub fn on_change_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&Awareness, &Event, Option<&Origin>) + Send + Sync + 'static,
    {
        self.on_change.subscribe_with(key.into(), Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    #[cfg(not(feature = "sync"))]
    pub fn on_change_with<K, F>(&self, key: K, f: F)
    where
        K: Into<Origin>,
        F: Fn(&Awareness, &Event, Option<&Origin>) + 'static,
    {
        self.on_change.subscribe_with(key.into(), Box::new(f))
    }

    /// Returns a channel receiver for an incoming awareness events. This channel can be cloned.
    pub fn unobserve_change<K>(&self, key: K) -> bool
    where
        K: Into<Origin>,
    {
        self.on_change.unsubscribe(&key.into())
    }

    /// Returns a read-only reference to an underlying [Doc].
    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// Returns a read-write reference to an underlying [Doc].
    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    /// Returns a globally unique client ID of an underlying [Doc].
    pub fn client_id(&self) -> ClientID {
        self.doc.client_id()
    }

    /// Returns a state map of all the clients tracked by current [Awareness] instance. Those
    /// states are identified by their corresponding [ClientID]s. The associated state is
    /// represented and replicated to other clients as a JSON string.
    pub fn iter(&self) -> AwarenessIter {
        AwarenessIter(self.states.iter())
    }

    /// Returns a JSON string state representation of a current [Awareness] instance.
    pub fn local_state<S: DeserializeOwned>(&self) -> Option<S> {
        let json_str = self.local_state_raw()?;
        serde_json::from_str(&json_str).ok()
    }

    /// Returns a JSON string state representation of a current [Awareness] instance.
    pub fn local_state_raw(&self) -> Option<Arc<str>> {
        let e = self.states.get(&self.doc.client_id())?;
        let json_str = e.value().clone().data?;
        Some(json_str)
    }

    /// Clears out a state of a given client, effectively marking it as disconnected.
    pub fn remove_state(&self, client_id: ClientID) {
        let is_removed = match self.states.entry(client_id) {
            Entry::Occupied(mut e) => {
                let state = e.get_mut();
                state.data = None;
                state.clock += 1;
                true
            }
            Entry::Vacant(e) => {
                e.insert(ClientState::new(1, self.clock.now(), None));
                false
            }
        };
        if is_removed && self.on_update.has_subscribers() || self.on_change.has_subscribers() {
            let e = Event::new(Vec::default(), Vec::default(), vec![client_id]);
            self.on_change.trigger(|fun| fun(self, &e, None));
            self.on_update.trigger(|fun| fun(self, &e, None));
        }
    }

    /// Gets the state of a particular client.
    pub fn state<D: DeserializeOwned>(&self, client_id: ClientID) -> Option<D> {
        let state = self.states.get(&client_id)?;
        let json = state.data.clone()?;
        serde_json::from_str(&json).ok()
    }

    /// Get the `(sequence number, last updated timestamp)` metadata associated with particular client.
    pub fn meta(&self, client_id: ClientID) -> Option<(u32, Timestamp)> {
        let state = self.states.get(&client_id)?;
        Some((state.clock, state.last_updated))
    }

    /// Clears out a state of a current client (see: [Awareness::client_id]),
    /// effectively marking it as disconnected.
    pub fn clean_local_state(&self) {
        let client_id = self.doc.client_id();
        self.remove_state(client_id);
    }

    /// Sets a current [Awareness] instance state to a corresponding JSON string. This state will
    /// be replicated to other clients as part of the [AwarenessUpdate] and it will trigger an event
    /// to be emitted if current instance was created using [Awareness::with_observer] method.
    pub fn set_local_state<S: Serialize>(&self, state: S) -> Result<(), Error> {
        let json = serde_json::to_string(&state)?;
        self.set_local_state_raw(json);
        Ok(())
    }

    /// Sets a current [Awareness] instance state to a corresponding JSON string. This state will
    /// be replicated to other clients as part of the [AwarenessUpdate] and it will trigger an event
    /// to be emitted if current instance was created using [Awareness::with_observer] method.
    pub fn set_local_state_raw<S: Into<Arc<str>>>(&self, json: S) {
        let client_id = self.doc.client_id();
        let now = self.clock.now();
        let json = json.into();
        let prev = match self.states.entry(client_id) {
            Entry::Occupied(mut e) => {
                let state = e.get_mut();
                state.last_updated = now;
                state.clock += 1;
                state.data.replace(json.clone())
            }
            Entry::Vacant(e) => {
                e.insert(ClientState::new(1, now, Some(json.clone())));
                None
            }
        };
        self.notify_subscribers(client_id, prev, json);
    }

    fn notify_subscribers(
        &self,
        client_id: ClientID,
        prev_state: Option<Arc<str>>,
        curr_state: Arc<str>,
    ) {
        if self.on_update.has_subscribers() || self.on_change.has_subscribers() {
            let mut added = vec![];
            let mut updated = vec![];
            let mut changed = vec![];
            match prev_state {
                None => added.push(client_id),
                Some(prev) => {
                    updated.push(client_id);
                    if prev != curr_state {
                        changed.push(client_id);
                    }
                }
            }
            let mut e = Event::new(added, changed, Vec::default());
            if !e.is_empty() {
                self.on_change.trigger(|fun| fun(self, &e, None));
            }
            e.summary.updated = updated;
            if !e.is_empty() {
                self.on_update.trigger(|fun| fun(self, &e, None));
            }
        }
    }

    /// Returns a serializable update object which is representation of a current Awareness state.
    /// This doesn't include states that have been removed in the past.
    pub fn update(&self) -> Result<AwarenessUpdate, Error> {
        let clients: Vec<_> = self
            .states
            .iter()
            .flat_map(|e| {
                if e.data.is_none() {
                    None
                } else {
                    Some(*e.key())
                }
            })
            .collect();
        self.update_with_clients(clients)
    }

    /// Returns a serializable update object which is representation of a current Awareness state.
    /// Unlike [Awareness::update], this method variant allows to prepare update only for a subset
    /// of known clients. These clients must all be known to a current [Awareness] instance,
    /// otherwise a [Error::ClientNotFound] error will be returned.
    pub fn update_with_clients<I: IntoIterator<Item = ClientID>>(
        &self,
        clients: I,
    ) -> Result<AwarenessUpdate, Error> {
        let mut res = HashMap::new();
        for client_id in clients {
            let (clock, json) = if let Some(meta) = self.states.get(&client_id) {
                let data = meta.data.clone().unwrap_or_else(|| NULL_STR.into());
                (meta.clock, data)
            } else {
                return Err(Error::ClientNotFound(client_id));
            };
            res.insert(client_id, AwarenessUpdateEntry { clock, json });
        }
        Ok(AwarenessUpdate { clients: res })
    }

    /// Applies an update (incoming from remote channel or generated using [Awareness::update] /
    /// [Awareness::update_with_clients] methods) and modifies a state of a current instance.
    ///
    /// If current instance has an observer channel (see: [Awareness::with_observer]), applied
    /// changes will also be emitted as events.
    pub fn apply_update(&self, update: AwarenessUpdate) -> Result<(), Error> {
        let gen_summary = self.on_update.has_subscribers() || self.on_change.has_subscribers();
        self.apply_update_internal(update, None, gen_summary)?;
        Ok(())
    }

    /// Applies an update (incoming from remote channel or generated using [Awareness::update] /
    /// [Awareness::update_with_clients] methods) and modifies a state of a current instance.
    ///
    /// If current instance has an observer channel (see: [Awareness::with_observer]), applied
    /// changes will also be emitted as events.
    pub fn apply_update_with<O: Into<Origin>>(
        &self,
        update: AwarenessUpdate,
        origin: O,
    ) -> Result<(), Error> {
        let gen_summary = self.on_update.has_subscribers() || self.on_change.has_subscribers();
        self.apply_update_internal(update, Some(origin.into()), gen_summary)?;
        Ok(())
    }

    /// Applies an update (incoming from remote channel or generated using [Awareness::update] /
    /// [Awareness::update_with_clients] methods) and modifies a state of a current instance.
    /// Returns an [AwarenessUpdateSummary] object informing about the changes that were applied.
    ///
    /// If current instance has an observer channel (see: [Awareness::with_observer]), applied
    /// changes will also be emitted as events.
    pub fn apply_update_summary(
        &self,
        update: AwarenessUpdate,
    ) -> Result<Option<AwarenessUpdateSummary>, Error> {
        self.apply_update_internal(update, None, true)
    }

    /// Applies an update (incoming from remote channel or generated using [Awareness::update] /
    /// [Awareness::update_with_clients] methods) and modifies a state of a current instance.
    /// Returns an [AwarenessUpdateSummary] object informing about the changes that were applied.
    ///
    /// If current instance has an observer channel (see: [Awareness::with_observer]), applied
    /// changes will also be emitted as events.
    pub fn apply_update_summary_with<O: Into<Origin>>(
        &self,
        update: AwarenessUpdate,
        origin: O,
    ) -> Result<Option<AwarenessUpdateSummary>, Error> {
        self.apply_update_internal(update, Some(origin.into()), true)
    }

    fn apply_update_internal(
        &self,
        update: AwarenessUpdate,
        origin: Option<Origin>,
        generate_summary: bool,
    ) -> Result<Option<AwarenessUpdateSummary>, Error> {
        let now = self.clock.now();

        let mut added = Vec::new();
        let mut updated = Vec::new();
        let mut changed = Vec::new();
        let mut removed = Vec::new();

        for (client_id, entry) in update.clients {
            let mut clock = entry.clock;
            let new: Option<_> = if entry.json.as_ref() == NULL_STR {
                None
            } else {
                Some(entry.json)
            };
            match self.states.entry(client_id) {
                Entry::Occupied(mut e) => {
                    let state: &mut ClientState = e.get_mut();
                    let is_removed = state.clock == clock && new.is_none() && state.data.is_some();
                    if state.clock < clock || is_removed {
                        match new {
                            None => {
                                // never let a remote client remove this local state
                                if client_id == self.doc.client_id() && state.data.is_some() {
                                    // remote client removed the local state. Do not remove state. Broadcast a message indicating
                                    // that this client still exists by increasing the clock
                                    clock += 1;
                                } else {
                                    state.data = None;
                                    if generate_summary {
                                        removed.push(client_id);
                                    }
                                }
                            }
                            new => {
                                if generate_summary {
                                    updated.push(client_id);
                                    if state.data.is_some() && state.data != new {
                                        changed.push(client_id);
                                    }
                                }
                                state.data = new;
                            }
                        }
                        state.clock = clock;
                        state.last_updated = now;
                        true
                    } else {
                        false
                    }
                }
                Entry::Vacant(e) => {
                    let has_data = new.is_some();
                    e.insert(ClientState::new(clock, now, new));
                    if has_data {
                        if generate_summary {
                            added.push(client_id);
                        }
                        true
                    } else {
                        false
                    }
                }
            };
        }

        if !added.is_empty() || !updated.is_empty() || !removed.is_empty() {
            let summary = if self.on_update.has_subscribers() || self.on_change.has_subscribers() {
                let mut e = Event::new(added, changed, removed);
                if !e.is_empty() {
                    self.on_change.trigger(|fun| fun(self, &e, origin.as_ref()));
                }
                e.summary.updated = updated;
                self.on_update.trigger(|fun| fun(self, &e, origin.as_ref()));
                e.summary
            } else {
                AwarenessUpdateSummary {
                    added,
                    updated,
                    removed,
                }
            };
            Ok(Some(summary))
        } else {
            Ok(None)
        }
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl Default for Awareness {
    fn default() -> Self {
        Awareness::new(Doc::new())
    }
}

impl std::fmt::Debug for Awareness {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Awareness");
        s.field("doc", &self.doc);
        s.field("states", &self.states);
        s.finish()
    }
}

pub struct AwarenessIter<'a>(dashmap::iter::Iter<'a, ClientID, ClientState>);

impl<'a> Iterator for AwarenessIter<'a> {
    type Item = (ClientID, ClientState);

    fn next(&mut self) -> Option<Self::Item> {
        let e = self.0.next()?;
        Some((*e.key(), e.value().clone()))
    }
}

/// Summary of applying an [AwarenessUpdate] over [Awareness::apply_update_summary] method.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AwarenessUpdateSummary {
    /// New clients added as part of the update.
    pub added: Vec<ClientID>,
    /// Existing clients that have been changed by the update.
    pub updated: Vec<ClientID>,
    /// Existing clients that have been removed by the update.
    pub removed: Vec<ClientID>,
}

impl AwarenessUpdateSummary {
    /// Returns a collection of all clients that have been changed by the [AwarenessUpdate].
    pub fn all_changes(&self) -> Vec<ClientID> {
        let mut res =
            Vec::with_capacity(self.added.len() + self.updated.len() + self.removed.len());
        res.extend_from_slice(&self.added);
        res.extend_from_slice(&self.updated);
        res.extend_from_slice(&self.removed);
        res
    }
}

/// A structure that represents an encodable state of an [Awareness] struct.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AwarenessUpdate {
    /// Client updates.
    pub clients: HashMap<ClientID, AwarenessUpdateEntry>,
}

impl Encode for AwarenessUpdate {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.clients.len());
        for (&client_id, e) in self.clients.iter() {
            encoder.write_var(client_id);
            encoder.write_var(e.clock);
            encoder.write_string(&e.json);
        }
    }
}

impl Decode for AwarenessUpdate {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, crate::encoding::read::Error> {
        let len: usize = decoder.read_var()?;
        let mut clients = HashMap::with_capacity(len);
        for _ in 0..len {
            let client_id: ClientID = decoder.read_var()?;
            let clock: u32 = decoder.read_var()?;
            let json: Arc<str> = decoder.read_string()?.into();
            clients.insert(client_id, AwarenessUpdateEntry { clock, json });
        }

        Ok(AwarenessUpdate { clients })
    }
}

/// A single client entry of an [AwarenessUpdate]. It consists of logical clock and JSON client
/// state represented as a string.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AwarenessUpdateEntry {
    /// Timestamp used to recognize which update is the latest one.
    pub clock: u32,
    /// String with JSON payload containing user data.
    pub json: Arc<str>,
}

/// Errors generated by an [Awareness] struct methods.
#[derive(Error, Debug)]
pub enum Error {
    /// Client ID was not found in [Awareness] metadata.
    #[error("client ID `{0}` not found")]
    ClientNotFound(ClientID),
    #[error("couldn't serialize awareness state: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientState {
    pub clock: u32,
    pub last_updated: Timestamp,
    pub data: Option<Arc<str>>,
}

impl ClientState {
    fn new(clock: u32, last_updated: Timestamp, state: Option<Arc<str>>) -> Self {
        ClientState {
            clock,
            last_updated,
            data: state,
        }
    }
}

/// Event type emitted by an [Awareness] struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    summary: AwarenessUpdateSummary,
}

impl Event {
    pub fn new(added: Vec<ClientID>, updated: Vec<ClientID>, removed: Vec<ClientID>) -> Self {
        Event {
            summary: AwarenessUpdateSummary {
                added,
                updated,
                removed,
            },
        }
    }

    pub fn summary(&self) -> &AwarenessUpdateSummary {
        &self.summary
    }

    /// Collection of new clients that have been added to an [Awareness] struct, that was not known
    /// before. Actual client state can be accessed via `awareness.clients().get(client_id)`.
    pub fn added(&self) -> &[ClientID] {
        &self.summary.added
    }

    /// Collection of new clients that have been updated within an [Awareness] struct since the last
    /// update. Actual client state can be accessed via `awareness.clients().get(client_id)`.
    pub fn updated(&self) -> &[ClientID] {
        &self.summary.updated
    }

    /// Collection of new clients that have been removed from [Awareness] struct since the last
    /// update.
    pub fn removed(&self) -> &[ClientID] {
        &self.summary.removed
    }

    /// Returns all the changed client IDs (added, updated and removed) combined.
    pub fn all_changes(&self) -> Vec<ClientID> {
        self.summary.all_changes()
    }

    fn is_empty(&self) -> bool {
        self.summary.added.is_empty()
            && self.summary.updated.is_empty()
            && self.summary.removed.is_empty()
    }
}

#[cfg(test)]
mod test {
    use arc_swap::ArcSwapOption;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::sync::awareness::{AwarenessUpdateSummary, Event};
    use crate::sync::Awareness;
    use crate::Doc;

    #[test]
    fn awareness() {
        fn exchange(recv: &ArcSwapOption<Event>, from: &Awareness, to: &mut Awareness) {
            let e = recv.swap(None);
            if let Some(e) = e.as_deref() {
                let total = [e.added(), e.updated(), e.removed()].concat();
                let u = from.update_with_clients(total).unwrap();
                to.apply_update(u).unwrap();
            }
        }

        let local = Awareness::new(Doc::with_client_id(1));
        let last_change_local = Arc::new(ArcSwapOption::default());
        let update = Arc::new(ArcSwapOption::default());
        let _sub_update = {
            let update = update.clone();
            local.on_update(move |_, e, _| update.store(Some(Arc::new(e.clone()))))
        };
        let _sub_local = {
            let last_change_local = last_change_local.clone();
            local.on_change(move |_, e, _| last_change_local.store(Some(Arc::new(e.clone()))))
        };

        let mut remote = Awareness::new(Doc::with_client_id(2));
        let last_change_remote = Arc::new(ArcSwapOption::default());
        let _sub_remote = {
            let last_change_remote = last_change_remote.clone();
            remote.on_change(move |_, e, _| last_change_remote.store(Some(Arc::new(e.clone()))))
        };

        assert!(local.on_change.has_subscribers(), "local has subscribers");
        assert!(remote.on_change.has_subscribers(), "remote has subscribers");
        local.set_local_state(json!({"x":3})).unwrap();
        exchange(&update, &local, &mut remote);
        let _e_local = last_change_local.swap(None).unwrap();
        assert_eq!(remote.state::<Value>(1).unwrap(), json!({"x":3}));
        assert_eq!(remote.states.get(&1).unwrap().clock, 1);
        assert_eq!(last_change_remote.swap(None).unwrap().added(), &[1]);

        local.set_local_state(json!({"x":4})).unwrap();
        exchange(&update, &local, &mut remote);
        let e_local = last_change_local.swap(None).unwrap();
        let e_remote = last_change_remote.swap(None).unwrap();
        assert_eq!(remote.state::<Value>(1).unwrap(), json!({"x":4}));
        assert_eq!(
            e_remote.summary,
            AwarenessUpdateSummary {
                added: vec![],
                updated: vec![1],
                removed: vec![]
            }
        );
        assert_eq!(e_remote.summary, e_local.summary);

        local.set_local_state(json!({"x":4})).unwrap();
        exchange(&update, &local, &mut remote);
        let e_local = last_change_local.swap(None);
        let e_remote = last_change_remote.swap(None);
        assert_eq!(remote.state::<Value>(1).unwrap(), json!({"x":4}));
        assert_eq!(remote.states.get(&1).unwrap().clock, 3);
        assert_eq!(e_remote, e_local);

        local.clean_local_state();
        exchange(&update, &local, &mut remote);
        let e_local = last_change_local.swap(None).unwrap();
        let e_remote = last_change_remote.swap(None).unwrap();
        assert_eq!(e_remote.removed().len(), 1);
        assert!(local.state::<Value>(1).is_none());
        assert_eq!(e_remote.summary, e_local.summary);
        assert_eq!(e_remote, e_local);
    }

    #[test]
    fn awareness_summary() -> Result<(), Box<dyn std::error::Error>> {
        let local = Awareness::new(Doc::with_client_id(1));
        let remote = Awareness::new(Doc::with_client_id(2));

        local.set_local_state(json!({"x":3})).unwrap();
        let update = local.update_with_clients([local.client_id()])?;
        let summary = remote.apply_update_summary(update)?;
        assert_eq!(remote.state::<Value>(1).unwrap(), json!({"x":3}));
        assert_eq!(remote.states.get(&1).unwrap().clock, 1);
        assert_eq!(
            summary,
            Some(AwarenessUpdateSummary {
                added: vec![1],
                updated: vec![],
                removed: vec![]
            })
        );

        local.set_local_state(json!({"x":4})).unwrap();
        let update = local.update_with_clients([local.client_id()])?;
        let summary = remote.apply_update_summary(update)?;
        assert_eq!(remote.state::<Value>(1).unwrap(), json!({"x":4}));
        assert_eq!(
            summary,
            Some(AwarenessUpdateSummary {
                added: vec![],
                updated: vec![1],
                removed: vec![]
            })
        );
        let local_states: HashMap<_, _> = local.iter().collect();
        let remote_states: HashMap<_, _> = remote.iter().collect();
        assert_eq!(local_states, remote_states);

        local.clean_local_state();
        let update = local.update_with_clients([local.client_id()])?;
        let summary = remote.apply_update_summary(update)?;
        assert_eq!(
            summary,
            Some(AwarenessUpdateSummary {
                added: vec![],
                updated: vec![],
                removed: vec![1]
            })
        );
        assert!(local.state::<Value>(1).is_none());
        let local_states: HashMap<_, _> = local.iter().collect();
        let remote_states: HashMap<_, _> = remote.iter().collect();
        assert_eq!(local_states, remote_states);
        Ok(())
    }
}
