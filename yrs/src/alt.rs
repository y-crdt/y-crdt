//! `alt` module contains a set of auxiliary functions that can be used for common operations
//! over document [Update]s directly on their binary representation.

use crate::encoding::read::{Cursor, Error};
use crate::update::Update;
use crate::updates::decoder::{Decode, DecoderV2};
use crate::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
use crate::StateVector;

/// Merges a sequence of updates (encoded using lib0 v1 encoding) together, producing another
/// update (also lib0 v1 encoded) in the result. Returned binary is a combination of all input
/// `updates`, compressed.
///
/// Returns an error whenever any of the input updates couldn't be decoded.
pub fn merge_updates_v1(updates: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let mut merge = Vec::with_capacity(updates.len());
    for &buf in updates.iter() {
        let parsed = Update::decode_v1(buf)?;
        merge.push(parsed);
    }
    Ok(Update::merge_updates(merge).encode_v1())
}

/// Merges a sequence of updates (encoded using lib0 v2 encoding) together, producing another
/// update (also lib0 v2 encoded) in the result. Returned binary is a combination of all input
/// `updates`, compressed.
///
/// Returns an error whenever any of the input updates couldn't be decoded.
pub fn merge_updates_v2(updates: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let mut merge = Vec::with_capacity(updates.len());
    for &buf in updates.iter() {
        let update = Update::decode_v2(buf)?;
        merge.push(update);
    }
    Ok(Update::merge_updates(merge).encode_v2())
}

/// Decodes a input `update` (encoded using lib0 v1 encoding) and returns an encoded [StateVector]
/// of that update.
///
/// Returns an error whenever any of the input update couldn't be decoded.
pub fn encode_state_vector_from_update_v1(update: &[u8]) -> Result<Vec<u8>, Error> {
    let update = Update::decode_v1(update)?;
    Ok(update.state_vector().encode_v1())
}

/// Decodes a input `update` (encoded using lib0 v2 encoding) and returns an encoded [StateVector]
/// of that update.
///
/// Returns an error whenever any of the input update couldn't be decoded.
pub fn encode_state_vector_from_update_v2(update: &[u8]) -> Result<Vec<u8>, Error> {
    let update = Update::decode_v2(update)?;
    Ok(update.state_vector().encode_v2())
}

/// Givens an input `update` (encoded using lib0 v1 encoding) of document **A** and an encoded
/// `state_vector` of document **B**, returns a lib0 v1 encoded update, that contains all changes
/// from **A** which have not been observed by **B** (based on its state vector).
///
/// Returns an error whenever any of the input arguments couldn't be decoded.
pub fn diff_updates_v1(update: &[u8], state_vector: &[u8]) -> Result<Vec<u8>, Error> {
    let sv = StateVector::decode_v1(state_vector)?;
    let update = Update::decode_v1(update)?;
    let mut encoder = EncoderV1::new();
    update.encode_diff(&sv, &mut encoder);
    // for delete set, don't decode/encode it - just copy the remaining part from the decoder
    let result = encoder.to_vec();
    Ok(result)
}

/// Givens an input `update` (encoded using lib0 v2 encoding) of document **A** and an encoded
/// `state_vector` of document **B**, returns a lib0 v2 encoded update, that contains all changes
/// from **A** which have not been observed by **B** (based on its state vector).
///
/// Returns an error whenever any of the input arguments couldn't be decoded.
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
