//! Real, production `InstallSink` for the A9 `GenerationPipeline` (product
//! gap 8/8, A9 desktop wiring fix).
//!
//! Previously (confirmed by exhaustive grep, task 10): `GenerationPipeline`
//! was constructed NOWHERE outside its own unit test file, and `InstallSink`
//! had exactly ONE implementor anywhere — `MockInstaller`, also test-only.
//! A9 was architecture + a well-tested library module ONLY, unreachable by
//! a real user.
//!
//! Real fix, additive (no A0-A9 redesign, no duplicate installer): this
//! `BundleInstallSink` is a thin adapter over the SAME, single
//! `BundleInstaller` every other real install path (local `.ocskill`,
//! marketplace) already uses — installing a generated bundle through this
//! sink goes through the exact same verify → deps → registry → activate →
//! audit → events pipeline. No second installer is introduced.

use super::designer::SkillDesign;
use super::pipeline::InstallSink;
use crate::openclaw::audit::AuditLedger;
use crate::openclaw::bundle::{BundleInstaller, SkillActivation};
use crate::openclaw::registry::SkillRegistry;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Installs a generated bundle through the real, single `BundleInstaller`.
pub struct BundleInstallSink {
    registry: Arc<SkillRegistry>,
    audit: Arc<AuditLedger>,
    store_dir: PathBuf,
    activation: Option<Arc<dyn SkillActivation>>,
}

impl BundleInstallSink {
    pub fn new(registry: Arc<SkillRegistry>, audit: Arc<AuditLedger>, store_dir: PathBuf) -> Self {
        Self {
            registry,
            audit,
            store_dir,
            activation: None,
        }
    }

    pub fn with_activation(mut self, activation: Arc<dyn SkillActivation>) -> Self {
        self.activation = Some(activation);
        self
    }
}

