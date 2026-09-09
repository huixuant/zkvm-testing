#!/usr/bin/env python3
"""
Compute the set of functions statically reachable from a specified function
in a Jolt guest ELF.

The call graph combines:

1. Direct calls recovered from llvm-objdump disassembly.
2. Inlined calls recovered from DWARF DW_TAG_inlined_subroutine entries.
3. Virtual calls resolved against vtables parsed from the symbol table and
   data sections.

Compressed forms (c.jalr / c.jr) are not matched by the jalr/jr regexes, so
compressed indirect calls fall back to "(indirect)".

Usage:
    python3 jolt_callgraph_static.py guest.disasm \
        --dwarf guest.dwarf \
        --symtab guest.symtab --sections guest.sections \
        --reachable harness \
        --dot callgraph.dot

--symtab/--sections are optional but must be given together; without them
virtual calls degrade to the synthetic "(indirect)" node.
"""

import argparse
import re
import sys
from collections import defaultdict

# ----- disassembly parsing --------------------------------------------------

# function header, e.g. "800014ce <harness>:"
FUNC_HDR_RE = re.compile(r"^([0-9a-fA-F]+)\s+<(.+)>:\s*$")

# instruction line: capture ADDRESS, mnemonic, operands
INSN_RE = re.compile(r"^\s*([0-9a-fA-F]+):\s+(\S+)\s*(.*?)\s*$")

# "<symbol>" at the end of an instruction line, e.g. "jal ra, 80003000 <hash_pair>
TARGET_RE = re.compile(r"<([^>]+)>\s*$")

# instructions that represent calls in risc-v
CALL_MNEMONICS = {"jal", "jalr", "c.jal", "c.jalr"}

# register-writing forms of auipc we care about (any dest reg)
AUIPC_RE = re.compile(r"^auipc\s+(\w+),\s*(-?\d+)")

# jalr/jr forms:  "jalr off(rs)" | "jalr rd, off(rs)" | "jr off(rs)"
#   also "jalr rs" (off implicitly 0) and pseudo "jalr rd, rs, off" (rare in objdump)
JALR_MEM_RE   = re.compile(r"^(?:jalr|jr)\s+(?:\w+,\s*)?(-?\d+)\((\w+)\)")
JALR_BARE_RE  = re.compile(r"^(?:jalr|jr)\s+(\w+)\s*$")

# addi that finalizes an auipc-computed address into a register (for tail/data;
# also used by some call sequences): "addi rd, rs, imm"
ADDI_RE = re.compile(r"^(?:addi|c\.addi)\s+(\w+),\s*(\w+),\s*(-?\d+)")

# memory load into a register: "ld rd, off(rs)" - the first half of a vtable
# dispatch sequence (ld rX, SLOT(rX); jalr rX).
LD_RE = re.compile(r"^(?:ld|c\.ld)\s+(\w+),\s*(-?\d+)\((\w+)\)")
 
# registers that hold frame/stack pointers: loads based on these are spills or
# locals, never vtable slots.
FRAME_REGS = {"sp", "s0", "fp", "gp", "tp"}

def _clean_target(name):
    return name.split("+", 1)[0].strip()

# Check if the instruction is a function call and not a jump/tail call 
# (destination register is x0).
def is_call(mnemonic, ops):
    if mnemonic not in CALL_MNEMONICS:
        return False
    first = ops.split(",", 1)[0].strip() if ops else ""
    if first in ("x0", "zero"):
        return False
    return True

# Used for auipc + jalr call whose target isn't explicitly labelled by objdump 
def _build_addr_index(headers):
    """headers: list of (addr:int, name:str) sorted by addr.
    Return a function/resolver that maps any address to the name of the function whose
    range contains it (the greatest header addr <= target)."""
    import bisect
    addrs = [a for a, _ in headers]
    names = [n for _, n in headers]

    def resolve(addr):
        i = bisect.bisect_right(addrs, addr) - 1
        if i < 0:
            return None
        return names[i]
    return resolve

