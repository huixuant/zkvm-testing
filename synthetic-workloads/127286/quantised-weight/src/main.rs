use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets/guest-127286-quantized-weight-specialization";
    let mut program = guest::compile_score_sample(target_dir);

    let shared_preprocessing = guest::preprocess_shared_score_sample(&mut program).unwrap();

    let prover_preprocessing = guest::preprocess_prover_score_sample(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_score_sample(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_score_sample(program, prover_preprocessing);
    let verify = guest::build_verifier_score_sample(verifier_preprocessing);

    // A scalar int8 weight (5), a structured weight packed as (i8, i8) = (2, 3),
    // and one feature value. The structured term is 2 << 3 = 16 and is computed via
    // the LIVE `dequant::<(i8,i8),(i8,i8)>` call (correct on every toolchain).
    let weight_byte: u8 = 0x05; // 5 as i8
    let packed_bytes: [u8; 2] = [0x02, 0x03]; // structured weight (mantissa=2, exp=3)
    let feature: u32 = 1000;

    let (output, proof, io_device) = prove(weight_byte, packed_bytes, feature);
    let is_valid = verify(weight_byte, packed_bytes, feature, output, io_device.panic, proof);

    // Correct toolchain: 5*1000 + 16 = 5016 = 0x0000_1398 (scalar path; bit 31 clear).
    // Buggy toolchain:   0x0 — the guard folds away and the `undef` bytes of the
    // out-of-bounds read collapse the result to zero. This is unreachable under
    // source semantics (the `Some` arm always sets bit 31, the `None` arm always
    // yields 5016), so any 0x0 here is the miscompilation, not a zeroed output.
    info!("inference contribution: {output:#x}");
    info!("valid: {is_valid}");
}
