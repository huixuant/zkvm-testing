# Pipeline scripts

These scripts run the differential miscompilation-testing pipeline: for one rustc
issue, sweep a corpus of Jolt guest programs and report which programs the
compiler fault *fires* in, which of those fired functions are *reachable* from
the guest entry point, and which produce a *behavioural divergence* between the
buggy and the baseline compiler arm.

```text
run_issue.sh              sweep one issue over a whole program list (entry point)
 └─ run_pair_full.sh      one program end-to-end: Stage 1 → 2 → 3 → triage
     └─ run_pair_common.sh  sourced library: CLI/spec parsing, port loop, helpers
         ├─ gen_inputs.sh          input generation (agent-driven)
         ├─ gate_instrumentation.sh Stage 1: did the fault fire at compile time?
         └─ reachable_fns.sh        Stage 2: static reachability from the harness
             └─ jolt_callgraph_static.py  call graph from ELF disasm + DWARF + vtables
```

`run_pair_common.sh` is a library — source it, never run it directly. All three
files must stay in the same directory.

---

## 1. Prerequisites

Install these and make sure they are on `PATH`:

| Tool | Used by | Notes |
| --- | --- | --- |
| `bash` (≥ 4.4) | all | needs associative arrays and `mapfile` |
| `yq`, `jq` | all | YAML spec reads and JSON result cards |
| `cargo`, `rustup` | all | host and guest builds |
| `python3` | reachability, agent JSON parsing | no third-party packages needed |
| `claude` | port / input-generation / triage agents | the Claude Code CLI |
| `llvm-objdump`, `llvm-dwarfdump` | `reachable_fns.sh` | e.g. `apt install llvm` |
| `rustfilt` | `reachable_fns.sh` | `cargo install rustfilt` |
| `cargo-download` | `run_issue.sh`, `crate` rows only | `cargo install cargo-download` |
| `jolt` CLI | guest builds | `cargo install --path .` in the Jolt checkout |

You also need a working Jolt checkout, plus the ZeroOS guest-build wrapper that
the specs' `mir_opt_toggle.build_rs` points at.

## 2. Install the instrumented toolchains

Each spec names a `buggy_toolchain` (e.g. `patch-137646`, `b5e10d8c00-rustc`).
That is a **rustup toolchain name**, selected for the guest build only via the
`ZEROOS_GUEST_TOOLCHAIN` environment variable; the host always stays on the
default toolchain.

Each buggy toolchain is a rustc build that (a) still contains the miscompilation
and (b) carries an *instrumentation patch* that appends a line such as
`MISCOMP rustc-137646 ...` to the file named by `$MISCOMP_LOG` whenever the
faulty transformation fires. Build each patched rustc, then link it in:

```bash
rustup toolchain link patch-137646 /path/to/rust/build/host/stage1
rustup which --toolchain patch-137646 rustc     # must succeed before running
```

The pipeline aborts early if `rustup which` cannot resolve the toolchain.

## 3. Point the specs and the template at your machine

Three things carry absolute, machine-specific paths and **must** be edited:

1. `pipeline/specs/rustc-*.yaml` — for `mir_opt_level` issues only, the
   `mir_opt_toggle` block:

   ```yaml
   mir_opt_toggle:
     build_rs: /path/to/zeroos-local/crates/zeroos-build/src/cmds/build.rs
     jolt_dir: /path/to/jolt
     rebuild_cmd: 'cargo install --path .'
     flag_line_marker: 'PIPELINE_TOGGLE_rustc-143491'
     buggy_state: commented        # or: mode: value + buggy_value/normal_value
   ```

   `flag_line_marker` must be a unique sentinel comment placed at the **end** of
   exactly one line in `build_rs` — the line carrying the `-Zmir-opt-level` flag.
   The pipeline toggles that line and re-runs `rebuild_cmd` to switch arms.

2. `pipeline/templates/guest-template/{guest,host}/Cargo.toml` — both pin
   `jolt-sdk` by absolute path to a local Jolt checkout. Repoint them at yours.

3. `run_issue.sh --template` — pass the template directory you just edited.

## 4. Write a programs list

`run_issue.sh --programs` takes a TSV file. It is **not** checked in; create it.
One program per line, tab-separated, `#` comments and blank lines ignored:

```text
<name>	<mode>	<source>
```

* `mode = crate` — `source` is `name@version`; fetched with `cargo download`.
  The version is mandatory.
* `mode = benchmark` — `source` is a **local path** to one guest program
  directory, or a git URL to a single-program repo.

For a monorepo, clone it once by hand and add one local-path row per
sub-directory; a bare repo URL clones everything into a single directory.

The checked-in corpus is already in benchmark shape, so the usual list is:

