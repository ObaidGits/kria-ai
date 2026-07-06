//! A9.1 The ONE Authoritative Generation Pipeline.
//!
//! Goal → requirement extraction → decision (reuse vs generate) → design → code →
//! materialize bundle → validate → sandbox test → repair loop → quality → approval →
//! package/sign/install (via the FROZEN installer) → registry/marketplace → execution.
//!
//! There is exactly ONE pipeline. Generated skills are ordinary `.ocskill` bundles and
//! flow through the same frozen lifecycle as manual/marketplace skills (A9.15).

use super::approval::ApprovalLayer;
use super::budget::{BudgetDimension, GenerationBudget};
use super::codegen;
use super::decision::{DecisionEngine, GenerationDecision, SkillCandidate};
use super::designer::SkillDesign;
use super::events::{GenerationEvent, GenerationEventStream};
use super::generator::{GeneratedArtifacts, SkillGenerator};
use super::quality::{QualityEvaluator, QualityScore};
use super::sandbox::SandboxTester;
use super::validator::SkillValidator;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Sink that installs a materialized+signed bundle through the FROZEN lifecycle
/// (publishing → verification → `BundleInstaller` → registry → semantic router).
///
/// Implemented by the host wiring; the pipeline never re-implements installation
/// (A9.9/A9.15). Returns the installed (slug, version).
#[async_trait]
pub trait InstallSink: Send + Sync {
    async fn install(
        &self,
        bundle_dir: &Path,
        design: &SkillDesign,
    ) -> Result<(String, String), String>;
}

/// Terminal outcome of a generation request (A9.1).
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineOutcome {
    /// Reused an existing skill instead of generating (A9.0).
    Reused { slug: String, similarity: f64 },
    /// Generated + installed a new skill.
    Generated {
        slug: String,
        version: String,
        quality: f64,
    },
    /// Bundle ready but installation awaits human approval (A9.0.3).
    AwaitingApproval {
        slug: String,
        bundle_dir: PathBuf,
        reasons: Vec<String>,
    },
    /// Policy AskUser: a human must choose reuse-vs-generate.
    AwaitingUser {
        best_match: Option<String>,
        similarity: f64,
    },
    /// Policy forbids generation and nothing suitable exists.
    Denied,
    /// Generation failed terminally.
    Failed { reason: String },
}

/// Configuration for one pipeline run.
pub struct PipelineConfig {
    /// Minimum overall quality for auto-install.
    pub quality_threshold: f64,
    /// ed25519 publisher public key (hex) used for signing the generated bundle.
    pub publisher_hex: String,
    /// ed25519 SIGNING key matching `publisher_hex` (real production bug fix,
    /// A9 desktop wiring: `emit_bundle` only materializes the manifest with
    /// the public key baked in — it never writes `MANIFEST.sha256`/
    /// `bundle.sig`. Without a real signing step, `BundleInstaller::install`
    /// (via `TrustPolicy::strict()`, `require_signature: true`) always
    /// rejects the generated bundle with "missing required file:
    /// MANIFEST.sha256" — confirmed by direct reproduction. The pipeline now
    /// signs the emitted bundle with this key before handing it to the
    /// `InstallSink`, using the SAME real primitives
    /// (`bundle::verify::{write_hash_tree, sign_bundle}`) every other real
    /// install path uses — no parallel signing system.
    pub signing_key: ed25519_dalek::SigningKey,
    /// Directory where generated bundles are materialized.
    pub work_dir: PathBuf,
}

/// The single generation pipeline (A9.1).
pub struct GenerationPipeline {
    generator: Arc<dyn SkillGenerator>,
    sandbox: Arc<dyn SandboxTester>,
    decision: DecisionEngine,
    approval: ApprovalLayer,
    events: GenerationEventStream,
    installer: Arc<dyn InstallSink>,
}

