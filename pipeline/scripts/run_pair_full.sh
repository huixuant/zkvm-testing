#!/usr/bin/env bash
# run_pair_full.sh - single-phase (full) pipeline: both arms built + diffed live,
# then differential execution. For copt_level issues and any run where you want
# the whole funnel in one process. Sources run_pair_common.sh for all setup,
# helpers, PORT, and reachability; this file is the live STAGE 1 -> 2 -> 3 funnel.
#
# Usage: see run_pair_common.sh for the flag list (--phase full is the default
# and the only supported value).
source "$(dirname "$0")/run_pair_common.sh"

# ============================================================================
# CONFIG SEQUENCE (mir_opt_level; no-op under copt_level):
#   The PORT loop in common just built the guest under the NORMAL config into
#   target-portcheck. Capture that ELF now for reachability (step 4 below), THEN
#   rebuild jolt to BUGGY for the Stage-1 gate. Stage 3 later rebuilds to NORMAL
#   for the normal arm. Net: 2 rebuilds per program (normal->buggy, buggy->normal).
# ============================================================================
# --- capture the NORMAL-config ELF for reachability (immune to later rebuilds) -
CALLGRAPH_ELF=""
if [ "$OPT_TOGGLE" = "mir_opt_level" ]; then
  _normal_elf=$(locate_guest_elf)
  if [ -n "$_normal_elf" ] && [ -f "$_normal_elf" ]; then
    CALLGRAPH_ELF="$work/callgraph-normal.elf"
    cp "$_normal_elf" "$CALLGRAPH_ELF"
    step "SETUP: captured normal-config ELF for reachability -> $CALLGRAPH_ELF"
  else
    step "SETUP: WARNING - no normal-config ELF found post-port; reachability will fall back to locate_guest_elf"
  fi
  # --- rebuild jolt to BUGGY for the compile-time gate -----------------------
  set_jolt_config buggy
fi


# ============================================================================
# REAL-INPUT HARVEST - capture the inputs the program ALREADY has (author's real
# cases; highest value, guaranteed to reach real code), before the gate so they
# feed every stage. Routed through gen_inputs.sh (the single input entry point),
# which scopes the agent to host-extraction (benchmark) or vector-harvest
# (crate) by --mode. Non-fatal. Set GENINPUTS_MIN=0 to disable agent input gen.
# ============================================================================
if [ "${GENINPUTS_MIN:-8}" -ne 0 ] && [ -x "$HERE/gen_inputs.sh" ]; then
  harvest_seed_inputs "REAL-INPUTS" any
fi

# choose one input file to drive the gate
mapfile -t INPUT_FILES < <(find "$INPUTS" -maxdepth 1 -type f | sort)
[ "${#INPUT_FILES[@]}" -gt 0 ] || die "no input files in $INPUTS after harvest+synthetic generation (cannot test; check the input-generator agent output and decode layout)"
# prefer a harvested real input (host-* from a benchmark, vec-* from a crate) to
# drive the gate, else any input. `|| true` so a no-match grep cannot abort under
# set -e (hand-authored inputs have neither prefix).
GATE_INPUT=$(printf '%s\n' "${INPUT_FILES[@]}" | grep -E '/(host|vec)-' | head -1 || true)
[ -z "$GATE_INPUT" ] && GATE_INPUT="${INPUT_FILES[0]}"
GATE_INPUT_ABS=$(cd "$(dirname "$GATE_INPUT")" && pwd)/$(basename "$GATE_INPUT")

# ###########################################################################
# ## STAGE 1 - instrumentation gate: did the fault FIRE? (compile-time)      #
# ###########################################################################
  step "STAGE 1: instrumentation gate (gate_instrumentation.sh)"
  set +e; FIRED_PATHS=$(run_gate "$GATE_INPUT_ABS"); GRC=$?; set -e
