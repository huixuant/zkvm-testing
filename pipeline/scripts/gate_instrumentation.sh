#!/usr/bin/env bash
# gate_instrumentation.sh — the instrumentation gate (ZeroOS guest-toolchain edition).
#
# Toolchain selection is now an ENVIRONMENT concern, not a `+<name>` flag:
#   * buggy guest build : ZEROOS_GUEST_TOOLCHAIN=<name> set
#   * fixed guest build : ZEROOS_GUEST_TOOLCHAIN unset (default guest toolchain)
# The var affects the GUEST compile only; the host/prover stays on the default
# toolchain either way, so we invoke plain `cargo run` (no `+`).
#
# The guest is compiled AS PART OF the host --emulate run (Jolt's build step),
# so gating and execution are the SAME invocation: one emulate run both fires
# the instrumentation (MISCOMP_LOG) and produces the guest output. We only need
# the buggy arm to gate, since the pass fires only under the buggy toolchain.
#
# Emits fired function paths on stdout, one per line (deduplicated).
# Exit: 0 = fired, 10 = did not fire, 3 = build/run failed, 2 = setup error.
#
# Usage:
#   ./gate_instrumentation.sh --spec specs/rustc-143491.yaml \
#                   --guest control/repro-143491 \
#                   --input inputs/repro-143491/keystream_cipher.bin \
#                   [--buggy-toolchain b5e10d8c00-rustc]   # overrides spec
#                   [--keep-log]
#
set -euo pipefail

SPEC=""; GUEST=""; INPUT=""; BUGGY_TC=""; KEEP_LOG=0
while [ $# -gt 0 ]; do
  case "$1" in
    --spec)             SPEC="$2";     shift 2 ;;
    --guest)            GUEST="$2";    shift 2 ;;
    --input)            INPUT="$2";    shift 2 ;;
    --buggy-toolchain)  BUGGY_TC="$2"; shift 2 ;;
    --keep-log)         KEEP_LOG=1;    shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

die () { echo "gate: FATAL: $*" >&2; exit 2; }
command -v cargo >/dev/null 2>&1 || die "cargo not found"
command -v yq    >/dev/null 2>&1 || die "yq required to read the spec"
[ -n "$SPEC" ] && [ -f "$SPEC" ] || die "--spec <file> required and must exist"
[ -n "$GUEST" ] && [ -d "$GUEST" ] || die "--guest <tree> required and must exist"
[ -n "$INPUT" ] && [ -f "$INPUT" ] || die "--input <file> required and must exist"

# --- read the instrumentation contract + toolchain from the spec -----------
ISSUE_ID=$(yq -r '.id'                       "$SPEC")
SIGNAL=$(yq   -r '.instrumentation.signal'   "$SPEC")   # expect: file
ENVVAR=$(yq   -r '.instrumentation.env'      "$SPEC")   # expect: MISCOMP_LOG
MATCH=$(yq    -r '.instrumentation.match'    "$SPEC")
[ "$MATCH" = "null" ] && die "spec has no instrumentation.match"

# Buggy guest toolchain: --buggy-toolchain overrides, else spec.buggy_toolchain.
[ -z "$BUGGY_TC" ] && BUGGY_TC=$(yq -r '.buggy_toolchain' "$SPEC")
{ [ -z "$BUGGY_TC" ] || [ "$BUGGY_TC" = "null" ] || [ "$BUGGY_TC" = "FILL" ]; } \
  && die "no buggy toolchain: set spec.buggy_toolchain or pass --buggy-toolchain"

MANIFEST="$GUEST/Cargo.toml"
[ -f "$MANIFEST" ] || die "no Cargo.toml in guest tree: $GUEST"

# --- fresh, ABSOLUTE log path (cargo compiles from varied CWDs) -------------
LOGDIR=$(mktemp -d)
LOGFILE="$LOGDIR/miscomp.log"
STDERR="$LOGDIR/run.stderr"
: > "$LOGFILE"
cleanup () { [ "$KEEP_LOG" -eq 1 ] || rm -rf "$LOGDIR"; }
trap cleanup EXIT

echo "gate: issue=$ISSUE_ID toolchain(buggy)=$BUGGY_TC" >&2
echo "gate: log=$LOGFILE input=$(basename "$INPUT")" >&2

# --- the instrumented BUGGY guest build, via the emulate host run ----------
# ZEROOS_GUEST_TOOLCHAIN selects the buggy guest compiler; MISCOMP_LOG turns on
# the detector. Host stays default. A separate CARGO_TARGET_DIR isolates this
# from the differential build's caches.
set +e
# Build the command as an array so the printed line matches exactly what runs.
CMD=(env "ZEROOS_GUEST_TOOLCHAIN=$BUGGY_TC" "$ENVVAR=$LOGFILE"
     "CARGO_TARGET_DIR=$LOGDIR/target"
     cargo run --ignore-rust-version --release -q --manifest-path "$MANIFEST" -p host --
     --emulate --input "$INPUT")
echo "gate: running: ${CMD[*]}" >&2
"${CMD[@]}" >"$LOGDIR/host.stdout" 2>"$STDERR"
RC=$?
set -e

