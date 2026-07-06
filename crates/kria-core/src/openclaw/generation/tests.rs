//! A9.16 ASGS tests. Deterministic mock generator + static sandbox — no live LLM/Docker.

use super::*;
use crate::safety::RiskLevel;
use async_trait::async_trait;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

// ── Mock generator producing a valid, high-quality skill ──

struct MockGenerator {
    /// How many sandbox/validation failures to inject before repair succeeds.
    fail_until_repair: u64,
    repairs: AtomicU64,
}

impl MockGenerator {
    fn perfect() -> Self {
        Self {
            fail_until_repair: 0,
            repairs: AtomicU64::new(0),
        }
    }
    fn needs_repair(n: u64) -> Self {
        Self {
            fail_until_repair: n,
            repairs: AtomicU64::new(0),
        }
    }
}

fn good_handler() -> String {
    // Production-ish JS handler with error handling + logging, no placeholders.
    "const log = (m) => console.error(m);\nmodule.exports = async (input) => {\n  try {\n    log('start');\n    if (!input) throw new Error('no input');\n    return { ok: true, result: input };\n  } catch (error) {\n    return { ok: false, error: String(error) };\n  }\n};\n".to_string()
}

#[async_trait]
impl SkillGenerator for MockGenerator {
    async fn extract_requirements(
        &self,
        prompt: &str,
    ) -> Result<(SkillRequirement, u64), GeneratorError> {
        let mut req = SkillRequirement::minimal(prompt, "productivity");
        req.tags = vec!["photos".into(), "exif".into()];
        req.implied_capabilities = vec!["filesystem_read".into(), "filesystem_write".into()];
        Ok((req, 100))
    }

    async fn design_skill(
        &self,
        req: &SkillRequirement,
    ) -> Result<(SkillDesign, u64), GeneratorError> {
        let caps = infer_capabilities(req);
        let risk = classify_risk(&caps);
        let design = SkillDesign {
            name: "Rename Photos By EXIF".into(),
            slug: "oc_rename_photos_exif".into(),
            description: "Rename photos using EXIF capture date".into(),
            category: "productivity".into(),
            tags: req.tags.clone(),
            version: "1.0.0".into(),
            capabilities: caps,
            dependencies: vec![],
            risk,
            schema: serde_json::json!({
                "type": "object",
                "properties": { "dir": { "type": "string" } },
                "required": ["dir"]
            }),
            examples: vec![
                SkillExample { description: "rename a folder".into(), params: serde_json::json!({"dir": "/photos"}) },
            ],
            documentation: "Renames each photo to its EXIF DateTimeOriginal in a stable format, preserving originals on conflict.".into(),
            runtime_kind: "docker".into(),
            entry: "handler/main.js".into(),
            resource_class: "light".into(),
        };
        Ok((design, 200))
    }

    async fn generate_code(
        &self,
        _design: &SkillDesign,
    ) -> Result<(GeneratedArtifacts, u64), GeneratorError> {
        // Inject an initial broken handler if repairs are required.
        let handler = if self.fail_until_repair > 0 {
            "module.exports = () => { TODO }".to_string() // placeholder → validation fails
        } else {
            good_handler()
        };
        Ok((
            GeneratedArtifacts {
                handler_code: handler,
                test_code: "test('runs', () => { require('../handler/main.js'); });".into(),
                examples_doc: "See README.".into(),
            },
            300,
        ))
    }

    async fn repair_code(
        &self,
        _design: &SkillDesign,
        _current: &GeneratedArtifacts,
        _failure: &str,
    ) -> Result<(GeneratedArtifacts, u64), GeneratorError> {
        let n = self.repairs.fetch_add(1, Ordering::SeqCst) + 1;
        let handler = if n >= self.fail_until_repair {
            good_handler()
        } else {
            "module.exports = () => { TODO }".to_string()
        };
        Ok((
            GeneratedArtifacts {
                handler_code: handler,
                test_code: "test('runs', () => { require('../handler/main.js'); });".into(),
                examples_doc: "See README.".into(),
            },
            50,
        ))
    }
}

// ── Mock install sink recording installs ──

