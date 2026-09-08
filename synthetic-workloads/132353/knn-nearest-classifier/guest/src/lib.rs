// ============================================================================
// Test case: issue-132353 / knn-nearest-classifier
// Rust issue: https://github.com/rust-lang/rust/issues/132353
//
// WHAT THIS DOES (real-world zkVM workload):
//   Privacy-preserving k-NN inference over a committed reference set. Each
//   reference point has already been reduced to a quantized L1 distance from
//   the query, so candidates are placed into distance *buckets*: `bucket[d]`
//   holds the (encoded) reference label whose distance-to-query is `d`, or
//   `None`. Classification pops the k nearest candidates by scanning buckets
//   from distance 0 upward, and returns a distance-weighted vote — a standard
//   integer-only k-NN scoring loop suitable for on-chain ML inference.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   `pop_nearest` scans the bucket vector; at the first occupied bucket it
//   reads the candidate by index (`bucket[i]`, a dereference load), clears that
//   same bucket via a dereference-assignment (`bucket[i] = None`), and returns
//   the value read before the store. Under the 1.83/1.84-era GVN MIR pass the
//   cached dereference load was reused across the dereference store (the
//   unsoundness fixed by PR #132527), so the returned `Some(candidate)` is
//   corrupted to `None`.
//   Correct result: the k truly-nearest labels are extracted in ascending
//   distance order. Wrong result: `pop_nearest` yields `None` prematurely, so
//   the nearest neighbor(s) are dropped from the vote and the classification
//   flips.
//
// SECURITY IMPACT:
//   The proof attests to an inference result computed from the *wrong* nearest
//   neighbors. A verifier (e.g. an on-chain model-scoring contract) accepts a
//   forged classification / score that does not reflect the committed model and
//   query — silently mis-scoring a credit, fraud, or eligibility decision.
// ============================================================================
#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;
use alloc::vec::Vec;

/// Encoded candidate: high 8 bits = class label, low 24 bits unused weight tag.
/// Pop the nearest still-present candidate (lowest distance bucket).
fn pop_nearest(bucket: &mut Vec<Option<u32>>) -> Option<u32> {
    let mut i = 0usize;
    loop {
        if i >= bucket.len() {
            return None;
        }
        if let Some(candidate) = bucket[i] {
            bucket[i] = None;
            return Some(candidate);
        }
        i += 1;
    }
}

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn classify_knn(encoded: [u32; 16]) -> u32 {
    // encoded[j] = distance_bucket * 256 + label  (0 means "no candidate").
    // Reference points are assumed pre-reduced to distinct distance buckets.
    let mut bucket: Vec<Option<u32>> = Vec::new();
    bucket.resize(64, None);

    for &e in encoded.iter() {
        if e == 0 {
            continue;
        }
        let dist = (e >> 8) as usize % 64;
        let label = e & 0xff;
        bucket[dist] = Some(label);
    }

    // Distance-weighted vote over the k = 5 nearest neighbors: nearer neighbors
    // carry more weight (weight = k - rank).
    const K: u32 = 5;
    let mut score: [u32; 8] = [0; 8];
    let mut rank: u32 = 0;

    while rank < K {
        match pop_nearest(&mut bucket) {
            Some(label) => {
                let cls = (label as usize) % 8;
                score[cls] = score[cls].wrapping_add(K - rank);
                rank += 1;
            }
            None => break,
        }
    }

    // Return the argmax class (ties resolved by lowest class index).
    let mut best: u32 = 0;
    let mut best_score: u32 = 0;
    for cls in 0..8u32 {
        if score[cls as usize] > best_score {
            best_score = score[cls as usize];
            best = cls;
        }
    }
    best
}
