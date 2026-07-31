//! `CMD-MG-COVERAGE` — the documented, read-only Memory Graph coverage/orphan
//! gate (task F0.1 / 0.1.5).
//!
//! ## Usage
//!
//! ```text
//! cargo run -p kria-eval --bin mg-coverage [-- <options>]
//!
//! Options:
//!   --spec-dir <path>   Spec directory to lint (default: the in-repo
//!                       memory-graph-production-redesign spec). Read-only.
//!   --out-dir <path>    Evidence output directory. When set, writes
//!                       reports/{id-inventory,coverage,reverse-orphans}.json
//!                       and commands/CMD-MG-COVERAGE.json under it. Must NOT
//!                       be a spec document path.
//!   --run-id <id>       Run identifier recorded in the command evidence.
//!   --quiet             Suppress the human-readable banner (JSON-only).
//!   -h, --help          Print this help and exit 0.
//! ```
//!
//! ## Exit codes
//!
//! * `0`  — gate passed: exact totals (48/48, 46/46, 65/65, 31/31), zero
//!   reverse orphans, zero error diagnostics.
//! * `1`  — gate failed closed (unmet totals, reverse orphans, or any other
//!   error-severity diagnostic).
//! * `2`  — internal error (spec could not be read/parsed).
//!
//! The command never modifies the spec: it reads the spec documents and, at
//! most, writes report artifacts under `--out-dir`.

use std::path::PathBuf;
use std::process::ExitCode;

use kria_eval::memory_graph::command::{self, RunConfig, EXIT_INTERNAL_ERROR};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let mut spec_dir: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut run_id: Option<String> = None;
    let mut quiet = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--quiet" => quiet = true,
            "--spec-dir" => {
                i += 1;
                match args.get(i) {
                    Some(v) => spec_dir = Some(PathBuf::from(v)),
                    None => return fail_usage("--spec-dir requires a path argument"),
                }
            }
            "--out-dir" => {
                i += 1;
                match args.get(i) {
                    Some(v) => out_dir = Some(PathBuf::from(v)),
                    None => return fail_usage("--out-dir requires a path argument"),
                }
            }
            "--run-id" => {
                i += 1;
                match args.get(i) {
                    Some(v) => run_id = Some(v.clone()),
                    None => return fail_usage("--run-id requires an argument"),
                }
            }
            other => return fail_usage(&format!("unknown argument: {other}")),
        }
        i += 1;
    }

    let config = RunConfig {
        spec_dir: spec_dir.unwrap_or_else(RunConfig::default_spec_dir),
        out_dir,
        run_id: run_id.unwrap_or_else(default_run_id),
        quiet,
    };

    let mut stdout = std::io::stdout().lock();
    match command::run(&config, &mut stdout) {
        Ok(outcome) => exit_code(outcome.exit_code),
        Err(error) => {
            eprintln!("{}: {error}", command::COMMAND_ID);
            exit_code(EXIT_INTERNAL_ERROR)
        }
    }
}

fn default_run_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("mg-coverage-{secs}")
}

fn fail_usage(message: &str) -> ExitCode {
    eprintln!("{}: {message}", command::COMMAND_ID);
    print_help();
    exit_code(EXIT_INTERNAL_ERROR)
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn print_help() {
    eprintln!(
        "{id} — Memory Graph coverage/orphan gate (read-only)\n\
         \n\
         USAGE:\n\
         \x20 cargo run -p kria-eval --bin mg-coverage [-- <options>]\n\
         \n\
         OPTIONS:\n\
         \x20 --spec-dir <path>   Spec directory to lint (default: in-repo spec)\n\
         \x20 --out-dir <path>    Write evidence artifacts under this directory\n\
         \x20 --run-id <id>       Run identifier for the command evidence record\n\
         \x20 --quiet             Suppress the human-readable banner (JSON-only)\n\
         \x20 -h, --help          Print this help and exit 0\n\
         \n\
         EXIT CODES: 0 pass · 1 gate failed · 2 internal error",
        id = command::COMMAND_ID
    );
}