case "$GRC" in
  0)  # Keep ALL fired functions for the Stage-2 intersection.
      FIRED_LIST="$FIRED_PATHS"
      FIRED_COUNT=$(printf '%s\n' "$FIRED_LIST" | grep -c . || true)
      echo "[stage1] FIRED in $FIRED_COUNT function(s)"
      printf '%s\n' "$FIRED_LIST" | grep -v '^$' | sed 's/^/    fired: /' >&2 ;;
  10) write_card "pass_not_fired" false false "not_attempted" \
        "instrumentation did not fire for this program"; exit 0 ;;
  3)  write_card "build_failed" false false "not_attempted" \
        "stage1 emulate build/run failed"; exit 0 ;;
  *)  die "gate error rc=$GRC" ;;
esac

# ###########################################################################
# ## STAGE 2 - reachability
# ###########################################################################
# bug_layer routing:
#   llvm/mir -> gate on fired ∩ reachable alone, then Stage 3.
#   others -> Stage 3 directly. NOTE: this leaves fired_and_reachable empty, so
#   run_issue.sh's funnel does not count these programs as gate_passed.
if [ "$BUG_LAYER" = "others" ]; then
  # fired function info is not avail at compile time but we know that the warning
  # was triggered, so move on to differential testing anyway 
  FIRED_REACHABLE=""
  step "STAGE 2 (others): match signal fired but function attribution info is unavail. Proceeding to Stage 3 on the fired signal alone..."
else
  REACHABLE_SET=""
  compute_reachable_set

  # printf '%s\n' "$REACHABLE_SET" | grep -v '^$' | sed 's/^/    reachable: /' >&2

  compute_fired_reachable_intersect () {
    # stdin: fired function names, one per line
    # env:   REACHABLE_SET (newline-separated reachable names)
    # out:   fired names reachable under EITHER representation, sorted, unique
    REACHABLE_SET="$REACHABLE_SET" python3 -c "$(cat <<'PY'
import os, re, sys

# "<TYPE as TRAIT_PATH>::METHOD" -> alias "TRAIT_PATH::METHOD".
# Greedy on both sides: anchors on the LAST " as " and LAST ">::" so generic
# traits like <X as From<Y>>::from alias to "From<Y>::from".
TRAIT_IMPL_RE = re.compile(r"<.+ as (.+)>::(.+)")
CLOSURE_V0_RE = re.compile(r"::\{closure#\d+\}")
CLOSURE_ANY_RE = re.compile(r"(::\{closure#\d+\}|::\{\{closure\}\})+$")
IMPL_IDX_RE = re.compile(r"\{impl#\d+\}")

def erase_impl_segments(name: str) -> str:
    """Canonical form with impl identity erased:
    '{impl#N}' and '<impl Trait for Type>' both become '{impl}'."""
    name = IMPL_IDX_RE.sub("{impl}", name)
    out, i = [], 0
    while i < len(name):
        if name.startswith("<impl ", i):
            depth, j = 0, i
            while j < len(name):
                if name[j] == "<":
                    depth += 1
                elif name[j] == ">":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            out.append("{impl}")
            i = j + 1
        else:
            out.append(name[i]); i += 1
    return "".join(out)

def function_aliases(name: str) -> set:
    aliases = {name}
    m = TRAIT_IMPL_RE.fullmatch(name)
    if m:
        trait_path, method = m.groups()
        aliases.add(f"{trait_path}::{method}")
    if CLOSURE_V0_RE.search(name):
        aliases.add(CLOSURE_V0_RE.sub("::{{closure}}", name))
    parent = CLOSURE_ANY_RE.sub("", name)
    if parent != name:
        aliases.add(parent)
        pm = TRAIT_IMPL_RE.fullmatch(parent)
        if pm:
            aliases.add(f"{pm.group(1)}::{pm.group(2)}")
    erased = erase_impl_segments(name)
    if erased != name:
        aliases.add(erased)
    return aliases

reachable_aliases = set()
for n in os.environ.get("REACHABLE_SET", "").splitlines():
    n = n.strip()
    if n:
        reachable_aliases |= function_aliases(n)

hits = set()
for line in sys.stdin:
    n = line.strip()
    if n and function_aliases(n) & reachable_aliases:
        hits.add(n)

for n in sorted(hits):
    print(n)
PY
)"
}

  # ---- llvm OR mir: compute fired ∩ reachable ONCE. -------------------------
  set +e
  FIRED_REACHABLE=$(printf '%s\n' "$FIRED_LIST" | compute_fired_reachable_intersect)
  set -e
  REACH_FIRED_COUNT=$(printf '%s\n' "$FIRED_REACHABLE" | grep -vc '^$' || true)

  if [ "${REACH_FIRED_COUNT:-0}" -eq 0 ]; then
    step "STAGE 2: GATE - no fired∩reachable function; not proceeding"
    write_card "no_reachable_fired" true false "not_attempted" \
      "no function is simultaneously reachable-from-harness AND fired MISCOMP (fired∩reachable empty); Stage 3 skipped."
    exit 0
  fi
  step "STAGE 2: fired∩reachable = ${REACH_FIRED_COUNT:-0} fn(s) - GATE PASSED"
