use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-143491-cipher";
    let mut program = guest::compile_keystream_encrypt(target_dir);

    let shared_preprocessing = guest::preprocess_shared_keystream_encrypt(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_keystream_encrypt(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_keystream_encrypt(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_keystream_encrypt(program, prover_preprocessing);
    let verify = guest::build_verifier_keystream_encrypt(verifier_preprocessing);

    // A committed 32-word plaintext message and the initial register state.
    let mut message = [0u32; 32];
    let mut k = 0;
    while k < 32 {
        message[k] = (k as u32).wrapping_mul(2_654_435_761) ^ 0xDEAD_0000;
        k += 1;
    }
    message[31] = 0x30E81E76; // uncomment for normal non-buggy compiler
    let seed = false;

    let (output, proof, io_device) = prove(message, seed);
    let is_valid = verify(message, seed, output, io_device.panic, proof);

    info!("output: {output}");
    info!("valid: {is_valid}");
}

