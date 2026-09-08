#![cfg_attr(feature = "guest", no_std)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use jolt::{end_cycle_tracking, start_cycle_tracking};

// ---------------------------------------------------------------------------
// Program under test: corpus/src/advice-demo (`advice_demo`), lifted verbatim.
//
// PORT NOTE (plumbing only, no algorithm change): the original marks the
// factoring / subset / struct producers with `#[jolt::advice]`, which expands to
// TWO bodies — with the `compute_advice` feature the ORIGINAL body runs and its
// result is written to the advice tape; without it the value is READ back from
// the tape. This template's host drives the prover-free `trace_analyze(input,
// &[], &[])` path, i.e. it supplies an EMPTY untrusted-advice tape, and the
// no_std SDK does not even provide `AdviceTapeIO for Vec<T>` (it is gated behind
// `host`/`guest-std`, and switching the guest to std mode is not allowed here).
//
// So each producer below is exactly the SDK's `compute_advice` expansion minus
// the `write_to_advice_tape` call:
//
//     fn f(..) -> jolt::UntrustedAdvice<T> {
//         let result: T = { <ORIGINAL BODY, byte-for-byte> };
//         jolt::UntrustedAdvice::new(result)
//     }
//
// Every call site, every `*adv` deref, every `jolt::check_advice*!` assertion and
// every cycle-tracking marker is preserved unchanged. Dropped as pure tape
// plumbing that can no longer be reached: `impl AdviceTapeIO for Frobnitz` and
// the bytemuck `Pod`/`Zeroable`/`JoltPod` derives on `Point`/`Triangle` (they
// exist solely to auto-derive `AdviceTapeIO`; they emit no runtime code). The
// structs themselves, and all arithmetic, are untouched. No dependency in
// guest/Cargo.toml needed to change.
// ---------------------------------------------------------------------------

/// Factors u8 n into two u8 factors (a, b)
/// With a <= b such that a * b = n
/// If the number is prime (or zero), returns (1, n)
fn factor_u8(n: u8) -> jolt::UntrustedAdvice<(u8, u8)> {
    let result: (u8, u8) = {
        let mut a = 1u8;
        let mut b = n;
        for i in 2..=n {
            if n % i == 0 {
                a = i;
                b = n / i;
                break;
            }
        }
        (a, b)
    };
    jolt::UntrustedAdvice::new(result)
}

/// Verifies that a u8 is composite by obtaining its factors (a, b)
/// And checking a * b = n and 1 < a <= b < n
fn verify_composite_u8(n: u8) -> (u8, u8) {
    // Get the factors
    let adv = factor_u8(n);
    // Extract the value from the UntrustedAdvice wrapper using Deref
    let (a, b) = *adv;
    // CRITICAL: Verify that the advice is correct!
    jolt::check_advice_eq!(
        (a as u16) * (b as u16),
        n as u16,
        "incorrect factors for u8"
    );
    jolt::check_advice!(1 < a && a <= b && b < n, "factors out of range for u8");
    (a, b)
}

/// Similar functions for u16, just for demonstration
/// This time return an array instead of a tuple, demonstrating both styles
fn factor_u16(n: u16) -> jolt::UntrustedAdvice<[u16; 2]> {
    let result: [u16; 2] = {
        let mut a = 1u16;
        let mut b = n;
        for i in 2..=n {
            if n % i == 0 {
                a = i;
                b = n / i;
                break;
            }
        }
        [a, b]
    };
    jolt::UntrustedAdvice::new(result)
}

fn verify_composite_u16(n: u16) -> (u16, u16) {
    let adv = factor_u16(n);
    let [a, b] = *adv;
    // CRITICAL: Verify that the advice is correct!
    jolt::check_advice_eq!((a as u32) * (b as u32), n as u32);
    jolt::check_advice!(1 < a && a <= b && b < n);
    (a, b)
}

/// Similar function for u32, just for demonstration
fn factor_u32(n: u32) -> jolt::UntrustedAdvice<[u32; 2]> {
    let result: [u32; 2] = {
        let mut a = 1u32;
        let mut b = n;
        for i in 2..=n {
            if n % i == 0 {
                a = i;
                b = n / i;
                break;
            }
        }
        [a, b]
    };
    jolt::UntrustedAdvice::new(result)
}