#[derive(Default)]
struct MockInstaller {
    installed: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl InstallSink for MockInstaller {
    async fn install(
        &self,
        bundle_dir: &Path,
        design: &SkillDesign,
    ) -> Result<(String, String), String> {
        // Prove the bundle is a real, openable, frozen-lifecycle bundle.
        crate::openclaw::bundle::Bundle::open(bundle_dir)
            .map_err(|e| format!("bundle open failed: {e}"))?;
        self.installed.lock().unwrap().push(design.slug.clone());
        Ok((design.slug.clone(), design.version.clone()))
    }
}

fn config(work: &TempDir) -> PipelineConfig {
    // A deterministic dev publisher key (hex of 32 bytes).
    let (sk, pubhex) = crate::openclaw::bundle::verify::keypair_from_seed([21u8; 32]);
    PipelineConfig {
        quality_threshold: 0.5,
        publisher_hex: pubhex,
        signing_key: sk,
        work_dir: work.path().to_path_buf(),
    }
}

fn pipeline(
    generator: Arc<dyn SkillGenerator>,
    installer: Arc<MockInstaller>,
    policy: GenerationPolicy,
    auto_approve: bool,
) -> GenerationPipeline {
    GenerationPipeline::new(
        generator,
        Arc::new(StaticSandbox),
        DecisionEngine::new(0.72, policy),
        ApprovalLayer::new(auto_approve),
        GenerationEventStream::new(),
        installer,
    )
}

// ── Capability inference + risk (A9.4) ──

#[test]
fn infers_filesystem_caps_and_risk() {
    let mut req = SkillRequirement::minimal("rename photos and save them", "productivity");
    req.tags = vec!["exif".into()];
    let caps = infer_capabilities(&req);
    assert!(caps.contains(&"filesystem_read".to_string()));
    assert!(caps.contains(&"filesystem_write".to_string()));
    // write → YELLOW
    assert_eq!(classify_risk(&caps), RiskLevel::Yellow);

    let shell_caps = vec!["subprocess".to_string()];
    assert_eq!(classify_risk(&shell_caps), RiskLevel::Red);
}

// ── Decision engine: reuse vs generate (A9.0) ──

#[test]
fn decision_reuses_similar_skill() {
    let engine = DecisionEngine::new(0.4, GenerationPolicy::GenerateIfMissing);
    let mut req = SkillRequirement::minimal("rename photos using exif date", "productivity");
    req.implied_capabilities = vec!["filesystem_write".into()];
    req.tags = vec!["exif".into()];
    let cand = SkillCandidate {
        slug: "oc_photo_renamer".into(),
        description: "rename photos using exif date".into(),
        category: "productivity".into(),
        tags: vec!["exif".into()],
        capabilities: vec!["filesystem_write".into()],
    };
    match engine.decide(&req, &[cand]) {
        GenerationDecision::Reuse { slug, .. } => assert_eq!(slug, "oc_photo_renamer"),
        other => panic!("expected reuse, got {other:?}"),
    }
}

#[test]
fn decision_generates_when_no_match() {
    let engine = DecisionEngine::default();
    let req = SkillRequirement::minimal("do something totally novel and unrelated", "misc");
    assert_eq!(engine.decide(&req, &[]), GenerationDecision::Generate);
}

#[test]
fn decision_never_generate_policy_denies() {
    let engine = DecisionEngine::new(0.72, GenerationPolicy::NeverGenerate);
    let req = SkillRequirement::minimal("novel thing", "misc");
    assert_eq!(engine.decide(&req, &[]), GenerationDecision::Denied);
}

// ── Full pipeline: generate → validate → sandbox → install (A9.1) ──

#[tokio::test]
async fn pipeline_generates_and_installs() {
    let work = TempDir::new().unwrap();
    let installer = Arc::new(MockInstaller::default());
    let pl = pipeline(
        Arc::new(MockGenerator::perfect()),
        installer.clone(),
        GenerationPolicy::GenerateIfMissing,
        true,
    );
    let budget = GenerationBudget::new(BudgetLimits::default());

    let outcome = pl
        .run(
            "goal1",
            "Create a skill that renames all photos using EXIF date",
            &[],
            &budget,
            &config(&work),
        )
        .await;
    match outcome {
        PipelineOutcome::Generated {
            slug,
            version,
            quality,
        } => {
            assert_eq!(slug, "oc_rename_photos_exif");
            assert_eq!(version, "1.0.0");
            assert!(quality >= 0.5);
        }
        other => panic!("expected Generated, got {other:?}"),
    }
    assert_eq!(installer.installed.lock().unwrap().len(), 1);
}

// ── Reuse path: existing skill short-circuits generation ──

#[tokio::test]
async fn pipeline_reuses_existing_skill() {
    let work = TempDir::new().unwrap();
    let installer = Arc::new(MockInstaller::default());
    let pl = pipeline(
        Arc::new(MockGenerator::perfect()),
        installer.clone(),
        GenerationPolicy::GenerateIfMissing,
        true,
    );
    let budget = GenerationBudget::new(BudgetLimits::default());

    // A candidate matching the mock requirement (intent = prompt, tags photos/exif, fs caps).
    let existing = vec![SkillCandidate {
        slug: "oc_existing".into(),
        description: "Create a skill that renames all photos using EXIF date".into(),
        category: "productivity".into(),
        tags: vec!["photos".into(), "exif".into()],
        capabilities: vec!["filesystem_read".into(), "filesystem_write".into()],
    }];

    let outcome = pl
        .run(
            "g",
            "Create a skill that renames all photos using EXIF date",
            &existing,
            &budget,
            &config(&work),
        )
        .await;
    match outcome {
        PipelineOutcome::Reused { slug, .. } => assert_eq!(slug, "oc_existing"),
        other => panic!("expected Reused, got {other:?}"),
    }
    // Nothing installed.
    assert_eq!(installer.installed.lock().unwrap().len(), 0);
}

// ── Approval gate: high-risk skill awaits approval when not auto-approved ──

#[tokio::test]
async fn pipeline_awaits_approval_for_high_risk() {
    let work = TempDir::new().unwrap();
    let installer = Arc::new(MockInstaller::default());
    // auto_approve = false → filesystem_write (high-risk) must await approval.
    let pl = pipeline(
        Arc::new(MockGenerator::perfect()),
        installer.clone(),
        GenerationPolicy::GenerateIfMissing,
        false,
    );
    let budget = GenerationBudget::new(BudgetLimits::default());

    let outcome = pl
        .run("g", "rename photos", &[], &budget, &config(&work))
        .await;
    match outcome {
        PipelineOutcome::AwaitingApproval { slug, reasons, .. } => {
            assert_eq!(slug, "oc_rename_photos_exif");
            assert!(reasons.contains(&"filesystem_write".to_string()));
        }
        other => panic!("expected AwaitingApproval, got {other:?}"),
    }
    assert_eq!(installer.installed.lock().unwrap().len(), 0);
}

// ── Repair loop: broken code repaired then installs (A9.8) ──

#[tokio::test]
async fn pipeline_repairs_then_installs() {
    let work = TempDir::new().unwrap();
    let installer = Arc::new(MockInstaller::default());
    // Fail (placeholder code) until the 2nd repair succeeds.
    let pl = pipeline(
        Arc::new(MockGenerator::needs_repair(2)),
        installer.clone(),
        GenerationPolicy::GenerateIfMissing,
        true,
    );
    let budget = GenerationBudget::new(BudgetLimits::default());

    let outcome = pl
        .run("g", "rename photos", &[], &budget, &config(&work))
        .await;
    assert!(
        matches!(outcome, PipelineOutcome::Generated { .. }),
        "got {outcome:?}"
    );
    assert_eq!(installer.installed.lock().unwrap().len(), 1);
}

// ── Budget exhaustion aborts safely (A9.0.4) ──

#[tokio::test]
async fn pipeline_budget_exhaustion_aborts() {
    let work = TempDir::new().unwrap();
    let installer = Arc::new(MockInstaller::default());
    let pl = pipeline(
        Arc::new(MockGenerator::needs_repair(100)),
        installer.clone(),
        GenerationPolicy::GenerateIfMissing,
        true,
    );
    // Tight repair budget → exhausts before success.
    let mut limits = BudgetLimits::default();
    limits.max_repair_attempts = 2;
    let budget = GenerationBudget::new(limits);

    let outcome = pl
        .run("g", "rename photos", &[], &budget, &config(&work))
        .await;
    assert!(
        matches!(outcome, PipelineOutcome::Failed { .. }),
        "got {outcome:?}"
    );
    assert_eq!(installer.installed.lock().unwrap().len(), 0);
}

// ── Validator rejects placeholder code + slug conflict ──

#[test]
fn validator_flags_placeholder_and_conflict() {
    let work = TempDir::new().unwrap();
    let design = SkillDesign {
        name: "X".into(),
        slug: "oc_x".into(),
        description: "d".into(),
        category: "misc".into(),
        tags: vec![],
        version: "1.0.0".into(),
        capabilities: vec![],
        dependencies: vec![],
        risk: RiskLevel::Green,
        schema: serde_json::json!({"type":"object"}),
        examples: vec![],
        documentation: "docs docs docs docs docs docs docs docs docs".into(),
        runtime_kind: "docker".into(),
        entry: "handler/main.js".into(),
        resource_class: "light".into(),
    };
    let (_sk, pubhex) = crate::openclaw::bundle::verify::keypair_from_seed([1u8; 32]);
    let artifacts = GeneratedArtifacts {
        handler_code: "module.exports = () => { TODO };".into(),
        test_code: "".into(),
        examples_doc: "".into(),
    };
    let dir = emit_bundle(&design, &artifacts, work.path(), &pubhex).unwrap();
    let issues = SkillValidator::validate(&dir, &design, &artifacts, &["oc_x".to_string()]);
    assert!(issues
        .iter()
        .any(|i| matches!(i, ValidationIssue::PlaceholderCode(_))));
    assert!(issues.iter().any(|i| matches!(i, ValidationIssue::NoTests)));
    assert!(issues
        .iter()
        .any(|i| matches!(i, ValidationIssue::SlugConflict(_))));
}

// ── Stress: 100 sequential generations, unique slugs ──

#[tokio::test]
async fn stress_100_generations() {
    let work = TempDir::new().unwrap();
    let installer = Arc::new(MockInstaller::default());
    let budget = GenerationBudget::new(BudgetLimits {
        max_generation_attempts: 0,
        ..Default::default()
    });

    for i in 0..100 {
        // Unique generator per run producing a unique slug.
        struct UniqueGen(u64);
        #[async_trait]
        impl SkillGenerator for UniqueGen {
            async fn extract_requirements(
                &self,
                p: &str,
            ) -> Result<(SkillRequirement, u64), GeneratorError> {
                Ok((SkillRequirement::minimal(p, "misc"), 10))
            }
            async fn design_skill(
                &self,
                _r: &SkillRequirement,
            ) -> Result<(SkillDesign, u64), GeneratorError> {
                Ok((
                    SkillDesign {
                        name: format!("S{}", self.0),
                        slug: format!("oc_gen_{}", self.0),
                        description: "generated skill".into(),
                        category: "misc".into(),
                        tags: vec![],
                        version: "1.0.0".into(),
                        capabilities: vec![],
                        dependencies: vec![],
                        risk: RiskLevel::Green,
                        schema: serde_json::json!({"type":"object","properties":{}}),
                        examples: vec![SkillExample {
                            description: "e".into(),
                            params: serde_json::json!({}),
                        }],
                        documentation: "documentation for the generated skill here.".into(),
                        runtime_kind: "docker".into(),
                        entry: "handler/main.js".into(),
                        resource_class: "light".into(),
                    },
                    20,
                ))
            }
            async fn generate_code(
                &self,
                _d: &SkillDesign,
            ) -> Result<(GeneratedArtifacts, u64), GeneratorError> {
                Ok((
                    GeneratedArtifacts {
                        handler_code: good_handler(),
                        test_code: "test('x',()=>{});".into(),
                        examples_doc: "docs".into(),
                    },
                    20,
                ))
            }
            async fn repair_code(
                &self,
                _d: &SkillDesign,
                c: &GeneratedArtifacts,
                _f: &str,
            ) -> Result<(GeneratedArtifacts, u64), GeneratorError> {
                Ok((c.clone(), 1))
            }
        }
        let pl = pipeline(
            Arc::new(UniqueGen(i)),
            installer.clone(),
            GenerationPolicy::GenerateIfMissing,
            true,
        );
        let outcome = pl
            .run(
                &format!("g{i}"),
                "make a thing",
                &[],
                &budget,
                &config(&work),
            )
            .await;
        assert!(
            matches!(outcome, PipelineOutcome::Generated { .. }),
            "run {i}: {outcome:?}"
        );
    }
    assert_eq!(installer.installed.lock().unwrap().len(), 100);
}