fi
step "STAGE 2: proceeding to Stage 3 (full phase)"

# ###########################################################################
# ## STAGE 3 - differential execution: did the fault CHANGE the output?      
# ###########################################################################
step "STAGE 3: differential execution over input suite"

# --- Order inputs by likelihood-of-divergence, and dedup identical files. -----
# Stage 3 breaks on the FIRST divergence, so running high-value inputs first
# makes a diverging program stop early (cheap) without dropping any input. Order:
# host-* (author's real case) > vec-* (test vectors) > rand-*/edge-* (synthetic).
# Dedup by content hash so identical files from different sources are run once.
# STAGE3_MAX_INPUTS caps the suite AFTER ordering, so a cap keeps the
# highest-value inputs. Unset means 12, NOT uncapped; set it to "" for no cap.
mapfile -t INPUT_FILES < <(order_and_dedup_inputs)
STAGE3_MAX_INPUTS="${STAGE3_MAX_INPUTS:-12}"
if [ "${STAGE3_MAX_INPUTS}" ] && [ "${#INPUT_FILES[@]}" -gt "$STAGE3_MAX_INPUTS" ]; then
  step "STAGE 3: capping suite ${#INPUT_FILES[@]} -> $STAGE3_MAX_INPUTS (highest-value kept)"
  INPUT_FILES=("${INPUT_FILES[@]:0:$STAGE3_MAX_INPUTS}")
fi

DIVERGED=false; DIV_INPUT=""; DIV_KIND=""; BOTH_PANIC=0; TESTED_OK=0; OOM_COUNT=0; OOM_INPUTS=""
echo "[stage3] ${#INPUT_FILES[@]} input(s), value-ordered - batched panic-aware divergence oracle"

# --- batched emulation: group by config so jolt rebuilds ONCE. --------------
# jolt is already BUGGY (from the Stage-1 gate). Emulate the BUGGY arm on every
# input first, collecting outputs; THEN rebuild jolt to NORMAL once and emulate
# the NORMAL arm on every input. Only after both arms are fully collected do we
# compare - so the config switch happens exactly once, not per input. (The old
# per-input `run_arm normal; run_arm buggy` loop never rebuilt between arms,
# which under mir_opt_level silently ran both "arms" on the same installed jolt.)
declare -A BUGGY_OUT NORMAL_OUT INP_ABS
for inp in "${INPUT_FILES[@]}"; do
  INP_ABS["$inp"]=$(cd "$(dirname "$inp")" && pwd)/$(basename "$inp")
done

step "STAGE 3: emulating BUGGY arm (jolt already buggy) over ${#INPUT_FILES[@]} input(s)"
for inp in "${INPUT_FILES[@]}"; do
  BUGGY_OUT["$inp"]=$(run_arm buggy "${INP_ABS[$inp]}")
