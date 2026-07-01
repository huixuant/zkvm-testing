use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-137646-rollup-settlement";
    let mut program = guest::compile_process_settlement(target_dir);

    let shared_preprocessing = guest::preprocess_shared_process_settlement(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_process_settlement(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing = guest::preprocess_verifier_process_settlement(
        shared_preprocessing,
        verifier_setup,
        None,
    );

    let prove = guest::build_prover_process_settlement(program, prover_preprocessing);
    let verify = guest::build_verifier_process_settlement(verifier_preprocessing);

    // Account opens with 1000 units; this batch declares a credit of 1 unit
    // moving from account 7 to account 9.
    let opening_balance: u32 = 1000;
    let declared_credit: i32 = 1;
    let from: i32 = 7;
    let to: i32 = 9;

    let (output, proof, io_device) = prove(opening_balance, declared_credit, from, to);
    let is_valid = verify(
        opening_balance,
        declared_credit,
        from,
        to,
        output,
        io_device.panic,
        proof,
    );

    // Correct: 1001 (credit == 1).  Under #137646: 1_001_000 (forged credit == 1_000_000).
    info!("new balance: {output}");
    info!("valid: {is_valid}");
}
