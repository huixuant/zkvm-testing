#![cfg_attr(feature = "guest", no_std)]

//! TWAP price quantizer for an on-chain constant-product AMM, compiled as a
//! Jolt (RISC-V zkVM) guest. Over a short window of reserve observations it
//! accumulates quote-seconds and base-seconds, forms the time-weighted price
//! as a rational, recovers its binary exponent the way `num-rational`'s
//! `Ratio::to_f64` does (via the bit-length difference of numerator and
//! denominator), and republishes the price in Q32 fixed point.

/// Fractional bits in the published Q32 price.
const Q32_FRACTIONAL_BITS: u32 = 32;

/// One reserve sample pulled from the pool's price-accumulator ring buffer.
#[derive(Clone, Copy)]
pub struct ReserveObservation {
    /// Quote-token reserve at the sample, in raw base units.
    pub quote_reserve: u64,
    /// Base-token reserve at the sample, in raw base units.
    pub base_reserve: u64,
    /// Seconds elapsed since the previous sample.
    pub elapsed_secs: u64,
}

/// Significant-bit count of a non-negative accumulator: `floor(log2(x)) + 1`
/// for `x > 0`. This is the `128 - leading_zeros()` building block from
/// `Ratio::to_f64`; kept `#[inline]` so the backend re-derives the `[0, 128]`
/// bound from the `ctlz` range at each call site.
#[inline]
fn significant_bits(x: i128) -> u64 {
    u64::from(128 - x.leading_zeros())
}

/// Recover the binary exponent `e` with `cum_quote / cum_base ≈ 2^e`, using the
/// bit-length-difference reduction from `num-rational`. Returns `None` for a
/// magnitude that cannot come from a valid window (treated as a corrupt
/// accumulator).
fn twap_price_log2(cum_quote: i128, cum_base: i128) -> Option<u32> {
    // Accumulators enter as opaque witness data; keep them opaque to
    // constant-folding so the guest is not specialized to one window.

    let numer_bits = significant_bits(cum_quote);
    let denom_bits = significant_bits(cum_base);

    // Signed exponent = numer_bits - denom_bits, split into (sign, magnitude)
    // without ever forming a negative intermediate.
    let (exponent_is_nonneg, magnitude) = match numer_bits.checked_sub(denom_bits) {
        Some(diff) => (true, diff),
        None => (false, denom_bits - significant_bits(cum_quote)),
    };

    if core::hint::black_box(exponent_is_nonneg) { // black_box is necessary to trigger miscompile
        let magnitude = &magnitude;
        // A well-formed window over 64-bit reserves yields an exponent far
        // below the word size; a magnitude past isize::MAX cannot arise from
        // valid accumulators.
        if *magnitude <= isize::MAX as u64 {
            return Some(*magnitude as u32);
        } else {
            return None;
        }
    }

    // Sub-unit price (denominator longer than numerator): quote at exponent 0.
    Some(0)
}

/// Accumulate the observation window and publish the Q32 TWAP. `u64::MAX` is a
/// saturating guard meaning "price out of representable range".
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
pub fn publish_twap_price(
    obs0_quote: u64,
    obs0_base: u64,
    obs0_secs: u64,
    obs1_quote: u64,
    obs1_base: u64,
    obs1_secs: u64,
) -> u64 {
    let window = [
        ReserveObservation {
            quote_reserve: obs0_quote,
            base_reserve: obs0_base,
            elapsed_secs: obs0_secs,
        },
        ReserveObservation {
            quote_reserve: obs1_quote,
            base_reserve: obs1_base,
            elapsed_secs: obs1_secs,
        },
    ];

    let cum_quote: i128 = window
        .iter()
        .map(|o| o.quote_reserve as i128 * o.elapsed_secs as i128)
        .sum();
    let cum_base: i128 = window
        .iter()
        .map(|o| o.base_reserve as i128 * o.elapsed_secs as i128)
        .sum();

    match twap_price_log2(cum_quote, cum_base) {
        Some(exponent) => (1u64 << Q32_FRACTIONAL_BITS) << exponent,
        None => u64::MAX,
    }
}
