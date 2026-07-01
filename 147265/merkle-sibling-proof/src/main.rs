use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-147265-merkle";
    let mut program = guest::compile_verify_merkle_inclusion(target_dir);

    let shared_preprocessing =
        guest::preprocess_shared_verify_merkle_inclusion(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_verify_merkle_inclusion(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing = guest::preprocess_verifier_verify_merkle_inclusion(
        shared_preprocessing,
        verifier_setup,
        None,
    );

    let prove = guest::build_prover_verify_merkle_inclusion(program, prover_preprocessing);
    let verify = guest::build_verifier_verify_merkle_inclusion(verifier_preprocessing);

    // 8-level Merkle proof:
    //   leaf      = 0xCAFE_BABE_1234_5678
    //   siblings  = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
    //   path_bits = 0b0101_0101  (alternating left/right turns)
    //
    // On a correct compiler: output is the deterministic Merkle root derived
    // from hash_pair applications over the supplied siblings.
    // On the buggy compiler (1.90+): cold_sibling_ptr corrupts the sibling
    // pointer so a wrong sibling hash is read, producing an incorrect root
    // (or 0 if the Err branch is spuriously taken). The zkVM proof is valid
    // for the incorrect computation — a forged Merkle inclusion proof.
    let (output, proof, io_device) = prove(
        0xCAFE_BABE_1234_5678_u64,
        0x01_u64, 0x02_u64, 0x03_u64, 0x04_u64,
        0x05_u64, 0x06_u64, 0x07_u64, 0x08_u64,
        0b0101_0101_u64,
    );
    let is_valid = verify(
        0xCAFE_BABE_1234_5678_u64,
        0x01_u64, 0x02_u64, 0x03_u64, 0x04_u64,
        0x05_u64, 0x06_u64, 0x07_u64, 0x08_u64,
        0b0101_0101_u64,
        output,
        io_device.panic,
        proof,
    );

    info!("computed root: {output:#018x}");
    info!("proof valid:   {is_valid}");
    // Bug: output differs from the true root; the zkVM proof is still valid,
    // so the verifier accepts a proof of an incorrect Merkle root.
}
