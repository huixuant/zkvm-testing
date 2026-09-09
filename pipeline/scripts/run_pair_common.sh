# run_pair_common.sh - shared library for run_pair_full.sh: CLI parse, spec
# reads, opt/bug-layer helpers, ALL functions, PORT loop, reachability.
# SOURCE this; do not run it directly.
# State is shared via globals by design; this file sets `set -euo pipefail`.
#
# ARCHITECTURE (2 files, all in the same directory):
#   run_pair_common.sh    (this file) - sourced prelude: parses the CLI, reads
#                         the spec, defines every helper, runs the PORT loop and
#                         reachability. Sourcing it leaves the guest ported and
#                         all globals/functions ready for a funnel to run.
#   run_pair_full.sh      - the only entrypoint: sources this, then runs the
#                         LIVE STAGE 1 -> 2 -> 3 -> TRIAGE funnel.
#   (run_pair_twophase.sh was removed; the store-backed phases below are dead.)
#
# PIPELINE CONCEPT (Marcozzi et al. OOPSLA'19, adapted to Jolt + instrumentation):
#   PORT    (agent)  : adapt the program into the frozen guest template.
#   STAGE 1 (script) : instrumentation gate - did the fault FIRE? (compile-time)
#   STAGE 2 (script) : static reachability - is a fired fn reachable from the
#                      harness? (funnel-narrowing, not a verdict)
#   STAGE 3 (script) : differential execution - buggy vs fixed emulate output.
#   TRIAGE  (agent)  : explain a confirmed divergence (hypothesis only).
# Toolchain: buggy = ZEROOS_GUEST_TOOLCHAIN=<name> (guest only); fixed = unset.
# LLMs port and explain; scripts build/compare/decide. Prove+verify is manual.
#
# THIS FILE, top to bottom: CLI parse + validation -> spec reads + opt/bug-layer
# helpers -> helper definitions (cards/agents, build, port, inputs, gate/oracle)
# -> PORT loop -> reachability. The entrypoints source it and then run a funnel.
set -euo pipefail

SPEC=""; PROG_SRC=""; PROG_NAME=""; MODE="crate"; TEMPLATE=""
INPUTS=""; OUT="findings"; KEEP_WORK=0; SKIP_CONTROL=0
TARGET="riscv64imac-unknown-none-elf"
# DEAD: only PHASE=full is reachable now; the two-phase entrypoint was removed,
# so CONFIG/MIRSTORE and the generate/diff/emulate/compare phases are never used.
PHASE="full"       # full = single-phase (both arms + diff), the only live value
CONFIG=""          # dead: buggy | normal (generate phase)
MIRSTORE=""        # dead: store dir for two-phase batching
while [ $# -gt 0 ]; do
  case "$1" in
    --spec)         SPEC="$2";      shift 2 ;;
    --program-src)  PROG_SRC="$2";  shift 2 ;;
    --program-name) PROG_NAME="$2"; shift 2 ;;
    --mode)         MODE="$2";      shift 2 ;;   # crate | benchmark
    --template)     TEMPLATE="$2";  shift 2 ;;
    --inputs)       INPUTS="$2";    shift 2 ;;
    --out)          OUT="$2";       shift 2 ;;
    --target)       TARGET="$2";    shift 2 ;;   # dead: TARGET is never read
    --phase)        PHASE="$2";     shift 2 ;;   # full | generate | diff
    --config)       CONFIG="$2";    shift 2 ;;   # buggy | normal (generate phase)
    --mirstore)     MIRSTORE="$2";  shift 2 ;;   # store dir for two-phase
    --keep-work)    KEEP_WORK=1;    shift ;;
    --skip-control) SKIP_CONTROL=1; shift ;;     # dead: never read
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
case "$PHASE" in full|generate|diff|emulate|compare) ;; *) echo "bad --phase: $PHASE" >&2; exit 2 ;; esac
if [ "$PHASE" = "generate" ] || [ "$PHASE" = "emulate" ]; then
  case "$CONFIG" in buggy|normal) ;; *) echo "--phase $PHASE needs --config buggy|normal" >&2; exit 2 ;; esac
  [ -n "$MIRSTORE" ] || { echo "--phase $PHASE needs --mirstore <dir>" >&2; exit 2 ; }
fi
{ [ "$PHASE" = "diff" ] || [ "$PHASE" = "compare" ]; } && { [ -n "$MIRSTORE" ] || { echo "--phase $PHASE needs --mirstore <dir>" >&2; exit 2; }; }

die () { echo "FATAL: $*" >&2; exit 1; }
need () { command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1"; }
# Diagnostic step logger: timestamped, to stderr, so progress is visible live.
step () { echo ">>> [$(date +%H:%M:%S)] $*" >&2; }
for b in yq jq cargo python3 claude; do need "$b"; done
HERE=$(cd "$(dirname "$0")" && pwd)
[ -f "$SPEC" ]       || die "spec not found: $SPEC"
[ -d "$PROG_SRC" ]   || die "program source not found: $PROG_SRC"
[ -d "$TEMPLATE" ]   || die "template not found: $TEMPLATE"
[ -n "$PROG_NAME" ]  || die "--program-name required"
[ -d "$INPUTS" ]     || die "inputs dir not found: $INPUTS"
mkdir -p "$OUT"

# ZEROOS guest-toolchain helper (buggy sets the var; fixed omits it).
guest_tc_env () { case "$1" in default|""|null) printf '' ;;
                               *) printf 'ZEROOS_GUEST_TOOLCHAIN=%s' "$1" ;; esac; }
tc_tag () { local t="${1:-default}"; echo "${t//[^A-Za-z0-9._-]/_}"; }

ISSUE_ID=$(yq -r '.id'              "$SPEC")
BUGGY=$(yq   -r '.buggy_toolchain'  "$SPEC")

# --- optimisation-toggle model (spec.opt_toggle) ---------------------------
# copt_level    : the two arms use the SAME (buggy) toolchain and differ ONLY in
#                 optimisation level - buggy = high (default 3), normal = low (0).
#                 The level is set via JOLT_GUEST_OPT on `jolt build`, and via
#                 -Copt-level=<n> in RUSTFLAGS on plain cargo commands (rustc/run).
#                 JOLT_GUEST_OPT is NOT set on plain cargo commands.
# mir_opt_level : the arms differ by -Zmir-opt-level (handled in
#                 emit_wholeguest_mir); the helpers below are NO-OPS so the
#                 #143491 two-phase flow is completely unchanged.
# WARNING: run_issue.sh defaults the same field to copt_level. A spec that omits
# opt_toggle is reported as one mode and run as the other - always set it.
OPT_TOGGLE=$(yq -r '.opt_toggle // "mir_opt_level"' "$SPEC" 2>/dev/null || echo mir_opt_level)
BUGGY_OPT="${BUGGY_OPT:-3}"     # buggy-arm optimisation level (copt_level)
NORMAL_OPT="${NORMAL_OPT:-0}"   # normal/baseline-arm optimisation level (copt_level)
BUG_LAYER=$(yq -r '.bug_layer // .instrumentation.layer // "mir"' "$SPEC" 2>/dev/null || echo mir)
case "$BUG_LAYER" in mir|llvm|others) ;; *) BUG_LAYER=mir ;; esac

