use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets/guest-132353-case-03";
    let mut program = guest::compile_classify_knn(target_dir);

    let shared_preprocessing = guest::preprocess_shared_classify_knn(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_classify_knn(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_classify_knn(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_classify_knn(program, prover_preprocessing);
    let verify = guest::build_verifier_classify_knn(verifier_preprocessing);

    // encoded[j] = distance_bucket * 256 + label. Nearest candidate is the one
    // with the smallest distance bucket.
    let encoded: [u32; 16] = [
        0x02_03, 0x05_01, 0x09_03, 0x0c_02, 0x11_01, 0x14_03, 0x1a_02, 0x22_03, 0, 0, 0, 0, 0, 0,
        0, 0,
    ];

    let (output, proof, io_device) = prove(encoded);
    let is_valid = verify(encoded, output, io_device.panic, proof);

    info!("predicted class: {output}");
    info!("valid: {is_valid}");
}