done

# switch jolt to NORMAL for the baseline arm (one rebuild; no-op under copt_level)
[ "$OPT_TOGGLE" = "mir_opt_level" ] && set_jolt_config normal
step "STAGE 3: emulating NORMAL arm over ${#INPUT_FILES[@]} input(s)"
for inp in "${INPUT_FILES[@]}"; do
  NORMAL_OUT["$inp"]=$(run_arm normal "${INP_ABS[$inp]}")
done

# --- compare all pairs at once (value-ordered; first divergence wins) --------
for inp in "${INPUT_FILES[@]}"; do
  of="${NORMAL_OUT[$inp]}"; ob="${BUGGY_OUT[$inp]}"
  fk="${of%%:*}"; bk="${ob%%:*}"
  case "$(classify_pair "$of" "$ob")" in
    value-divergence)
      TESTED_OK=$((TESTED_OK+1))
      DIVERGED=true; DIV_KIND="value-divergence"; DIV_INPUT="${INP_ABS[$inp]}"; OB="$ob"; OF="$of"
      echo "[stage3] VALUE DIVERGENCE on $(basename "$inp")"; break ;;
    ok-same)
      TESTED_OK=$((TESTED_OK+1)) ;;
    oom)
      # exit 137 (OOM killer) on one or both arms: infrastructure limit, not a
      # divergence. Record it, keep going - do NOT break, do NOT trigger triage.
      OOM_COUNT=$((OOM_COUNT+1))
      _which=""
      [ "$fk" = OOM ] && _which="normal"
      [ "$bk" = OOM ] && _which="${_which:+$_which,}buggy"
      OOM_INPUTS="${OOM_INPUTS}$(basename "$inp") ($_which); "
      echo "[stage3] OUT-OF-MEMORY (exit 137) on $(basename "$inp") [${_which# } arm(s)] - logging and skipping (not a divergence)" ;;
    both-panic)
      BOTH_PANIC=$((BOTH_PANIC+1))
      echo "[stage3] both arms panicked on $(basename "$inp") - not a divergence, skipping" ;;
    panic-divergence)
      DIVERGED=true; DIV_KIND="panic-divergence"; DIV_INPUT="${INP_ABS[$inp]}"; OB="$ob"; OF="$of"
      echo "[stage3] PANIC DIVERGENCE on $(basename "$inp") (fixed=$fk buggy=$bk)"; break ;;
  esac
done

step "STAGE 3: DIVERGED=$DIVERGED${DIV_KIND:+ ($DIV_KIND)}  ok-compared=$TESTED_OK  both-panicked=$BOTH_PANIC  oom=$OOM_COUNT"
 
# WARNING: OOM_NOTE is built here but never passed to write_card or
# triage_divergence, so OOM'd inputs do NOT reach the card.
OOM_NOTE=""
[ "$OOM_COUNT" -gt 0 ] && OOM_NOTE=" | OUT-OF-MEMORY (exit 137) on $OOM_COUNT input(s): ${OOM_INPUTS} these were skipped (infrastructure limit, not a divergence) - consider smaller inputs."

if [ "$DIVERGED" = false ]; then
  # Distinguish "we compared real results and saw no difference" from "every
  # input was rejected by both arms" - the latter means the suite never actually
  # exercised the computation, so the negative is uninformative.
  if [ "$TESTED_OK" -eq 0 ] && [ "$BOTH_PANIC" -gt 0 ]; then
    write_card "no_divergence" true false "not_attempted" \
      "every input ($BOTH_PANIC) panicked on BOTH arms - the computation was never exercised to a normal result. Inputs are likely invalid per decode layout; regenerate with valid inputs."
  else
    write_card "no_divergence" true false "not_attempted" \
      "fault fired and a fired function was reachable; $TESTED_OK input(s) ran to a result on both arms with no difference ($BOTH_PANIC panicked on both, skipped). Consider richer/steered inputs."
  fi
  exit 0
fi

triage_divergence