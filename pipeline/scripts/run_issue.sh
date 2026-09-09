#!/usr/bin/env bash
# run_issue.sh - sweep a whole program list against ONE issue.
#
# Thin wrapper around the run_pair split scripts, which own the 3-stage pipeline
# (port -> Stage 1 gate -> Stage 2 MIR diff -> Stage 3 differential -> confirm ->
# triage):
#   run_pair_full.sh      - the single funnel entrypoint (copt_level and
#                           mir_opt_level; jolt config switching is internal).
# This script only: fetches sources, generates tier-1 inputs, dispatches each
# program to the right entrypoint, is resumable, and prints the Marcozzi FUNNEL
# summary (fired N -> mir-differs M -> diverged K -> proof-valid P).
#
# All toolchain / permission / build / agent logic lives in the run_pair scripts;
# this wrapper stays agnostic to it, so downstream changes don't require edits here.
# Both run_pair files (run_pair_common.sh, run_pair_full.sh) must sit in the
# SAME directory as this script - as must gen_inputs.sh, gate_instrumentation.sh,
# reachable_fns.sh and jolt_callgraph_static.py, which they resolve the same way.
#
# Usage:
#   ./run_issue.sh --spec specs/rustc-143491.yaml \
#                  --programs corpus/programs.tsv \
#                  --template templates/guest-template \
#                  [--jobs 1] [--inputs-root inputs] [--out findings]
#
# programs.tsv: one program per line, TAB-separated:  <name>\t<mode>\t<source>
#   mode   = crate | benchmark
#   source = crate     -> name@version            (fetched via cargo download)
#            benchmark -> a LOCAL PATH to a single guest program directory,
#                         OR a git URL to a single-program repo.
#   For a MONOREPO, clone it once by hand and add local-path rows per subdir
#   (a bare repo URL would clone everything into one dir - see programs.tsv).
#   blank lines and #-comments ignored.
#
# NOTE on parallelism: keep --jobs low (1-2). Each job runs its own guest builds
# and, more importantly, the guest build cache (jolt's target dir) may be a
# FIXED shared path - concurrent jobs could clobber each other's guest ELF and
# corrupt results. Run --jobs 1 unless you have verified the cache is per-job.
set -euo pipefail

SPEC=""; PROGRAMS=""; TEMPLATE=""; JOBS=1
INPUTS_ROOT="inputs"; OUT="findings"; SRC_CACHE="corpus/src"
while [ $# -gt 0 ]; do
  case "$1" in
    --spec)        SPEC="$2";        shift 2 ;;
    --programs)    PROGRAMS="$2";    shift 2 ;;
    --template)    TEMPLATE="$2";    shift 2 ;;
    --jobs)        JOBS="$2";        shift 2 ;;
    --inputs-root) INPUTS_ROOT="$2"; shift 2 ;;
    --out)         OUT="$2";         shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
die () { echo "FATAL: $*" >&2; exit 1; }
for b in yq jq; do command -v "$b" >/dev/null 2>&1 || die "missing dependency: $b"; done
for f in "$SPEC" "$PROGRAMS"; do [ -f "$f" ] || die "not found: $f"; done
[ -d "$TEMPLATE" ] || die "template not found: $TEMPLATE"
HERE=$(cd "$(dirname "$0")" && pwd)
# The pipeline is now split across two entrypoints (both source run_pair_common.sh):
#   run_pair_full.sh - the single funnel entrypoint (all opt_toggles)
[ -x "$HERE/run_pair_full.sh" ]     || die "run_pair_full.sh not found/executable next to this script"
[ -f "$HERE/run_pair_common.sh" ]   || die "run_pair_common.sh not found next to this script (sourced by both entrypoints)"
ISSUE_ID=$(yq -r '.id' "$SPEC")
mkdir -p "$OUT" "$SRC_CACHE" "$INPUTS_ROOT"

