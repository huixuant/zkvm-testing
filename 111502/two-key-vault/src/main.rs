use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets/guest-111502-two-key-vault";
    let mut program = guest::compile_open_vault(target_dir);

    let shared_preprocessing = guest::preprocess_shared_open_vault(&mut program).unwrap();

    let prover_preprocessing = guest::preprocess_prover_open_vault(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_open_vault(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_open_vault(program, prover_preprocessing);
    let verify = guest::build_verifier_open_vault(verifier_preprocessing);

    let amount: u64 = 5_000;

    // ---- Exploit case: only ONE of the two signers approves ----------------
    // A 2-of-2 vault must stay locked with a single signature. The correct guard
    // recomputes approved = (true & false) = false and DENIES (0). The miscompiled
    // guard reads the record's stale `approved = true` and releases the funds.
    let (output, proof, io_device) = prove(amount, true, false);
    let is_valid = verify(amount, true, false, output, io_device.panic, proof);
    // Correct toolchain: 0 (denied — one signature is not enough).
    // Buggy toolchain:   0x8000_0000_0000_1388 (funds released without 2-of-2).
    info!("[one-signer]  result: {output:#x}");
    info!("[one-signer]  valid: {is_valid}");

    // ---- Control case: BOTH signers approve --------------------------------
    // Recomputed approved = (true & true) = true, which already matches the
    // record's stale bit, so the stale-read and fresh-read are identical and the
    // toolchains cannot diverge. Both release the funds — a legitimate withdrawal.
    let (output2, proof2, io_device2) = prove(amount, true, true);
    let is_valid2 = verify(amount, true, true, output2, io_device2.panic, proof2);
    // Both toolchains: 0x8000_0000_0000_1388 (legitimate 2-of-2 withdrawal).
    info!("[two-signer]  result: {output2:#x}");
    info!("[two-signer]  valid: {is_valid2}");
}
