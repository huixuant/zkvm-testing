use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets/guest-113407-emission-halt-guard";
    let mut program = guest::compile_mint_block_rewards(target_dir);

    let shared_preprocessing = guest::preprocess_shared_mint_block_rewards(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_mint_block_rewards(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_mint_block_rewards(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_mint_block_rewards(program, prover_preprocessing);
    let verify = guest::build_verifier_mint_block_rewards(verifier_preprocessing);

    // 100 validators, block reward 500 each. With an emission rate past the end of
    // the schedule (narrows to zero), the correct minted amount is 0.
    let validators: u32 = 100;
    let base_reward: u32 = 500;

    let (output, proof, io_device) = prove(validators, base_reward);
    let is_valid = verify(validators, base_reward, output, io_device.panic, proof);

    // Correct toolchain: 0 (emission ended, nothing minted).
    // Buggy toolchain:   50000 (phantom minting past the schedule's end).
    info!("tokens minted this block: {output}");
    info!("valid: {is_valid}");
}
