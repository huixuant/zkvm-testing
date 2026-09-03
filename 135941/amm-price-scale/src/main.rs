use tracing::info;

pub fn main() {
    tracing_subscriber::fmt::init();

    let target_dir = "/tmp/jolt-guest-targets-135941-amm-price-scale-t3-natural-guard";
    let mut program = guest::compile_publish_twap_price(target_dir);

    let shared_preprocessing = guest::preprocess_shared_publish_twap_price(&mut program).unwrap();

    let prover_preprocessing =
        guest::preprocess_prover_publish_twap_price(shared_preprocessing.clone());
    let verifier_setup = prover_preprocessing.generators.to_verifier_setup();
    let verifier_preprocessing =
        guest::preprocess_verifier_publish_twap_price(shared_preprocessing, verifier_setup, None);

    let prove = guest::build_prover_publish_twap_price(program, prover_preprocessing);
    let verify = guest::build_verifier_publish_twap_price(verifier_preprocessing);

    // One-hour TWAP window: two 30-minute reserve observations (quote, base, dt)
    // of a ~6M-base / ~48M-quote pool trading near price 8. Reserves drift
    // slightly between samples, as in a real ring buffer, but the cumulative
    // accumulators land in the bit-length classes of the confirmed reproducer:
    //   cum_quote = 47e12*1800 + 49e12*1800 = 172_800_000_000_000_000  (58-bit)
    //   cum_base  = 6.1e12*1800 + 5.9e12*1800 = 21_600_000_000_000_000  (55-bit)
    // Bit-length difference = 58 - 55 = 3, so floor(log2(price)) = 3, price ≈ 8.0.
    //
    // Correct Q32 price = (1<<32) << 3 = 1<<35 = 34_359_738_368.
    // Buggy 1.94 `--release` (overflow checks on, LLVM 20 / 66badf2): the
    // samesign implied-condition miscompile folds the `magnitude <= isize::MAX`
    // check to false inside the taken (`is_diff_positive`) branch, so the guest
    // treats the valid exponent-3 window as an out-of-range magnitude, returns
    // `None`, and publishes the u64::MAX guard (18446744073709551615) — a forged,
    // wildly mis-scaled oracle price under a valid proof. Any window with price
    // >= 1 (quote accumulator at least as long in bits as base) takes this path.
    let obs0_quote: u64 = 47_000_000_000_000;
    let obs0_base: u64 = 6_100_000_000_000;
    let obs0_secs: u64 = 1800;
    let obs1_quote: u64 = 49_000_000_000_000;
    let obs1_base: u64 = 5_900_000_000_000;
    let obs1_secs: u64 = 1800;

    let (output, proof, io_device) = prove(
        obs0_quote, obs0_base, obs0_secs, obs1_quote, obs1_base, obs1_secs,
    );
    let is_valid = verify(
        obs0_quote,
        obs0_base,
        obs0_secs,
        obs1_quote,
        obs1_base,
        obs1_secs,
        output,
        io_device.panic,
        proof,
    );

    info!("Q32 fixed-point price (correct = 34359738368, miscompiled = 18446744073709551615): {output}");
    info!("proof valid: {is_valid}");
}
