#!/usr/bin/env bash
# reachable_fns.sh - wrapper script that emits the set of functions STATICALLY REACHABLE 
# from the guest entry (harness), computed using the Python callgraph script. 
#
# Pipeline:
#   llvm-objdump -d --no-show-raw-insn <elf> | rustfilt > guest.disasm
#   llvm-dwarfdump --debug-info        <elf> | rustfilt > guest.dwarf
#   llvm-objdump -t <elf> | rustfilt > guest.symtab ; llvm-objdump -s <elf> > guest.sections
#   python3 jolt_callgraph_static.py guest.disasm --dwarf guest.dwarf \
#           --symtab guest.symtab --sections guest.sections --reachable harness
#
# Usage: reachable_fns.sh --elf <path> [--root harness] [--workdir <dir>]
# Exit:  0 = reachable set emitted on stdout,
#        2 = bad argument,
#        3 = ELF not found / tooling missing / callgraph tool failed.

# Exit script immediately if:
#    (1) Any command exits with non-zero status (set -e)
#    (2) Any variable is referenced before being set (set -u)
#    (3) Any command in a pipeline fails (set -o pipefail)
set -euo pipefail

# Parse command-line arguments and initialise variables 
ELF=""; ROOT="harness,guest::main::__jolt_guest_harness,__jolt_guest_harness"; WORKDIR=""
while [ $# -gt 0 ]; do
  case "$1" in
    --elf)     ELF="$2";     shift 2 ;;
    --root)    ROOT="$2";    shift 2 ;;
    --workdir) WORKDIR="$2"; shift 2 ;;
    *) echo "reachable_fns: unknown arg: $1" >&2; exit 2 ;;
  esac
done

log () { echo "reachable: $*" >&2; }
HERE=$(cd "$(dirname "$0")" && pwd)
CG="$HERE/jolt_callgraph_static.py"

# Establish that all necessary files and commands exist on the system 
[ -n "$ELF" ] && [ -f "$ELF" ] || { log "ELF not found: '${ELF:-<empty>}'"; exit 3; }
[ -f "$CG" ] || { log "callgraph tool missing: $CG"; exit 3; }
for t in llvm-objdump llvm-dwarfdump rustfilt python3; do
  command -v "$t" >/dev/null 2>&1 || { log "missing tool: $t (cannot build call graph)"; exit 3; }
done

WORKDIR="${WORKDIR:-$(mktemp -d)}"
mkdir -p "$WORKDIR"
DIS="$WORKDIR/guest.disasm"
DWA="$WORKDIR/guest.dwarf"
SYM="$WORKDIR/guest.symtab"
SEC="$WORKDIR/guest.sections"
CGOUT="$WORKDIR/reachable.out"

# Generate dissassembly and DWARF files from the ELF;
# llvm-dwarfdump parses DWARF debug data into human-readable form
log "disassembling $ELF"
llvm-objdump -d --no-show-raw-insn "$ELF" | rustfilt > "$DIS" 2>/dev/null || { log "objdump failed"; exit 3; }
log "extracting DWARF"
llvm-dwarfdump --debug-info "$ELF" | rustfilt > "$DWA" 2>/dev/null || { log "dwarfdump failed"; exit 3; }
log "extracting symbol table"
llvm-objdump -t "$ELF" | rustfilt > "$SYM" 2>/dev/null || { log "symtab failed"; exit 3; }
log "dumping data sections"
llvm-objdump -s "$ELF" > "$SEC" 2>/dev/null || { log "section dump failed"; exit 3; }

# Obtain reachable function set, specifying the root node as "harness"
python3 "$CG" "$DIS" --dwarf "$DWA" --symtab "$SYM" --sections "$SEC" \
    --reachable "$ROOT" --dot "$WORKDIR/callgraph.dot" > "$CGOUT" \
    2>>"$WORKDIR/callgraph.err" || {
  log "callgraph tool failed; stderr tail:"; tail -5 "$WORKDIR/callgraph.err" >&2; exit 3; }

# Display diagnostic information (ENABLED by default; SHOW_CALLGRAPH=0 to silence)
if [ "${SHOW_CALLGRAPH:-1}" -ne 0 ]; then
  echo "reachable: ---- jolt_callgraph_static diagnostics ----" >&2
  sed 's/^/reachable:   /' "$WORKDIR/callgraph.err" >&2
  echo "reachable: ---- end diagnostics ----" >&2
fi

# stdout of the Python script is exactly the reachable function set, one per line.
# Remove empty strings, de-duplicate and sort. The root itself is always included (trivially reachable).
grep -v '^[[:space:]]*$' "$CGOUT" | sort -u