# --- opt-level baseline mechanism (Change 4) --------------------------------
# opt_toggle decides how the NORMAL/baseline arm is produced:
#   copt_level    -> per-build env (JOLT_GUEST_OPT=0), no rebuild
#   mir_opt_level -> edit build.rs + rebuild jolt, done INSIDE run_pair_full.sh
# Read here for the banner only. WARNING: run_pair_common.sh defaults the same
# field to mir_opt_level, so a spec that omits it is reported/run inconsistently.
OPT_TOGGLE=$(yq -r '.opt_toggle // "copt_level"' "$SPEC" 2>/dev/null || echo copt_level)


# fetch_source <mode> <source> -> echoes local dir (empty + rc1 on failure)
fetch_source () {
  local mode="$1" src="$2"
  if [ "$mode" = "crate" ]; then
    command -v cargo-download >/dev/null 2>&1 || { echo ""; return 1; }
    # Split declarations: `local a=.. b=$a` does NOT reliably see $a under set -u.
    local name ver dir
    name="${src%@*}"
    ver="${src#*@}"
    # if the row had no @version, src%@* == src#*@ == src; require an explicit version
    if [ "$ver" = "$src" ] || [ -z "$ver" ]; then
      echo "" >&2
      echo "[fetch] crate row '$src' has no @version (need name@version)" >&2
      echo ""; return 1
    fi
    dir="$SRC_CACHE/${name}-${ver}"
    if [ ! -d "$dir" ]; then
      # cargo download -x extracts INTO -o; point -o at a per-crate dir so the
      # layout is predictable ($SRC_CACHE/<name>-<ver>/), not scattered.
      local tmp; tmp=$(mktemp -d "$SRC_CACHE/.dl-XXXXXX")
      if cargo download "${name}==${ver}" -x -o "$tmp" >/dev/null 2>&1; then
        # cargo download may extract contents directly or into a subdir; normalise
        if [ -f "$tmp/Cargo.toml" ]; then mv "$tmp" "$dir";
        else mv "$tmp"/*/ "$dir" 2>/dev/null && rm -rf "$tmp"; fi
      else rm -rf "$tmp"; echo ""; return 1; fi
    fi
    [ -d "$dir" ] && echo "$dir" || { echo ""; return 1; }
  else
    if [ -d "$src" ]; then echo "$src"
    else  # git URL -> shallow clone into the cache (single-program repos only)
      local dir="$SRC_CACHE/$(basename "$src" .git)"
      [ -d "$dir" ] || git clone --depth 1 "$src" "$dir" >/dev/null 2>&1 \
        || { echo ""; return 1; }
      echo "$dir"
    fi
  fi
}

process_one () {
  local name="$1" mode="$2" src="$3"
  # NOTE: write_card builds this name with `tr '/@:' '___'`; a name containing
  # @ or : produces a mismatch here, so such a program is never skipped.
  local card="$OUT/${ISSUE_ID}__${name//\//_}.json"
  # Parallel path prints its own per-program banner here (the sequential loop
  # prints one WITH an i/total counter, and does not set _SWEEP_TOTAL, so this
  # does not double up).
  [ -n "${_SWEEP_TOTAL:-}" ] && {
    echo
    echo ">>>>>> [full] PROGRAM: $name (mode=$mode) <<<<<<"
  }
  # Resumability: a completed pair is never redone. Delete its card to retry.
  [ -f "$card" ] && { echo "[skip] $name (card exists)"; return 0; }

  echo "[run ] $name (mode=$mode)"
  local dir; dir=$(fetch_source "$mode" "$src") || true
  if [ -z "$dir" ] || [ ! -d "$dir" ]; then
    jq -n --arg g "$name" --arg i "$ISSUE_ID" \
      '{issue:$i, program:$g, verdict:"fetch_failed", ts:(now|todate)}' > "$card"
    echo "[fail] $name (fetch)"; return 0
  fi

  local inputs="$INPUTS_ROOT/${name}"
  # Inputs are generated INSIDE the run_pair funnel by the input-generator agent
  # (real harvest before the gate) - it needs the decode layout / port that only
  # exist post-port. So we no longer pre-seed a structural corpus here; just
  # ensure the directory exists. run_pair_full.sh will populate it. (A pre-existing
  # hand-authored corpus, e.g. the reproducer's, is left untouched and used as-is.)
  mkdir -p "$inputs"

  # run_pair_full.sh runs the whole funnel in one call (jolt config switching,
  # if any, happens inside run_pair_full via the shared machinery).
  "$HERE/run_pair_full.sh" --spec "$SPEC" --program-src "$dir" --program-name "$name" \
                      --mode "$mode" --template "$TEMPLATE" --inputs "$inputs" --out "$OUT" --keep-work \
    || echo "[err ] $name (run_pair_full exited non-zero; see its diagnostics)"
}
export -f process_one fetch_source
export SPEC TEMPLATE OUT INPUTS_ROOT SRC_CACHE ISSUE_ID HERE

mapfile -t ROWS < <(grep -vE '^\s*(#|$)' "$PROGRAMS")
[ "${#ROWS[@]}" -gt 0 ] || die "no program rows in $PROGRAMS"
echo "== sweeping ${#ROWS[@]} programs against $ISSUE_ID (jobs=$JOBS, opt_toggle=$OPT_TOGGLE) =="
run_row () { IFS=$'\t' read -r name mode src <<<"$1"; [ -n "${name:-}" ] && process_one "$name" "$mode" "$src"; }
export -f run_row


# ---- SWEEP: one run_pair_full call per program (single funnel; jolt config
# switching now happens INSIDE run_pair_full via the shared machinery). --------
  # Each program runs end-to-end in ONE run_pair_full.sh call. Under mir_opt_level
  # that call rebuilds jolt internally (buggy for the gate, normal for the
  # baseline arm); under copt_level no rebuilds happen. Progress signposting below.
  _total="${#ROWS[@]}"
  echo
  echo "########################################################################"
  echo "## sweep (opt_toggle=$OPT_TOGGLE): $_total program(s)"
  echo "########################################################################"
  if [ "$JOBS" -gt 1 ]; then
    # parallel: per-program banners come from process_one (no reliable counter
    # across concurrent jobs, so they omit the i/total index).
    export _SWEEP_TOTAL="$_total"
    printf '%s\n' "${ROWS[@]}" | xargs -d '\n' -P "$JOBS" -I{} bash -c 'run_row "$@"' _ {}
  else
    _i=0
    for r in "${ROWS[@]}"; do
      IFS=$'\t' read -r _n _m _s <<<"$r"; [ -n "${_n:-}" ] || continue
      _i=$((_i+1))
      echo
      echo ">>>>>> [full $_i/$_total] PROGRAM: $_n (mode=$_m) <<<<<<"
      process_one "$_n" "$_m" "$_s"
      echo "[full $_i/$_total] $_n - done"
    done
  fi

# NOTE: no pass_not_fired row (the largest bucket); and gate_passed keys on
# fired_and_reachable, which bug_layer=others never populates.
echo; echo "== funnel for $ISSUE_ID (Marcozzi propagation) =="
jq -s --arg i "$ISSUE_ID" '
  map(select(.issue==$i)) | {
    programs:              length,
    fetch_failed:          map(select(.verdict=="fetch_failed"))|length,
    build_failed:          map(select(.verdict=="build_failed"))|length,
    stage1_fired:          map(select(.stage1_fired==true))|length,
    reachable_computed:    map(select((.reachable_count // null) != null))|length,
    gate_passed:           map(select(((.fired_and_reachable // []) | length) > 0))|length,
    gated_out_no_reach_fired: map(select(.verdict=="no_reachable_fired"))|length,
    stage3_diverged:       map(select(.stage3_diverged==true))|length,
    no_divergence:         map(select(.verdict=="no_divergence"))|length,
    confirmed_miscompile:  map(select(.verdict=="confirmed_miscompile"))|length
  }' "$OUT"/${ISSUE_ID}__*.json 2>/dev/null | tee "$OUT/funnel-${ISSUE_ID}.json"

echo; echo "Programs that fired+differed but did NOT diverge (candidates for richer/steered inputs):"
jq -sr --arg i "$ISSUE_ID" \
  'map(select(.issue==$i and .verdict=="no_divergence")) | .[].program' \
  "$OUT"/${ISSUE_ID}__*.json 2>/dev/null | sed 's/^/  /' || true