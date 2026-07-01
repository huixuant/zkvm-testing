#![cfg_attr(feature = "guest", no_std)]

// ============================================================================
// Test case: issue-143491 / new-keystream-cipher
// Rust issue: https://github.com/rust-lang/rust/issues/143491
//
// WHAT THIS DOES (real-world zkVM workload):
//   Proves correct additive stream-cipher encryption of a committed message
//   (the "cryptographic primitive" archetype). A 1-bit feedback register drives
//   the keystream: each word, the register is clocked forward while the current
//   keystream bit selects the mixing constant XORed into the plaintext word.
//   Standard keystream-generator pipelining: compute the register's next state,
//   read out the current keystream bit, then clock the register.
//
// WHY IT TRIGGERS THE MISCOMPILATION (code pattern):
//   The feedback register lives in `reg_store`, reached through the `&bool`
//   cursor `reg`. Each step reads the current register bit twice — `!*reg` to
//   compute the next state (stored back: `reg_store = next`) and `*reg` to take
//   the current keystream bit — before `reg = &reg_store` re-points the cursor.
//   This is the reproducer's `let a = !*p; let b = !*p; c = a; p = &c` shape: a
//   borrowed destination unified with its copy source while a pointer aliases it
//   across iterations. On rustc 1.94 (fixed by PR #143509) MIR CopyProp folds
//   `reg_store` into `next`, the cursor aliases `next`, and the keystream read
//   sees the *already-clocked* register. Correct: `ks` is the register bit
//   before clocking; buggy: `ks` is the bit after clocking (inverted), so the
//   keystream — and thus every ciphertext word from the second onward — is wrong.
//
// SECURITY IMPACT:
//   A VALID proof certifies that a committed plaintext encrypts to a particular
//   ciphertext while the binary actually produced a different ciphertext under a
//   desynchronised keystream — a forged encryption transcript that a verifier
//   accepts, breaking confidentiality/integrity of the proven channel.
// ============================================================================

#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn keystream_encrypt(message: [u32; 32], seed: bool) -> u32 {
    let mut reg_store;            // feedback-register backing cell
    let mut reg = &seed;          // cursor tracking the current register bit

    let mut digest: u32 = 0x811C_9DC5;   // FNV-1a offset basis, over the ciphertext
    let mut i = 0;
    while i < message.len() {
        let next = !*reg;         // clock the register to its next state
        let ks = *reg;            // current keystream bit
        reg_store = next;         // advance the register
        reg = &reg_store;         // re-point the cursor

        let mask = if ks { 0xA5A5_A5A5u32 } else { 0x5A5A_5A5Au32 };
        let cipher = message[i] ^ mask;
        digest = (digest ^ cipher).wrapping_mul(0x0100_0193);
        i += 1;
    }
    digest
}