impl GenerationPipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        generator: Arc<dyn SkillGenerator>,
        sandbox: Arc<dyn SandboxTester>,
        decision: DecisionEngine,
        approval: ApprovalLayer,
        events: GenerationEventStream,
        installer: Arc<dyn InstallSink>,
    ) -> Self {
        Self {
            generator,
            sandbox,
            decision,
            approval,
            events,
            installer,
        }
    }

    pub fn events(&self) -> GenerationEventStream {
        self.events.clone()
    }

    /// Run the full pipeline for a user prompt against the installed skill set.
    pub async fn run(
        &self,
        goal_id: &str,
        prompt: &str,
        existing: &[SkillCandidate],
        budget: &GenerationBudget,
        config: &PipelineConfig,
    ) -> PipelineOutcome {
        self.events.emit(GenerationEvent::Started {
            goal_id: goal_id.to_string(),
            prompt: prompt.to_string(),
        });

        // A9.2: requirement extraction.
        let (req, tokens) = match self.generator.extract_requirements(prompt).await {
            Ok(x) => x,
            Err(e) => return self.fail(goal_id, format!("requirement extraction failed: {e}")),
        };
        if budget.charge_tokens(tokens).is_err() {
            return self.budget_abort(goal_id, BudgetDimension::Tokens);
        }
        self.events.emit(GenerationEvent::RequirementsExtracted {
            goal_id: goal_id.to_string(),
            intent: req.intent.clone(),
        });

        // A9.0: decision — reuse vs generate.
        match self.decision.decide(&req, existing) {
            GenerationDecision::Reuse { slug, similarity } => {
                self.events.emit(GenerationEvent::ReusedExisting {
                    goal_id: goal_id.to_string(),
                    skill_id: slug.clone(),
                    similarity,
                });
                return PipelineOutcome::Reused { slug, similarity };
            }
            GenerationDecision::Denied => return PipelineOutcome::Denied,
            GenerationDecision::AskUser {
                best_match,
                similarity,
            } => {
                return PipelineOutcome::AwaitingUser {
                    best_match,
                    similarity,
                };
            }
            GenerationDecision::Generate => {}
        }

        // Existing slugs for validation (never regenerate installed).
        let existing_slugs: Vec<String> = existing.iter().map(|c| c.slug.clone()).collect();

        // A9 generation attempt loop.
        loop {
            let attempt = match budget.generation_attempt() {
                Ok(n) => n,
                Err(_) => return self.budget_abort(goal_id, BudgetDimension::GenerationAttempts),
            };

            match self
                .attempt_generation(goal_id, &req, &existing_slugs, budget, config, attempt)
                .await
            {
                AttemptResult::Success {
                    design,
                    quality,
                    bundle_dir,
                } => {
                    return self
                        .finalize(goal_id, design, quality, bundle_dir, config)
                        .await;
                }
                AttemptResult::Budget(dim) => return self.budget_abort(goal_id, dim),
                AttemptResult::Retry(reason) => {
                    self.events.emit(GenerationEvent::RepairAttempt {
                        goal_id: goal_id.to_string(),
                        slug: req.intent.clone(),
                        attempt,
                        reason,
                    });
                    continue;
                }
                AttemptResult::Fatal(reason) => return self.fail(goal_id, reason),
            }
        }
    }

    fn fail(&self, goal_id: &str, reason: String) -> PipelineOutcome {
        self.events.emit(GenerationEvent::Failed {
            goal_id: goal_id.to_string(),
            reason: reason.clone(),
        });
        PipelineOutcome::Failed { reason }
    }

    fn budget_abort(&self, goal_id: &str, dim: BudgetDimension) -> PipelineOutcome {
        self.events.emit(GenerationEvent::BudgetExhausted {
            goal_id: goal_id.to_string(),
            dimension: dim.as_str().to_string(),
        });
        PipelineOutcome::Failed {
            reason: format!("budget exhausted: {}", dim.as_str()),
        }
    }
}

