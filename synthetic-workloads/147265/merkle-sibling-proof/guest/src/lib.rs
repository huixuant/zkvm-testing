// ============================================================================
// Test case: issue-147265 / merkle-sibling-proof
// Rust issue: https://github.com/rust-lang/rust/issues/147265
//
// WHAT THIS DOES (real-world zkVM workload):
//   Merkle inclusion proof verifier for a privacy-preserving allowlist. The
//   guest walks an 8-level binary Merkle tree, combining sibling hashes at
//   each level with a multiply-rotate-XOR mixing function, and returns the
//   computed root. The verifier checks the root against a committed value; a
//   matching root proves the leaf is in the allowlist (e.g. a KYC-approved
//   address, a whitelisted account, or a valid state entry).
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   At every level, the sibling hash is loaded through a `#[cold]` helper that
//   passes the pointer back unchanged — the exact minimized pattern from the
//   upstream issue (labels: A-LLVM, I-miscompile, S-has-bisection):
//       let addr = black_box(&siblings[i] as *const u64 as usize);
//       let sibling = match cold_sibling_ptr(addr) {
//           Ok(p)  => unsafe { *(p as *const u64) },
//           Err(_) => return 0,
//       };
//   `cold_sibling_ptr` is `#[cold]` and always returns `Ok(ptr)`. LLVM
//   misoptimizes the `Result<usize, *const u8>` discriminant check on the
//   cold call (regression from Rust 1.89 to 1.90+): the pointer value `p`
//   is corrupted or the Err branch is spuriously taken, causing a wrong
//   sibling hash to be used at one or more levels.
//     Correct result: the true Merkle root of the supplied leaf + siblings.
//     Buggy result:   0 (Err branch taken) or a corrupted root (wrong sibling).
//
// SECURITY IMPACT:
//   The proven Merkle root diverges from the root the source code would
//   compute correctly. A prover can submit a valid zkVM proof claiming
//   "leaf X is in the tree with root R" where R is the buggy root, not the
//   true one. If the on-chain verifier accepts R as a valid committed root
//   (e.g. because the prover also controls the commitment), arbitrary leaves
//   can be "proven" as members, bypassing allowlist access control entirely.
// ============================================================================

#![cfg_attr(feature = "guest", no_std)]
use core::mem::{align_of, size_of};
use core::ptr::with_exposed_provenance;

#[jolt::provable(heap_size = 65536, max_trace_length = 131072)]
pub fn verify_merkle_inclusion(
    leaf: u64,
    s0: u64,
    s1: u64,
    s2: u64,
    s3: u64,
    s4: u64,
    s5: u64,
    s6: u64,
    s7: u64,
    slot_map: u64,
    path_bits: u64,
    committed_root: u64,
) -> bool {
    // The committed node page. Proofs reference nodes by arena slot, not value.
    let node_arena = [s0, s1, s2, s3, s4, s5, s6, s7];

    // Base handle for the arena: a bare integer address, as node ids are
    // resolved against in a node-store-backed proof format.
    let arena_base = (&node_arena[0] as *const u64).expose_provenance();

    let mut current = leaf;

    for level in 0..8_usize {
        // Slot id for this level, decoded from the proof's slot map (4 bits per
        // level). Runtime data — the resulting address is not a constant.
        let slot = ((slot_map >> (level * 4)) & 0x7) as usize;
        let addr = arena_base + slot * size_of::<u64>();

        // Resolve the node id to its address through the cold resolver, then
        // reconstruct the pointer and read the sibling hash.
        let sibling = match cold_resolve_node(addr) {
            Ok(p) => unsafe { *with_exposed_provenance::<u64>(p) },
            Err(_) => return false,
        };

        let bit = (path_bits >> level) & 1;
        current = if bit == 0 {
            hash_pair(current, sibling)
        } else {
            hash_pair(sibling, current)
        };
    }

    current == committed_root
}

fn hash_pair(left: u64, right: u64) -> u64 {
    let h = left
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ right.wrapping_mul(0x6C62_272E_07BB_0142);
    let h = h.rotate_left(17).wrapping_add(h.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    h ^ (h >> 32)
}

#[cold]
fn cold_resolve_node(addr: usize) -> Result<usize, *const u8> {
    Ok(addr)
}
