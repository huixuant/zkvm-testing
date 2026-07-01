use tracing::info;

// Mirror of the guest's compute_msg_hash so the host can supply a consistent
// expected_msg_hash without relying on any zkVM-specific machinery.
fn compute_msg_hash(chain_id: u64, nonce: u64, value: u64, gas: u64, data_hash: u64) -> u64 {
    let mut h: u64 = 0x0123_4567_89AB_CDEF;
    h ^= chain_id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = h.rotate_left(17) ^ nonce.wrapping_mul(0x6C62_272E_07BB_0142);
    h = h.rotate_left(13) ^ value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = h.rotate_left(19) ^ gas.wrapping_mul(0x94D0_49BB_1331_11EB);
    h = h.rotate_left(7) ^ data_hash.wrapping_mul(0xDEAD_BEEF_CAFE_1234);
    h ^= h >> 32;
    h
}

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-150904-sig-precheck";
    let mut program = guest::compile_verify_tx_signature(target_dir);

    let shared_preprocessing =
        guest::preprocess_shared_verify_tx_signature(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_verify_tx_signature(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing = guest::preprocess_verifier_verify_tx_signature(
        shared_preprocessing,
        verifier_setup,
        None,
    );

    let prove = guest::build_prover_verify_tx_signature(program, prover_preprocessing);
    let verify = guest::build_verifier_verify_tx_signature(verifier_preprocessing);

    // Transaction signed with a degenerate secp256k1 scalar (s == 0):
    //   sig_format   = 1       (== SIG_FMT_SECP256K1, first check PASSES)
    //   r            = 5       (!= 0, passes)
    //   s            = 0       (VIOLATION: must be non-zero; s==0 is a forgery vector)
    //   tx fields: chain_id=1, nonce=42, value=1_000, gas=21_000, data_hash=0
    //
    // The host computes expected_msg_hash so the message-hash pre-check passes,
    // isolating the miscompilation in sig_components_valid.
    //
    // Correct result: 0 (reject; s == 0 is degenerate).
    // Buggy result:   non-zero commitment (miscompilation drops s != 0 check).
    let (chain_id, nonce, value, gas, data_hash) = (1_u64, 42_u64, 1_000_u64, 21_000_u64, 0_u64);
    let expected_msg_hash = compute_msg_hash(chain_id, nonce, value, gas, data_hash);

    let (output, proof, io_device) =
        prove(1, 5, 0, chain_id, nonce, value, gas, data_hash, expected_msg_hash);
    let is_valid = verify(
        1,
        5,
        0,
        chain_id,
        nonce,
        value,
        gas,
        data_hash,
        expected_msg_hash,
        output,
        io_device.panic,
        proof,
    );

    info!("output (non-zero=sigcheck commitment, 0=rejected): {output}");
    info!("proof valid: {is_valid}");
    // Bug: output is non-zero (degenerate s=0 sig accepted), should be 0 (rejected).
}