# _opt_for <buggy|normal> -> numeric opt level for that arm
_opt_for () { case "$1" in buggy) printf '%s' "$BUGGY_OPT" ;; normal) printf '%s' "$NORMAL_OPT" ;; *) printf '%s' "$BUGGY_OPT" ;; esac; }

# jolt_opt_env <buggy|normal> -> "JOLT_GUEST_OPT=<n>" for `jolt build` (copt_level
# only; empty otherwise). Spliced (unquoted) into an `env ...` prefix.
jolt_opt_env () {
  [ "$OPT_TOGGLE" = "copt_level" ] || { printf ''; return 0; }
  printf 'JOLT_GUEST_OPT=%s' "$(_opt_for "$1")"
}

# cargo_opt_rustflags <buggy|normal> -> "-Copt-level=<n>" for plain cargo
# (rustc/run) under copt_level; empty otherwise. Caller splices into RUSTFLAGS.
cargo_opt_rustflags () {
  [ "$OPT_TOGGLE" = "copt_level" ] || { printf ''; return 0; }
  printf -- '-Copt-level=%s' "$(_opt_for "$1")"
}


# --- jolt-config machinery (mir_opt_level): rebuild jolt between buggy/normal
# arms. Moved here from run_issue so run_pair_full can switch configs itself
# (single-pass funnel, batched Stage-3 emulation). No-op under copt_level.
# Defines: set_jolt_config <buggy|normal>, current_buildrs_config, and an EXIT
# trap. State tracked in JOLT_STATE_FILE, cross-checked against build.rs.
if [ "$OPT_TOGGLE" = "mir_opt_level" ]; then
  BUILD_RS=$(yq -r '.mir_opt_toggle.build_rs'         "$SPEC")
  FLAG_MARK=$(yq -r '.mir_opt_toggle.flag_line_marker' "$SPEC")
  JOLT_DIR=$(yq  -r '.mir_opt_toggle.jolt_dir'         "$SPEC")
  REBUILD=$(yq   -r '.mir_opt_toggle.rebuild_cmd'      "$SPEC")
  # toggle MODE: how the buggy<->normal switch is expressed on the sentinel line.
  #   comment (default) : flip the LEADING // on the sentinel line (line present
  #                       vs absent). This is the original behaviour; every
  #                       existing spec uses it and is unchanged.
  #   value             : rewrite the NUMERIC LEVEL on the sentinel line - buggy
  #                       sets mir_opt_toggle.buggy_value, normal sets
  #                       mir_opt_toggle.normal_value. Use this when the two arms
  #                       differ by the LEVEL, not by presence: e.g. commenting the
  #                       line out leaves rustc's DEFAULT mir-opt-level, which still
  #                       triggers the miscompilation, so 'normal' must EXPLICITLY
  #                       set -Zmir-opt-level=0 (buggy_value=2, normal_value=0).
  TOGGLE_MODE=$(yq -r '.mir_opt_toggle.mode // "comment"' "$SPEC" 2>/dev/null || echo comment)
  case "$TOGGLE_MODE" in
    comment|value) ;;
    *) echo "[config] WARNING: unknown mir_opt_toggle.mode='$TOGGLE_MODE'; defaulting to 'comment'" >&2
       TOGGLE_MODE=comment ;;
  esac
  # value-mode operands (only read/validated when mode=value). Defaults 2/0 match
  # the common "-Zmir-opt-level=2 is buggy, =0 is the safe baseline" case.
  BUGGY_VALUE=$(yq  -r '.mir_opt_toggle.buggy_value  // "2"' "$SPEC" 2>/dev/null || echo 2)
  NORMAL_VALUE=$(yq -r '.mir_opt_toggle.normal_value // "0"' "$SPEC" 2>/dev/null || echo 0)
  # buggy_state: which build.rs comment-state corresponds to the BUGGY arm. The
  # polarity of the toggle is ISSUE-SPECIFIC:
  #   commented   (default) : the buggy arm has the flag line COMMENTED (leading
  #                           //). Uncommenting it produces the NORMAL baseline.
  #                           This is the original behaviour (e.g. #143491) and is
  #                           the default, so every existing spec is unchanged.
  #   uncommented           : the buggy arm has the flag line UNCOMMENTED. Adding
  #                           a leading // produces the NORMAL baseline. Use this
  #                           for issues whose reproducer enables the miscompiling
  #                           pass by an active (uncommented) line.
  # (comment mode only; ignored in value mode, where the level rewrite carries the
  # polarity via buggy_value/normal_value.)
  # Accept a couple of friendly synonyms; anything else falls back to the default
  # with a warning rather than silently toggling the wrong way.
  BUGGY_STATE=$(yq -r '.mir_opt_toggle.buggy_state' "$SPEC")
  case "$BUGGY_STATE" in
    commented|comment|off)      BUGGY_STATE=commented ;;
    uncommented|uncomment|on)   BUGGY_STATE=uncommented ;;
    *) echo "[config] WARNING: unknown mir_opt_toggle.buggy_state='$BUGGY_STATE'; defaulting to 'commented'" >&2
       BUGGY_STATE=commented ;;
  esac
  [ -f "$BUILD_RS" ] || die "mir_opt_toggle.build_rs not found: $BUILD_RS"
  [ -d "$JOLT_DIR" ] || die "mir_opt_toggle.jolt_dir not found: $JOLT_DIR"
  # locate the flag line ONCE and refuse to proceed if it's absent/ambiguous, so
  # we never edit the wrong line. Count occurrences of the marker; also grep a
  # looser 'mir-opt-level' so a not-found error can show what IS there.
  _flag_count=$(grep -cF "$FLAG_MARK" "$BUILD_RS" 2>/dev/null || echo 0)
  if [ "${_flag_count:-0}" -lt 1 ]; then
    echo "FATAL: flag marker '$FLAG_MARK' not found in $BUILD_RS" >&2
    echo "Lines mentioning 'mir-opt' / 'mir_opt' in that file (set flag_line_marker to a substring of the right one):" >&2
    grep -niE 'mir.opt.level|PIPELINE_TOGGLE' "$BUILD_RS" | sed 's/^/    /' >&2 || echo "    (none - is this the right file?)" >&2
    exit 1
  fi
  if [ "$TOGGLE_MODE" = "value" ]; then
    echo "== two-phase (mir_opt_level): flag '$FLAG_MARK' in $BUILD_RS ($_flag_count occurrence(s)); mode=value (buggy=$BUGGY_VALUE normal=$NORMAL_VALUE) =="
  else
    echo "== two-phase (mir_opt_level): flag '$FLAG_MARK' in $BUILD_RS ($_flag_count occurrence(s)); mode=comment polarity: buggy=$BUGGY_STATE =="
  fi

  # Persistent record of which config the INSTALLED jolt was last built as, so we
  # can skip a rebuild when it already matches. Lives next to the jolt install (a
  # stable location) so it survives across pipeline runs. Format: one word,
  # 'buggy' or 'normal'. Override the path with JOLT_STATE_FILE.
  JOLT_STATE_FILE="${JOLT_STATE_FILE:-$JOLT_DIR/.pipeline-jolt-config}"

  # current_buildrs_config: infer the config the build.rs sentinel line currently
  # encodes.
  #   value mode   : read the -Zmir-opt-level=N value on the sentinel line and map
  #                  N==BUGGY_VALUE -> buggy, N==NORMAL_VALUE -> normal, else
  #                  'unknown' (forces a rebuild, never a wrong skip).
  #   comment mode : detect whether the line is COMMENTED (leading //), then map
  #                  that comment-state to buggy/normal via BUGGY_STATE:
  #                    BUGGY_STATE=commented   -> commented => buggy,  uncommented => normal
  #                    BUGGY_STATE=uncommented -> commented => normal, uncommented => buggy
  # Used to cross-check the state file against the actual source so external edits
  # are detected.
  current_buildrs_config () {
    local ln commented lvl
    ln=$(grep -F "$FLAG_MARK" "$BUILD_RS" | head -1)
    if [ "$TOGGLE_MODE" = "value" ]; then
      # extract the numeric level from '-Zmir-opt-level=N' on the sentinel line.
      # If the line is commented out, there is no active level -> 'unknown'.
      if printf '%s' "$ln" | grep -qE '^[[:space:]]*//'; then echo unknown; return; fi
      lvl=$(printf '%s' "$ln" | sed -nE 's/.*mir-opt-level=([0-9]+).*/\1/p' | head -1)
      if   [ "$lvl" = "$BUGGY_VALUE" ];  then echo buggy
      elif [ "$lvl" = "$NORMAL_VALUE" ]; then echo normal
      else echo unknown; fi
      return
    fi
    if printf '%s' "$ln" | grep -qE '^[[:space:]]*//'; then commented=yes; else commented=no; fi
    if [ "$BUGGY_STATE" = "commented" ]; then
      [ "$commented" = yes ] && echo buggy || echo normal
    else
      [ "$commented" = yes ] && echo normal || echo buggy
    fi
  }

  # set_jolt_config <buggy|normal>: toggle the flag line and rebuild jolt - but
  # ONLY if the installed jolt is not already in that config. State is tracked in
  # JOLT_STATE_FILE and cross-checked against the build.rs sentinel so external
  # drift (manual rebuild / edit) forces a rebuild rather than trusting a stale
  # state. Set JOLT_FORCE_REBUILD=1 to always rebuild.
  #   buggy  => flag COMMENTED (pass runs, miscompilation present)
  #   normal => flag UNCOMMENTED (pass disabled, correct baseline)
  set_jolt_config () {
    local cfg="$1" bak cur_state cur_src
    cur_state=$(cat "$JOLT_STATE_FILE" 2>/dev/null || echo "")
    cur_src=$(current_buildrs_config)
    # Skip the rebuild iff BOTH the recorded install-state AND the build.rs source
    # already match the requested config (and no forced rebuild). If they disagree
    # with each other, state is untrustworthy -> rebuild to be safe.
    if [ "${JOLT_FORCE_REBUILD:-0}" -eq 0 ] \
       && [ "$cur_state" = "$cfg" ] && [ "$cur_src" = "$cfg" ]; then
      echo "[config] jolt already built as '$cfg' (state=$cur_state, build.rs=$cur_src) - SKIPPING rebuild"
      return 0
    fi
    [ "$cur_state" = "$cfg" ] && [ "$cur_src" != "$cfg" ] && \
      echo "[config] state says '$cur_state' but build.rs says '$cur_src' (external drift) - rebuilding to be safe" >&2

    bak="${BUILD_RS}.pipeline-bak"
    cp -f "$BUILD_RS" "$bak"                       # always keep a restore point
    # FLAG_MARK is a UNIQUE SENTINEL on exactly ONE line (e.g.
    # PIPELINE_TOGGLE_rustc-143491), placed at the END of that line, so toggling
    # the LEADING // never disturbs it and other -Zmir-opt-level lines are untouched.
    if [ "$TOGGLE_MODE" = "value" ]; then
      # value mode: set the numeric level to the requested arm's value, and ensure
      # the line is ACTIVE (strip any leading //). Just commenting the line would
      # leave rustc's DEFAULT mir-opt-level, which still triggers the miscompile -
      # so 'normal' MUST explicitly carry -Zmir-opt-level=NORMAL_VALUE (e.g. 0).
      local want_value
      [ "$cfg" = "buggy" ] && want_value="$BUGGY_VALUE" || want_value="$NORMAL_VALUE"
      # 1) uncomment the sentinel line if it was commented, so the level is live
      sed -i -E "\|${FLAG_MARK}|{ s|^([[:space:]]*)//[[:space:]]?|\1| }" "$BUILD_RS"
      # 2) rewrite the level number to want_value (matches -Zmir-opt-level=<digits>)
      sed -i -E "\|${FLAG_MARK}|{ s|(mir-opt-level=)[0-9]+|\1${want_value}| }" "$BUILD_RS"
    else
      local want_commented
      if [ "$cfg" = "buggy" ]; then
        [ "$BUGGY_STATE" = "commented" ] && want_commented=yes || want_commented=no
      else
        [ "$BUGGY_STATE" = "commented" ] && want_commented=no  || want_commented=yes
      fi
      if [ "$want_commented" = "yes" ]; then
        # comment the flag line: add a leading // if not already present
        sed -i -E "\|${FLAG_MARK}|{ /^[[:space:]]*\/\// b; s|^([[:space:]]*)|\1// | }" "$BUILD_RS"
      else
        # uncomment the flag line: strip ONE leading //
        sed -i -E "\|${FLAG_MARK}|{ s|^([[:space:]]*)//[[:space:]]?|\1| }" "$BUILD_RS"
      fi
    fi
    echo "[config] set to $cfg; sentinel line now:"
    grep -nF "$FLAG_MARK" "$BUILD_RS" | sed 's/^/    /'
    echo "[config] rebuilding jolt ($REBUILD in $JOLT_DIR) - this is slow (progress below)..."
    # tee: show cargo's progress live on screen AND keep the full log. pipefail
    # (set at top) means a failing cargo install still fails the pipeline even
    # though tee itself exits 0, so the `if !` below still catches a bad rebuild.
    if ! ( cd "$JOLT_DIR" && eval "$REBUILD" ) 2>&1 | tee "$OUT/jolt-rebuild-$cfg.log"; then
      echo "[config] jolt rebuild FAILED for $cfg (see $OUT/jolt-rebuild-$cfg.log)" >&2
      cp -f "$bak" "$BUILD_RS"                     # restore on failure
      die "jolt rebuild failed for config=$cfg (build.rs restored)"
    fi
    # verify the installed jolt actually rebuilt (binary mtime moved)
    command -v jolt >/dev/null 2>&1 || die "jolt not on PATH after rebuild"
    # record the new install-state so future passes/runs can skip a matching rebuild
    printf '%s' "$cfg" > "$JOLT_STATE_FILE" 2>/dev/null || \
      echo "[config] WARNING: could not write state file $JOLT_STATE_FILE" >&2
    echo "[config] jolt rebuilt OK for config=$cfg ($(command -v jolt)); state recorded"
  }
  # On exit: by default we LEAVE build.rs matching the installed jolt config, so
  # the build-state cache lets the NEXT run skip a matching rebuild. This means
  # the pipeline's sentinel line may be left toggled - which is correct, since it
  # reflects the actually-installed jolt. Set RESTORE_BUILD_RS=1 to instead
  # restore the original source on exit; in that case we also invalidate the
  # state cache (the installed jolt no longer matches the restored source, so the
  # next run must rebuild).
  _restore_build_rs () {
    if [ "${RESTORE_BUILD_RS:-0}" -ne 0 ] && [ -f "${BUILD_RS}.pipeline-bak" ]; then
      cp -f "${BUILD_RS}.pipeline-bak" "$BUILD_RS"
      rm -f "$JOLT_STATE_FILE" 2>/dev/null || true   # source no longer matches install
      echo "[config] restored $BUILD_RS from backup on exit (state cache invalidated)" >&2
    else
      echo "[config] leaving build.rs matching installed jolt ($(cat "$JOLT_STATE_FILE" 2>/dev/null || echo '?')) so the next run can skip a matching rebuild (RESTORE_BUILD_RS=1 to restore)" >&2
    fi
  }
  trap _restore_build_rs EXIT
