# Issue specs

One YAML file per rustc issue under test, named `rustc-<issue>.yaml`. A spec is
the pipeline's entire description of a compiler fault: which toolchain carries
it, how to tell when it fires, and how to build the two arms that get compared.
`run_issue.sh --spec <file>` takes exactly one of these and sweeps the whole
program corpus against it.

Everything the scripts know about an issue comes from here — there is no
per-issue code anywhere else.

## Fields

### Identity

| Field | Meaning | Required |
| --- | --- | --- |
| `id` | Issue identifier, e.g. `rustc-143491`. Names every result card (`<id>__<program>.json`) and the funnel summary. | **Yes** |
| `upstream.rust_issue` | Link to the upstream issue. Documentation only. | No |
| `riscv_triggering` | Whether the fault was confirmed to trigger on `riscv64imac`. Documentation only, but a spec marked otherwise makes negative results meaningless. | No |

### Toolchains

| Field | Meaning | Required |
| --- | --- | --- |
| `buggy_toolchain` | rustup toolchain name of the instrumented, still-buggy rustc. Selected for the **guest** build only, via `ZEROOS_GUEST_TOOLCHAIN`; the host stays on the default toolchain. | **Yes** |
| `fixed_toolchain` | Records the baseline (`default` = stock guest toolchain). Documentation only — no script reads it; the baseline arm is produced by `opt_toggle`, not by switching toolchain. | No |

### Instrumentation contract

How Stage 1 decides the fault *fired*. The patched rustc appends a line to a log
whenever the faulty transformation runs; these fields say where to look and what
to look for.

| Field | Meaning | Required |
| --- | --- | --- |
| `instrumentation.env` | Environment variable naming the log path, in practice `MISCOMP_LOG`. Set on the guest compile. | **Yes** |
| `instrumentation.signal` | Where the signal lands: `file` (the log named by `env`) or `stderr`. Anything else is rejected. | **Yes** |
| `instrumentation.match` | Regex identifying a fired detection, e.g. `'^MISCOMP rustc-143491'`. Must be unique to this issue — one instrumented toolchain can host several detectors. The gate refuses to run without it. | **Yes** |
| `instrumentation.pass` | Internal label for the instrumented pass. Documentation only. | No |
| `instrumentation.layer` | Compiler layer that emits the signal (`mir`, `llvm`, `ctfe`, …). Read only as a fallback for `bug_layer`. | No |

### Arm construction

`opt_toggle` decides how the **baseline** arm is produced — this is the single
most consequential field in the file.

| Field | Meaning | Required |
| --- | --- | --- |
| `opt_toggle` | `copt_level` — both arms use the buggy toolchain and differ only in optimisation level (`JOLT_GUEST_OPT`, buggy `3` / baseline `0`); no rebuild needed. `mir_opt_level` — the arms differ by `-Zmir-opt-level`, which means editing `build.rs` and reinstalling `jolt` between arms. | **Yes in practice** |
| `bug_layer` | `mir` / `llvm` — the fired functions are attributable, so Stage 2 gates on fired ∩ reachable. `others` — attribution is impossible (e.g. const-eval), so Stage 2 is skipped and Stage 3 runs on the fire alone. Falls back to `instrumentation.layer`, then `mir`. | No (defaults to `mir`) |

`opt_toggle` must **always be set explicitly.**

### `mir_opt_toggle` — required only when `opt_toggle: mir_opt_level`

Ignored entirely under `copt_level`. Describes how to flip the installed `jolt`
between the two arms; all four of the first block are mandatory, and the paths
are absolute and machine-specific.

| Field | Meaning | Required |
| --- | --- | --- |
| `build_rs` | Path to the source file holding the `-Zmir-opt-level` flag line. | **Yes** |
| `flag_line_marker` | Unique sentinel substring placed at the **end** of that one line, e.g. `PIPELINE_TOGGLE_rustc-143491`. The pipeline refuses to run if it is missing or matches nothing. | **Yes** |
| `jolt_dir` | Jolt checkout to rebuild from. | **Yes** |
| `rebuild_cmd` | Command run in `jolt_dir` to reinstall, normally `cargo install --path .`. | **Yes** |
| `mode` | `comment` (default) — the arms differ by whether the flag line is commented out. `value` — the arms differ by the numeric level on the line. | No |
| `buggy_state` | `comment` mode only: which comment state is the buggy arm, `commented` or `uncommented`. Unknown values warn and fall back to `commented`. | Under `mode: comment` |
| `buggy_value` / `normal_value` | `value` mode only: the `-Zmir-opt-level` numbers for each arm (defaults `2` / `0`). Use this when commenting the line out would leave rustc's default level, which may still trigger the fault. | Under `mode: value` |

`buggy_state` and the `value`-mode fields are mutually exclusive in effect —
whichever `mode` selects wins, and the other is ignored. One checked-in spec
carries both; only the `mode: value` fields take effect there.

## Minimum viable spec

```yaml
id: rustc-XXXXXX
buggy_toolchain: patch-XXXXXX
opt_toggle: copt_level
bug_layer: mir
instrumentation:
  signal: file
  env: MISCOMP_LOG
  match: '^MISCOMP rustc-XXXXXX'
```

That is enough to run a full sweep. Add `mir_opt_toggle` only if you set
`opt_toggle: mir_opt_level`; everything else in the existing specs is commentary
for the reader, not input to the scripts.

## Adding a new issue

1. Build an instrumented rustc that still contains the fault and emits a line
   matching your `match` regex, then `rustup toolchain link` it.
2. Copy the closest existing spec, set `id`, `buggy_toolchain` and `match`.
3. Choose `opt_toggle`: prefer `copt_level` — it needs no rebuilds and is far
   faster. Use `mir_opt_level` only when the fault cannot be turned off by
   optimisation level alone.
4. Set `bug_layer` to `others` if the instrumentation cannot name the function
   that fired; otherwise leave it at the compiler layer.
5. Confirm the fault reproduces on `riscv64imac` before trusting any negative
   result, and record that in `riscv_triggering`.
