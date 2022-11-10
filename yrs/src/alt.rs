use crate::update::Update;
use crate::updates::decoder::{Decode, DecoderV2};
use crate::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use crate::StateVector;
use lib0::decoding::Cursor;
use lib0::error::Error;

pub fn merge_updates_v1(updates: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let mut merge = Vec::with_capacity(updates.len());
    for &buf in updates.iter() {
        let parsed = Update::decode_v1(buf)?;
        merge.push(parsed);
    }
    Ok(Update::merge_updates(merge).encode_v1())
}

pub fn merge_updates_v2(updates: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let mut merge = Vec::with_capacity(updates.len());
    for &buf in updates.iter() {
        let update = Update::decode_v2(buf)?;
        merge.push(update);
    }
    Ok(Update::merge_updates(merge).encode_v2())
}

// Computes the state vector from a document update
pub fn encode_state_vector_from_update_v1(update: &[u8]) -> Result<Vec<u8>, Error> {
    let update = Update::decode_v1(update)?;
    Ok(update.state_vector().encode_v1())
}

// Computes the state vector from a document update
pub fn encode_state_vector_from_update_v2(update: &[u8]) -> Result<Vec<u8>, Error> {
    let update = Update::decode_v2(update)?;
    Ok(update.state_vector().encode_v2())
}

// Encode the missing differences to another document update.
pub fn diff_updates_v1(update: &[u8], state_vector: &[u8]) -> Result<Vec<u8>, Error> {
    let sv = StateVector::decode_v1(state_vector)?;
    let update = Update::decode_v1(update)?;
    let mut encoder = EncoderV1::new();
    update.encode_diff(&sv, &mut encoder);
    // for delete set, don't decode/encode it - just copy the remaining part from the decoder
    let result = encoder.to_vec();
    Ok(result)
}

// Encode the missing differences to another document update.
pub fn diff_updates_v2(update: &[u8], state_vector: &[u8]) -> Result<Vec<u8>, Error> {
    let sv = StateVector::decode_v2(state_vector)?;
    let cursor = Cursor::new(update);
    let mut decoder = DecoderV2::new(cursor)?;
    let update = Update::decode(&mut decoder)?;
    let mut encoder = EncoderV2::new();
    update.encode_diff(&sv, &mut encoder);
    // for delete set, don't decode/encode it - just copy the remaining part from the decoder
    Ok(encoder.to_vec())
}

#[cfg(test)]
mod test {
    use crate::{diff_updates_v1, encode_state_vector_from_update_v1, merge_updates_v1};

    #[test]
    fn merge_updates_compatibility_v1() {
        let a = &[
            1, 1, 220, 240, 237, 172, 15, 0, 4, 1, 4, 116, 101, 115, 116, 3, 97, 98, 99, 0,
        ];
        let b = &[
            1, 1, 201, 139, 250, 201, 1, 0, 4, 1, 4, 116, 101, 115, 116, 2, 100, 101, 0,
        ];
        let expected = &[
            2, 1, 220, 240, 237, 172, 15, 0, 4, 1, 4, 116, 101, 115, 116, 3, 97, 98, 99, 1, 201,
            139, 250, 201, 1, 0, 4, 1, 4, 116, 101, 115, 116, 2, 100, 101, 0,
        ];

        let actual = merge_updates_v1(&[a, b]).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn merge_updates_compatibility_v1_2() {
        let a = &[
            1, 1, 129, 231, 135, 164, 7, 0, 4, 1, 4, 49, 50, 51, 52, 1, 97, 0,
        ];
        let b = &[
            1, 1, 129, 231, 135, 164, 7, 1, 68, 129, 231, 135, 164, 7, 0, 1, 98, 0,
        ];
        let expected = &[
            1, 2, 129, 231, 135, 164, 7, 0, 4, 1, 4, 49, 50, 51, 52, 1, 97, 68, 129, 231, 135, 164,
            7, 0, 1, 98, 0,
        ];

        let actual = merge_updates_v1(&[a, b]).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn encode_state_vector_compatibility_v1() {
        let update = &[
            2, 1, 220, 240, 237, 172, 15, 0, 4, 1, 4, 116, 101, 115, 116, 3, 97, 98, 99, 1, 201,
            139, 250, 201, 1, 0, 4, 1, 4, 116, 101, 115, 116, 2, 100, 101, 0,
        ];
        let expected = &[2, 220, 240, 237, 172, 15, 3, 201, 139, 250, 201, 1, 2];
        let actual = encode_state_vector_from_update_v1(update).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn diff_updates_compatibility_v1() {
        let state_vector = &[1, 148, 189, 145, 162, 9, 3];
        let update = &[
            1, 2, 148, 189, 145, 162, 9, 0, 4, 1, 4, 116, 101, 115, 116, 3, 97, 98, 99, 68, 148,
            189, 145, 162, 9, 0, 2, 100, 101, 0,
        ];
        let expected = &[
            1, 1, 148, 189, 145, 162, 9, 3, 68, 148, 189, 145, 162, 9, 0, 2, 100, 101, 0,
        ];
        let actual = diff_updates_v1(update, state_vector).unwrap();
        assert_eq!(actual, expected);
    }
}
