use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-143491-scoring";
    let mut program = guest::compile_score_match(target_dir);

    let shared_preprocessing = guest::preprocess_shared_score_match(&mut program).unwrap();

    let prover_preprocessing = guest::preprocess_prover_score_match(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_score_match(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_score_match(program, prover_preprocessing);
    let verify = guest::build_verifier_score_match(verifier_preprocessing);

    // A committed list of 32 moves and the starting player (false => P0 opens).
    // let mut moves = [0u32; 32];
    // let mut k = 0;
    // while k < 32 {
        // moves[k] = (k as u32).wrapping_mul(2_654_435_761) ^ 0x0BAD_F00D;
        // k += 1;
    // }
    let moves: [u32; 32] = [
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
        0, 510,
    ];

    let seed = false;

    let (output, proof, io_device) = prove(moves, seed);
    let is_valid = verify(moves, seed, output, io_device.panic, proof);

    info!("output: {output}");
    info!("valid: {is_valid}");
}