def parse_disasm(path):
    """Return (edges, headers).
    edges: dict caller -> {callee -> kind}; kind in {'direct','indirect'}.
    headers: sorted list of (entry_addr, name) for every real function symbol -
    also needed by the vtable pass, so it is returned rather than recomputed.
    Now resolves split auipc+jalr / auipc+jr PC-relative calls to real symbols
    instead of dumping them into the '(indirect)' bucket.
    """
    edges = defaultdict(dict)

    # Collect function headers (addr,name) so we can map a resolved
    # target ADDRESS back to the function that contains it.
    # objdump prints a `<name>:` header for EVERY symbol at an address.
    headers = []  # (int_addr, name) 
    with open(path) as f:
        for line in f:
            h = FUNC_HDR_RE.match(line)
            if h:
                name = h.group(2)
                # Function headers' regex may include local assembler labels like
                # `.Lpcrel_hiN` and `.L0`/`.L<n>`, which we aren't interested in, 
                # so we drop any name starting with ".L".
                if name.startswith(".L"):
                    continue          
                headers.append((int(h.group(1), 16), name))
    headers.sort()
    resolve_addr = _build_addr_index(headers)

    # Walk instructions, tracking per-register auipc bases so a later
    # jalr/jr off(reg) can be resolved to auipc_addr + (imm<<12) + off.
    current = None
    # reg -> (computed_base_address). Reset conservatively at each new function.
    reg_base = {}
    # reg -> byte offset of the vtable-style load that produced it
    # ("ld rX, OFF(rY)" with rY not a frame register). Reset per function.
    reg_slot = {}

    with open(path) as f:
        for line in f:
            h = FUNC_HDR_RE.match(line)
            if h:
                name = h.group(2)
                if name.startswith(".L"):
                    # Local label (.Lpcrel_hiN / .L0 basic block) inside the
                    # current function. Do not change `current` (stay in the real
                    # function) and do not reset reg_base.
                    continue
                current = name
                reg_base = {}          # real function boundary: clear auipc state
                reg_slot = {}
                continue
            m = INSN_RE.match(line)
            if not m or current is None:
                continue
            iaddr = int(m.group(1), 16)
            mnemonic, ops = m.group(2), m.group(3)
            full = (mnemonic + " " + ops).strip()

            # --- track auipc: reg = iaddr + (imm << 12) --------------------
            am = AUIPC_RE.match(full)
            if am:
                reg = am.group(1)
                imm = int(am.group(2))
                # RISC-V auipc: imm is a 20-bit value; objdump prints it as the
                # raw (possibly huge, e.g. 1048575) decimal. Treat as 20-bit.
                imm20 = imm & 0xFFFFF
                # sign-extend 20-bit
                if imm20 & 0x80000: # check if sign bit is set 
                    imm20 -= 0x100000 
                reg_base[reg] = (iaddr + (imm20 << 12)) & 0xFFFFFFFFFFFFFFFF
                reg_slot.pop(reg, None)
                continue

            # --- addi that advances an auipc base (rd = rs + imm) ----------
            dm = ADDI_RE.match(full)
            if dm:
                rd, rs, imm = dm.group(1), dm.group(2), int(dm.group(3))
                if rs in reg_base:
                    reg_base[rd] = (reg_base[rs] + imm) & 0xFFFFFFFFFFFFFFFF
                elif rd in reg_base and rd != rs:
                    # rs is not tracked, invalidate
                    reg_base.pop(rd, None)
                reg_slot.pop(rd, None)
                continue

            # --- track vtable-style loads: rd = *(rs + off) ----------------
            lm = LD_RE.match(full)
            if lm:
                rd, off, rs = lm.group(1), int(lm.group(2)), lm.group(3)
                reg_base.pop(rd, None)
                if rs not in FRAME_REGS and off >= 0:
                    # candidate vtable slot load; remember the byte offset
                    reg_slot[rd] = off
                else:
                    reg_slot.pop(rd, None)
                continue

            # --- resolved <symbol> direct call ---------
            if is_call(mnemonic, ops):
                tm = TARGET_RE.search(ops)
                if tm:
                    callee = _clean_target(tm.group(1))
                    if not callee.startswith(".L") and callee != current:
                        edges[current].setdefault(callee, "direct")
                    continue
                # --- split auipc+jalr: resolve via reg_base ----------------
                mem = JALR_MEM_RE.match(full)
                if mem:
                    off = int(mem.group(1)); reg = mem.group(2)
                    if reg in reg_base:
                        target = (reg_base[reg] + off) & 0xFFFFFFFFFFFFFFFF
                        callee = resolve_addr(target)
                        if callee and callee != current and not callee.startswith(".L"):
                            edges[current].setdefault(callee, "direct")
                            continue
                    # couldn't resolve 
                    edges[current].setdefault("(indirect)", "indirect")
                    continue
                bare = JALR_BARE_RE.match(full) # for offset implicitly 0
                if bare:
                    reg = bare.group(1)
                    if reg in reg_base:
                        target = reg_base[reg] & 0xFFFFFFFFFFFFFFFF
                        callee = resolve_addr(target)
                        if callee and callee != current and not callee.startswith(".L"):
                            edges[current].setdefault(callee, "direct")
                            continue
                    if reg in reg_slot:
                        # vtable dispatch: ld reg, OFF(vptr); jalr reg
                        # record a pseudo callee to be resolved against vtables.
                        edges[current].setdefault(f"(vcall@{reg_slot[reg]})", "indirect")
                        continue                    
                    edges[current].setdefault("(indirect)", "indirect")
                    continue
                # any other unresolved call form
                edges[current].setdefault("(indirect)", "indirect")
                continue

            # a jalr/jr as a TAIL call also creates an edge
            # Example of a tail call: 
            # fn foo() -> i32 {
            #     bar()
            # }
            # objdump prints tail calls as `jr off(reg)` (rd=x0). Those are real
            # control transfers to another function; treat jr-to-another-function
            # as a (tail) call edge so inlined-return tail calls aren't lost.
            if mnemonic in ("jr", "c.jr", "jalr") and ops:
                mem = JALR_MEM_RE.match(full)
                if mem:
                    off = int(mem.group(1)); reg = mem.group(2)
                    if reg in reg_base:
                        target = (reg_base[reg] + off) & 0xFFFFFFFFFFFFFFFF
                        callee = resolve_addr(target)
                        # only an edge if it lands at (near) a DIFFERENT function
                        if callee and callee != current and not callee.startswith(".L"):
                            edges[current].setdefault(callee, "direct")

    return edges, headers

