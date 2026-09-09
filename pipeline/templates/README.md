# Templates

Frozen scaffolding that the pipeline copies for every program it tests.

## `guest-template/`

A complete, buildable two-crate Jolt workspace — a `guest` crate and a `host`
driver — that every program under test is ported *into*. For each program the
pipeline copies this directory to a scratch work tree and hands the copy to the
guest-porter agent, which fills in the harness body and adjusts the guest's
dependencies. The template itself is never modified by a run.

```text
guest-template/
├── Cargo.toml            workspace + [patch.crates-io] for the arkworks fork
├── rust-toolchain.toml   pinned host toolchain
├── NOTES.md              the resolved Jolt SDK API this template is built on
├── guest/src/lib.rs      the frozen `harness` provable function (porter fills the body)
├── guest/src/main.rs     `#![no_main]` guest entry
└── host/src/main.rs      the differential driver
```

Why it is frozen:

* **One shared source tree.** Both differential arms compile the same tree; they
  differ only in the guest toolchain or optimisation flags set outside the
  source. Anything program-specific has to live in the harness body, not in the
  scaffolding.
* **One stable I/O contract.** The provable function is always
  `fn harness(input: Vec<u8>) -> Vec<u8>`, and the host prints exactly one line
  on stdout (`output=<hex>|panic=<bool>|…`). Every script in the pipeline parses
  that line, so the shape has to hold across every issue and every program.
* **Codegen-neutral settings.** The release profile matches `jolt new`'s output,
  with optimisation deliberately left on — a harness hunting compiler
  miscompilations wants the buggy optimiser engaged.

The host supports two mutually exclusive modes:

```bash
cargo run --release -p host -- --emulate --input <PATH> [--trace-fn <FN_PATH>]
cargo run --release -p host -- --prove --verify --input <PATH>
```

`--emulate` is the prover-free path the pipeline uses for both the
instrumentation gate and Stage-3 differential execution; `--prove --verify` is
the manual confirmation path for a divergence that has already been found.

## Before first use

`guest/Cargo.toml` and `host/Cargo.toml` pin `jolt-sdk` by **absolute path** to a
local Jolt checkout, so that the guest runtime, the host prover and the installed
`jolt` CLI are all the same version. Repoint both at your own checkout before
running the pipeline.

See `NOTES.md` for the exact Jolt SDK identifiers this template depends on, with
source citations, and for how `--emulate` and `--prove` are kept byte-identical
in their `output=` field.
