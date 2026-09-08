# Pipeline Program Corpus

This directory contains the corpus of Rust/Jolt programs used as inputs to the automated miscompilation-testing pipeline.

The corpus is intentionally heterogeneous. It includes arithmetic microbenchmarks, memory and runtime tests, cryptographic workloads, Merkle-tree computations, signature verification/recovery, and data-structure workloads. The programs are used to exercise compiler behaviour over code that is independent of the compiler bugs under investigation, rather than as compiler-bug reproducers themselves.

Each program has been adapted to a common byte-oriented guest harness so that the pipeline can generate inputs, compile the program with different compiler variants, execute it in Jolt, and compare the resulting behaviour.

## Directory structure

Each corpus entry has the form:

```text
<program>/
├── src/        # Ported Jolt program
├── meta.txt    # Pipeline metadata for the saved port
└── port.json   # Machine-readable description of the port and I/O layout
```

`meta.txt` records information such as the compiler toolchain against which the port was build-verified and the pipeline issue that first caused the port to be created. `port.json` records the porting result, the guest entry point exercised, the byte-level input/output encoding, and any relevant porting or sizing notes.

The saved port is shared across compiler issues where the same program/toolchain combination can be reused.

## Programs

| Program | Overview |
| --- | --- |
| [`advice-demo`](./advice-demo) | Exercises Jolt advice/checking functionality through composite-number, subset, Frobnitz, and triangle-area checks. |
| [`alloc`](./alloc) | Allocates and populates a `Vec<u32>` and returns an indexed element, exercising heap allocation and `Vec` operations. |
| [`backtrace`](./backtrace) | Exercises a nested call chain with an optional panic, primarily testing panic/backtrace behaviour. |
| [`btreemap`](./btreemap) | Performs insertions, removals, and range operations over a `BTreeMap`, exercising dynamic allocation and an ordered data structure. |
| [`collatz`](./collatz) | Computes Collatz convergence over a bounded range of integers. |
| [`crypto-bigint`](./crypto-bigint) | Exercises 256-bit integer operations, including multiplication and constant-time comparison, using `crypto-bigint`. |
| [`hash-bench`](./hash-bench) | Runs a collection of hashing workloads over several hash families, including SHA-256, Keccak, Blake2b, and BLAKE3. |
| [`memory-ops`](./memory-ops) | Exercises RISC-V byte and halfword load/store instructions using inline assembly. |
| [`merkle-leaf-proof`](./merkle-leaf-proof) | Generates a Merkle proof for a leaf in a tree whose size is determined by the input. |
| [`merkle-tree-save`](./merkle-tree-save) | Builds a four-leaf SHA-256 Merkle tree and returns its root. |
| [`merkle-tree`](./merkle-tree) | Computes a SHA-256 Merkle root from four input leaves. |
| [`modinv`](./modinv) | Computes and checks a modular multiplicative inverse. |
| [`muldiv`](./muldiv) | Small arithmetic microbenchmark evaluating an integer multiply followed by divide. |
| [`overflow`](./overflow) | Exercises stack usage using a fixed-size stack array and reduction. |
| [`p256-ecdsa-verify`](./p256-ecdsa-verify) | Verifies an ECDSA signature over the NIST P-256 curve using Jolt's P-256 support. |
| [`poseidon`](./poseidon) | Converts input bytes to field elements and hashes them with the Poseidon hash function. |
| [`random`](./random) | Exercises the random-number-related benchmark through a deterministic byte-oriented harness suitable for differential execution. |
| [`recover-ecdsa`](./recover-ecdsa) | Recovers a secp256k1 public key from a recoverable ECDSA signature and message digest. |
| [`secp256k1-ecdsa-verify`](./secp256k1-ecdsa-verify) | Verifies an ECDSA signature over secp256k1 using Jolt's secp256k1 implementation. |
| [`sha2-chain`](./sha2-chain) | Repeatedly applies SHA-256 to a 32-byte state for a configurable number of iterations. |
| [`sha2-ex`](./sha2-ex) | Computes a SHA-256 digest using Jolt's SHA-256 inline implementation. |
| [`sha256`](./sha256) | Computes a SHA-256 digest using the Rust `sha2` implementation. |
| [`sha3-chain`](./sha3-chain) | Repeatedly applies Keccak-256 to a 32-byte state. |
| [`sha3-ex`](./sha3-ex) | Computes a Keccak-256 digest using Jolt's Keccak inline implementation. |
| [`sig-recovery`](./sig-recovery) | Decodes EIP-2718 Ethereum transactions and recovers their transaction signers. |
| [`std-lib`](./std-lib) | Exercises string allocation and formatting by concatenating decimal representations of a sequence of integers. |
| [`tiny-keccak`](./tiny-keccak) | Computes SHA3-256 using the `tiny-keccak` crate. |

## Corpus coverage

At a high level, the programs cover several classes of computation:

- **Arithmetic and control flow:** `collatz`, `modinv`, `muldiv`
- **Memory, allocation, and runtime behaviour:** `alloc`, `backtrace`, `btreemap`, `memory-ops`, `overflow`, `random`, `std-lib`
- **Hashing and field arithmetic:** `crypto-bigint`, `hash-bench`, `poseidon`, `sha2-chain`, `sha2-ex`, `sha256`, `sha3-chain`, `sha3-ex`, `tiny-keccak`
- **Merkle-tree operations:** `merkle-leaf-proof`, `merkle-tree`, `merkle-tree-save`
- **Public-key cryptography:** `p256-ecdsa-verify`, `recover-ecdsa`, `secp256k1-ecdsa-verify`
- **Blockchain-oriented workloads:** `sig-recovery`
- **Jolt-specific functionality:** `advice-demo`

These programs are not all performance benchmarks in the strict sense; several are examples or feature-oriented tests. In this repository, they are treated uniformly as a **program corpus** for compiler-impact analysis.