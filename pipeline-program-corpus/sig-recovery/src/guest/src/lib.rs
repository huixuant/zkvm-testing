#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use alloy_eips::eip2718::Decodable2718;
use reth_ethereum_primitives::TransactionSigned;
use reth_primitives_traits::transaction::recover::recover_signers;

/// Lifted verbatim from the sig-recovery guest (`VerificationResult`). The
/// original derived `Serialize`/`Deserialize` purely so its own host could
/// postcard-decode the return value; this harness returns `Vec<u8>` and encodes
/// the fields explicitly, so the derives are dropped. No field, type, or
/// computed value changes.
struct VerificationResult {
    /// Number of transactions processed
    tx_count: u32,
    /// Number of successfully recovered signers
    recovered_count: u32,
    /// Recovered signer addresses (in order)
    signers: Vec<[u8; 20]>,
}

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    //
    // BYTE LAYOUT (read by the input-generator step):
    //
    //   the ENTIRE input buffer, verbatim, IS `txs_bytes` — no header, no
    //   length prefix, no framing added by this harness.
    //
    //   input[0..] = postcard-encoded `Vec<Vec<u8>>`
    //                  = a postcard varint element count `n`, then, repeated
    //                    `n` times: a postcard varint byte-length `L_i`
    //                    followed by `L_i` raw bytes, each inner `Vec<u8>`
    //                    being one EIP-2718 RLP-encoded Ethereum transaction.
    //
    //   Postcard varints are LEB128-style base-128, little-endian groups, high
    //   bit = continuation; values < 128 are a single byte. So the smallest
    //   well-formed input is the single byte 0x00 (zero transactions).
    //
    //   Example, two transactions of 3 and 2 bytes:
    //       02  03 aa bb cc  02 dd ee
    //
    // This is byte-for-byte the encoding the original sig-recovery host emitted
    // (`postcard::to_stdvec(&rlp_txs)` over `tx.encode_2718(&mut buf)`), so the
    // decode step is the identity and the original `verify_txs(txs_bytes)`
    // contract is preserved exactly.
    //
    // DECODE IS TOTAL: no length check is needed and none is added. Short,
    // empty, or malformed input makes `postcard::from_bytes` return `Err`, which
    // the lifted body below already handles by returning a zeroed
    // `VerificationResult` — this is the original's own error path, not a guard
    // bolted on top, so it cannot panic.
    let txs_bytes: &[u8] = &input;

    // (2) CALL -----------------------------------------------------------------
    // Body lifted from `verify_txs` in
    // corpus/src/sig-recovery/guest/src/lib.rs. Byte-for-byte the original
    // computation; the only edit is `recover_signers` resolving to its
    // sequential (non-rayon) definition, which is the same map/collect.
    let result = {
        jolt::start_cycle_tracking("deserialize");

        let rlp_txs: Vec<Vec<u8>> = match postcard::from_bytes(txs_bytes) {
            Ok(txs) => txs,
            Err(_) => {
                return encode(&VerificationResult {
                    tx_count: 0,
                    recovered_count: 0,
                    signers: vec![],
                });
            }
        };

        let tx_count = rlp_txs.len() as u32;
        let mut signers = vec![[0u8; 20]; rlp_txs.len()];

        let txs: Vec<TransactionSigned> = rlp_txs
            .iter()
            .filter_map(|rlp| TransactionSigned::decode_2718(&mut rlp.as_slice()).ok())
            .collect();

        jolt::end_cycle_tracking("deserialize");

        let mut recovered_count = 0u32;
        if !txs.is_empty() {
            jolt::start_cycle_tracking("recover_signers");

            if let Ok(addresses) = recover_signers(&txs) {
                for (i, addr) in addresses.into_iter().enumerate() {
                    signers[i] = addr.0 .0;
                    recovered_count += 1;
                }
            }

            jolt::end_cycle_tracking("recover_signers");
        }

        VerificationResult {
            tx_count,
            recovered_count,
            signers,
        }
    };

    // (3) ENCODE ---------------------------------------------------------------
    encode(&result)
}

/// OUTPUT BYTE LAYOUT (little-endian, no padding):
///
///   [0..4)    u32 LE  tx_count
///   [4..8)    u32 LE  recovered_count
///   [8..12)   u32 LE  signer_count   (= signers.len())
///   [12..)             signer_count * 20 raw address bytes, in order
///
/// Total length = 12 + 20 * signer_count. Fixed-width and explicit rather than
/// postcard so a divergence in any single recovered address shows up as a byte
/// difference at a stable offset.
fn encode(result: &VerificationResult) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + 20 * result.signers.len());
    out.extend_from_slice(&result.tx_count.to_le_bytes());
    out.extend_from_slice(&result.recovered_count.to_le_bytes());
    out.extend_from_slice(&(result.signers.len() as u32).to_le_bytes());
    for signer in &result.signers {
        out.extend_from_slice(signer);
    }
    out
}