fi

# NOTE: run_issue.sh predicts this name with ${name//\//_} (slashes only), so a
# program name containing @ or : yields a card it will never find -> never skipped.
CARD="$OUT/${ISSUE_ID}__$(echo "$PROG_NAME" | tr '/@:' '___').json"

# Candidate-selection state, initialised so early cards (written before Stage 1/2
# under set -u) never hit an unbound variable.
FIRED_LIST=""; FIRED_COUNT=""
FIRED_REACHABLE=""   # fired ∩ reachable (functions that fired AND are reachable)
REACHABLE_COUNT=""

# funnel-aware result card
# write_card <verdict> <fired> <diverged> <proof> <notes>
# arg 4 was stage3_reached; it is now the card's `proof` field, and every call
# site passes "not_attempted" (prove+verify is manual).
write_card () {
  jq -n \
    --arg issue "$ISSUE_ID" --arg spec "$SPEC" --arg prog "$PROG_NAME" \
    --arg verdict "$1" --argjson fired "$2" \
    --argjson div "$3" --arg proof "$4" --arg notes "$5" \
    --arg ob "${OB:-}" --arg of "${OF:-}" \
    --arg buggy "$BUGGY" \
    --arg firedcount "${FIRED_COUNT:-}" \
    --arg firedlist "${FIRED_LIST:-}" \
    --arg firedreach "${FIRED_REACHABLE:-}" \
    --arg reachcount "${REACHABLE_COUNT:-}" \
    '{issue:$issue, spec:$spec, program:$prog,
      toolchain:{$buggy},
      stage1_fired:$fired, stage1_fired_count:($firedcount|tonumber? // null),
      reachable_count:($reachcount|tonumber? // null),
      stage3_diverged:$div,
      fired_functions:      ($firedlist   | split("\n") | map(select(length>0))),
      fired_and_reachable:  ($firedreach  | split("\n") | map(select(length>0))),
      buggy_output:$ob, fixed_output:$of,
      proof:$proof, verdict:$verdict, notes:$notes, ts:(now|todate)}' > "$CARD"
  echo "[card] $1 -> $CARD"
}

call_agent () {  # $1=role-file ; prompt on stdin -> clean JSON object on stdout
  local role="$1"
  [ -f "$role" ] || die "agent role file not found: $role"
  local raw inner errlog="${AGENT_ERRLOG:-/tmp/agent.err}"
  raw=$(claude -p "$(cat)" --system-prompt-file "$role" \
          --permission-mode acceptEdits --allowedTools "Read,Write,Edit,Glob,Grep" \
          --output-format json 2>"$errlog") || true
  # Save the RAW JSON response too. The CLI writes diagnostics (agent reasoning,
  # permission_denials, tool calls) to STDOUT-as-JSON, not stderr - so the .err
  # file is usually empty even on failure. The .json is where the real evidence
  # is when something goes wrong (e.g. writes denied).
  if [ -n "${errlog%.err}" ]; then printf '%s' "$raw" > "${errlog%.err}.json" 2>/dev/null || true; fi
  if [ -z "$raw" ]; then
    echo "[agent] WARNING: empty response from $(basename "$role"); see $errlog and ${errlog%.err}.json" >&2
    return 0
  fi
  # The CLI wraps the agent's answer in a JSON envelope: the real text is in
  # .result. That text is often PROSE + a ```json block```, not bare JSON, so
  # fence-stripping alone is not enough. Unwrap .result, then extract the JSON
  # OBJECT with a real parser (python) rather than sed heuristics.
  inner=$(printf '%s' "$raw" | jq -r '.result // .' 2>/dev/null)
  printf '%s' "$inner" | python3 -c '
import sys, json, re
t = sys.stdin.read()
# 1) fenced ```json ... ``` block, if present
m = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", t, re.S)
cand = m.group(1) if m else None
# 2) else the last balanced {...} span in the text
if cand is None:
    start = t.find("{"); end = t.rfind("}")
    cand = t[start:end+1] if (start!=-1 and end>start) else None
# 3) else maybe the whole thing already is JSON
if cand is None:
    cand = t.strip()
try:
    print(json.dumps(json.loads(cand)))   # normalise; guarantees valid JSON out
except Exception:
    print("{}")                           # unparseable -> empty object, caller defaults
'
}

# ============================================================================
# PORT (agent) - produce the ONE shared guest tree, with a build-verify-repair
# loop so the porter can diagnose and fix its own compile errors before the
# stages run. The porter must fix build errors WITHOUT changing semantics.
# ============================================================================
MAX_PORT_ATTEMPTS="${MAX_PORT_ATTEMPTS:-3}"
# Work tree lives UNDER the launch directory ($HERE).
work=$(mktemp -d "$HERE/.work-XXXXXX")
GUEST_TREE="$work/src"

reset_guest_tree() {
  rm -rf "$GUEST_TREE"
  cp -r "$TEMPLATE" "$GUEST_TREE"
}

# dead: computed but never read
_guest_key=$(printf '%s' "$GUEST_TREE" | (command -v sha1sum >/dev/null && sha1sum || md5sum) | awk '{print $1}')
if [ "$KEEP_WORK" -ne 1 ]; then
  trap 'rm -rf "$work"' EXIT
fi
reset_guest_tree
step "PORT: work dir = $work ; guest tree = $GUEST_TREE"
# record the template's lib.rs size so we can tell if the porter actually edits it
_LIB="$GUEST_TREE/guest/src/lib.rs"
step "PORT: template guest lib.rs = $([ -f "$_LIB" ] && wc -l < "$_LIB" || echo MISSING) lines before port"

BUILD_ERR="" # build_guest_normal -> 0 if the guest builds via `jolt build` 
BUGGY_RUSTC=""

resolve_toolchain () {
  BUGGY_RUSTC=$(rustup which --toolchain "$BUGGY" rustc 2>/dev/null || true)
  if [ -z "$BUGGY_RUSTC" ] || [ ! -x "$BUGGY_RUSTC" ]; then
    die "could not resolve rustc for toolchain '$BUGGY' via 'rustup which --toolchain $BUGGY rustc' (got: '${BUGGY_RUSTC:-<empty>}'). Is the toolchain installed?"
  fi
  step "BUILD: using RUSTC=$BUGGY_RUSTC (toolchain $BUGGY)"
}

build_guest_normal () {
  local cfg="${1:-normal}" errf="$work/build.err"
  [ -n "$BUGGY_RUSTC" ] || resolve_toolchain
  step "BUILD: compiling guest via 'jolt build' (RUSTC=$BUGGY toolchain$([ "$OPT_TOGGLE" = copt_level ] && printf ', JOLT_GUEST_OPT=%s' "$(_opt_for "$cfg")"), JOLT_BACKTRACE=1)..."
  # copt_level: set JOLT_GUEST_OPT on the jolt build (3 buggy / 0 normal). For
  # mir_opt_level jolt_opt_env is empty, so this is `env  CARGO_TARGET_DIR=...`.
  if ( cd "$GUEST_TREE" && \
       env $(jolt_opt_env "$cfg") \
           CARGO_TARGET_DIR="$work/target-portcheck" \
           JOLT_BACKTRACE=1 \
           RUSTC="$BUGGY_RUSTC" \
         jolt build -p guest -- --release --features guest ) >"$errf" 2>&1; then
    BUILD_ERR=""; step "BUILD: guest built OK"; return 0
  fi

  # keep only compiler/linker error lines + a little context
  BUILD_ERR=$(grep -E '^error|^ *-->|^ *\||^ *= |cannot find|no `std`|unresolved|not found|undefined symbol|linking with' \
                "$errf" | head -40 || true)
  [ -n "$BUILD_ERR" ] || BUILD_ERR=$(tail -40 "$errf")
  step "BUILD: guest FAILED. First error: $(grep -m1 -E '^error|undefined symbol' "$errf" || echo '(see '"$errf"')')"
  return 1
}

_hash () { 
  (command -v sha1sum >/dev/null && sha1sum || md5sum) | awk '{print $1}'; 
}

_dir_hash () { 
  find "$1" -type f -not -path '*/.git/*' -print0 2>/dev/null | sort -z \
    | xargs -0 sha1sum 2>/dev/null | _hash
}

try_load_port_cache () {
  if [ "${PORT_CACHE:-1}" -ne 0 ] && [ -d "$PORT_CACHE_ENTRY/src" ] \
    && [ -f "$PORT_CACHE_ENTRY/port.json" ]; then
    step "PORT: cache HIT ($PORT_CACHE_ENTRY) - reusing prior confirmed port, skipping agent"
    rm -rf "$GUEST_TREE"
    cp -r "$PORT_CACHE_ENTRY/src" "$GUEST_TREE"
    PORT=$(cat "$PORT_CACHE_ENTRY/port.json")
    
    _need_build=0
    case "$PHASE" in
      full) _need_build=1 ;;
      generate) [ "$CONFIG" = "buggy" ] && _need_build=1 ;;
    esac

    if [ "$_need_build" -eq 0 ]; then
      step "PORT: cache HIT - $PHASE${CONFIG:+/$CONFIG} needs no jolt build (skipping; saves a full guest build)"
      PORT_OK=1
    else
      step "PORT: building cached tree to produce the guest ELF (cache saves the port, not the build)"
      if build_guest_normal; then
        PORT_OK=1
      elif [ "${PORT_CACHE_VERIFY:-1}" -ne 0 ]; then
        step "PORT: cached tree did not build under this toolchain ($BUGGY); re-porting to adapt (PORT_CACHE_VERIFY default)"
        reset_guest_tree
        PORT=""
      else
        write_card "build_failed" false false "not_attempted" \
          "cached port failed to build and PORT_CACHE_VERIFY=0 (no re-port); PORT_CACHE=0 to bypass, or clear ported/$(basename "$PORT_CACHE_ENTRY")"
        exit 0
      fi
    fi
  fi
}

