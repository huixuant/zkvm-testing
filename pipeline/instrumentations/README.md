# Instrumentation patches

The pipeline's Stage-1 gate asks one question: *did the compiler fault fire while
compiling this program?* It can only answer that because each buggy compiler is
built from a **patched** source tree — patched both to keep the miscompilation
alive and to make it announce itself.

This folder holds those patches — normally one file per buggy compiler, two where
the fix has to be undone separately — so the toolchains can be rebuilt from
upstream sources without shipping a Rust or LLVM checkout.

## File naming

```text
<toolchain>.<tree>.patch     the detector patch — the main file
<toolchain>.revert.patch     bug reintroduction only, applied BEFORE the detector
```

* `<toolchain>` is exactly the `buggy_toolchain` value in the issue spec, and
  also the name to give `rustup toolchain link`.
* `<tree>` is which source tree the patch applies to — `rust` for a rust-lang/rust
  checkout, `llvm` for an LLVM checkout.

Three things follow from this naming that are easy to miss:

* **A patch may serve more than one issue.** One instrumented compiler can carry
  several detectors, told apart by their match regex rather than by the
  toolchain. The header's `Serves issue spec(s)` line is authoritative.
* **A toolchain may need more than one file**, or a file plus manual steps, when
  the instrumentation lives in LLVM rather than in rustc. The header says so.
* **A `<toolchain>.revert.patch` must be applied before the detector patch.** It
  appears only where the base commit already contains the upstream fix and that
  fix is too large to undo by hand. It carries no instrumentation, so it has no
  `Detector` block; it has an `Order of application` block instead. The detector
  patch's header names it and repeats the order.

## Reading a file

Each file is a plain unified diff with a `#`-comment header block on top. The
header carries everything needed to place the diff:

| Header field | What it tells you |
| --- | --- |
| `Toolchain name` | the `buggy_toolchain` to link the finished build as |
| `Serves issue spec(s)` | which specs in `../specs/` this compiler is for |
| `Target repository` | which upstream repo to clone |
| `Base commit` | the exact commit to check out before applying, with its date and subject |
| `Apply inside` | which checkout root to run `git apply` from |
| `Files modified` | the compiler source files the diff touches |
| `Detector` | the env var the detector writes to, and the regex that identifies a fire |
| `What this patch contains` | whether the diff reverts a fix, adds a detector, or both |
| `Reproduction / EXTRA STEPS` | anything that must happen before or besides `git apply` |

The header is comment-only, so the file is **directly appliable as-is** — both
`git apply` and GNU `patch -p1` skip the leading block. There is no need to strip
anything.

## Reading the diff

A patch generally does two separate jobs, and the header's
`What this patch contains` block labels which hunks do which:

* **REVERT** — undoes an upstream fix so the base commit's compiler miscompiles
  again. Needed only when the chosen base commit already contains the fix; where
  the base predates it, the patch is detector-only and says so. Usually these are
  hunks inside the detector patch; where the fix is too large for that, they are
  split into the companion `.revert.patch` described above.
* **DETECT** — adds the instrumentation itself. Every detector follows the same
  contract: it appends one line to the file named by the `MISCOMP_LOG`
  environment variable, and that line starts with `MISCOMP <issue-id>`. Nothing
  is written when the variable is unset, so an instrumented compiler behaves
  normally in ordinary use.

A detector never changes what the compiler emits. Reverting a fix does, by
design — that is the bug under test.

## Rebuilding a toolchain

The general shape, with the exact commands in each file's header:

1. Clone the repository the header names and check out its base commit.
2. Perform any EXTRA STEPS first — submodule moves, external-LLVM setup.
3. `git apply` from the checkout root: the `.revert.patch` first if one exists,
   then the detector patch.
4. Build the compiler (`./x build --stage 1 compiler` for a rust tree).
5. `rustup toolchain link <toolchain> build/<host-triple>/stage1`.

Then confirm the detector is live before trusting any pipeline result:

```bash
MISCOMP_LOG=/tmp/m.log ZEROOS_GUEST_TOOLCHAIN=<toolchain> <build any guest>
grep -c '^MISCOMP <issue-id>' /tmp/m.log
```

A zero count on a program you *expect* to fire means the build did not pick up
the instrumentation — not that the program is clean. The pipeline cannot tell
those two apart, so it is worth checking once per toolchain against a known
reproducer.

## Practical notes

* Base commits are upstream SHAs, so a full clone is needed; a shallow clone will
  not have them.
* `build/<host-triple>/stage1` is the path in the headers written for
  `x86_64-unknown-linux-gnu`; substitute your own host triple.
* These are **stage-1** builds, and they are used for the *guest* compile only —
  the pipeline selects them through `ZEROOS_GUEST_TOOLCHAIN`, never for the host.
* The patches are recorded against the trees that produced the results in
  `../findings/`. Rebuilding from a different base commit may still reproduce the
  bug, but it is no longer the same compiler the recorded runs used.