/// Internal per-attempt result.
enum AttemptResult {
    Success {
        design: SkillDesign,
        quality: QualityScore,
        bundle_dir: PathBuf,
    },
    Retry(String),
    Budget(BudgetDimension),
    Fatal(String),
}

impl GenerationPipeline {
    /// One full design→code→validate→sandbox→quality attempt with an inner repair loop.
    async fn attempt_generation(
        &self,
        goal_id: &str,
        req: &super::requirements::SkillRequirement,
        existing_slugs: &[String],
        budget: &GenerationBudget,
        config: &PipelineConfig,
        _attempt: u32,
    ) -> AttemptResult {
        // A9.3: design.
        let (design, tokens) = match self.generator.design_skill(req).await {
            Ok(x) => x,
            Err(e) => return AttemptResult::Retry(format!("design failed: {e}")),
        };
        if budget.charge_tokens(tokens).is_err() {
            return AttemptResult::Budget(BudgetDimension::Tokens);
        }
        self.events.emit(GenerationEvent::Designed {
            goal_id: goal_id.to_string(),
            slug: design.slug.clone(),
        });

        // A9.5: code.
        let (mut artifacts, tokens) = match self.generator.generate_code(&design).await {
            Ok(x) => x,
            Err(e) => return AttemptResult::Retry(format!("codegen failed: {e}")),
        };
        if budget.charge_tokens(tokens).is_err() {
            return AttemptResult::Budget(BudgetDimension::Tokens);
        }
        self.events.emit(GenerationEvent::CodeGenerated {
            goal_id: goal_id.to_string(),
            slug: design.slug.clone(),
        });

        // A9.8: repair loop over validate + sandbox.
        loop {
            let bundle_dir = match codegen::emit_bundle(
                &design,
                &artifacts,
                &config.work_dir,
                &config.publisher_hex,
            ) {
                Ok(d) => d,
                Err(e) => return AttemptResult::Fatal(format!("codegen emit failed: {e}")),
            };

            // Real fix (A9 desktop wiring bug): sign the emitted bundle with
            // the SAME real primitives every other real install path uses —
            // `emit_bundle` alone never produces `MANIFEST.sha256`/
            // `bundle.sig`, and `BundleInstaller::install` (strict trust
            // policy) rejects an unsigned bundle outright.
            if let Err(e) = super::super::bundle::verify::write_hash_tree(&bundle_dir) {
                return AttemptResult::Fatal(format!("bundle hash-tree write failed: {e}"));
            }
            if let Err(e) =
                super::super::bundle::verify::sign_bundle(&bundle_dir, &config.signing_key)
            {
                return AttemptResult::Fatal(format!("bundle signing failed: {e}"));
            }

            // A9.6: validate.
            let issues = SkillValidator::validate(&bundle_dir, &design, &artifacts, existing_slugs);
            // Slug conflict is fatal for this attempt (never regenerate installed skill).
            if issues
                .iter()
                .any(|i| matches!(i, super::validator::ValidationIssue::SlugConflict(_)))
            {
                return AttemptResult::Fatal("slug conflicts with an installed skill".into());
            }
            let validation_ok = issues.is_empty();
            self.events.emit(GenerationEvent::Validated {
                goal_id: goal_id.to_string(),
                slug: design.slug.clone(),
                passed: validation_ok,
            });

            // A9.7: sandbox test.
            let sandbox = if validation_ok {
                self.sandbox.test(&bundle_dir, &design).await
            } else {
                super::sandbox::SandboxResult::fail(format!("validation issues: {issues:?}"))
            };
            self.events.emit(GenerationEvent::SandboxTested {
                goal_id: goal_id.to_string(),
                slug: design.slug.clone(),
                passed: sandbox.passed,
            });

            if validation_ok && sandbox.passed && sandbox.clean {
                // A9.0.5: quality.
                let quality = QualityEvaluator::evaluate(&design, &artifacts, true, true);
                self.events.emit(GenerationEvent::QualityScored {
                    goal_id: goal_id.to_string(),
                    slug: design.slug.clone(),
                    overall: quality.overall,
                });
                if !quality.meets(config.quality_threshold) {
                    // Try to repair for quality once via the repair budget below.
                    let failure = format!(
                        "quality {:.2} below threshold {:.2}",
                        quality.overall, config.quality_threshold
                    );
                    match self.try_repair(&design, &artifacts, &failure, budget).await {
                        RepairOutcome::Repaired(new) => {
                            artifacts = new;
                            continue;
                        }
                        RepairOutcome::Budget(dim) => return AttemptResult::Budget(dim),
                        RepairOutcome::Give => {
                            return AttemptResult::Fatal(format!(
                                "quality below threshold and repair exhausted: {:.2}",
                                quality.overall
                            ))
                        }
                    }
                }
                return AttemptResult::Success {
                    design,
                    quality,
                    bundle_dir,
                };
            }

            // Failed → repair (A9.8).
            let failure = sandbox
                .failure
                .clone()
                .unwrap_or_else(|| format!("validation issues: {issues:?}"));
            match self.try_repair(&design, &artifacts, &failure, budget).await {
                RepairOutcome::Repaired(new) => {
                    artifacts = new;
                    continue;
                }
                RepairOutcome::Budget(dim) => return AttemptResult::Budget(dim),
                RepairOutcome::Give => {
                    return AttemptResult::Retry(format!("repair exhausted: {failure}"))
                }
            }
        }
    }