# --- collect the signal ----------------------------------------------------
# IMPORTANT: read the MISCOMP log BEFORE gating on the run's exit code. The
# instrumentation writes the log at COMPILE time, which completes before the
# emulator ever runs. A non-zero RC here is almost always a DOWNSTREAM emulator
# panic (e.g. Jolt MMU stack-overflow/canary, or "No inline registered for
# opcode" - a missing precompile handler) that happens AFTER the guest compiled
# and AFTER the detector already fired. In that case the log is already fully
# populated and the panic is irrelevant to gating - so we must NOT discard the
# fire just because emulation crashed afterwards. We only care about RC when the
# log is empty (then we genuinely can't tell "did not fire" from "crash stopped
# us gating").
case "$SIGNAL" in
  file)   SRC="$LOGFILE" ;;
  stderr) SRC="$STDERR"  ;;
  *) die "unsupported signal type: $SIGNAL (expected file|stderr)" ;;
esac

FIRED=$(grep -E "$MATCH" "$SRC" 2>/dev/null || true)

if [ -n "$FIRED" ]; then
  # The detector fired at compile time. Gate PASSES regardless of whether the
  # emulate run later panicked - we have the compile-time signal we need.
  if [ "$RC" -ne 0 ]; then
    echo "gate: emulate run exited rc=$RC AFTER the detector fired at compile time - proceeding on the log (downstream emulator panic ignored for gating)" >&2
    sed 's/^/gate:   (post-fire emulator panic) /' "$STDERR" | tail -5 >&2
  fi
else
  # No fire in the log. Whether that is a trustworthy "did not fire" depends on
  # whether the COMPILE succeeded: the detector runs at compile time, so an empty
  # log is only meaningful if the guest actually compiled.
  #
  # We decide this by a caching-PROOF POSITIVE signal: does the compiled host
  # binary exist on disk? Text heuristics do NOT work here - on a warm-cached
  # build cargo prints NOTHING (no "Compiling", no "Finished", and under `-q` not
  # even that), so run.stderr may contain ONLY a later emulator panic with no
  # build output at all. But a successful build (fresh OR fully cached) always
  # leaves the `host` binary under CARGO_TARGET_DIR. So:
  #   binary exists    -> the guest compiled; the detector had its full chance;
  #                       an empty log means it did NOT fire (rc 10), even if the
  #                       emulator then panicked at runtime (tracer/mmu/inline
  #                       panics are all POST-compile).
  #   binary absent    -> the build genuinely produced no artifact => compile
  #                       failed => empty log is uninformative => cannot gate (rc 3).
  _hostbin=""
  for _b in "$LOGDIR/target/release/host" "$LOGDIR"/target/*/release/host; do
    [ -x "$_b" ] && { _hostbin="$_b"; break; }
  done
  if [ "$RC" -ne 0 ] && [ -z "$_hostbin" ]; then
    echo "gate: emulate run FAILED (rc=$RC) and no host binary was produced (build genuinely failed); empty log is uninformative - cannot gate this crate" >&2
    sed 's/^/gate:   /' "$STDERR" | tail -20 >&2
    exit 3
  fi
  if [ "$RC" -ne 0 ]; then
    # host binary exists => the guest compiled. Empty log => detector did not fire.
    # The rc!=0 is a POST-compile emulator panic (missing inline opcode, MMU stack
    # overflow, input-too-long, etc.) and is irrelevant to the gate result.
    echo "gate: guest COMPILED (host binary present: $_hostbin) but the detector did not fire; the emulator then panicked at runtime (rc=$RC). Recording 'did not fire', not a build failure." >&2
    sed 's/^/gate:   (post-compile emulator panic, ignored for gating) /' "$STDERR" | tail -5 >&2
  fi
  echo "gate: pass did NOT fire" >&2
  exit 10
fi

N=$(printf '%s\n' "$FIRED" | grep -c . || true)
echo "gate: pass fired ($N line(s))" >&2
printf 'FIRED_RAW_COUNT=%s\n' "$N"

# --- extract fired function paths ------------------------------------------
# Handles three instrumentation line formats:
#   1. rustc, explicit:  ... path=<def_path_str> ...
#   2. rustc, DefId tail: ... def=DefId(0:7 ~ crate[hash]::module::func) ...
#   3. LLVM/ValueTracking: ... fn=<mangled-symbol>        (no DefId/path)
# For (3) the symbol is a MANGLED Rust name; demangle it with rustfilt so the
# result is comparable to the def_path_str values from (1)/(2).
printf '%s\n' "$FIRED" \
  | sed -nE '
      s/.*symbol=([^[:space:],)]+).*/\1/p; t
      s/.*[[:space:]]path=(.+)[[:space:]]+def=.*/\1/p; t
      s/.*def=DefId\([^~]*~[[:space:]]*([^)]+)\).*/\1/p; t
      s/.*[[:space:]]fn=([^[:space:]]+).*/\1/p
    ' \
  | { if command -v rustfilt >/dev/null 2>&1; then rustfilt; else cat; fi } \
  | sed -E 's/\[[0-9a-f]+\]//g' \
  | awk 'NF' | sort -u

exit 0