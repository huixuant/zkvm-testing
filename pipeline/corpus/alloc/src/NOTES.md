# Template notes

## Resolved Jolt API (from the installed SDK, not guessed)

All identifiers below were read from the macro source in this checkout, not from
`cargo expand` guesswork. Citations are `file:line`.

| What | Identifier / path | Source |
|---|---|---|
| Guest fn | `harness` | `guest/src/lib.rs` |
| Prover-free emulation | `guest::compile_harness(target_dir) -> jolt::host::Program`, then `Program::trace_analyze::<F>(&input, &untrusted, &trusted)` | `jolt-sdk/macros/src/lib.rs:425,428`; `jolt-core/src/host/program.rs:349` |
| Generated analyze wrapper | `analyze_harness(..) -> jolt::host::analyze::ProgramSummary` | `jolt-sdk/macros/src/lib.rs:319,340,357` |
| Emulation result type | `ProgramSummary { trace: Vec<Cycle>, bytecode, memory_init, io_device: JoltDevice }` | `jolt-core/src/host/analyze.rs:11-17` |
| Output bytes | `summary.io_device.outputs` | `common/src/jolt_device.rs:28-34` |
| Panic flag | `summary.io_device.panic` | `common/src/jolt_device.rs:28-34` |
| Execution trace | `summary.trace` — **yes, reachable from the same value** | `jolt-core/src/host/analyze.rs:11-17` |
| Instruction PC | `cycle.instruction().normalize().address` | `tracer` — `normalize()` is on `Instruction`, not `Cycle` |
| Prove/verify | `preprocess_shared_harness`, `preprocess_prover_harness`, `preprocess_verifier_harness`, `build_prover_harness`, `build_verifier_harness` | `jolt-sdk/macros/src/lib.rs:467-468` |

The prover-free path **does** expose the output, so no fallback to proving was
needed. We call `compile_harness` + `trace_analyze` directly rather than
`analyze_harness` for one reason: we also need `program.elf` (the ELF path) for
symbol resolution, and `trace_analyze` consumes `self`. No prover is
instantiated on this path.

**Output decoding.** The prover decodes its return value by padding the raw
output region to `max_output_size` and then postcard-decoding
(`jolt-sdk/macros/src/lib.rs:643-647`). `decode_output()` replicates this exactly,
so `--emulate` and `--prove --verify` emit byte-identical `output=` hex.

## Sizing attribute names (exact)

From `jolt-sdk/macros/src/lib.rs`: `max_input_size`, `max_output_size`,
`stack_size`, `heap_size`, `max_trace_length`.

Used here: `#[jolt::provable(max_input_size = 65536, max_output_size = 65536, stack_size = 8388608)]`

## `--trace-fn` reachability

`jolt build` passes `-Cstrip=symbols` to the guest by default (`jolt/src/main.rs:238-241`),
which removes `.symtab` entirely and makes symbol resolution impossible. The host
sets `JOLT_BACKTRACE=1` before compiling, which opts out of that strip.

**This is codegen-neutral**, which is what makes it safe for a miscompilation
harness. Stripping is a link-time step, and the CLI deliberately keeps
`-Cforce-frame-pointers` (which *does* change generated code) on the separate
`--backtrace` path, never on `JOLT_BACKTRACE` (`jolt/src/main.rs:244-248`).
Verified empirically by matched-pair builds differing only in that variable:
`.text` is byte-identical (sha256 `37c587e5...`, 9328 bytes both ways).

`reached` semantics:
- `true` — a traced PC fell inside a resolved symbol range
- `false` — symbol(s) resolved, none entered
- `unknown` — no `--trace-fn`, or the name resolved to no symbol (e.g. fully inlined)

Because of `lto = "fat"` + `-Copt-level=3`, small functions are inlined and lose
their symbols; `harness` itself is inlined into `main`. Use `#[inline(never)]` on
the function you want to track.

## Why not the `cargo llvm-cov` methodology

Tested, not assumed. `-Cinstrument-coverage` fails outright on this target:

```
error[E0463]: can't find crate for `profiler_builtins`
  = note: the compiler may have been built without the profiler runtime
```

`profiler_builtins` is not shipped for `riscv64imac-unknown-none-elf`; obtaining it
would require `-Zbuild-std=core,alloc,profiler_builtins`, which is nightly-only
while this template pins stable 1.94. Even if built, the LLVM profiling runtime
dumps `.profraw` via filesystem syscalls that a Jolt guest does not have, so
counters would have to be scraped out of guest memory by hand.

The decisive objection is methodological rather than practical: instrumentation
inserts counter increments into the code being compiled, changing inlining and
optimization decisions — precisely the thing this harness is trying to observe.
It could mask or manufacture a miscompilation. Symbol-table resolution is
strictly better here because it is provably codegen-neutral.

## Commands

```bash
cargo build --release -p host

# Emulator only (no prover)
./target/release/host --emulate --input <PATH> [--trace-fn <FN_PATH>]
# -> output=<hex>|panic=<bool>|reached=<true|false|unknown>

# Full prove + verify
./target/release/host --prove --verify --input <PATH>
# -> output=<hex>|panic=<bool>|valid=<bool>
```

## Verified behavior

| Case | Result |
|---|---|
| `--emulate`, 10-byte input | `output=0a00000000000000\|panic=false\|reached=unknown` |
| `--trace-fn main` | `reached=true` |
| `--trace-fn core::fmt::write` | `reached=false` |
| `--trace-fn no::such::function` | `reached=unknown` |
| empty input | `output=0000000000000000` |
| 40000-byte input (64 KiB sizing) | `output=409c000000000000` |
| `--prove --verify` | `output=0a00000000000000\|panic=false\|valid=true` (~31 s) |
| bad args / missing file | exit 1, no stdout |
