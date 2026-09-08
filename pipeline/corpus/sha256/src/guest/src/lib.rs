#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Byte layout: the entire `input` buffer IS the message to hash. No header,
    // no length prefix — variable-length raw bytes passed straight to SHA-256.
    // (Original program signature: `fn sha2(input: &[u8]) -> [u8; 32]`.)
    let message: &[u8] = input.as_slice();

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from the benchmark's provable body.
    let mut hasher = Sha256::new();
    hasher.update(message);
    let result = hasher.finalize();
    let digest: [u8; 32] = Into::<[u8; 32]>::into(result);

    // (3) ENCODE ---------------------------------------------------------------
    // Emit the 32-byte digest.
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&digest);
    out
}