    /// A9.8: one repair step, budget-guarded.
    async fn try_repair(
        &self,
        design: &SkillDesign,
        current: &GeneratedArtifacts,
        failure: &str,
        budget: &GenerationBudget,
    ) -> RepairOutcome {
        if budget.repair_attempt().is_err() {
            return RepairOutcome::Budget(BudgetDimension::RepairAttempts);
        }
        match self.generator.repair_code(design, current, failure).await {
            Ok((new, tokens)) => {
                if budget.charge_tokens(tokens).is_err() {
                    return RepairOutcome::Budget(BudgetDimension::Tokens);
                }
                RepairOutcome::Repaired(new)
            }
            Err(_) => RepairOutcome::Give,
        }
    }

    /// A9.0.3 approval gate → package/sign/install via the frozen sink → events.
    async fn finalize(
        &self,
        goal_id: &str,
        design: SkillDesign,
        quality: QualityScore,
        bundle_dir: PathBuf,
        _config: &PipelineConfig,
    ) -> PipelineOutcome {
        // A9.0.3: approval gate. Generation is complete; installation may wait.
        if !self.approval.may_install(&design) {
            let reasons = match ApprovalLayer::requirement(&design) {
                super::approval::ApprovalRequirement::Required(r) => r,
                _ => Vec::new(),
            };
            self.events.emit(GenerationEvent::AwaitingApproval {
                goal_id: goal_id.to_string(),
                slug: design.slug.clone(),
                reasons: reasons.clone(),
            });
            return PipelineOutcome::AwaitingApproval {
                slug: design.slug,
                bundle_dir,
                reasons,
            };
        }

        // A9.9/A9.15: install through the FROZEN lifecycle.
        let start = Instant::now();
        match self.installer.install(&bundle_dir, &design).await {
            Ok((slug, version)) => {
                self.events.emit(GenerationEvent::Installed {
                    goal_id: goal_id.to_string(),
                    slug: slug.clone(),
                    version: version.clone(),
                });
                self.events.emit(GenerationEvent::ExecutionSuccess {
                    goal_id: goal_id.to_string(),
                    slug: slug.clone(),
                    latency_ms: start.elapsed().as_millis() as u64,
                });
                PipelineOutcome::Generated {
                    slug,
                    version,
                    quality: quality.overall,
                }
            }
            Err(e) => self.fail(goal_id, format!("install failed: {e}")),
        }
    }
}

enum RepairOutcome {
    Repaired(GeneratedArtifacts),
    Budget(BudgetDimension),
    Give,
}
