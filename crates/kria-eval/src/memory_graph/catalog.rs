//! Command-catalog wrapper layer for the Memory Graph Production Redesign spec
//! (task F0.4 / 0.4.3).
//!
//! `validation.md` §4 ("Command Catalog") enumerates every command the evidence
//! matrix (§5) may invoke, each tagged either `existing target` (a command whose
//! underlying recipe/script already exists in the repository) or `planned
//! target` (a command that is declared but *not yet built*). This module renders
//! that catalog as strongly-typed Rust values and provides an **honest**
//! invocation layer:
//!
//! * every catalog command carries its canonical ID, its exact argv +
//!   working directory (verbatim from §4), and its declared status;
//! * [`CatalogCommand::availability`] classifies each command as
//!   [`Availability::Implemented`], [`Availability::Planned`], or
//!   [`Availability::Absent`] *from evidence* — a `planned target` is always
//!   `Planned`, and an `existing target` is only `Implemented` when its
//!   underlying recipe/script/program is actually found on disk (otherwise it is
//!   `Absent`);
//! * [`CatalogCommand::run`] is **fail-closed against fabrication**: it only
//!   executes a command that is `Implemented`. For a `Planned` or `Absent`
//!   command it returns [`RunOutcome::NotAvailable`] *without executing anything
//!   and without fabricating output*, so a manifest can honestly record that the
//!   command was not run;
//! * [`execute_step`] is the real, exit-code-capturing executing wrapper: it
//!   runs one command step and captures its argv, working directory, and exit
//!   code into a [`CommandInvocation`] (reusing the manifest type). A program
//!   that cannot be spawned yields an I/O error — never a fabricated success.
//!
//! ## Scope boundary (0.4.3 only)
//!
//! This task builds the faithful catalog + invocation-record producer. It does
//! **not** enforce reviewer independence or sign-off (that is 0.4.4), and it
//! does **not** resolve the predecessor/gate promotion chain (that is 0.4.5).
//! It also does not *run* the heavy real suites (`CMD-GUI-E2E`,
//! `CMD-ADVERSARIAL`, etc.) — it models/wraps them so they can be invoked
//! honestly, but never invents a result for a command that did not actually run.
//!
//! `CMD-MG-COVERAGE` (the F0.1 read-only coverage gate, see [`super::command`])
//! is intentionally *not* part of this catalog: it is not one of the §4 catalog
//! entries. This module models exactly the fourteen commands `validation.md` §4
//! declares and does not invent additional entries.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::manifest::CommandInvocation;

/// The status `validation.md` §4 assigns to a catalog command.
///
/// This is the *declared* status straight from the catalog table; it is distinct
/// from the evidence-derived [`Availability`] computed at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeclaredStatus {
    /// `existing target`: the underlying recipe/script is expected to exist.
    ExistingTarget,
    /// `planned target`: declared but not yet built; must never be executed or
    /// have its success fabricated.
    PlannedTarget,
}

/// The evidence-derived availability of a catalog command.
///
/// Task-critical invariant ("without pretending absent commands exist"): a
/// command is only [`Availability::Implemented`] when concrete evidence shows it
/// can run. A `planned target` is always [`Availability::Planned`]; an
/// `existing target` whose underlying recipe/script/program is not found is
/// [`Availability::Absent`], never `Implemented`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    /// The command's underlying target exists and can be executed.
    Implemented,
    /// The command is declared as a planned target and is not yet built.
    Planned,
    /// The command is declared existing, but its underlying target is missing.
    Absent,
}

impl Availability {
    /// Whether a command with this availability may actually be executed.
    pub fn is_runnable(self) -> bool {
        matches!(self, Availability::Implemented)
    }

    /// Stable machine code for reports/manifests.
    pub fn code(self) -> &'static str {
        match self {
            Availability::Implemented => "implemented",
            Availability::Planned => "planned",
            Availability::Absent => "absent",
        }
    }
}

