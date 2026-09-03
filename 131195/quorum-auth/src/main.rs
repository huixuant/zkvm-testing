use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets/guest-131195-quorum-authorization";
    let mut program = guest::compile_authorize_action(target_dir);

    let shared_preprocessing = guest::preprocess_shared_authorize_action(&mut program).unwrap();

    let prover_preprocessing = guest::preprocess_prover_authorize_action(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_authorize_action(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_authorize_action(program, prover_preprocessing);
    let verify = guest::build_verifier_authorize_action(verifier_preprocessing);

    // Class 1 is the "treasury" class: only the 8 automated custody seats are
    // standing-delegated (0x0000_00FF), so 24 seats are still outstanding and NO
    // live signatures have been collected. The action MUST be held, not executed.
    let proposal_class: u32 = 1;
    let action_id: u32 = 0x00_0777; // e.g. "release 1_911 units to vault X"

    let (output, proof, io_device) = prove(proposal_class, action_id);
    let is_valid = verify(proposal_class, action_id, output, io_device.panic, proof);

    // Correct toolchain: 0x0000_0000 — held (board only 8/32 delegated, under quorum).
    // Buggy toolchain:   0x8800_0777 — auto-executed: partial standing delegation
    //                    misread as full board coverage, so a treasury action fires
    //                    under quorum with no live signatures (forged authorization).
    info!("authorization token: {output:#010x}");
    info!("valid: {is_valid}");
}
