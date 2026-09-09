#!/usr/bin/env bash
# gen_inputs.sh - THE single entry point for input generation.
#
# One --source argument selects WHAT to produce; gen_inputs.sh owns the mapping
# from source -> mechanism (shell vs agent) and, for agent sources, the exact
# prompt scoping. Nothing else in the pipeline calls the input-generator agent
# directly - everything goes through here, so "what a source means" is defined
# in ONE place.
#
# All input generation is AGENT-based (no shell/structural source).
# Sources:
#   real         AGENT. The program's OWN inputs. For a benchmark: reconstruct
#                the host/driver's inputs (host-*.bin). For a crate: the test/
#                doctest/bench vectors (vec-*.bin). Needs decode-layout; does
#                NOT need fired-fn (runs before the gate). If run with no fired
#                fn and no real inputs exist, use 'steered' unsteered instead.
#   steered      AGENT. Synthetic inputs (boundary + seeded random) matching the
#                decode layout, biased to REACH the fired function when a
#                --fired-fn is given (edge-*.bin, rand-*.bin). Without --fired-fn
#                it generates broadly (unsteered) - used as the pre-gate fallback
#                when no real inputs exist. This is the ONLY synthetic source.
#   fuzz         Manual native coverage-guided procedure (printed, not run).
#
# Usage:
#   gen_inputs.sh <src-dir> <out-dir> --source real    --mode crate|benchmark \
#                                      --decode-layout <str>
#   gen_inputs.sh <src-dir> <out-dir> --source steered  --mode crate|benchmark \
#                                      --decode-layout <str> [--fired-fn <path>]
#   gen_inputs.sh <src-dir> <out-dir> --source fuzz
#
# Back-compat: --tier 1|2|3 accepted (1=real, 2=steered, 3=fuzz).
set -euo pipefail

SRC=""; OUT=""; SOURCE=""; DECODE_LAYOUT=""; FIRED_FN=""; MODE="crate"; MAX_FILES=""
[ $# -ge 2 ] && { SRC="$1"; OUT="$2"; shift 2; }
while [ $# -gt 0 ]; do
  case "$1" in
    --source)        SOURCE="$2";        shift 2 ;;
    --tier)          # back-compat numeric tiers (structural removed)
      case "$2" in 1) SOURCE="real";; 2) SOURCE="steered";; 3) SOURCE="fuzz";; *) SOURCE="$2";; esac
      shift 2 ;;
    --decode-layout) DECODE_LAYOUT="$2"; shift 2 ;;
    --fired-fn)      FIRED_FN="$2";      shift 2 ;;
    --mode)          MODE="$2";          shift 2 ;;
    --max-files)     MAX_FILES="$2";     shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
: "${SRC:?src dir required}"; : "${OUT:?output inputs dir required}"
: "${SOURCE:?--source required (real|steered|fuzz)}"
mkdir -p "$OUT"
HERE=$(cd "$(dirname "$0")" && pwd)
ROLE="$HERE/agents/inputs-generator.CLAUDE.md"

# ---------------------------------------------------------------------------
# Shared agent caller: scoped prompt in, role JSON out. This is the ONLY place
# the input-generator agent is invoked from, so prompt-scoping lives here.
# ---------------------------------------------------------------------------
run_input_agent () {   # $1 = task text (the source-specific scoping)
  [ -f "$ROLE" ] || { echo "[gen] role file not found: $ROLE" >&2; return 2; }
  command -v claude >/dev/null 2>&1 || { echo "[gen] 'claude' CLI not found" >&2; return 2; }
  local task="$1" errlog resp
  errlog=$(mktemp "${TMPDIR:-/tmp}/geninputs-XXXX.err")
  local budget_line=""
  if [ -n "$MAX_FILES" ]; then
    local _existing
    _existing=$(find "$OUT" -maxdepth 1 -type f 2>/dev/null | wc -l)
    budget_line="Soft file-count budget: aim for AT MOST ${MAX_FILES} input files in the output dir IN TOTAL. It already contains ${_existing} file(s) from earlier passes, so add only enough to approach (not exceed) ${MAX_FILES}. Prefer fewer, higher-value inputs over many near-duplicates."
  fi
  resp=$(claude -p "$(cat <<EOF
Task: $task

Original program source (READ-ONLY): $SRC
Input mode: $MODE
Output directory (WRITE .bin files here): $OUT
Decode layout (encode values to match this EXACTLY):
${DECODE_LAYOUT:-<infer from the provable/entry function signature in the source>}
Fired function path(s) for context (steer across ALL of them, not one; optional):
${FIRED_FN:-<none provided - use whole-program judgment; report coverage_note accordingly>}

Emit ONLY your role's JSON.
EOF
)" --system-prompt-file "$ROLE" \
     --permission-mode acceptEdits --allowedTools "Read,Write,Edit,Glob,Grep" \
     --output-format json 2>"$errlog") || true
  if [ -z "$resp" ]; then echo "[gen] WARNING: empty agent response; see $errlog" >&2; return 0; fi
  # unwrap CLI .result envelope, extract JSON object, print a one-line summary
  printf '%s' "$resp" | jq -r '.result // .' 2>/dev/null | python3 -c '
import sys, json, re
t=sys.stdin.read()
m=re.search(r"```(?:json)?\s*(\{.*?\})\s*```", t, re.S)
cand=m.group(1) if m else None
if cand is None:
    s=t.find("{"); e=t.rfind("}"); cand=t[s:e+1] if (s!=-1 and e>s) else None
try:
    o=json.loads(cand if cand else t)
    print("[gen] agent wrote %s file(s); coverage_note=%s%s"
          % (o.get("written","?"), o.get("coverage_note","?"),
             ("; notes: "+o["notes"]) if o.get("notes") else ""))
except Exception:
    print("[gen] could not parse agent JSON")
'
}