/// One concrete command step: exact argv plus repository-relative working
/// directory. Most catalog commands have a single step; `CMD-MG-CONTRACT` has
/// two, and `CMD-MG-SBOM` has none (its command is "to be added and pinned").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandStep {
    /// Exact argv, element 0 is the program (verbatim from `validation.md` §4).
    pub argv: Vec<String>,
    /// Repository-relative working directory (`"."` = repository root).
    pub working_directory: String,
}

impl CommandStep {
    /// Build a step from an argv slice and a repository-relative working dir.
    fn new(argv: &[&str], working_directory: &str) -> Self {
        CommandStep {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            working_directory: working_directory.to_string(),
        }
    }
}

/// A single `validation.md` §4 catalog command modeled faithfully.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCommand {
    /// Canonical command ID (e.g. `CMD-RUST-UNIT`).
    pub command_id: String,
    /// The status declared in the catalog table.
    pub declared_status: DeclaredStatus,
    /// The concrete command step(s) this command runs.
    pub steps: Vec<CommandStep>,
    /// The "intended use" column, verbatim from the catalog.
    pub intended_use: String,
}

/// The outcome of a guarded catalog-command run.
///
/// The `Executed` variant only ever appears when the command was
/// [`Availability::Implemented`] and each step was actually spawned. The
/// `NotAvailable` variant records — honestly — that nothing was executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RunOutcome {
    /// The command was not runnable; nothing was executed or fabricated.
    NotAvailable {
        /// Why the command was not run (`Planned` or `Absent`).
        availability: Availability,
        /// Deterministic human-readable reason.
        reason: String,
    },
    /// The command ran; one [`CommandInvocation`] per executed step.
    Executed {
        /// Captured argv/cwd/exit-code records, one per step.
        invocations: Vec<CommandInvocation>,
    },
}

impl RunOutcome {
    /// Whether this outcome represents an actually-executed command.
    pub fn was_executed(&self) -> bool {
        matches!(self, RunOutcome::Executed { .. })
    }
}

impl CatalogCommand {
    fn new(
        command_id: &str,
        declared_status: DeclaredStatus,
        steps: Vec<CommandStep>,
        intended_use: &str,
    ) -> Self {
        CatalogCommand {
            command_id: command_id.to_string(),
            declared_status,
            steps,
            intended_use: intended_use.to_string(),
        }
    }

    /// Classify this command's availability from evidence under `repo_root`.
    ///
    /// * A `planned target` is always [`Availability::Planned`] — even when the
    ///   underlying tool (e.g. `cargo`) happens to exist, the command itself is
    ///   not yet built, so we never pretend it is runnable.
    /// * An `existing target` is [`Availability::Implemented`] only when *every*
    ///   step's underlying recipe/script/program is found; otherwise it is
    ///   [`Availability::Absent`]. It is never classified `Implemented` on
    ///   missing evidence.
    pub fn availability(&self, repo_root: &Path) -> Availability {
        match self.declared_status {
            DeclaredStatus::PlannedTarget => Availability::Planned,
            DeclaredStatus::ExistingTarget => {
                if self.steps.is_empty() {
                    return Availability::Absent;
                }
                if self.steps.iter().all(|s| step_target_exists(s, repo_root)) {
                    Availability::Implemented
                } else {
                    Availability::Absent
                }
            }
        }
    }

    /// Guarded run: execute the command only if it is [`Availability::Implemented`].
    ///
    /// For a `Planned` or `Absent` command this returns
    /// [`RunOutcome::NotAvailable`] *without executing anything* — it does not
    /// spawn a process and does not fabricate a `CommandInvocation`. For an
    /// `Implemented` command it runs each step and captures a real
    /// [`CommandInvocation`] (argv/cwd/exit code) per step.
    pub fn run(&self, repo_root: &Path) -> std::io::Result<RunOutcome> {
        let availability = self.availability(repo_root);
        match availability {
            Availability::Planned => Ok(RunOutcome::NotAvailable {
                availability,
                reason: format!(
                    "{} is a declared planned target; not yet implemented — not executed",
                    self.command_id
                ),
            }),
            Availability::Absent => Ok(RunOutcome::NotAvailable {
                availability,
                reason: format!(
                    "{} is declared existing but its underlying target was not found — not executed",
                    self.command_id
                ),
            }),
            Availability::Implemented => {
                let mut invocations = Vec::with_capacity(self.steps.len());
                for step in &self.steps {
                    invocations.push(execute_step(&self.command_id, step, repo_root)?);
                }
                Ok(RunOutcome::Executed { invocations })
            }
        }
    }
}