```bash
for d in pipeline/corpus/*/; do
  n=$(basename "$d"); [ -d "$d/src" ] && printf '%s\tbenchmark\t%s\n' "$n" "$d/src"
done > pipeline/corpus/programs.tsv
```

## 5. Run a sweep

From the `pipeline/` directory:

```bash
./scripts/run_issue.sh \
  --spec      specs/rustc-137646.yaml \
  --programs  corpus/programs.tsv \
  --template  templates/guest-template \
  --inputs-root inputs \
  --out       findings \
  --jobs      1
```

**Keep `--jobs 1`.** The template host builds the guest into a fixed path
(`/tmp/jolt-guest-targets-harness`), so concurrent jobs clobber each other's
guest ELF and silently corrupt results.

The sweep is resumable: a program whose result card already exists is skipped.
Delete `findings/<issue>__<program>.json` to re-run just that program.

### What one program run does

| Stage | Owner | Question | Outcome |
| --- | --- | --- | --- |
| Port | agent | Can the program be adapted into the frozen guest harness? | `build_failed` if not |
| Inputs | agent | What inputs does the program actually take? | real harvest, then synthetic top-up |
| Stage 1 | `gate_instrumentation.sh` | Did the fault **fire** at compile time? | `pass_not_fired` if not |
| Stage 2 | `reachable_fns.sh` | Is any fired function **reachable** from the harness? | `no_reachable_fired` if not |
| Stage 3 | `run_pair_full.sh` | Do the two arms produce **different** output? | `no_divergence` if not |
| Triage | agent | What explains the divergence? | `confirmed_miscompile` / `unattributed` |

Stage 2 is skipped for specs with `bug_layer: others`, where the instrumentation
cannot attribute the fire to a function; those go straight to Stage 3.

## 6. Results

Written under `--out` (default `findings/`):

* `<issue>__<program>.json` — one result card per program (verdict, fired
  functions, reachable count, both arms' outputs, notes).
* `<issue>__<program>.port.diff` — the porter's diff against the template.
* `funnel-<issue>.json` — the aggregate funnel (`programs → fired → gate passed
  → diverged → confirmed`), also printed at the end of the sweep.
* `jolt-rebuild-{buggy,normal}.log` — `mir_opt_level` rebuild logs.

## 7. Environment knobs

| Variable | Default | Effect |
| --- | --- | --- |
| `MAX_PORT_ATTEMPTS` | `3` | porter build-repair attempts before giving up |
| `PORT_CACHE` | `1` | reuse a previously confirmed port (`0` disables) |
| `PORT_CACHE_DIR` | `<scripts>/ported` | where confirmed ports are cached |
| `PORT_CACHE_VERIFY` | `1` | re-port if a cached tree no longer builds |
| `GENINPUTS_MIN` | `8` | set to `0` to skip agent input generation entirely |
| `STEERED_MIN` / `STEERED_MAX` | `8` / `12` | input-suite target size / budget |
| `STAGE3_MAX_INPUTS` | `12` | cap on Stage-3 inputs (set to `""` for no cap) |
| `JOLT_FORCE_REBUILD` | `0` | always rebuild jolt on a config switch |
| `JOLT_STATE_FILE` | `<jolt_dir>/.pipeline-jolt-config` | records the installed jolt's arm |
| `RESTORE_BUILD_RS` | `0` | restore `build.rs` on exit (invalidates the state cache) |
| `SHOW_CALLGRAPH` | `1` | print callgraph diagnostics to stderr |
| `BUGGY_OPT` / `NORMAL_OPT` | `3` / `0` | per-arm `-Copt-level` under `copt_level` |

## 8. Running a single stage by hand

```bash
# Stage 1 only, against an already-ported guest tree:
./scripts/gate_instrumentation.sh --spec specs/rustc-137646.yaml \
  --guest /path/to/ported/src --input inputs/foo/host-a.bin --keep-log
# exit 0 = fired (fired paths on stdout), 10 = did not fire, 3 = build failed

# Reachability only, against a built guest ELF:
./scripts/reachable_fns.sh --elf /tmp/jolt-guest-targets-harness/.../guest \
  --root harness --workdir /tmp/cg
```

## 9. Before you run: known blockers
* the three agent role files are looked up at `<scripts>/agents/…` but live at
  `pipeline/agents/…`, so every port aborts with "agent role file not found".
  To fix without touching the scripts, move all seven of them (`*.sh` and
  `jolt_callgraph_static.py`) up into `pipeline/`, so that the directory they
  resolve paths against becomes the one holding `agents/` — then invoke them as
  `./run_issue.sh …` rather than `./scripts/run_issue.sh …`.