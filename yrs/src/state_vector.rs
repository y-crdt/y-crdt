use crate::block::ClientID;
use crate::encoding::read::Error;
use crate::updates::decoder::{Decode, Decoder};
use crate::updates::encoder::{Encode, Encoder};
use crate::utils::client_hasher::ClientHasher;
use crate::{DeleteSet, ID};
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::iter::FromIterator;

/// State vector is a compact representation of all known blocks inserted and integrated into
/// a given document. This descriptor can be serialized and used to determine a difference between
/// seen and unseen inserts of two replicas of the same document, potentially existing in different
/// processes.
///
/// Another popular name for the concept represented by state vector is
/// [Version Vector](https://en.wikipedia.org/wiki/Version_vector).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct StateVector(HashMap<ClientID, u32, BuildHasherDefault<ClientHasher>>);

impl StateVector {
    /// Checks if current state vector contains any data.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns a number of unique clients observed by a document, current state vector corresponds
    /// to.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn new(map: HashMap<ClientID, u32, BuildHasherDefault<ClientHasher>>) -> Self {
        StateVector(map)
    }

    /// Checks if current state vector includes given block identifier. Blocks, which identifiers
    /// can be found in a state vectors don't need to be encoded as part of an update, because they
    /// were already observed by their remote peer, current state vector refers to.
    pub fn contains(&self, id: &ID) -> bool {
        id.clock <= self.get(&id.client)
    }

    pub fn contains_client(&self, client_id: &ClientID) -> bool {
        self.0.contains_key(client_id)
    }

    /// Get the latest clock sequence number value for a given `client_id` as observed from
    /// the perspective of a current state vector.
    pub fn get(&self, client_id: &ClientID) -> u32 {
        match self.0.get(client_id) {
            Some(state) => *state,
            None => 0,
        }
    }

    /// Updates a state vector observed clock sequence number for a given `client` by incrementing
    /// it by a given `delta`.
    pub fn inc_by(&mut self, client: ClientID, delta: u32) {
        if delta > 0 {
            let e = self.0.entry(client).or_default();
            *e = *e + delta;
        }
    }

    /// Updates a state vector observed clock sequence number for a given `client` by setting it to
    /// a minimum value between an already present one and the provided `clock`. In case if state
    /// vector didn't contain any value for that `client`, a `clock` value will be used.
    pub fn set_min(&mut self, client: ClientID, clock: u32) {
        match self.0.entry(client) {
            Entry::Occupied(e) => {
                let value = e.into_mut();
                *value = (*value).min(clock);
            }
            Entry::Vacant(e) => {
                e.insert(clock);
            }
        }
    }

    /// Updates a state vector observed clock sequence number for a given `client` by setting it to
    /// a maximum value between an already present one and the provided `clock`. In case if state
    /// vector didn't contain any value for that `client`, a `clock` value will be used.
    pub fn set_max(&mut self, client: ClientID, clock: u32) {
        let e = self.0.entry(client).or_default();
        *e = (*e).max(clock);
    }

    /// Returns an iterator which enables to traverse over all clients and their known clock values
    /// described by a current state vector.
    pub fn iter(&self) -> std::collections::hash_map::Iter<ClientID, u32> {
        self.0.iter()
    }

    /// Merges another state vector into a current one. Since vector's clock values can only be
    /// incremented, whenever a conflict between two states happen (both state vectors have
    /// different clock values for the same client entry), a highest of these to is considered to
    /// be the most up-to-date.
    pub fn merge(&mut self, other: Self) {
        for (client, clock) in other.0 {
            let e = self.0.entry(client).or_default();
            *e = (*e).max(clock);
        }
    }
}

impl FromIterator<(ClientID, u32)> for StateVector {
    fn from_iter<T: IntoIterator<Item = (ClientID, u32)>>(iter: T) -> Self {
        StateVector::new(
            HashMap::<ClientID, u32, BuildHasherDefault<ClientHasher>>::from_iter(iter),
        )
    }
}

impl Decode for StateVector {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let len = decoder.read_var::<u32>()? as usize;
        let mut sv = HashMap::with_capacity_and_hasher(len, BuildHasherDefault::default());
        let mut i = 0;
        while i < len {
            let client = decoder.read_var()?;
            let clock = decoder.read_var()?;
            sv.insert(client, clock);
            i += 1;
        }
        Ok(StateVector(sv))
    }
}