/// The repository root, resolved relative to this crate (`crates/kria-eval`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Build the full `validation.md` §4 command catalog, in table order.
pub fn catalog() -> Vec<CatalogCommand> {
    use DeclaredStatus::{ExistingTarget, PlannedTarget};

    vec![
        // ── Existing targets ────────────────────────────────────────────────
        CatalogCommand::new(
            "CMD-RUST-UNIT",
            ExistingTarget,
            vec![CommandStep::new(&["just", "test"], ".")],
            "workspace library regression; not sufficient alone",
        ),
        CatalogCommand::new(
            "CMD-COGNITION",
            ExistingTarget,
            vec![CommandStep::new(&["just", "test-cognition"], ".")],
            "cognition regression; not sufficient alone",
        ),
        CatalogCommand::new(
            "CMD-GUI-E2E",
            ExistingTarget,
            vec![CommandStep::new(&["just", "test-e2e"], ".")],
            "sandboxed GUI E2E",
        ),
        CatalogCommand::new(
            "CMD-ADVERSARIAL",
            ExistingTarget,
            vec![CommandStep::new(&["just", "test-adversarial"], ".")],
            "adversarial GUI cases",
        ),
        CatalogCommand::new(
            "CMD-UI-UNIT",
            ExistingTarget,
            vec![CommandStep::new(&["npm", "run", "test:run"], "ui")],
            "frontend unit/component tests",
        ),
        CatalogCommand::new(
            "CMD-UI-E2E",
            ExistingTarget,
            vec![CommandStep::new(&["npm", "run", "e2e"], "ui")],
            "Playwright E2E",
        ),
        CatalogCommand::new(
            "CMD-UI-A11Y",
            ExistingTarget,
            vec![CommandStep::new(&["npm", "run", "e2e:a11y"], "ui")],
            "current accessibility target; suite content must be extended",
        ),
        CatalogCommand::new(
            "CMD-UI-PERF",
            ExistingTarget,
            vec![CommandStep::new(&["npm", "run", "e2e:performance"], "ui")],
            "current performance target; suite content must be extended",
        ),
        // ── Planned targets ─────────────────────────────────────────────────
        CatalogCommand::new(
            "CMD-MG-CORE",
            PlannedTarget,
            vec![CommandStep::new(
                &[
                    "cargo",
                    "test",
                    "-p",
                    "kria-core",
                    "--test",
                    "memory_graph_v2",
                    "--",
                    "--nocapture",
                ],
                ".",
            )],
            "authority/schema/policy/semantic suites",
        ),
        CatalogCommand::new(
            "CMD-MG-EVAL",
            PlannedTarget,
            vec![CommandStep::new(
                &[
                    "cargo",
                    "run",
                    "-p",
                    "kria-eval",
                    "--",
                    "memory-graph",
                    "--manifest",
                    "<run-root>/manifest.json",
                ],
                ".",
            )],
            "fixtures, retrieval, performance, artifact emission",
        ),
        CatalogCommand::new(
            "CMD-MG-CONTRACT",
            PlannedTarget,
            vec![
                CommandStep::new(
                    &[
                        "cargo",
                        "test",
                        "-p",
                        "kria-desktop",
                        "--test",
                        "memory_v2_contract",
                    ],
                    ".",
                ),
                CommandStep::new(
                    &[
                        "cargo",
                        "test",
                        "-p",
                        "kria-server",
                        "--test",
                        "memory_v2_contract",
                    ],
                    ".",
                ),
            ],
            "normalized transport parity",
        ),
        CatalogCommand::new(
            "CMD-MG-VISUAL",
            PlannedTarget,
            vec![CommandStep::new(
                &[
                    "npm",
                    "run",
                    "e2e",
                    "--",
                    "memory-control-center.visual.spec.ts",
                ],
                "ui",
            )],
            "deterministic visual-semantic matrix",
        ),
        CatalogCommand::new(
            "CMD-MG-ORCA",
            PlannedTarget,
            vec![CommandStep::new(
                &[
                    "npm",
                    "run",
                    "e2e",
                    "--",
                    "memory-control-center.orca.spec.ts",
                ],
                "ui",
            )],
            "Orca transcript and keyboard tasks",
        ),
        CatalogCommand::new(
            "CMD-MG-SBOM",
            PlannedTarget,
            // "repository release evidence command, to be added and pinned" —
            // no concrete argv exists yet, so the command has no steps.
            Vec::new(),
            "SBOM/license/vulnerability production",
        ),
    ]
}

