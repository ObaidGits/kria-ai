//! MVP acceptance gates (memory-upgrade task 20): crash/restart recovery.
//!
//! Verifies invariant L1/L2/L10 in practice: a file-backed authority survives a
//! process-equivalent restart (drop + reopen) with zero data loss, and derived
//! memory + retrieval are reconstructable after reopen (CP-6).

use std::sync::Arc;

use async_trait::async_trait;
use kria_core::memory::api::{MemoryConfig, MemorySystem};
use kria_core::memory::error::MemoryResult;
use kria_core::memory::stores::ports::Embedder;
use kria_core::memory::types::{Availability, ModelVersion, WriteCandidate};
use uuid::Uuid;

/// Deterministic embedder so the test needs no ONNX model on disk.
struct FakeEmbedder;

#[async_trait]
impl Embedder for FakeEmbedder {
    fn model_version(&self) -> ModelVersion {
        ModelVersion("fake_v1".into())
    }
    fn dim(&self) -> usize {
        16
    }
    async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; 16];
                for (i, b) in t.bytes().enumerate() {
                    v[i % 16] += b as f32 / 255.0;
                }
                v
            })
            .collect())
    }
    async fn health(&self) -> Availability {
        Availability::Up
    }
}

#[tokio::test]
async fn authority_survives_restart_with_zero_loss() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("kria_memory.db");
    let config = MemoryConfig {
        db_path: db_path.to_string_lossy().to_string(),
        ..MemoryConfig::default()
    };
    let session = Uuid::now_v7();

    // First "process": remember + flush, then drop (simulating shutdown).
    {
        let sys = MemorySystem::open_for_test(config.clone(), Arc::new(FakeEmbedder)).unwrap();
        let d = sys
            .remember(WriteCandidate::user(
                session,
                "kria persists across restarts",
            ))
            .unwrap();
        assert!(matches!(
            d,
            kria_core::memory::types::WriteDecision::Queued { .. }
        ));
        let processed = sys.flush().await.unwrap();
        assert_eq!(processed, 1);
        assert_eq!(sys.health().await.unwrap().memory_count, 1);
        sys.shutdown();
        // `sys` dropped here → connections closed, WAL checkpointed on close.
    }

    // Second "process": reopen the same file. Data must still be there and the
    // derived memory retrievable (indexes rebuilt/persisted).
    {
        let sys = MemorySystem::open_for_test(config, Arc::new(FakeEmbedder)).unwrap();
        let health = sys.health().await.unwrap();
        assert_eq!(
            health.memory_count, 1,
            "authority must survive restart (L2)"
        );
        assert!(
            health.event_count >= 1,
            "event log must survive restart (L1)"
        );

        let res = sys.search("persists", None).await.unwrap();
        assert!(
            !res.hits.is_empty(),
            "derived memory must be retrievable after restart"
        );
        assert!(res.hits[0].memory.content.contains("persists"));
    }
}

#[tokio::test]
async fn incognito_leaves_no_trace_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("kria_memory.db");
    let config = MemoryConfig {
        db_path: db_path.to_string_lossy().to_string(),
        ..MemoryConfig::default()
    };
    let session = Uuid::now_v7();
    {
        let sys = MemorySystem::open_for_test(config.clone(), Arc::new(FakeEmbedder)).unwrap();
        sys.set_mode(session, kria_core::memory::types::MemoryMode::Incognito);
        let _ = sys.remember(WriteCandidate::user(session, "should never persist"));
        sys.flush().await.unwrap();
    }
    {
        let sys = MemorySystem::open_for_test(config, Arc::new(FakeEmbedder)).unwrap();
        assert_eq!(sys.health().await.unwrap().memory_count, 0);
        assert_eq!(sys.health().await.unwrap().event_count, 0);
    }
}
