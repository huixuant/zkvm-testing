# Pipeline agents

Each file here is a **system prompt** (a role definition) for one LLM agent used
by the pipeline. The scripts do all building, comparing and deciding; the agents
only do the parts that need judgment about unfamiliar source code — adapting a
program, inventing inputs for it, and explaining a result.

Every agent is invoked non-interactively through the Claude Code CLI:

```bash
claude -p "<task>" --system-prompt-file <role>.CLAUDE.md \
       --permission-mode acceptEdits --allowedTools "Read,Write,Edit,Glob,Grep" \
       --output-format json
```

Each role ends with a strict JSON schema and must emit **only** that object; the
calling script unwraps the CLI envelope, extracts the JSON, and falls back to
safe defaults if it cannot be parsed. No agent has shell access.

| Role file | Called by | Responsibility |
| --- | --- | --- |
| `guest-porter.CLAUDE.md` | `run_pair_common.sh` (port loop) | Adapt a program into the frozen guest harness |
| `inputs-generator.CLAUDE.md` | `gen_inputs.sh` | Produce the input corpus for one ported program |
| `divergence-triage.CLAUDE.md` | `run_pair_common.sh` (`triage_divergence`) | Explain a buggy-vs-baseline divergence |

## `guest-porter.CLAUDE.md`

Copies of the frozen guest template are handed to the porter with a program's
source (read-only). It fills in the harness body's three parts — decode the
input bytes, call the code under test, encode the result — and adjusts the guest
`Cargo.toml` so the dependency tree builds `no_std`.

Its single invariant is that **both differential arms compile one shared source
tree**: no toolchain-conditional code, no `cfg(version)`, no algorithm changes.
If a program genuinely cannot be made `no_std`, it must return
`verdict: "unbuildable"` rather than force it.

It is the only agent with write access, and only to the copied guest tree. It
runs in a build-verify-repair loop (`MAX_PORT_ATTEMPTS`, default 3): on a compile
failure it is re-invoked in *repair mode* with the trimmed compiler errors. It
also records the `decode_layout` — the exact byte layout the harness expects —
which is what steers input generation downstream. Confirmed ports are cached and
shared across issues on the same toolchain.

## `inputs-generator.CLAUDE.md`

Writes raw `.bin` input files matching the porter's `decode_layout`. It produces
inputs only, never expected outputs — the differential between the two compiler
arms is the oracle, so any valid input is a usable test.

It draws from four sources, in descending value, each with its own filename
prefix that the pipeline uses to rank inputs:

| Prefix | Source |
| --- | --- |
| `host-` | reconstructed from the program's own host/driver — the author's real case |
| `vec-` | harvested from the crate's tests, doctests, benches, README examples |
| `edge-` | structural and boundary values implied by the layout |
| `rand-seed<N>-` | a modest number of seeded-random valid inputs |

`gen_inputs.sh` scopes it to one source per call: a `real` pass (host extraction
for benchmarks, vector harvesting for crates) runs before the gate, and a
`steered` pass tops the suite up with synthetic inputs only if the real harvest
came back thin. Its hard rules are to keep the corpus small and high-value, and
to bound execution-trace size — oversized inputs exhaust memory during
differential emulation.

## `divergence-triage.CLAUDE.md`

Invoked only when Stage 3 found a divergence. It is meant to confirm the
divergence is stable, attribute it to the fired function along the
version-constant axis (hold the toolchain fixed, toggle the optimisation level),
explain the MIR-level change, and draft a root-cause hypothesis — never a final
call, which stays with the human. It must preserve the triggering case verbatim:
no minimisation, no delta-debugging, no reduced reproducer.

> **This agent was never properly exercised or tested in the pipeline.** No real
> divergence was ever found in the program corpus, so the role never ran in the
> job it was written for. The only times it fired were on the hand-written
> `repro-*` reproducer programs — known-positive controls that are guaranteed to
> diverge — and on a small number of panic-divergences that were not genuine
> miscompilations. Its output has therefore never been validated against a real
> corpus finding.
>
> It is kept here for completeness: the funnel is defined all the way through
> triage, and the role documents what that final stage is supposed to do. Treat
> its output, and the `confirmed_miscompile` verdict it can emit, as untested.
>
> Note also that the role assumes capabilities the harness does not currently
> give it — it is told to re-run the divergence, toggle `-Copt-level`, and read
> MIR dumps, but it is invoked without shell access and is handed only one arm's
> output. Anyone reviving this stage should reconcile the role with what
> `triage_divergence` actually passes it.