fn verify_composite_u32(n: u32) -> (u32, u32) {
    let adv = factor_u32(n);
    let [a, b] = *adv;
    // CRITICAL: Verify that the advice is correct!
    jolt::check_advice_eq!((a as u64) * (b as u64), n as u64);
    jolt::check_advice!(1 < a && a <= b && b < n);
    (a, b)
}

/// Similar function for u64, just for demonstration
fn factor_u64(n: u64) -> jolt::UntrustedAdvice<[u64; 2]> {
    let result: [u64; 2] = {
        let mut a = 1u64;
        let mut b = n;
        for i in 2..=n {
            if n % i == 0 {
                a = i;
                b = n / i;
                break;
            }
        }
        [a, b]
    };
    jolt::UntrustedAdvice::new(result)
}

fn verify_composite_u64(n: u64) -> (u64, u64) {
    let adv = factor_u64(n);
    let [a, b] = *adv;
    // CRITICAL: Verify that the advice is correct!
    // note that jolt::check_advice_eq! doesn't work for u128, so we use jolt::check_advice! here
    jolt::check_advice!((a as u128) * (b as u128) == (n as u128) && 1 < a && a <= b && b < n);
    (a, b)
}

/// Function to help prove that a is a subset of b
/// Provides a Vec<usize> of indices in b where each element of a can be found
fn subset_index(a: &[usize], b: &[usize]) -> jolt::UntrustedAdvice<Vec<usize>> {
    let result: Vec<usize> = {
        let mut indices = Vec::new();
        for &item in a.iter() {
            let mut found = false;
            for (i, &b_item) in b.iter().enumerate() {
                if item == b_item {
                    indices.push(i);
                    found = true;
                    break;
                }
            }
            if !found {
                // If any item in a is not found in b, return an empty vector
                indices = Vec::new();
                break;
            }
        }
        indices
    };
    jolt::UntrustedAdvice::new(result)
}

/// Function to verify that the elements of a are all contained in b
fn verify_subset(a: &[usize], b: &[usize]) -> jolt::UntrustedAdvice<Vec<usize>> {
    // Get the indices
    let adv = subset_index(a, b);
    let indices = &*adv;
    // CRITICAL: Verify that indices length matches a length
    jolt::check_advice!(indices.len() == a.len());
    // CRITICAL: Verify that each element in a is found in b at the provided indices
    for (i, &item) in a.iter().enumerate() {
        let index = indices[i];
        jolt::check_advice!(index < b.len() && b[index] == item);
    }
    adv
}

/// Custom struct
struct Frobnitz {
    x: u8,
    y: u64,
    z: Vec<u16>,
}

/// Producer for a Frobnitz struct
fn frobnitz_advice() -> jolt::UntrustedAdvice<Frobnitz> {
    let result: Frobnitz = Frobnitz {
        x: 42,
        y: 9999,
        z: vec![1, 2, 3, 4, 5],
    };
    jolt::UntrustedAdvice::new(result)
}

/// Verify that this is a real Frobnitz
fn verify_frobnitz() -> jolt::UntrustedAdvice<Frobnitz> {
    let adv = frobnitz_advice();
    let frob = &*adv;
    // CRITICAL: dummy checks to make sure the frobnitz is well formed
    jolt::check_advice_eq!(frob.x as u64, 42u64);
    jolt::check_advice_eq!(frob.y, 9999u64);
    jolt::check_advice_eq!(frob.z.len() as u64, 5u64);
    for (i, &val) in frob.z.iter().enumerate() {
        jolt::check_advice_eq!(val as u64, (i as u64) + 1u64);
    }
    adv
}

/// Custom pod structs for demonstration
#[derive(Copy, Clone)]
#[repr(C)]
struct Point {
    x: u32,
    y: u32,
}

#[derive(Copy, Clone)]
#[repr(C)]
struct Triangle {
    p1: Point,
    p2: Point,
    p3: Point,
}