case "$SOURCE" in

  # -------------------------------------------------------------------------
  real)         # AGENT: the program's own inputs (host for benchmark, vectors for crate)
    if [ "$MODE" = "benchmark" ]; then
      run_input_agent "HOST-EXTRACTION ONLY (source 0 of your role). Reconstruct the inputs the program's own host/driver (host/src/main.rs, example/bench mains, and the helpers they call) actually passes to the provable function. Trace constructed/computed values to their concrete result; encode per decode_layout; write host-<desc>.bin. Do NOT generate structural or random inputs."
    else
      run_input_agent "HARVESTED-VECTORS ONLY (source 1 of your role). This is a LIBRARY crate (no host). Reconstruct the concrete inputs the crate's own tests, doctests, benches, and README examples pass to its primary API. Trace literals/constructors to their concrete bytes; encode per decode_layout; write vec-<desc>.bin. Do NOT generate structural or random inputs."
    fi
    ;;

  # -------------------------------------------------------------------------
  steered)      # AGENT: synthetic inputs from whole-program context, OOM-aware
    run_input_agent "STEERED GENERATION (sources 2 and 3 of your role). The 'real' pass harvested too few inputs, so generate ADDITIONAL synthetic inputs to broaden coverage. Consider the program AS A WHOLE - its entry/provable function, the code paths it exercises, and ALL functions that fired the instrumentation (not any single one) - and generate the inputs YOU judge most interesting: boundary/edge values (edge-<desc>.bin) and seeded-random valid inputs (rand-seed<N>-<desc>.bin), all matching decode_layout. Aim to exercise varied code paths across the fired functions and the program's core logic. Do NOT re-harvest host/test vectors (a prior 'real' pass already did). IMPORTANT: avoid inputs that would produce very large execution traces - large sizes/lengths, deep recursion, or high loop counts can exhaust memory during emulation. Prefer small-to-moderate inputs that still reach interesting behaviour; when a boundary calls for a 'large' size, use the smallest size that exercises it, not the maximum the layout allows."
    ;;

  # -------------------------------------------------------------------------
  fuzz)         # manual native coverage-guided procedure (printed, not run)
    echo "[fuzz] native coverage-guided corpus discovery (manual procedure)"
    cat <<'EOF'
  Run only if 'steered' left the candidate function unreached.
  1. cargo install cargo-fuzz
  2. fuzz target calling the SAME decode+call path as the guest:
       #![no_main]
       use libfuzzer_sys::fuzz_target;
       fuzz_target!(|data: &[u8]| { let _ = shared_decode::harness_body(data.to_vec()); });
  3. Seed with existing inputs:  mkdir -p fuzz/corpus/reach && cp "$OUT"/*.bin fuzz/corpus/reach/
  4. Discover briefly:           cargo fuzz run reach -- -max_total_time=300 -max_len=4096
  5. Minimise + harvest:         cargo fuzz cmin reach && cp fuzz/corpus/reach/* "$OUT"/
  6. Confirm reachability:       cargo llvm-cov --html   (is the candidate fn covered?)
  If step 6 shows it is never covered, record verdict=not_reached and move on.
EOF
    ;;

  *) echo "unknown --source: $SOURCE (want real|steered|fuzz)" >&2; exit 2 ;;
esac