make_port_task () {
  local attempt="$1"

  if [ "$attempt" -eq 1 ]; then
    printf '%s\n' \
      "Adapt this program into the harness per your role. Force deps no_std." \
      "Record the decode byte-layout."
  else
    printf '%s\n' \
      "REPAIR MODE. Your previous port did not COMPILE for the guest target" \
      "under the trusted toolchain. Fix the build errors below by editing the tree in" \
      "place. Typical fixes: add a crate's \`alloc\` feature, set \`default-features =" \
      "false\` on a dependency pulling std, repoint \`std::\` to \`alloc::\`/\`core::\`," \
      "add a missing import. DO NOT change algorithm semantics, DO NOT add" \
      "toolchain-conditional code, DO NOT switch to std/musl mode. If the failure is" \
      "because a dependency or the code fundamentally needs the OS and cannot be made" \
      "no_std, return verdict \"unbuildable\" instead of forcing it." \
      "" \
      "Build errors (trimmed):" \
      "$BUILD_ERR"
  fi
}

call_porter () {
  local attempt="$1"
  local task="$2"

  AGENT_ERRLOG="$work/porter-attempt-$attempt.err"

  PORT=$(
    {
      printf '%s\n' "Issue spec:"
      cat "$SPEC"
      printf '\n'

      printf '%s\n' "Input mode: $MODE   (crate = wrap as library; benchmark = lift provable body)"
      printf '%s\n' "Guest scaffold (edit IN PLACE, shared tree): $GUEST_TREE"
      printf '%s\n' "Program source (read-only): $PROG_SRC"
      printf '%s\n' "Program: $PROG_NAME"
      printf '%s\n' "Attempt: $attempt of $MAX_PORT_ATTEMPTS"
      printf '\n'

      printf '%s\n' "$task"
      printf '%s\n' "Emit ONLY your role's JSON."
    } | call_agent "$PORTER_ROLE"
  )
}