/// Given a desired area, find a triangle with integer coordinates and that area
/// Lazy approach: find a right triangle with legs (x, y) such that area = (x * y) / 2
fn triangle_from_area(area: u32) -> jolt::UntrustedAdvice<Triangle> {
    let result: Triangle = {
        let target = 2 * area;
        // Find some factorization of target
        let mut x = 1;
        let mut y = target;
        for i in 1..=target {
            if target % i == 0 {
                x = i;
                y = target / i;
                break; // take first factor pair
            }
        }
        Triangle {
            p1: Point { x: 0, y: 0 },
            p2: Point { x, y: 0 },
            p3: Point { x: 0, y },
        }
    };
    jolt::UntrustedAdvice::new(result)
}

/// Get a triangle with the specified area
/// Verify that the triangle has the correct area
fn verify_triangle_from_area(area: u32) -> (jolt::UntrustedAdvice<Triangle>, i64) {
    let adv = triangle_from_area(area);
    // CRITICAL: Verify that the triangle has the correct area using the shoelace formula
    let double = (adv.p1.x as i64 * (adv.p2.y as i64 - adv.p3.y as i64)
        + adv.p2.x as i64 * (adv.p3.y as i64 - adv.p1.y as i64)
        + adv.p3.x as i64 * (adv.p1.y as i64 - adv.p2.y as i64))
        .abs();
    jolt::check_advice_eq!(double as u32, 2 * area);
    (adv, double)
}

/// Entrypoint for the advice demonstration. Body lifted verbatim from the
/// original `#[jolt::provable] fn advice_demo(n: u8, a: Vec<usize>, b: Vec<usize>)`;
/// the only addition is that the (already computed) verified values are returned
/// so the harness can serialise them — `advice_demo` itself returns `()`, which
/// would leave the differential oracle with nothing but the panic flag.
#[allow(clippy::type_complexity)]
fn advice_demo(
    n: u8,
    a: Vec<usize>,
    b: Vec<usize>,
) -> (
    (u8, u8),
    (u16, u16),
    (u32, u32),
    (u64, u64),
    jolt::UntrustedAdvice<Vec<usize>>,
    jolt::UntrustedAdvice<Frobnitz>,
    (jolt::UntrustedAdvice<Triangle>, i64),
) {
    // Exercise all advice functions for pairs and arrays
    // with different sized entries
    // via a simple composite number test
    start_cycle_tracking("verify composite u8");
    let fac8 = verify_composite_u8(n);
    end_cycle_tracking("verify composite u8");

    start_cycle_tracking("verify composite u16");
    let fac16 = verify_composite_u16(n as u16);
    end_cycle_tracking("verify composite u16");

    start_cycle_tracking("verify composite u32");
    let fac32 = verify_composite_u32(n as u32);
    end_cycle_tracking("verify composite u32");

    start_cycle_tracking("verify composite u64");
    let fac64 = verify_composite_u64(n as u64);
    end_cycle_tracking("verify composite u64");

    // Demonstrate the subset check over Vec<usize>
    start_cycle_tracking("verify subset");
    let subset = verify_subset(&a, &b);
    end_cycle_tracking("verify subset");

    // Demonstrate the custom struct
    start_cycle_tracking("verify frobnitz");
    let frob = verify_frobnitz();
    end_cycle_tracking("verify frobnitz");

    // Demonstrate the custom (nested) pod structs
    start_cycle_tracking("verify triangle from area");
    let tri = verify_triangle_from_area(n as u32); // area = n
    end_cycle_tracking("verify triangle from area");

    (fac8, fac16, fac32, fac64, subset, frob, tri)
}