/// Look up a catalog command by its canonical ID.
pub fn find(command_id: &str) -> Option<CatalogCommand> {
    catalog().into_iter().find(|c| c.command_id == command_id)
}

/// The real, exit-code-capturing executing wrapper.
///
/// Runs `step` under `repo_root`/`step.working_directory` and captures its argv,
/// working directory, and process exit code into a [`CommandInvocation`]. If the
/// program cannot be spawned, the underlying I/O error is returned — this
/// function never fabricates a success for a command that did not run.
pub fn execute_step(
    command_id: &str,
    step: &CommandStep,
    repo_root: &Path,
) -> std::io::Result<CommandInvocation> {
    let program = step.argv.first().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "command step has empty argv",
        )
    })?;

    let cwd = repo_root.join(&step.working_directory);
    let status = Command::new(program)
        .args(&step.argv[1..])
        .current_dir(&cwd)
        .status()?;

    Ok(CommandInvocation {
        command_id: command_id.to_string(),
        argv: step.argv.clone(),
        working_directory: step.working_directory.clone(),
        // `code()` is `None` only when the process was killed by a signal; we
        // record `-1` rather than inventing a plausible exit status.
        exit_code: status.code().unwrap_or(-1),
    })
}

/// Whether a step's underlying target exists as evidence under `repo_root`.
///
/// * `just <recipe>` → the `justfile` defines `<recipe>`;
/// * `npm run <script>` → the working-dir `package.json` defines `<script>`;
/// * any other program → the program is found on `PATH` (or as a direct path).
fn step_target_exists(step: &CommandStep, repo_root: &Path) -> bool {
    match step.argv.split_first() {
        None => false,
        Some((program, rest)) => match program.as_str() {
            "just" => rest
                .first()
                .map(|recipe| justfile_has_recipe(repo_root, recipe))
                .unwrap_or(false),
            "npm" => {
                // Expect `npm run <script> ...`.
                if rest.len() >= 2 && rest[0] == "run" {
                    npm_script_exists(repo_root, &step.working_directory, &rest[1])
                } else {
                    false
                }
            }
            _ => program_on_path(program),
        },
    }
}

/// Whether the repository `justfile` defines a recipe named `recipe`.
fn justfile_has_recipe(repo_root: &Path, recipe: &str) -> bool {
    let content = std::fs::read_to_string(repo_root.join("justfile"))
        .or_else(|_| std::fs::read_to_string(repo_root.join("Justfile")));
    let Ok(content) = content else {
        return false;
    };
    for line in content.lines() {
        // Recipe definitions start at column 0 and are not comments.
        if line.starts_with(char::is_whitespace) || line.starts_with('#') {
            continue;
        }
        if !line.contains(':') {
            continue;
        }
        // The recipe name is the first token up to whitespace or ':'.
        let name = line.split([':', ' ', '\t']).next().unwrap_or("");
        if name == recipe {
            return true;
        }
    }
    false
}

/// Whether the `package.json` under `repo_root`/`working_dir` defines `script`.
fn npm_script_exists(repo_root: &Path, working_dir: &str, script: &str) -> bool {
    let pkg = repo_root.join(working_dir).join("package.json");
    let Ok(content) = std::fs::read_to_string(pkg) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value.get("scripts").and_then(|s| s.get(script)).is_some()
}