# ----- DWARF parsing (inlined calls) ----------------------------------------

# debugging information entity (e.g. DW_TAG_subprogram, DW_TAG_inlined_subroutine)
DIE_RE = re.compile(r"^(0x[0-9a-fA-F]+):(\s*)DW_TAG_(\w+)")

# name (and linkage name) print as: DW_AT_name ("name") 
AT_NAME_RE = re.compile(r'DW_AT_name\s+\("([^"]+)"\)')
AT_LINKAGE_RE = re.compile(r'DW_AT_linkage_name\s+\("([^"]+)"\)')

# abstract_origin prints as: DW_AT_abstract_origin (0xNNNN "name")
AT_ORIGIN_RE = re.compile(r'DW_AT_abstract_origin\s+\(0x[0-9a-fA-F]+\s+"([^"]+)"\)')

SCOPE_TAGS = {"subprogram", "inlined_subroutine"}

def parse_dwarf(path, edges):
    """Add inlined-call edges to `edges`. A DW_TAG_inlined_subroutine yields an
    edge (enclosing scope -> abstract_origin name), and nests, so inlined-within-
    inlined is attributed correctly."""

    class Die:
        __slots__ = ("indent", "tag", "name", "linkage", "origin")

        def __init__(self, indent, tag):
            self.indent = indent # describes nesting in DWARF tree 
            self.tag = tag       # DIE type (subprogram, inlined_subroutine, etc.)
            self.name = None
            self.linkage = None
            self.origin = None

        # Identity of subprogram comes from name/linkage name, whereas
        # identity of inlined subroutine comes from its abstract origin 
        def resolved(self):
            if self.tag == "inlined_subroutine":
                return self.origin
            return self.linkage or self.name

    stack = []          # scope DIEs currently open (subprogram/inlined)
    pending = None      # DIE whose attribute lines we're still reading

    def finalize(die):
        nonlocal stack
        if die is None:
            return
        # pop scopes that are siblings/shallower than this DIE
        # to illustrate, suppose we have:
        # `harness`                     indent 0
        #   `verify_merkle_inclusion`   indent 2
        #     `hash_pair`               indent 4
        # 
        # And the next DIE is at indent 2 (cold_sibling_ptr)
        # We pop everything in the stack until what remains is [harness]
        while stack and stack[-1].indent >= die.indent:
            stack.pop()
        
        # find nearest named enclosing function
        caller = None
        for s in reversed(stack):
            if s.resolved():
                caller = s.resolved()
                break

        if die.tag == "inlined_subroutine":
            callee = die.resolved()
            if caller and callee:
                edges[caller].setdefault(callee, "inlined")

        # current die becomes parent of future nested DIEs
        # irrelevant DIE tags (e.g. compile_unit) are ignored
        if die.tag in SCOPE_TAGS and die.resolved():
            stack.append(die)

    with open(path) as f:
        for line in f:
            m = DIE_RE.match(line)
            if m: # new DIE encountered
                finalize(pending)
                indent = len(m.group(2))
                pending = Die(indent, m.group(3))
                continue
            if pending is None: # ignore attributes if no DIE 
                continue
            nm = AT_NAME_RE.search(line)
            if nm:
                pending.name = nm.group(1)
                continue
            lk = AT_LINKAGE_RE.search(line)
            if lk:
                pending.linkage = lk.group(1)
                continue
            og = AT_ORIGIN_RE.search(line)
            if og:
                pending.origin = og.group(1)
    finalize(pending)
    return edges