log_porter_attempt_result () {
  local attempt="$1"
  local now
  local changed_files

  step "PORT attempt $attempt: agent returned $(printf '%s' "$PORT" | wc -c) bytes of JSON"

  now=$([ -f "$_LIB" ] && wc -l < "$_LIB" || echo MISSING)

  if git diff --no-index --quiet "$TEMPLATE" "$GUEST_TREE" 2>/dev/null; then
    step "PORT attempt $attempt: WARNING - guest tree is UNCHANGED from template (agent wrote nothing). Check $AGENT_ERRLOG"
  else
    changed_files=$(
      git diff --no-index --name-only "$TEMPLATE" "$GUEST_TREE" 2>/dev/null \
        | xargs -n1 basename 2>/dev/null \
        | tr '\n' ' ' || true
    )

    step "PORT attempt $attempt: guest tree modified (lib.rs now $now lines); files: $changed_files"
  fi
}

save_port_to_cache () {
    if [ "${PORT_CACHE:-1}" -ne 0 ] && [ "$PORT_ATTEMPTS" -gt 0 ]; then
      mkdir -p "$PORT_CACHE_ENTRY"
      rm -rf "$PORT_CACHE_ENTRY/src"
      cp -r "$GUEST_TREE" "$PORT_CACHE_ENTRY/src"
      printf '%s' "$PORT" > "$PORT_CACHE_ENTRY/port.json"
      {
        echo "program:          $PROG_NAME"
        echo "buggy_toolchain:  $BUGGY   (port build-verified against this)"
        echo "mode:             $MODE"
        echo "key:              $PORT_KEY"
        echo "created_by_issue: $ISSUE_ID   (shared across issues on this toolchain)"
        echo "saved:            $(date -u +%Y-%m-%dT%H:%M:%SZ)"
      } > "$PORT_CACHE_ENTRY/meta.txt"
      step "PORT: cached confirmed port -> $PORT_CACHE_ENTRY (shared across issues on toolchain $BUGGY)"
  fi
}

