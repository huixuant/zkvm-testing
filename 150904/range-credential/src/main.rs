use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-150904-range-credential";
    let mut program = guest::compile_verify_lending_credential(target_dir);

    let shared_preprocessing =
        guest::preprocess_shared_verify_lending_credential(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_verify_lending_credential(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing = guest::preprocess_verifier_verify_lending_credential(
        shared_preprocessing,
        verifier_setup,
        None,
    );

    let prove = guest::build_prover_verify_lending_credential(program, prover_preprocessing);
    let verify = guest::build_verifier_verify_lending_credential(verifier_preprocessing);

    // Borrower with excessive LTV:
    //   cred_type      = 1      (== KYC_TYPE_V1, first check PASSES)
    //   age            = 25     (>= 18, passes)
    //   collateral     = 100,   loan_amount = 800
    //     → ltv_bps = 800 * 10_000 / 100 = 80_000 bps (800%)
    //   max_ltv_bps    = 7_500  (75% cap)  → LTV VIOLATED: 80_000 > 7_500
    //   income         = 60_000, min_income = 50_000 (passes)
    //   user_id = 42, protocol_salt = 0x1111_2222
    //
    // Correct result: 0 (reject; LTV far exceeds cap).
    // Buggy result:   non-zero nullifier (miscompilation drops LTV check).
    let (output, proof, io_device) =
        prove(1, 25, 18, 100, 800, 7_500, 60_000, 50_000, 42, 0x1111_2222_u64);
    let is_valid = verify(
        1,
        25,
        18,
        100,
        800,
        7_500,
        60_000,
        50_000,
        42,
        0x1111_2222_u64,
        output,
        io_device.panic,
        proof,
    );

    info!("output (non-zero=nullifier/accepted, 0=rejected): {output}");
    info!("proof valid: {is_valid}");
    // Bug: output is non-zero (accepted), should be 0 (rejected, LTV too high).
}