# ----- vtable parsing (precise virtual-call resolution) ----------------------
 
# llvm-objdump -t symbol line, e.g.
# "0000000080049a08 l     O .rodata\t0000000000000030 <() as ledger::Settler>::{vtable}"
SYM_RE = re.compile(
    r"^([0-9a-fA-F]{8,16})\s+.*?(\.\S+)\s+([0-9a-fA-F]{8,16})\s+(.*\S)\s*$")
 
# llvm-objdump -s hex-dump line, e.g.
# " 80049a08 e6620080 00000000 00000000 00000000  .b.............."
SECT_HDR_RE  = re.compile(r"^Contents of section (\S+):")
SECT_LINE_RE = re.compile(r"^ ([0-9a-fA-F]+) ((?:[0-9a-fA-F]{2,8} ?)+)")
 
# Rust vtable layout: [drop_in_place, size, align, method0, method1, ...]
VTABLE_HEADER_BYTES = 24

def parse_sections(path, skip=(r"\.text", r"\.debug", r"\.comment", r"\.riscv")):
    """Parse `llvm-objdump -s` output into a sparse addr->byte map (data
    sections only)."""
    mem = {}
    keep = False
    skip_re = re.compile("|".join(skip))
    with open(path) as f:
        for line in f:
            h = SECT_HDR_RE.match(line)
            if h:
                keep = not skip_re.match(h.group(1))
                continue
            if not keep:
                continue
            m = SECT_LINE_RE.match(line)
            if not m:
                continue
            addr = int(m.group(1), 16)
            hexstr = m.group(2).replace(" ", "")
            for i in range(0, len(hexstr) - 1, 2):
                mem[addr + i // 2] = int(hexstr[i:i + 2], 16)
    return mem
 
def _read_u64(mem, addr):
    """Little-endian u64 at addr, or None if any byte is missing."""
    val = 0
    for i in range(8):
        b = mem.get(addr + i)
        if b is None:
            return None
        val |= b << (8 * i)
    return val
 
def parse_vtables(symtab_path, mem, func_entry_addrs):
    """Return list of (vtable_name, {byte_slot_offset: function_name}).
 
    Primary strategy: symbols whose demangled name mentions 'vtable'.
    Fallback (legacy mangling emits unnamed local vtables): structural scan of
    every data symbol - slot0 must be a function entry (drop_in_place), slot2 a
    power-of-two alignment, and at least one method slot a function entry."""
    entries = dict(func_entry_addrs)   # addr -> function name (exact entry)
    symbols = []
    with open(symtab_path) as f:
        for line in f:
            m = SYM_RE.match(line)
            if not m:
                continue
            addr, sect, size, name = (int(m.group(1), 16), m.group(2),
                                      int(m.group(3), 16), m.group(4))
            if sect.startswith(".text") or size == 0:
                continue
            symbols.append((addr, size, name))
 
    def methods_of(addr, size):
        out = {}
        off = VTABLE_HEADER_BYTES
        while off + 8 <= size:
            fn = entries.get(_read_u64(mem, addr + off))
            if fn:
                out[off] = fn
            off += 8
        return out
 
    named = [(nm, methods_of(a, sz)) for a, sz, nm in symbols
             if "vtable" in nm.lower()]
    named = [(nm, ms) for nm, ms in named if ms]
    if named:
        return named
 
    # structural fallback
    out = []
    for a, sz, nm in symbols:
        if sz < VTABLE_HEADER_BYTES + 8:
            continue
        drop  = _read_u64(mem, a)
        size  = _read_u64(mem, a + 8)
        align = _read_u64(mem, a + 16)
        # drop_in_place slot: a function entry, or NULL for types with no
        # drop glue (rustc emits a null pointer there for !needs_drop types).
        if drop != 0 and drop not in entries:
            continue
        if size is None or size >= (1 << 24):
            continue
        if not align or align & (align - 1) or align > 8192:
            continue
        ms = methods_of(a, sz)
        if ms:
            out.append((nm, ms))
    return out
 
def resolve_vcalls(edges, vtables):
    """Replace (vcall@N) pseudo callees with 'virtual' edges to slot N of every
    parsed vtable; unmatched pseudo callees degrade to plain (indirect)."""
    stats = {"resolved": 0, "unmatched": 0}
    for caller in list(edges):
        pseudo = [c for c in edges[caller] if c.startswith("(vcall@")]
        for p in pseudo:
            slot = int(p[len("(vcall@"):-1])
            targets = {fn for _, ms in vtables for off, fn in ms.items()
                       if off == slot}
            del edges[caller][p]
            if targets:
                for t in targets:
                    edges[caller].setdefault(t, "virtual")
                stats["resolved"] += 1
            else:
                edges[caller].setdefault("(indirect)", "indirect")
                stats["unmatched"] += 1
    return stats

# ----- graph analysis -------------------------------------------------------
def extract_nodes_and_successors(edges, include_indirect):
    """Return all nodes and a mapping from each node to its list of successor nodes."""
    nodes = set()
    succ = defaultdict(list)
    for caller, callees in edges.items():
        nodes.add(caller)
        for callee in callees:
            if callee == "(indirect)" and not include_indirect:
                continue
            nodes.add(callee)
            succ[caller].append(callee)
    return nodes, succ

def reachable_from(succ, starts):
    """Return the SET of all nodes reachable from any start node. The start nodes 
    are included in the result (they are trivially reachable from themselves).
    """
    seen = set(starts)
    stack = list(starts)
    while stack:
        n = stack.pop()
        for nxt in succ.get(n, ()):
            if nxt not in seen:
                seen.add(nxt)
                stack.append(nxt)
    return seen

# ----- main -----------------------------------------------------------------
def _emit_dot(path, edges, include_indirect):
    """Write the merged call graph to `path` in Graphviz DOT format."""
    style = {"direct": "solid", "inlined": "dashed", "indirect": "dotted",
             "virtual": "bold"}
    color = {"direct": "black", "inlined": "#1f77b4", "indirect": "#d62728",
             "virtual": "#9467bd"}
    with open(path, "w") as f:
        f.write("digraph callgraph {\n  rankdir=LR;\n"
                "  node [shape=box,fontsize=10];\n")
        for c in edges:
            for callee, kind in edges[c].items():
                if callee == "(indirect)" and not include_indirect:
                    continue
                f.write(f'  "{c}" -> "{callee}" '
                        f'[style={style[kind]},color="{color[kind]}"];\n')
        f.write("}\n")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--reachable", metavar="NAMES", required=True,
                help="comma-separated list of EXACT node names to use as "
                     "reachability roots, e.g. 'harness,guest::main::__jolt_guest_harness'")
    ap.add_argument("disasm", help="llvm-objdump -d output (pipe through rustfilt)")
    ap.add_argument("--dwarf", help="llvm-dwarfdump --debug-info output (rustfilt)")
    ap.add_argument("--symtab", help="llvm-objdump -t output (rustfilt); enables "
                                     "precise virtual-call resolution")
    ap.add_argument("--sections", help="llvm-objdump -s output (raw); data-section "
                                       "bytes for vtable slot lookup")
    ap.add_argument("--dot", default="callgraph.dot")
    ap.add_argument("--include-indirect", action="store_true",
                    help="keep the synthetic (indirect) node in the graph")
    args = ap.parse_args()

    edges, headers = parse_disasm(args.disasm)

    if bool(args.symtab) != bool(args.sections):
        sys.exit("--symtab and --sections must be given together")
    if args.symtab:
        mem = parse_sections(args.sections)
        vtables = parse_vtables(args.symtab, mem, headers)
        stats = resolve_vcalls(edges, vtables)
        print(f"vtables: {len(vtables)} parsed; vcalls resolved="
              f"{stats['resolved']} unmatched={stats['unmatched']}",
              file=sys.stderr)
    else:
        # no vtable inputs: degrade pseudo edges to plain (indirect)
        resolve_vcalls(edges, [])

    if args.dwarf:
        parse_dwarf(args.dwarf, edges)

    if not edges:
        sys.exit("no calls found - is the disassembly demangled and non-empty?")
    
    nodes, succ = extract_nodes_and_successors(edges, args.include_indirect)
    root_names = {n.strip() for n in args.reachable.split(",") if n.strip()}
    starts = sorted(n for n in nodes if n in root_names)
    if not starts:
        sys.exit(f"'{args.reachable}' matched no node")
    else:
        print(f"reachability starts ({len(starts)}): "
                f"{', '.join(starts[:5])}{' ...' if len(starts) > 5 else ''}",
                file=sys.stderr)
    reachable = reachable_from(succ, starts)
    for name in sorted(reachable):
        print(name)
    # can be rendered with `dot -Tsvg callgraph.dot -o callgraph.svg`
    _emit_dot(args.dot, edges, args.include_indirect)
    print(f"wrote {args.dot}", file=sys.stderr)
    print(f"reachable functions: {len(reachable)}", file=sys.stderr)
    return

if __name__ == "__main__":
    main()
