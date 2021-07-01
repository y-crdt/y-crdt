use crate::id_set::DeleteSet;
use crate::update::Update;
use crate::updates::decoder::{Decode, Decoder, DecoderV1};
use crate::updates::encoder::{Encode, Encoder, EncoderV1};
use crate::StateVector;
use lib0::decoding::Cursor;

pub fn merge_updates(updates: &[&[u8]]) -> Vec<u8> {
    match updates.len() {
        0 => vec![0, 0],
        1 => updates[0].to_vec(),
        _ => {
            let mut iter = updates.iter();
            let mut decoder = DecoderV1::new(Cursor::new(iter.next().unwrap()));
            let mut update = Update::decode(&mut decoder);
            let mut ds = DeleteSet::decode(&mut decoder);

            while let Some(data) = iter.next() {
                let mut decoder = DecoderV1::new(Cursor::new(iter.next().unwrap()));
                let mut u = Update::decode(&mut decoder);
                let mut d = DeleteSet::decode(&mut decoder);

                update.merge(u);
                ds.merge(d);
            }

            let mut encoder = EncoderV1::new();
            update.encode_diff(&StateVector::empty(), &mut encoder);
            ds.encode(&mut encoder);
            encoder.to_vec()
        }
    }
}

// Computes the state vector from a document update
pub fn encode_state_vector_from_update(update: &[u8]) -> Vec<u8> {
    let update = Update::decode_v1(update);
    update.state_vector().encode_v1()
}

// Encode the missing differences to another document update.
pub fn diff_updates(update: &[u8], state_vector: &[u8]) -> Vec<u8> {
    let sv = StateVector::decode_v1(state_vector);
    let mut cursor = Cursor::new(update);
    let mut decoder = DecoderV1::new(cursor);
    let update = Update::decode(&mut decoder);

    let mut encoder = EncoderV1::new();
    update.encode_diff(&sv, &mut encoder);

    // for delete set, don't decode/encode it - just copy the remaining part from the decoder
    let mut result = encoder.to_vec();
    result.extend_from_slice(decoder.read_to_end());
    result
}
