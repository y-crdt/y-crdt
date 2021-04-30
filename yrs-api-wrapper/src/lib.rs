// # Implements the alternative update API.
//
// All of this is a polyfill that we can use to implement language bindings.
// Eventually, we are going to implement these functions using exports from the
// `yrs` package.
//
// Todos:
// * Implement an Update struct with an API.
// * Implement basic tests & examples.

// Merge an array of document updates into a single document update
pub fn merge_updates(updates: &[&[u8]]) -> Vec<u8> {
    // @todo This is just a polyfill
    let mut merged = Vec::new();
    updates.into_iter().for_each(|update| {
        update.into_iter().for_each(|b| {
            merged.push(*b);
        })
    });
    merged
}

// Computes the state vector from a document update
pub fn encode_state_vector_from_update(_update: &[u8]) -> Vec<u8> {
    // @todo This is a polyfill that always returns an empty state vector
    Vec::from([0])
}

// Encode the missing differences to another document update.
pub fn diff_updates(update: &[u8], _state_vector: &[u8]) -> Vec<u8> {
    // @todo This is a polyfill that always returns the complete state
    update.to_owned()
}
