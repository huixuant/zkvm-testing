//! Differential-testing host for the Jolt `harness` guest.
//!
//! Two mutually exclusive modes, each printing EXACTLY ONE stdout line:
//!
//!   --emulate --input <PATH> [--trace-fn <FN_PATH>]
//!       output=<hex>|panic=<bool>|reached=<true|false|unknown>
//!
//!   --prove --verify --input <PATH>
//!       output=<hex>|panic=<bool>|valid=<bool>
//!
//! All logging goes to stderr. Any internal error exits non-zero and never
//! prints a contract line.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

type BoxErr = Box<dyn Error>;

/// Where the guest ELF is built. A fixed path keeps the (slow) guest build
/// cached across repeated invocations on the same input set.
const TARGET_DIR: &str = "/tmp/jolt-guest-targets-harness";

fn main() -> ExitCode {
    // stdout is reserved for the single contract line; everything else is stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::WARN)
        .init();

    match run() {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("host error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, BoxErr> {
    // `jolt build` locates the guest by walking UP from the current directory
    // looking for a Cargo.toml with `[workspace]` (zeroos_build::find_workspace_root).
    // The driver may launch us from anywhere, so pin cwd to OUR workspace root
    // (the parent of this crate) or it would find the outer Jolt workspace.
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("cannot locate workspace root from CARGO_MANIFEST_DIR")?
        .to_path_buf();
    std::env::set_current_dir(&workspace_root)?;

    // The `jolt` CLI passes `-Cstrip=symbols` to the guest build by default, which
    // removes .symtab entirely and makes --trace-fn resolution impossible. Setting
    // JOLT_BACKTRACE opts out of that strip (jolt/src/main.rs:238-241).
    //
    // This is CODEGEN-NEUTRAL and therefore safe for a miscompilation harness:
    // stripping happens at link time, and the CLI deliberately keeps
    // `-Cforce-frame-pointers` (which DOES change generated code) on the separate
    // `--backtrace` path, never on JOLT_BACKTRACE. Verified empirically: the .text
    // section is byte-identical with and without this variable.
    std::env::set_var("JOLT_BACKTRACE", "1");

    let args = Args::parse(std::env::args().skip(1))?;
    let input = fs::read(&args.input)?;

    match args.mode {
        Mode::Emulate => emulate(input, args.trace_fn.as_deref()),
        Mode::ProveVerify => prove_verify(input),
    }
}

fn emulate(input: Vec<u8>, trace_fn: Option<&str>) -> Result<String, BoxErr> {
    // Prover-free path. `compile_harness` builds the guest ELF and returns a
    // `Program` whose `.elf` field points at it (needed for symbol resolution).
    // `trace_analyze` runs the emulator ONLY and returns the full
    // ProgramSummary { trace, io_device } — no prover is instantiated.
    let program = guest::compile_harness(TARGET_DIR);
    let elf_path = program.elf.clone();

    let input_bytes = jolt_sdk::postcard::to_stdvec(&input)?;
    let summary = program.trace_analyze::<jolt_sdk::F>(&input_bytes, &[], &[]);

    let panic = summary.io_device.panic;
    let output = decode_output(&summary.io_device)?;

    let reached = match trace_fn {
        None => "unknown".to_string(),
        Some(fn_path) => {
            match elf_path
                .as_deref()
                .and_then(|p| fs::read(p).ok())
                .and_then(|data| function_ranges(&data, fn_path))
            {
                // "true" if any traced instruction's PC lands inside a resolved
                // symbol range; "false" if we resolved range(s) but none were
                // entered; "unknown" if the ELF/symbol is unavailable (e.g. the
                // function was fully inlined) — we never claim "true" unconfirmed.
                Some(ranges) if !ranges.is_empty() => {
                    let hit = summary.trace.iter().any(|cycle| {
                        // Cycle -> its decoded Instruction -> normalized PC (address).
                        let pc = cycle.instruction().normalize().address as u64;
                        ranges.iter().any(|(start, end)| pc >= *start && pc < *end)
                    });
                    if hit { "true" } else { "false" }.to_string()
                }
                _ => "unknown".to_string(),
            }
        }
    };

    Ok(format!(
        "output={}|panic={}|reached={}",
        hex::encode(&output),
        panic,
        reached
    ))
}

fn prove_verify(input: Vec<u8>) -> Result<String, BoxErr> {
    let mut program = guest::compile_harness(TARGET_DIR);

    let shared = guest::preprocess_shared_harness(&mut program)?;
    let prover_prep = guest::preprocess_prover_harness(shared.clone());
    let verifier_setup = prover_prep.generators.to_verifier_setup();
    let verifier_prep = guest::preprocess_verifier_harness(shared, verifier_setup, None);

    let prove = guest::build_prover_harness(program, prover_prep);
    let verify = guest::build_verifier_harness(verifier_prep);

    let (output, proof, io_device) = prove(input.clone());
    let panic = io_device.panic;
    let valid = verify(input, output.clone(), panic, proof);

    Ok(format!(
        "output={}|panic={}|valid={}",
        hex::encode(&output),
        panic,
        valid
    ))
}

/// Decode the guest's `Vec<u8>` return from the raw output region, matching the
/// prover's own decode (`make_prove_func`): pad to `max_output_size`, then
/// postcard-decode. This makes --emulate and --prove emit identical `output=`.
fn decode_output(io: &jolt_sdk::JoltDevice) -> Result<Vec<u8>, BoxErr> {
    let mut outputs = io.outputs.clone();
    outputs.resize(io.memory_layout.max_output_size as usize, 0);
    let value: Vec<u8> = jolt_sdk::postcard::from_bytes(&outputs)?;
    Ok(value)
}

/// Address ranges [start, start+size) of every function symbol whose demangled
/// path equals or contains `fn_path`. `None` if the ELF can't be parsed.
fn function_ranges(elf: &[u8], fn_path: &str) -> Option<Vec<(u64, u64)>> {
    use object::{Object, ObjectSymbol, SymbolKind};

    let file = object::File::parse(elf).ok()?;
    let mut ranges = Vec::new();
    for sym in file.symbols() {
        if sym.kind() != SymbolKind::Text || sym.size() == 0 {
            continue;
        }
        let Ok(raw) = sym.name() else { continue };
        // `{:#}` demangles WITHOUT the trailing `::h<hash>` disambiguator.
        let demangled = format!("{:#}", rustc_demangle::demangle(raw));
        if demangled == fn_path || demangled.contains(fn_path) {
            let addr = sym.address();
            ranges.push((addr, addr + sym.size()));
        }
    }
    Some(ranges)
}

enum Mode {
    Emulate,
    ProveVerify,
}

struct Args {
    mode: Mode,
    input: PathBuf,
    trace_fn: Option<String>,
}

impl Args {
    fn parse(argv: impl Iterator<Item = String>) -> Result<Args, BoxErr> {
        let mut emulate = false;
        let mut prove = false;
        let mut verify = false;
        let mut input: Option<PathBuf> = None;
        let mut trace_fn: Option<String> = None;

        let mut argv = argv.peekable();
        while let Some(arg) = argv.next() {
            match arg.as_str() {
                "--emulate" => emulate = true,
                "--prove" => prove = true,
                "--verify" => verify = true,
                "--input" => {
                    input = Some(
                        argv.next()
                            .ok_or("--input requires a <PATH> value")?
                            .into(),
                    )
                }
                "--trace-fn" => {
                    trace_fn = Some(argv.next().ok_or("--trace-fn requires a <FN_PATH> value")?)
                }
                other => return Err(format!("unknown argument: {other}").into()),
            }
        }

        let mode = match (emulate, prove, verify) {
            (true, false, false) => Mode::Emulate,
            (false, true, true) => Mode::ProveVerify,
            (true, _, _) if prove || verify => {
                return Err("--emulate is mutually exclusive with --prove/--verify".into())
            }
            (false, true, false) | (false, false, true) => {
                return Err("--prove and --verify must be used together".into())
            }
            _ => {
                return Err(
                    "specify exactly one mode: `--emulate` or `--prove --verify`".into(),
                )
            }
        };

        Ok(Args {
            mode,
            input: input.ok_or("--input <PATH> is required")?,
            trace_fn,
        })
    }
}