PORT=""; PORT_ATTEMPTS=0; PORT_OK=0
# BROKEN: $HERE is this scripts dir; the role files live in ../agents/.
PORTER_ROLE="$HERE/agents/guest-porter.CLAUDE.md"

# Porting is expensive, so cache confirmed-buildable ports and reuse them, 
# keyed by a hash of (source, template, porter-role, mode). Disable with PORT_CACHE=0; 
# force a rebuild-verify on hit with PORT_CACHE_VERIFY=1.
# NOTE: the committed ports in ../corpus/<name>/ are NOT in this layout (entries
# here are <name>__<key>), so they never produce a cache hit.
PORT_CACHE_DIR="${PORT_CACHE_DIR:-$HERE/ported}"

PORT_KEY=$(printf '%s|%s|%s|%s' \
  "$(_dir_hash "$PROG_SRC")" "$(_dir_hash "$TEMPLATE")" \
  "$([ -f "$PORTER_ROLE" ] && sha1sum "$PORTER_ROLE" | _hash || echo noRole)" \
  "$MODE" | _hash)

# Entry path: program name + key hash. Port is shared across all issues and all toolchains.
PORT_CACHE_ENTRY="$PORT_CACHE_DIR/${PROG_NAME//\//_}__${PORT_KEY}"

try_load_port_cache
# ======================= PORT LOOP =========================================
while [ "$PORT_OK" -ne 1 ] && [ "$PORT_ATTEMPTS" -lt "$MAX_PORT_ATTEMPTS" ]; do
  PORT_ATTEMPTS=$((PORT_ATTEMPTS + 1))
  if [ "$PORT_ATTEMPTS" -gt 1 ]; then
    echo "[port] attempt $PORT_ATTEMPTS: repairing build failure" >&2
  fi

  TASK=$(make_port_task "$PORT_ATTEMPTS")

  step "PORT attempt $PORT_ATTEMPTS/$MAX_PORT_ATTEMPTS: calling porter agent..."
  call_porter "$PORT_ATTEMPTS" "$TASK"
  log_porter_attempt_result "$PORT_ATTEMPTS"

  PSTATUS=$(jq  -r '.status  // "failed"'      <<<"$PORT" 2>/dev/null || echo failed)
  PVERDICT=$(jq -r '.verdict // "unbuildable"' <<<"$PORT" 2>/dev/null || echo unbuildable)
  step "PORT attempt $PORT_ATTEMPTS: parsed status=$PSTATUS verdict=$PVERDICT"

  # Porter declares it genuinely can't be made no_std -> stop, do not burn tries.
  if [ "$PVERDICT" = "unbuildable" ]; then
    write_card "build_failed" false false "not_attempted" \
      "porter declared unbuildable after $PORT_ATTEMPTS attempt(s): $(jq -r '.blocker // "unknown"' <<<"$PORT" 2>/dev/null)"
    exit 0
  fi
  if [ "$PSTATUS" != "ok" ]; then
    BUILD_ERR="porter returned status!=ok: $(jq -r '.blocker // "no blocker given"' <<<"$PORT" 2>/dev/null)"
    continue
  fi

  # Verify it compiles under the fixed toolchain.
  if build_guest_normal; then PORT_OK=1; break; fi
  echo "[port] attempt $PORT_ATTEMPTS built tree but it did not compile" >&2
done

# Save port diff always 
git diff --no-index "$TEMPLATE" "$GUEST_TREE" > "$OUT/${ISSUE_ID}__${PROG_NAME//\//_}.port.diff" 2>/dev/null || true

if [ "$PORT_OK" -ne 1 ]; then
  write_card "build_failed" false false "not_attempted" \
    "guest did not compile after $PORT_ATTEMPTS repair attempt(s). Last errors: $(echo "$BUILD_ERR" | head -5 | tr '\n' ' ')"
  exit 0
fi
echo "[port] OK after $PORT_ATTEMPTS attempt(s): mode=$(jq -r .mode <<<"$PORT") entry=$(jq -r .entry_called <<<"$PORT")"

save_port_to_cache

# capture the decode layout the porter recorded, to steer input generation later
DECODE_LAYOUT=$(jq -r '.decode_layout // ""' <<<"$PORT" 2>/dev/null || echo "")

# echoes the guest ELF path built by `jolt build`.
locate_guest_elf () {
  local roots=() r
  for r in "$work/target-portcheck" /tmp/jolt-guest-targets-harness; do
    [ -d "$r" ] && roots+=("$r")
  done
  [ "${#roots[@]}" -gt 0 ] || { printf ''; return 0; }
  find "${roots[@]}" -type f -path "*/riscv64imac-unknown-none-elf/release/*" \
    \( -name 'guest' -o -name 'guest.elf' \) 2>/dev/null | head -1 || true
}

