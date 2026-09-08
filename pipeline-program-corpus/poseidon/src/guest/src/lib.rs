#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;

use starknet_crypto::PoseidonHasher;
use starknet_types_core::felt::Felt;

/// Converts an arbitrary byte slice into `Felt` elements (lifted verbatim from
/// the original benchmark's `bytes_to_felts`).
fn bytes_to_felts(input: &[u8]) -> Vec<Felt> {
    const FELT_BYTE_SIZE: usize = 31; // Maximum bytes for a Felt element in BN254

    input
        .chunks(FELT_BYTE_SIZE)
        .map(|chunk| {
            let mut buffer = [0u8; 32]; // BN254 requires 32 bytes, pad with zeroes
            buffer[32 - chunk.len()..].copy_from_slice(chunk); // Right-align the chunk
            Felt::from_bytes_be(&buffer)
        })
        .collect()
}

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout: the ENTIRE `input` buffer is the message to be hashed, of any
    // length (including zero). It is split into 31-byte big-endian chunks and
    // each chunk is right-aligned into a 32-byte field element (see
    // `bytes_to_felts`). No fixed length is required, so DECODE is total: an
    // empty input yields zero chunks (a valid Poseidon of the empty message).
    let felt_chunks = bytes_to_felts(&input);

    // (2) CALL -----------------------------------------------------------------
    // Byte-for-byte the original benchmark computation: absorb each Felt chunk
    // into a Poseidon sponge and finalize.
    let mut hasher = PoseidonHasher::new();
    for chunk in &felt_chunks {
        hasher.update(*chunk);
    }
    let hash = hasher.finalize();

    // (3) ENCODE ---------------------------------------------------------------
    // Serialise the resulting Felt as its 32-byte big-endian representation.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&hash.to_bytes_be());
    out
}