/// Whether `program` resolves to an existing file, either as a direct path or by
/// searching `PATH`.
fn program_on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full set of command IDs `validation.md` §4 declares, and their
    /// declared statuses.
    const EXISTING: &[&str] = &[
        "CMD-RUST-UNIT",
        "CMD-COGNITION",
        "CMD-GUI-E2E",
        "CMD-ADVERSARIAL",
        "CMD-UI-UNIT",
        "CMD-UI-E2E",
        "CMD-UI-A11Y",
        "CMD-UI-PERF",
    ];
    const PLANNED: &[&str] = &[
        "CMD-MG-CORE",
        "CMD-MG-EVAL",
        "CMD-MG-CONTRACT",
        "CMD-MG-VISUAL",
        "CMD-MG-ORCA",
        "CMD-MG-SBOM",
    ];

    #[test]
    fn catalog_enumerates_exactly_the_validation_md_commands() {
        let ids: Vec<String> = catalog().iter().map(|c| c.command_id.clone()).collect();

        // Every expected existing/planned ID is present exactly once.
        for id in EXISTING.iter().chain(PLANNED.iter()) {
            assert_eq!(
                ids.iter().filter(|got| got.as_str() == *id).count(),
                1,
                "catalog missing or duplicating {id}"
            );
        }
        // No unexpected extras (e.g. CMD-MG-COVERAGE must not leak in).
        assert_eq!(ids.len(), EXISTING.len() + PLANNED.len());
    }

    #[test]
    fn declared_status_matches_validation_md() {
        for cmd in catalog() {
            if EXISTING.contains(&cmd.command_id.as_str()) {
                assert_eq!(
                    cmd.declared_status,
                    DeclaredStatus::ExistingTarget,
                    "{} should be an existing target",
                    cmd.command_id
                );
            } else if PLANNED.contains(&cmd.command_id.as_str()) {
                assert_eq!(
                    cmd.declared_status,
                    DeclaredStatus::PlannedTarget,
                    "{} should be a planned target",
                    cmd.command_id
                );
            } else {
                panic!("unexpected catalog command {}", cmd.command_id);
            }
        }
    }

    #[test]
    fn existing_targets_classify_implemented_against_real_repo() {
        let root = repo_root();
        for cmd in catalog() {
            if cmd.declared_status == DeclaredStatus::ExistingTarget {
                assert_eq!(
                    cmd.availability(&root),
                    Availability::Implemented,
                    "{} should resolve to Implemented (its just recipe / npm script exists)",
                    cmd.command_id
                );
            }
        }
    }

    #[test]
    fn planned_targets_classify_planned_and_never_run() {
        let root = repo_root();
        for cmd in catalog() {
            if cmd.declared_status == DeclaredStatus::PlannedTarget {
                assert_eq!(
                    cmd.availability(&root),
                    Availability::Planned,
                    "{} must be Planned even if its base tool exists",
                    cmd.command_id
                );
                // Guarded run must refuse to execute and must not fabricate.
                let outcome = cmd.run(&root).expect("run is infallible for planned");
                match outcome {
                    RunOutcome::NotAvailable { availability, .. } => {
                        assert_eq!(availability, Availability::Planned);
                    }
                    RunOutcome::Executed { .. } => {
                        panic!("{} was executed but is only planned", cmd.command_id)
                    }
                }
            }
        }
    }

    #[test]
    fn cmd_mg_eval_is_planned_even_though_cargo_and_kria_eval_exist() {
        // The "without pretending absent commands exist" invariant: cargo and
        // the kria-eval crate really exist, but the `memory-graph` subcommand is
        // not built, so the command stays Planned and never runs.
        let root = repo_root();
        let cmd = find("CMD-MG-EVAL").expect("catalog has CMD-MG-EVAL");
        assert_eq!(cmd.availability(&root), Availability::Planned);
        assert!(!cmd.run(&root).expect("infallible").was_executed());
    }

    #[test]
    fn absent_existing_target_never_classifies_implemented() {
        let root = repo_root();
        // A synthetic "existing" command whose just recipe does not exist.
        let ghost = CatalogCommand::new(
            "CMD-GHOST",
            DeclaredStatus::ExistingTarget,
            vec![CommandStep::new(
                &["just", "this-recipe-does-not-exist-xyz"],
                ".",
            )],
            "synthetic absent command",
        );
        assert_eq!(ghost.availability(&root), Availability::Absent);

        // And an existing target that names a nonexistent program is Absent too.
        let ghost_prog = CatalogCommand::new(
            "CMD-GHOST-PROG",
            DeclaredStatus::ExistingTarget,
            vec![CommandStep::new(&["kria-nonexistent-binary-xyz"], ".")],
            "synthetic absent program",
        );
        assert_eq!(ghost_prog.availability(&root), Availability::Absent);

        // Guarded run refuses to execute an absent command.
        let outcome = ghost.run(&root).expect("infallible");
        match outcome {
            RunOutcome::NotAvailable { availability, .. } => {
                assert_eq!(availability, Availability::Absent)
            }
            RunOutcome::Executed { .. } => panic!("absent command was executed"),
        }
    }

    #[test]
    fn availability_is_never_implemented_on_missing_evidence() {
        // Invariant over a range of synthetic existing-target shapes: if the
        // underlying target is missing, availability is Absent, never Implemented.
        let root = repo_root();
        let shapes: Vec<Vec<CommandStep>> = vec![
            vec![],                                                    // no steps
            vec![CommandStep::new(&["just", "nope-xyz"], ".")],        // missing recipe
            vec![CommandStep::new(&["npm", "run", "nope-xyz"], "ui")], // missing script
            vec![CommandStep::new(&["nope-binary-xyz"], ".")],         // missing program
            vec![
                CommandStep::new(&["just", "test"], "."),     // one real...
                CommandStep::new(&["just", "nope-xyz"], "."), // ...one missing
            ],
        ];
        for steps in shapes {
            let cmd = CatalogCommand::new(
                "CMD-SYNTH",
                DeclaredStatus::ExistingTarget,
                steps,
                "synthetic",
            );
            assert_ne!(
                cmd.availability(&root),
                Availability::Implemented,
                "missing-evidence command must never be Implemented"
            );
        }
    }

    #[test]
    fn execute_step_captures_argv_cwd_and_zero_exit() {
        let root = repo_root();
        let step = CommandStep::new(&["true"], ".");
        let inv = execute_step("CMD-TRIVIAL", &step, &root).expect("true runs");
        assert_eq!(inv.command_id, "CMD-TRIVIAL");
        assert_eq!(inv.argv, vec!["true".to_string()]);
        assert_eq!(inv.working_directory, ".");
        assert_eq!(inv.exit_code, 0);
    }

    #[test]
    fn execute_step_captures_nonzero_exit_without_fabrication() {
        let root = repo_root();
        let step = CommandStep::new(&["false"], ".");
        let inv = execute_step("CMD-TRIVIAL-FAIL", &step, &root).expect("false runs");
        assert_eq!(inv.exit_code, 1);
    }

    #[test]
    fn execute_step_errors_when_program_missing() {
        let root = repo_root();
        let step = CommandStep::new(&["kria-nonexistent-binary-xyz"], ".");
        // A missing program yields an I/O error — never a fabricated success.
        assert!(execute_step("CMD-MISSING", &step, &root).is_err());
    }

    #[test]
    fn guarded_run_executes_only_implemented_trivial_command() {
        let root = repo_root();
        // A synthetic Implemented command wrapping the trivial `true` program.
        let cmd = CatalogCommand::new(
            "CMD-TRIVIAL-OK",
            DeclaredStatus::ExistingTarget,
            vec![CommandStep::new(&["true"], ".")],
            "trivial implemented command",
        );
        assert_eq!(cmd.availability(&root), Availability::Implemented);
        let outcome = cmd.run(&root).expect("runs");
        match outcome {
            RunOutcome::Executed { invocations } => {
                assert_eq!(invocations.len(), 1);
                assert_eq!(invocations[0].exit_code, 0);
                assert_eq!(invocations[0].argv, vec!["true".to_string()]);
            }
            RunOutcome::NotAvailable { .. } => panic!("implemented command should execute"),
        }
    }
}