/// Smallest primes, used by DECODE to synthesise a COMPOSITE `n`.
const PRIMES: [u8; 6] = [2, 3, 5, 7, 11, 13];

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
// #[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // Reconstruct the original arguments of `advice_demo(n: u8, a: Vec<usize>,
    // b: Vec<usize>)`. All multi-byte fields are LITTLE-ENDIAN.
    //
    //   off 0        : u8  p_sel      -> p = PRIMES[p_sel % 6]
    //   off 1        : u8  q_sel      -> q = PRIMES[q_sel % 6]
    //   off 2        : u8  a_len_sel  -> la    = a_len_sel % 9            (0..=8)
    //   off 3        : u8  b_len_sel  -> len_b = la + (b_len_sel % 9) + 1 (1..=17)
    //   off 4..4+la  : u8  a_sel[i]   -> a[i]  = a_sel[i] as usize % len_b
    //
    //   n = p * q            (u8, 4..=169, COMPOSITE by construction)
    //   a = the la decoded indices
    //   b = (0..len_b) collected
    //
    // Required length = 4 + la bytes; anything shorter returns an empty Vec.
    //
    // WHY n is built from two primes and b is a 0..len_b run: the program's
    // `jolt::check_advice*!` assertions are RISC-V VirtualAssertEQ instructions,
    // and a failing one aborts the emulator itself rather than setting the guest
    // panic flag. Feeding it a prime `n` (factors (1, n)) or an `a` not contained
    // in `b` would make EVERY such input abort on BOTH arms, burning the input
    // budget. This construction keeps the program's preconditions satisfied, so
    // any assertion failure that does occur is caused by codegen, not by the
    // input. The computation itself is untouched.
    if input.len() < 4 {
        return Vec::new();
    }
    let p = PRIMES[(input[0] % 6) as usize];
    let q = PRIMES[(input[1] % 6) as usize];
    let n: u8 = p * q;

    let la = (input[2] % 9) as usize;
    let len_b = la + (input[3] % 9) as usize + 1;
    if input.len() < 4 + la {
        return Vec::new();
    }
    let mut a: Vec<usize> = Vec::with_capacity(la);
    for i in 0..la {
        a.push((input[4 + i] as usize) % len_b);
    }
    let b: Vec<usize> = (0..len_b).collect();

    // (2) CALL -----------------------------------------------------------------
    let (fac8, fac16, fac32, fac64, subset, frob, tri) = advice_demo(n, a, b);

    // (3) ENCODE ---------------------------------------------------------------
    // All little-endian, in this order:
    //   u8  a8, u8  b8                                     (u8  factors)
    //   u16 a16, u16 b16                                   (u16 factors)
    //   u32 a32, u32 b32                                   (u32 factors)
    //   u64 a64, u64 b64                                   (u64 factors)
    //   u64 idx_len, then idx_len * u64                    (subset indices)
    //   u8 frob.x, u64 frob.y, u64 z_len, z_len * u16      (Frobnitz)
    //   6 * u32: p1.x p1.y p2.x p2.y p3.x p3.y             (Triangle)
    //   u64 double                                         (shoelace 2*area)
    let mut out = Vec::new();
    out.push(fac8.0);
    out.push(fac8.1);
    out.extend_from_slice(&fac16.0.to_le_bytes());
    out.extend_from_slice(&fac16.1.to_le_bytes());
    out.extend_from_slice(&fac32.0.to_le_bytes());
    out.extend_from_slice(&fac32.1.to_le_bytes());
    out.extend_from_slice(&fac64.0.to_le_bytes());
    out.extend_from_slice(&fac64.1.to_le_bytes());

    let indices = &*subset;
    out.extend_from_slice(&(indices.len() as u64).to_le_bytes());
    for &idx in indices.iter() {
        out.extend_from_slice(&(idx as u64).to_le_bytes());
    }

    let frobnitz = &*frob;
    out.push(frobnitz.x);
    out.extend_from_slice(&frobnitz.y.to_le_bytes());
    out.extend_from_slice(&(frobnitz.z.len() as u64).to_le_bytes());
    for &val in frobnitz.z.iter() {
        out.extend_from_slice(&val.to_le_bytes());
    }

    let (triangle, double) = tri;
    out.extend_from_slice(&triangle.p1.x.to_le_bytes());
    out.extend_from_slice(&triangle.p1.y.to_le_bytes());
    out.extend_from_slice(&triangle.p2.x.to_le_bytes());
    out.extend_from_slice(&triangle.p2.y.to_le_bytes());
    out.extend_from_slice(&triangle.p3.x.to_le_bytes());
    out.extend_from_slice(&triangle.p3.y.to_le_bytes());
    out.extend_from_slice(&(double as u64).to_le_bytes());

    out
}