#[async_trait]
impl InstallSink for BundleInstallSink {
    async fn install(
        &self,
        bundle_dir: &Path,
        _design: &SkillDesign,
    ) -> Result<(String, String), String> {
        let mut installer = BundleInstaller::new(
            self.registry.clone(),
            self.audit.clone(),
            self.store_dir.clone(),
        );
        if let Some(act) = &self.activation {
            installer = installer.with_activation(act.clone());
        }

        // BundleInstaller::install is synchronous (real signature/hash
        // verification, filesystem copy, SQLite writes) — run it on a
        // blocking-safe thread so this async fn never blocks the executor.
        let bundle_dir = bundle_dir.to_path_buf();
        let outcome = tokio::task::spawn_blocking(move || installer.install(&bundle_dir))
            .await
            .map_err(|e| format!("install task panicked: {e}"))?
            .map_err(|e| e.to_string())?;

        Ok((outcome.skill_id, outcome.version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmBackend;
    use crate::openclaw::bundle::verify::keypair_from_seed;
    use crate::openclaw::generation::approval::ApprovalLayer;
    use crate::openclaw::generation::budget::{BudgetLimits, GenerationBudget};
    use crate::openclaw::generation::decision::{DecisionEngine, GenerationPolicy, SkillCandidate};
    use crate::openclaw::generation::events::GenerationEventStream;
    use crate::openclaw::generation::llm_generator::LlmSkillGenerator;
    use crate::openclaw::generation::pipeline::{
        GenerationPipeline, PipelineConfig, PipelineOutcome,
    };
    use crate::openclaw::generation::sandbox::StaticSandbox;
    use crate::openclaw::registry::ProductionSkillRegistry;
    use crate::openclaw::ToolRegistryActivation;
    use async_trait::async_trait;

    /// A deterministic fixture LLM backend (NOT a real llama.cpp connection —
    /// tagged `LlmMode::Fixture` in the eval harness). Produces well-formed
    /// JSON that satisfies `LlmSkillGenerator`'s real parsing/prompting
    /// logic, proving the REAL `LlmSkillGenerator` -> real `ModelRouter`
    /// shape (just with a fixture backend swapped in, exactly like task
    /// 11.1's existing fixture-LLM coverage).
    struct FixtureLlmBackend;

    #[async_trait]
    impl LlmBackend for FixtureLlmBackend {
        fn model_label(&self) -> &str {
            "fixture-a9-wiring-test"
        }
        fn capabilities(&self) -> &[String] {
            &[]
        }
        fn is_configured(&self) -> bool {
            true
        }

        async fn chat(
            &self,
            messages: &[crate::llm::ChatMessage],
            _tools: Option<&[crate::llm::ToolSchema]>,
            _temperature: f32,
            _max_tokens: u32,
        ) -> anyhow::Result<crate::llm::LlmResponse> {
            // Route by inspecting the STABLE system-message identifier for
            // which real pipeline stage is asking (mirrors LlmSkillGenerator's
            // real prompting shape without needing a real network call).
            let system = messages
                .first()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            let content = if system.contains("requirement analyst") {
                r#"{"intent":"organize downloads","category":"productivity","tags":["files"],"implied_capabilities":["filesystem_read","filesystem_write"]}"#.to_string()
            } else if system.contains("skill designer") {
                r#"{"name":"Organize Downloads","slug":"oc_organize_downloads_a9test","description":"Organizes files in Downloads by type.","version":"1.0.0","schema":{"type":"object","properties":{"dir":{"type":"string"}}},"documentation":"Moves files into type-named subfolders.","resource_class":"light"}"#.to_string()
            } else {
                // code generator or repair engine — same shape either way.
                r#"{"handler_code":"module.exports = async (input) => { try { return { ok: true, result: input }; } catch (e) { return { ok: false, error: String(e) }; } };","test_code":"test('runs', () => {});","examples_doc":"See README."}"#.to_string()
            };
            Ok(crate::llm::LlmResponse {
                content,
                model: "fixture-a9-wiring-test".into(),
                usage: None,
                tool_calls: None,
            })
        }

        async fn chat_stream(
            &self,
            _messages: &[crate::llm::ChatMessage],
            _tools: Option<&[crate::llm::ToolSchema]>,
            _temperature: f32,
            _max_tokens: u32,
        ) -> anyhow::Result<std::pin::Pin<Box<dyn futures::Stream<Item = String> + Send>>> {
            unimplemented!("not used by LlmSkillGenerator, only chat() is called")
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    /// FIX PROOF, real end-to-end: runs the REAL `GenerationPipeline` with a
    /// real `LlmSkillGenerator` (fixture backend, LlmMode::Fixture — never
    /// counts for freeze per design.md, but proves the REAL wiring shape),
    /// the REAL `StaticSandbox`, and this module's REAL `BundleInstallSink`
    /// (backed by the SAME `BundleInstaller` every other path uses) —
    /// confirming A9 generation is reachable end-to-end through production
    /// code, not just its own unit tests.
    #[tokio::test]
    async fn real_pipeline_with_bundle_install_sink_generates_and_installs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("a9_wiring_test.db");
        let registry = Arc::new(ProductionSkillRegistry::new(&db_path).expect("registry"));
        let audit =
            Arc::new(AuditLedger::open(&db_path, b"a9-wiring-test-key".to_vec()).expect("audit"));
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).expect("store dir");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).expect("work dir");

        let sink = BundleInstallSink::new(registry.clone(), audit, store)
            .with_activation(Arc::new(ToolRegistryActivation::new()));

        let generator = Arc::new(LlmSkillGenerator::new(Arc::new(FixtureLlmBackend)));
        let sandbox = Arc::new(StaticSandbox);
        let decision = DecisionEngine::new(0.85, GenerationPolicy::GenerateIfMissing);
        let approval = ApprovalLayer::new(true); // auto-approve for this real-wiring proof
        let events = GenerationEventStream::new();

        let pipeline = GenerationPipeline::new(
            generator,
            sandbox,
            decision,
            approval,
            events,
            Arc::new(sink),
        );

        let (signing_key, publisher_hex) = keypair_from_seed([42u8; 32]);
        let config = PipelineConfig {
            quality_threshold: 0.0, // fixture handler is minimal; don't gate on quality here
            publisher_hex,
            signing_key,
            work_dir: work,
        };
        let budget = GenerationBudget::new(BudgetLimits::default());
        let existing: Vec<SkillCandidate> = vec![];

        let outcome = pipeline
            .run(
                "a9-wiring-goal",
                "organize my downloads folder",
                &existing,
                &budget,
                &config,
            )
            .await;

        match outcome {
            PipelineOutcome::Generated { slug, .. } => {
                // The REAL registry (same one execute_semantic reads) must
                // now contain the generated skill, indistinguishable from a
                // handcrafted install.
                let installed = registry.get(&slug);
                assert!(
                    installed.is_ok(),
                    "REGRESSION: generated skill must be findable in the REAL registry after pipeline.run(), got {installed:?}"
                );
            }
            other => panic!("expected PipelineOutcome::Generated for this fixture, got {other:?}"),
        }
    }
}