# harvest_seed_inputs <log-prefix> <empty-test> : wave-1 input generation into
# $INPUTS - a real harvest, then an UNSTEERED-synthetic top-up whenever the suite
# still holds fewer than STEERED_MIN files. Skips entirely at >= STEERED_MIN.
# <empty-test> is accepted but IGNORED (both call sites pass "any").
# Caller must have already gated on GENINPUTS_MIN!=0 && gen_inputs.sh executable.
harvest_seed_inputs () {
  local pfx="$1" empty_test="${2:-any}" _n
  local STEERED_MIN="${STEERED_MIN:-8}"
  local STEERED_MAX="${STEERED_MAX:-12}"
  _n=$(find "$INPUTS" -maxdepth 1 -type f 2>/dev/null | wc -l)
  if [ "$_n" -ge "$STEERED_MIN" ]; then
    step "$pfx: inputs dir already has $_n input(s) (>= $STEERED_MIN); skipping generation (no agent call)"
    return 0
  fi
  step "$pfx: harvesting the program's own inputs ($MODE) via gen_inputs.sh --source real"
  "$HERE/gen_inputs.sh" "$PROG_SRC" "$INPUTS" --source real \
      --mode "$MODE" --decode-layout "$DECODE_LAYOUT" \
      || step "$pfx: real harvest failed (non-fatal); proceeding with existing suite"
  _n=$(find "$INPUTS" -maxdepth 1 -type f 2>/dev/null | wc -l)
  step "$pfx: real harvest produced $_n input(s)"
  local do_synth=0
  [ "$_n" -lt "$STEERED_MIN" ] && do_synth=1
  if [ "$do_synth" -eq 1 ]; then
    step "$pfx: only $_n input(s) (< $STEERED_MIN); generating synthetic inputs (unsteered) to top up the suite"
    "$HERE/gen_inputs.sh" "$PROG_SRC" "$INPUTS" --source steered \
        --mode "$MODE" --decode-layout "$DECODE_LAYOUT" --max-files "$STEERED_MAX" \
        || step "$pfx: synthetic generation failed (non-fatal)"
    _n=$(find "$INPUTS" -maxdepth 1 -type f 2>/dev/null | wc -l)
    step "$pfx: synthetic produced $_n input(s)"
    if [ "$_n" -eq 0 ]; then
      step "$pfx: WARNING - no inputs from real OR synthetic. Check the decode layout ($DECODE_LAYOUT) and the input-generator agent."
    fi
  fi
}

# run_gate <input> [keep-log] : one instrumentation-gate invocation on <input>.
# Echoes the gate's stdout (fired paths) and RETURNS its exit code (0 fired,
# 10 compiled-not-fired, 3 gate/emulate failure) without set -e aborting on
# non-zero. Pass any non-empty second arg to add --keep-log (generate loop keeps
# logs for the survivor store; the full-phase single call does not need them).
run_gate () {
  local inp="$1" keep="${2:-}" out rc extra=()
  [ -n "$keep" ] && extra=(--keep-log)
  set +e
  out=$("$HERE/gate_instrumentation.sh" --spec "$SPEC" --guest "$GUEST_TREE" \
          --input "$inp" --buggy-toolchain "$BUGGY" "${extra[@]}")
  rc=$?
  set -e
  # dead: set here but never read by any caller
  FIRED_RAW_COUNT=$(printf '%s\n' "$out" | sed -n 's/^FIRED_RAW_COUNT=//p' | head -1)
  printf '%s' "$out" | grep -v '^FIRED_RAW_COUNT=' || true
  return "$rc"
}


# ==== helpers hoisted from the pipeline body (each defined once) ====

bare_name () {   # dead: stdin fully-qualified names -> stdout bare last component
  sed -E 's/<[^<>]*>//g; s/<.*>//g' | sed -E 's/.*:://; s/^[[:space:]]+//; s/[[:space:]]+$//' \
    | grep -v '^$'
}
sanitise_fns () {   # dead: stdin raw fn names -> stdout cleaned, one per line
  sed -E 's/^[0-9]+//; s/[[:space:]]*~[[:space:]]*$//; s/^[[:space:]]+//; s/[[:space:]]+$//' \
    | grep -v '^$' | sort -u
}

classify_emulate_output () {
  local rc="$1" out="$2"
  if [ "$rc" -eq 137 ]; then
    printf 'OOM:%s' "$out"
    return 0
  fi
  if [ "$rc" -ne 0 ] || printf '%s' "$out" | grep -qE "${PANIC_MARKERS:-panicked at|-> RUNTIME ERROR|guest aborted|attempt to .* with overflow|index out of bounds}"; then
    printf 'PANIC:%s' "$out"
  else
    printf 'OK:%s' "$out"
  fi
}

# NOTE: no default arm - if either side is empty (e.g. host produced no contract
# line) nothing is echoed and the caller silently drops that input.
classify_pair () {
  local of="$1" ob="$2" fk="${1%%:*}" bk="${2%%:*}"
  case "$fk/$bk" in 
    OOM/*|*/OOM) echo oom; return 0 ;; 
  esac
  case "$fk/$bk" in
    OK/OK)              [ "$(payload_of "$ob")" != "$(payload_of "$of")" ] && echo value-divergence || echo ok-same ;;
    PANIC/PANIC)        echo both-panic ;;
    OK/PANIC|PANIC/OK)  echo panic-divergence ;;
  esac
}

# dead: superseded by run_arm (this one keys the target dir on $CONFIG, which is
# only set in the removed generate/emulate phases).
emulate_one () {
  local inp="$1" tag="${2:-emu}" out rc joptenv
  joptenv="$(jolt_opt_env "$CONFIG")"
  # shellcheck disable=SC2046,SC2086
  out=$(CARGO_TARGET_DIR="$work/target-$tag" \
          env $(guest_tc_env "$BUGGY") $joptenv \
            cargo run --ignore-rust-version --release -q --manifest-path "$GUEST_TREE/Cargo.toml" -p host -- \
              --emulate --input "$inp" 2>&1)
  rc=$?
  classify_emulate_output "$rc" "$out"
}

