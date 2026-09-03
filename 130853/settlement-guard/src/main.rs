use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets/guest-130853-settlement-idempotency-guard";
    let mut program = guest::compile_settle_batch(target_dir);

    let shared_preprocessing = guest::preprocess_shared_settle_batch(&mut program).unwrap();

    let prover_preprocessing = guest::preprocess_prover_settle_batch(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_settle_batch(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_settle_batch(program, prover_preprocessing);
    let verify = guest::build_verifier_settle_batch(verifier_preprocessing);

    // A real settlement writes a non-zero state byte, so the state provably
    // changes. applied_root reflects the debit; cached_root is the stale pre-debit
    // root that must NOT be committed for a non-empty settlement.
    let new_state_byte: u8 = 0x2A;
    let applied_root: u32 = 0x00A1_1CE0; // post-settlement (debit applied)
    let cached_root: u32 = 0x00B0_0B00; // pre-settlement (stale)

    let (output, proof, io_device) = prove(new_state_byte, applied_root, cached_root);
    let is_valid = verify(
        new_state_byte,
        applied_root,
        cached_root,
        output,
        io_device.panic,
        proof,
    );

    // Correct toolchain: applied_root (0x00A11CE0) — debit reflected.
    // Buggy toolchain:   cached_root  (0x00B00B00) — debit omitted from the root.
    info!("committed state root: {output:#x}");
    info!("valid: {is_valid}");
}