impl Encode for StateVector {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        encoder.write_var(self.len());
        for (&client, &clock) in self.iter() {
            encoder.write_var(client);
            encoder.write_var(clock);
        }
    }
}

impl PartialOrd for StateVector {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let mut result = Some(Ordering::Equal);

        for (client, clock) in self.iter() {
            let other_clock = other.get(client);
            match clock.cmp(&other_clock) {
                Ordering::Less if result == Some(Ordering::Greater) => return None,
                Ordering::Greater if result == Some(Ordering::Less) => return None,
                Ordering::Equal => { /* unchanged */ }
                other => result = Some(other),
            }
        }

        for (other_client, other_clock) in other.iter() {
            let clock = self.get(other_client);
            match clock.cmp(&other_clock) {
                Ordering::Less if result == Some(Ordering::Greater) => return None,
                Ordering::Greater if result == Some(Ordering::Less) => return None,
                Ordering::Equal => { /* unchanged */ }
                other => result = Some(other),
            }
        }

        result
    }
}

/// Snapshot describes a state of a document store at a given point in (logical) time. In practice
/// it's a combination of [StateVector] (a summary of all observed insert/update operations)
/// and a [DeleteSet] (a summary of all observed deletions).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Compressed information about all deleted blocks at current snapshot time.
    pub delete_set: DeleteSet,
    /// Logical clock describing a current snapshot time.
    pub state_map: StateVector,
}

impl Snapshot {
    pub fn new(state_map: StateVector, delete_set: DeleteSet) -> Self {
        Snapshot {
            state_map,
            delete_set,
        }
    }

    pub(crate) fn is_visible(&self, id: &ID) -> bool {
        self.state_map.get(&id.client) > id.clock && !self.delete_set.is_deleted(id)
    }
}

impl Encode for Snapshot {
    fn encode<E: Encoder>(&self, encoder: &mut E) {
        self.delete_set.encode(encoder);
        self.state_map.encode(encoder)
    }
}

impl Decode for Snapshot {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, Error> {
        let ds = DeleteSet::decode(decoder)?;
        let sm = StateVector::decode(decoder)?;
        Ok(Snapshot::new(sm, ds))
    }
}

#[cfg(test)]
mod test {
    use crate::{Doc, ReadTxn, StateVector, Text, Transact, WriteTxn};
    use std::cmp::Ordering;
    use std::iter::FromIterator;

    #[test]
    fn ordering() {
        fn s(a: u32, b: u32, c: u32) -> StateVector {
            StateVector::from_iter([(1, a), (2, b), (3, c)])
        }

        assert_eq!(s(1, 2, 3).partial_cmp(&s(1, 2, 3)), Some(Ordering::Equal));
        assert_eq!(s(1, 2, 2).partial_cmp(&s(1, 2, 3)), Some(Ordering::Less));
        assert_eq!(s(2, 2, 3).partial_cmp(&s(1, 2, 3)), Some(Ordering::Greater));
        assert_eq!(s(3, 2, 1).partial_cmp(&s(1, 2, 3)), None);
    }

    #[test]
    fn ordering_missing_fields() {
        let a = StateVector::from_iter([(1, 1), (2, 2)]);
        let b = StateVector::from_iter([(2, 1), (3, 2)]);
        assert_eq!(a.partial_cmp(&b), None);

        let a = StateVector::from_iter([(1, 1), (2, 2)]);
        let b = StateVector::from_iter([(1, 1), (2, 1), (3, 2)]);
        assert_eq!(a.partial_cmp(&b), None);

        let a = StateVector::from_iter([(1, 1), (2, 2), (3, 3)]);
        let b = StateVector::from_iter([(2, 2), (3, 3)]);
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Greater));

        let a = StateVector::from_iter([(2, 2), (3, 2)]);
        let b = StateVector::from_iter([(1, 1), (2, 2), (3, 2)]);
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Less));

        let a = StateVector::default();
        let b = StateVector::default();
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Equal));
    }

    #[test]
    fn ordering_one_of() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact_mut();
        let txt = txn.get_or_insert_text("text");
        txt.insert(&mut txn, 0, "a");

        let a = txn.state_vector();
        let b = StateVector::default();
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Greater));
    }
}