compute_reachable_set () {
  local elf
  local rc
  local count 

  if [ ! -x "$HERE/reachable_fns.sh" ]; then
    write_card "build_failed" true false "not_attempted" \
      "reachable_fns.sh is missing or not executable; cannot compute mandatory reachable set"
    exit 0
  fi

  step "CALLGRAPH: locating guest ELF under target-portcheck"
  # Prefer a pre-captured snapshot (the normal-config ELF saved right after the
  # port build) if the caller set CALLGRAPH_ELF - this makes reachability immune
  # to any later build that writes into target-portcheck (e.g. a buggy-config
  # rebuild), guaranteeing reachability runs on the NORMAL-config ELF. Fall back
  # to locate_guest_elf when no snapshot was set.
  if [ -n "${CALLGRAPH_ELF:-}" ] && [ -f "$CALLGRAPH_ELF" ]; then
    elf="$CALLGRAPH_ELF"
    step "CALLGRAPH: using captured normal-config ELF snapshot = $elf"
  else
    elf=$(locate_guest_elf)
  fi
  if [ -z "$elf" ] || [ ! -f "$elf" ]; then
    write_card "build_failed" true false "not_attempted" \
      "guest ELF not found under $work/target-portcheck; cannot compute mandatory reachable set"
    exit 0
  fi

  step "CALLGRAPH: guest ELF = $elf"

  set +e
  REACHABLE_SET=$("$HERE/reachable_fns.sh" \
                    --elf "$elf" \
                    --root "harness,guest::main::__jolt_guest_harness,__jolt_guest_harness" \
                    --workdir "$work/callgraph")
  rc=$?
  set -e

  if [ "$rc" -ne 0 ] || [ -z "$REACHABLE_SET" ]; then
    write_card "build_failed" true false "not_attempted" \
      "reachable_fns.sh failed or produced an empty reachable set (rc=$rc); cannot continue"
    exit 0
  fi

  REACHABLE_COUNT=$(printf '%s\n' "$REACHABLE_SET" | grep -vc '^$' || true)
  count="$REACHABLE_COUNT"

  if [ "$count" -eq 0 ]; then
    write_card "build_failed" true false "not_attempted" \
      "reachable_fns.sh produced zero reachable functions; cannot continue"
    exit 0
  fi

  step "CALLGRAPH: $count reachable function(s) from harness"

}

run_arm () {   # $1=config (buggy|normal)  $2=input
  # Emits "OK:<output>" if the guest ran to a normal result, or "PANIC:<output>"
  # if it panicked/aborted/errored. A panic is NOT a wrong answer - it's "no
  # answer" (the guest rejected this input) - so the oracle must not treat two
  # (possibly textually-different) panics as a value divergence.
  local cfg="$1" inp="$2" out rc tc
  # copt_level: BOTH arms use the buggy toolchain; the guest opt level is set via
  # JOLT_GUEST_OPT on the --emulate rebuild (buggy=3, normal=0). Legacy toggles:
  # normal=fixed toolchain, buggy=buggy.
  # copt_level: the --emulate run rebuilds the GUEST via jolt/zeroos-build, so its
  # opt level is set by JOLT_GUEST_OPT (same knob as jolt build), NOT -Copt-level.
  local joptenv=""
  case "$OPT_TOGGLE" in
    copt_level) tc="$BUGGY"; joptenv="$(jolt_opt_env "$cfg")" ;;
    *) tc="$BUGGY" ;;   # mir_opt_level: both arms share the buggy toolchain; the
                        # buggy/normal difference is the INSTALLED jolt's
                        # -Zmir-opt-level (rebuilt between arms by set_jolt_config),
                        # not the toolchain or any per-run env.
  esac
  # Per-arm dir separates the HOST artifacts only: the template host builds the
  # guest into a FIXED path (/tmp/jolt-guest-targets-harness), so both arms share
  # one guest target dir and rely on cargo's fingerprint to rebuild. This is why
  # --jobs > 1 is unsafe.
  # shellcheck disable=SC2046,SC2086
  out=$(CARGO_TARGET_DIR="$work/target-arm-$cfg" \
          env $(guest_tc_env "$tc") $joptenv \
            cargo run --ignore-rust-version --release -q --manifest-path "$GUEST_TREE/Cargo.toml" -p host -- \
              --emulate --input "$inp" 2>&1)
  rc=$?
  classify_emulate_output "$rc" "$out"
}

payload_of () {
  local s="$1" p
  p=$(printf '%s\n' "$s" | grep -o 'output=.*' | tail -1)
  if [ -n "$p" ]; then
    printf '%s' "$p"
  else
    printf '%s' "${s#*:}"   # strip the OK:/PANIC: prefix, keep the rest
  fi
}

# NOTE: the triage role expects both arms' outputs, the fired list and a
# prove+verify result; this prompt sends only $OB, and the agent has no shell.
triage_divergence () {
  local DT DVERDICT TRIAGE_SUMMARY
  DT=$(
    {
      echo "Issue spec:"
      cat "$SPEC"
      echo
      echo "Program: $PROG_NAME"
      echo "Divergence kind: $DIV_KIND"
      echo "Diverging input: $DIV_INPUT"
      echo "Toolchain ($BUGGY): $OB"
      echo "Shared guest tree: $GUEST_TREE"
      echo
      echo 'Note on kind: "value-divergence" = both arms ran to a normal result and the'
      echo 'outputs differ. "panic-divergence" = one arm ran normally and the other panicked'
      echo 'on the same input (a toolchain-induced behavioural change, not a wrong value).'
      echo 'Attribute via the version-constant O0/opt toggle on the buggy toolchain. Emit ONLY your role'\''s JSON.'
    } | call_agent "$HERE/agents/divergence-triage.CLAUDE.md"   # BROKEN path: see PORTER_ROLE
  )
  DVERDICT=$(jq -r '.verdict // "confirmed_miscompile"' <<<"$DT" 2>/dev/null || echo confirmed_miscompile)
  TRIAGE_SUMMARY=$(jq -c '{root_cause,mir_explanation,proof_impact}' <<<"$DT" 2>/dev/null || echo '')
  write_card "$DVERDICT" true true "not_attempted" \
    "kind=$DIV_KIND; $TRIAGE_SUMMARY"
  echo
  echo "=== REVIEW REQUIRED ==============================================="
  echo "  verdict: $DVERDICT   (agent hypothesis - you confirm)"
  echo "  kind:    $DIV_KIND"
  echo "  work:    $work  (--keep-work to retain)"
  echo "  diverging input: $DIV_INPUT  (prove+verify MANUALLY on this input)"
  [ "$DIV_KIND" = "panic-divergence" ] && \
    echo "  NOTE: panic-divergence - buggy arm panicked where fixed produced output"
  echo "  Confirm the O0/opt localisation lands on a fired+differing fn before"
  echo "  this enters the thesis."
  echo "==================================================================="
}

order_and_dedup_inputs () {
  local f h; declare -A seen
  # rank prefix: lower = higher priority
  for f in "${INPUT_FILES[@]}"; do
    h=$(sha1sum "$f" 2>/dev/null | awk '{print $1}')
    [ -n "$h" ] && [ -n "${seen[$h]:-}" ] && continue   # skip duplicate content
    [ -n "$h" ] && seen[$h]=1
    case "$(basename "$f")" in
      host-*)        printf '0\t%s\n' "$f" ;;
      vec-*)         printf '1\t%s\n' "$f" ;;
      rand-*|edge-*) printf '2\t%s\n' "$f" ;;
      *)             printf '3\t%s\n' "$f" ;;
    esac
  done | sort -s -k1,1n | cut -f2-
}