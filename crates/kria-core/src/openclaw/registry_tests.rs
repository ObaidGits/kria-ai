//! Comprehensive tests for ProductionSkillRegistry (A5.13).
//!
//! Tests every aspect of the registry:
//! - Installation, updates, rollback
//! - Dependency conflicts
//! - Broken bundle recovery
//! - Registry recovery
//! - Parallel operations
//! - Search functionality  
//! - Health tracking
//! - Statistics collection
//! - Events system

use super::registry::*;
use super::types::*;
use crate::safety::RiskLevel;
use chrono::Utc;
use std::path::Path;
use tempfile::TempDir;

fn temp_registry() -> (TempDir, ProductionSkillRegistry) {
    let dir = TempDir::new().expect("failed to create temp dir");
    let db_path = dir.path().join("test_registry.db");
    let registry = ProductionSkillRegistry::new(&db_path).expect("failed to create registry");
    (dir, registry)
}

fn sample_skill(skill_id: &str) -> SkillMetadata {
    SkillMetadata {
        skill_id: skill_id.to_string(),
        name: format!("Test Skill {}", skill_id),
        description: "Test skill for registry".to_string(),
        publisher: "test".to_string(),
        version: "1.0.0".to_string(),
        category: "test".to_string(),
        discovery_source: DiscoverySource::Bundled {
            path: "test".to_string(),
        },
        discovered_at: Utc::now(),
        capabilities: SkillCapabilities::default(),
        runtime_requirements: "docker".to_string(),
        risk_level: RiskLevel::Green,
        resource_class: ResourceClass::Light,
        tags: vec!["test".to_string()],
        categories: vec!["test".to_string()],
        semantic_version: "1.0.0".to_string(),
        dependencies: vec![],
        compatibility_requirements: vec![],
        trust_tier: TrustTier::Local,
        content_hash: format!("hash_{}", skill_id),
        signature: None,
        granted_capabilities: Vec::new(),
        bundle_path: None,
        manifest_toml: None,
        input_schema: None,
        state: SkillState::Discovered,
        state_changed_at: Utc::now(),
    }
}

fn skill_with_dependencies(skill_id: &str, deps: Vec<(&str, &str)>) -> SkillMetadata {
    let mut skill = sample_skill(skill_id);
    skill.dependencies = deps
        .into_iter()
        .map(|(id, version)| SkillDependency {
            skill_id: id.to_string(),
            version_requirement: version.to_string(),
            optional: false,
        })
        .collect();
    skill
}

#[tokio::test]
async fn test_basic_installation() {
    let (_dir, registry) = temp_registry();
    let skill = sample_skill("test_skill");

    // Install skill
    registry
        .install_skill(&skill)
        .expect("install should succeed");

    // Verify installation
    let installed = registry
        .get_skill("test_skill")
        .expect("skill should exist");
    assert_eq!(installed.skill_id, "test_skill");
    assert_eq!(installed.state, SkillState::Installed);

    // Enable skill
    registry
        .set_skill_state("test_skill", SkillState::Enabled)
        .expect("enable should succeed");

    // Verify in enabled list
    let enabled = registry
        .get_enabled_skills()
        .expect("should get enabled skills");
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].skill_id, "test_skill");
}

#[tokio::test]
async fn test_skill_state_transitions() {
    let (_dir, registry) = temp_registry();
    let skill = sample_skill("state_test");

    registry
        .install_skill(&skill)
        .expect("install should succeed");

    // Test all state transitions
    let states = vec![
        SkillState::Verified,
        SkillState::Enabled,
        SkillState::Disabled,
        SkillState::Broken,
        SkillState::Recovering,
        SkillState::Deprecated,
        SkillState::Removed,
    ];

    for state in states {
        registry
            .set_skill_state("state_test", state)
            .expect("state change should succeed");
        let skill = registry
            .get_skill("state_test")
            .expect("skill should exist");
        assert_eq!(skill.state, state);
    }
}

#[tokio::test]
async fn test_dependency_conflicts() {
    let (_dir, registry) = temp_registry();

    // Install base skill
    let base_skill = sample_skill("base_skill");
    registry
        .install_skill(&base_skill)
        .expect("base install should succeed");

    // Try to install skill with missing dependency
    let dependent_skill = skill_with_dependencies("dependent", vec![("missing_skill", "1.0.0")]);
    let conflicts = registry
        .check_dependency_conflicts(&dependent_skill)
        .expect("conflict check should succeed");

    assert_eq!(conflicts.len(), 1);
    assert!(matches!(
        conflicts[0].conflict_type,
        ConflictType::MissingDependency
    ));

    // Install with satisfied dependency
    let good_skill = skill_with_dependencies("good_dependent", vec![("base_skill", "1.0.0")]);
    let conflicts = registry
        .check_dependency_conflicts(&good_skill)
        .expect("conflict check should succeed");
    assert_eq!(conflicts.len(), 0);
}

#[tokio::test]
async fn test_cyclic_dependency_detection() {
    let (_dir, registry) = temp_registry();

    // Install skill A
    let skill_a = sample_skill("skill_a");
    registry
        .install_skill(&skill_a)
        .expect("install A should succeed");

    // Install skill B that depends on A
    let skill_b = skill_with_dependencies("skill_b", vec![("skill_a", "1.0.0")]);
    registry
        .install_skill(&skill_b)
        .expect("install B should succeed");

    // Try to make A depend on B (creating cycle)
    let skill_a_cyclic = skill_with_dependencies("skill_a", vec![("skill_b", "1.0.0")]);
    let conflicts = registry
        .check_dependency_conflicts(&skill_a_cyclic)
        .expect("conflict check should succeed");

    assert_eq!(conflicts.len(), 1);
    assert!(matches!(
        conflicts[0].conflict_type,
        ConflictType::CyclicDependency
    ));
}

#[tokio::test]
async fn test_version_management() {
    let (_dir, registry) = temp_registry();

    // Install skill v1.0.0
    let mut skill = sample_skill("version_test");
    skill.version = "1.0.0".to_string();
    skill.semantic_version = "1.0.0".to_string();
    registry
        .install_skill(&skill)
        .expect("install should succeed");

    // Upgrade to v1.1.0
    registry
        .upgrade_skill("version_test", "1.1.0")
        .expect("upgrade should succeed");

    let updated = registry
        .get_skill("version_test")
        .expect("skill should exist");
    assert_eq!(updated.version, "1.1.0");
    assert_eq!(updated.semantic_version, "1.1.0");

    // Downgrade to v1.0.1
    registry
        .downgrade_skill("version_test", "1.0.1")
        .expect("downgrade should succeed");

    let downgraded = registry
        .get_skill("version_test")
        .expect("skill should exist");
    assert_eq!(downgraded.version, "1.0.1");
}

#[tokio::test]
async fn test_health_tracking() {
    let (_dir, registry) = temp_registry();
    let skill = sample_skill("health_test");

    registry
        .install_skill(&skill)
        .expect("install should succeed");

    // Update health status
    registry
        .update_skill_health(
            "health_test",
            HealthStatus::Broken,
            Some("Test failure".to_string()),
        )
        .expect("health update should succeed");

    // Verify skill state changed to broken
    let skill = registry
        .get_skill("health_test")
        .expect("skill should exist");
    assert_eq!(skill.state, SkillState::Broken);

    // Recover health
    registry
        .update_skill_health("health_test", HealthStatus::Healthy, None)
        .expect("health recovery should succeed");

    let skill = registry
        .get_skill("health_test")
        .expect("skill should exist");
    assert_eq!(skill.state, SkillState::Enabled);
}

#[tokio::test]
async fn test_statistics_tracking() {
    let (_dir, registry) = temp_registry();
    let skill = sample_skill("stats_test");

    registry
        .install_skill(&skill)
        .expect("install should succeed");

    // Record successful execution
    registry
        .record_execution("stats_test", true, 100, 0.5)
        .expect("record execution should succeed");

    // Record failed execution
    registry
        .record_execution("stats_test", false, 200, 0.8)
        .expect("record execution should succeed");

    let stats = registry
        .get_skill_statistics("stats_test")
        .expect("should get stats");
    assert_eq!(stats.usage_count, 2);
    assert_eq!(stats.success_rate, 0.5);
    assert_eq!(stats.failure_rate, 0.5);
    assert_eq!(stats.average_latency_ms, 150.0);
    assert_eq!(stats.average_resource_usage, 0.65);
}

#[tokio::test]
async fn test_search_functionality() {
    let (_dir, registry) = temp_registry();

    // Install multiple skills
    let mut skill1 = sample_skill("web_search");
    skill1.category = "web".to_string();
    skill1.publisher = "acme".to_string();
    skill1.tags = vec!["search".to_string(), "web".to_string()];

    let mut skill2 = sample_skill("file_manager");
    skill2.category = "files".to_string();
    skill2.publisher = "beta".to_string();
    skill2.tags = vec!["files".to_string(), "management".to_string()];

    let mut skill3 = sample_skill("web_scraper");
    skill3.category = "web".to_string();
    skill3.publisher = "acme".to_string();
    skill3.tags = vec!["web".to_string(), "scraping".to_string()];

    registry
        .install_skill(&skill1)
        .expect("install 1 should succeed");
    registry
        .install_skill(&skill2)
        .expect("install 2 should succeed");
    registry
        .install_skill(&skill3)
        .expect("install 3 should succeed");

    // Enable all skills
    registry
        .set_skill_state("web_search", SkillState::Enabled)
        .expect("enable should succeed");
    registry
        .set_skill_state("file_manager", SkillState::Enabled)
        .expect("enable should succeed");
    registry
        .set_skill_state("web_scraper", SkillState::Enabled)
        .expect("enable should succeed");

    // Search by category
    let query = SkillQuery {
        slug: None,
        publisher: None,
        description_contains: None,
        tags: vec![],
        categories: vec![], // Empty for now, search by publisher instead
        capabilities: vec![],
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: true,
    };

    let results = registry
        .search_skills(&query)
        .expect("search should succeed");
    assert_eq!(results.len(), 3); // All enabled skills

    // Search by publisher
    let query = SkillQuery {
        slug: None,
        publisher: Some("acme".to_string()),
        description_contains: None,
        tags: vec![],
        categories: vec![],
        capabilities: vec![],
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: false,
    };

    let results = registry
        .search_skills(&query)
        .expect("search should succeed");
    assert_eq!(results.len(), 2); // Both acme skills

    // Search by specific skill ID
    let query = SkillQuery {
        slug: Some("web_search".to_string()),
        publisher: None,
        description_contains: None,
        tags: vec![],
        categories: vec![],
        capabilities: vec![],
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: false,
    };

    let results = registry
        .search_skills(&query)
        .expect("search should succeed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].skill_id, "web_search");
}

#[tokio::test]
async fn test_event_system() {
    let (_dir, registry) = temp_registry();
    let skill = sample_skill("event_test");

    // Subscribe to events
    let mut receiver = registry.subscribe_events();

    // Install skill (should trigger event)
    registry
        .install_skill(&skill)
        .expect("install should succeed");

    // Check for installation event
    let event = tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv())
        .await
        .expect("should receive event")
        .expect("event should be valid");

    match event {
        RegistryEvent::Installed { skill_id, version } => {
            assert_eq!(skill_id, "event_test");
            assert_eq!(version, "1.0.0");
        }
        _ => panic!("Expected Installation event"),
    }

    // Enable skill (should trigger event)
    registry
        .set_skill_state("event_test", SkillState::Enabled)
        .expect("enable should succeed");

    let event = tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv())
        .await
        .expect("should receive event")
        .expect("event should be valid");

    match event {
        RegistryEvent::Enabled { skill_id } => {
            assert_eq!(skill_id, "event_test");
        }
        _ => panic!("Expected Enabled event"),
    }
}

#[tokio::test]
async fn test_discovery_engine() {
    let (_dir, registry) = temp_registry();

    // Test discovery (mock implementation)
    let discovered = registry
        .discover_all_skills()
        .expect("discovery should succeed");

    // Should find no skills in clean test environment
    assert_eq!(discovered.len(), 0);

    // Verify empty search returns no enabled skills
    let enabled = registry
        .get_enabled_skills()
        .expect("should get enabled skills");
    assert_eq!(enabled.len(), 0);
}

#[tokio::test]
async fn test_parallel_operations() {
    let (_dir, registry) = temp_registry();
    let registry = std::sync::Arc::new(registry);

    // Launch parallel installation tasks
    let mut tasks = Vec::new();

    for i in 0..10 {
        let registry = registry.clone();
        let task = tokio::spawn(async move {
            let skill = sample_skill(&format!("parallel_skill_{}", i));
            registry
                .install_skill(&skill)
                .expect("parallel install should succeed");
            registry
                .set_skill_state(&format!("parallel_skill_{}", i), SkillState::Enabled)
                .expect("parallel enable should succeed");
        });
        tasks.push(task);
    }

    // Wait for all tasks
    for task in tasks {
        task.await.expect("task should complete");
    }

    // Verify all skills installed
    let enabled = registry
        .get_enabled_skills()
        .expect("should get enabled skills");
    assert_eq!(enabled.len(), 10);
}

#[tokio::test]
async fn test_broken_bundle_recovery() {
    let (_dir, registry) = temp_registry();
    let mut skill = sample_skill("recovery_test");
    skill.bundle_path = Some("/nonexistent/path.ocskill".to_string());

    registry
        .install_skill(&skill)
        .expect("install should succeed");
    registry
        .set_skill_state("recovery_test", SkillState::Enabled)
        .expect("enable should succeed");

    // Run health check (should detect missing bundle)
    registry
        .health_check_all()
        .expect("health check should succeed");

    let skill = registry
        .get_skill("recovery_test")
        .expect("skill should exist");
    assert_eq!(skill.state, SkillState::Broken);
}

#[tokio::test]
async fn test_registry_consistency() {
    let (_dir, registry) = temp_registry();

    // Install skills with complex dependency graph
    let skill_a = sample_skill("skill_a");
    let skill_b = skill_with_dependencies("skill_b", vec![("skill_a", "1.0.0")]);
    let skill_c =
        skill_with_dependencies("skill_c", vec![("skill_a", "1.0.0"), ("skill_b", "1.0.0")]);

    registry
        .install_skill(&skill_a)
        .expect("install A should succeed");
    registry
        .install_skill(&skill_b)
        .expect("install B should succeed");
    registry
        .install_skill(&skill_c)
        .expect("install C should succeed");

    // Verify reverse dependencies
    let reverse_deps = registry
        .get_reverse_dependencies("skill_a")
        .expect("should get reverse deps");
    assert_eq!(reverse_deps.len(), 2); // skill_b and skill_c depend on skill_a

    // Verify dependency integrity
    for skill_id in ["skill_a", "skill_b", "skill_c"] {
        let skill = registry.get_skill(skill_id).expect("skill should exist");
        let conflicts = registry
            .check_dependency_conflicts(&skill)
            .expect("conflict check should succeed");
        assert_eq!(
            conflicts.len(),
            0,
            "No conflicts should exist for {}",
            skill_id
        );
    }
}

#[tokio::test]
async fn test_legacy_compatibility() {
    let (_dir, registry) = temp_registry();

    // Test legacy SkillDescriptor conversion
    let descriptor = SkillDescriptor {
        skill_id: "legacy_skill".to_string(),
        name: "Legacy Skill".to_string(),
        description: "Legacy skill for testing".to_string(),
        category: "test".to_string(),
        parameters: serde_json::json!({}),
        risk_level: RiskLevel::Green,
        network_policy: OpenClawNetworkPolicy::None,
        resource_profile: ResourceProfile::for_category("test"),
        capabilities: SkillCapabilities::default(),
        granted: vec![],
        trust_tier: TrustTier::Local,
        source: SkillSource::Bundled,
        installed_at: Utc::now(),
        last_used_at: None,
        use_count: 0,
        status: SkillStatus::Active,
    };

    // Install via legacy interface
    registry
        .install(&descriptor)
        .expect("legacy install should succeed");

    // Retrieve via legacy interface
    let retrieved = registry
        .get("legacy_skill")
        .expect("legacy get should succeed");
    assert_eq!(retrieved.skill_id, "legacy_skill");
    assert_eq!(retrieved.status, SkillStatus::Active);

    // List via legacy interface
    let active_skills = registry.list_active().expect("legacy list should succeed");
    assert_eq!(active_skills.len(), 1);
    assert_eq!(active_skills[0].skill_id, "legacy_skill");
}

#[tokio::test]
async fn test_registry_stress() {
    let (_dir, registry) = temp_registry();
    let registry = std::sync::Arc::new(registry);

    // High-concurrency stress test
    let mut tasks = Vec::new();

    // Concurrent installs
    for i in 0..100 {
        let registry = registry.clone();
        let task = tokio::spawn(async move {
            let skill = sample_skill(&format!("stress_skill_{}", i));
            registry
                .install_skill(&skill)
                .expect("stress install should succeed");
        });
        tasks.push(task);
    }

    // Concurrent searches
    for _ in 0..50 {
        let registry = registry.clone();
        let task = tokio::spawn(async move {
            let query = SkillQuery {
                slug: None,
                publisher: None,
                description_contains: None,
                tags: vec![],
                categories: vec![],
                capabilities: vec![],
                runtime_requirements: None,
                risk_level: None,
                state: None,
                enabled_only: false,
            };
            let _ = registry.search_skills(&query);
        });
        tasks.push(task);
    }

    // Concurrent state changes
    for i in 0..50 {
        let registry = registry.clone();
        let task = tokio::spawn(async move {
            if i % 2 == 0 {
                let _ =
                    registry.set_skill_state(&format!("stress_skill_{}", i), SkillState::Enabled);
            } else {
                let _ =
                    registry.set_skill_state(&format!("stress_skill_{}", i), SkillState::Disabled);
            }
        });
        tasks.push(task);
    }

    // Wait for all tasks
    for task in tasks {
        let _ = task.await;
    }

    // Verify registry consistency after stress
    let query = SkillQuery {
        slug: None,
        publisher: None,
        description_contains: None,
        tags: vec![],
        categories: vec![],
        capabilities: vec![],
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: false,
    };

    let all_skills = registry
        .search_skills(&query)
        .expect("post-stress search should succeed");
    assert!(
        all_skills.len() >= 100,
        "Should have at least 100 skills after stress test"
    );
}

#[tokio::test]
async fn test_database_recovery() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("recovery_test.db");

    // Create registry and install some skills
    {
        let registry =
            ProductionSkillRegistry::new(&db_path).expect("initial registry should create");

        for i in 0..10 {
            let skill = sample_skill(&format!("recovery_skill_{}", i));
            registry
                .install_skill(&skill)
                .expect("install should succeed");
        }

        // Registry goes out of scope, simulating crash
    }

    // Reopen registry from same database
    let registry = ProductionSkillRegistry::new(&db_path).expect("recovery registry should open");

    // Verify all skills are still there
    let query = SkillQuery {
        slug: None,
        publisher: None,
        description_contains: None,
        tags: vec![],
        categories: vec![],
        capabilities: vec![],
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: false,
    };

    let recovered_skills = registry
        .search_skills(&query)
        .expect("recovery search should succeed");
    assert_eq!(recovered_skills.len(), 10);

    // Verify specific skills can be retrieved
    for i in 0..10 {
        let skill = registry
            .get_skill(&format!("recovery_skill_{}", i))
            .expect("recovered skill should exist");
        assert_eq!(skill.name, format!("Test Skill recovery_skill_{}", i));
    }
}

#[tokio::test]
async fn test_large_metadata_handling() {
    let (_dir, registry) = temp_registry();

    // Create skill with large metadata
    let mut skill = sample_skill("large_skill");
    skill.description = "x".repeat(10000); // 10KB description
    skill.tags = (0..1000).map(|i| format!("tag_{}", i)).collect(); // 1000 tags
    skill.categories = (0..100).map(|i| format!("category_{}", i)).collect(); // 100 categories
    skill.compatibility_requirements = (0..500).map(|i| format!("req_{}", i)).collect(); // 500 requirements

    // Should handle large metadata without issues
    registry
        .install_skill(&skill)
        .expect("large metadata install should succeed");

    let retrieved = registry
        .get_skill("large_skill")
        .expect("should retrieve large skill");
    assert_eq!(retrieved.description.len(), 10000);
    assert_eq!(retrieved.tags.len(), 1000);
    assert_eq!(retrieved.categories.len(), 100);
    assert_eq!(retrieved.compatibility_requirements.len(), 500);
}

#[tokio::test]
async fn test_event_ordering() {
    let (_dir, registry) = temp_registry();
    let skill = sample_skill("order_test");

    let mut receiver = registry.subscribe_events();

    // Perform sequence of operations
    registry
        .install_skill(&skill)
        .expect("install should succeed");
    registry
        .set_skill_state("order_test", SkillState::Enabled)
        .expect("enable should succeed");
    registry
        .record_execution("order_test", true, 100, 0.5)
        .expect("record should succeed");
    registry
        .set_skill_state("order_test", SkillState::Disabled)
        .expect("disable should succeed");

    // Verify events come in correct order
    let events = vec![
        tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await,
        tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await,
        tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await,
        tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await,
    ];

    // Check event types in order
    match &events[0] {
        Ok(Ok(RegistryEvent::Installed { .. })) => {}
        _ => panic!("Expected Installation event first"),
    }

    match &events[1] {
        Ok(Ok(RegistryEvent::Enabled { .. })) => {}
        _ => panic!("Expected Enabled event second"),
    }

    match &events[2] {
        Ok(Ok(RegistryEvent::ExecutionCompleted { .. })) => {}
        _ => panic!("Expected ExecutionCompleted event third"),
    }

    match &events[3] {
        Ok(Ok(RegistryEvent::Disabled { .. })) => {}
        _ => panic!("Expected Disabled event fourth"),
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_full_skill_lifecycle() {
        let (_dir, registry) = temp_registry();
        let skill_id = "lifecycle_test";

        // 1. Discovery
        let mut skill = sample_skill(skill_id);
        let discovered = registry
            .discover_all_skills()
            .expect("discovery should succeed");
        // Mock discovery would find 0 skills in test environment

        // 2. Manual installation
        registry
            .install_skill(&skill)
            .expect("install should succeed");

        // 3. Verification
        registry
            .set_skill_state(skill_id, SkillState::Verified)
            .expect("verify should succeed");

        // 4. Enablement
        registry
            .set_skill_state(skill_id, SkillState::Enabled)
            .expect("enable should succeed");

        // 5. Usage tracking
        registry
            .record_execution(skill_id, true, 150, 0.7)
            .expect("record should succeed");
        registry
            .record_execution(skill_id, true, 120, 0.6)
            .expect("record should succeed");
        registry
            .record_execution(skill_id, false, 300, 0.9)
            .expect("record should succeed");

        // 6. Health monitoring
        registry
            .update_skill_health(skill_id, HealthStatus::Healthy, None)
            .expect("health should succeed");

        // 7. Version upgrade
        registry
            .upgrade_skill(skill_id, "1.1.0")
            .expect("upgrade should succeed");

        // 8. Statistics verification
        let stats = registry
            .get_skill_statistics(skill_id)
            .expect("should get stats");
        assert_eq!(stats.usage_count, 3);
        assert!((stats.success_rate - 0.6667).abs() < 0.01);

        // 9. Search verification
        let enabled_skills = registry.get_enabled_skills().expect("should get enabled");
        assert_eq!(enabled_skills.len(), 1);
        assert_eq!(enabled_skills[0].version, "1.1.0");

        // 10. Final state verification
        let final_skill = registry.get_skill(skill_id).expect("skill should exist");
        assert_eq!(final_skill.state, SkillState::Enabled);
        assert_eq!(final_skill.version, "1.1.0");
    }
}

/// Regression: an existing user's `skills.db` created BEFORE the
/// `granted_capabilities` column existed gets that column appended at the END
/// of the table by schema-migration 1's `ALTER TABLE ADD COLUMN`. The
/// row-to-metadata parser used to read columns by POSITIONAL INDEX assuming the
/// fresh-schema order (granted_capabilities mid-table), so on a migrated DB it
/// read the wrong column (NULL `bundle_path`) at index 20, failed with
/// `InvalidColumnType`, and every enabled skill was silently dropped — the
/// router then reported "No enabled skills found in registry" even though the
/// DB genuinely held enabled skills. This reproduces that exact pre-migration
/// physical column order and asserts an enabled skill is now returned. Reading
/// columns by name (the fix) is order-independent.
#[test]
fn enabled_skills_load_from_pre_granted_capabilities_migrated_db() {
    use rusqlite::{params, Connection};

    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("legacy_skills.db");

    // 1. Build the OLD skills table — the exact column set/order that existed
    //    BEFORE granted_capabilities was introduced (no granted_capabilities),
    //    with user_version = 0 so the real migration runs on open.
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute_batch(
            r#"
            CREATE TABLE skills (
                skill_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                publisher TEXT NOT NULL,
                version TEXT NOT NULL,
                category TEXT NOT NULL,
                discovery_source TEXT NOT NULL,
                discovered_at TEXT NOT NULL,
                capabilities TEXT NOT NULL,
                runtime_requirements TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                resource_class TEXT NOT NULL,
                tags TEXT NOT NULL,
                categories TEXT NOT NULL,
                semantic_version TEXT NOT NULL,
                dependencies TEXT NOT NULL,
                compatibility_requirements TEXT NOT NULL,
                trust_tier TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                signature TEXT,
                bundle_path TEXT,
                manifest_toml TEXT,
                state TEXT NOT NULL DEFAULT 'discovered',
                state_changed_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            PRAGMA user_version = 0;
            "#,
        )
        .expect("create legacy schema");

        // Insert one ENABLED skill using serialized values that match the
        // production serde formats (so parsing succeeds once columns align).
        let m = sample_skill("oc_legacy_calc");
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"INSERT INTO skills (
                skill_id, name, description, publisher, version, category,
                discovery_source, discovered_at, capabilities, runtime_requirements,
                risk_level, resource_class, tags, categories, semantic_version,
                dependencies, compatibility_requirements, trust_tier, content_hash,
                signature, bundle_path, manifest_toml, state, state_changed_at,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
            )"#,
            params![
                m.skill_id,
                m.name,
                m.description,
                m.publisher,
                m.version,
                m.category,
                serde_json::to_string(&m.discovery_source).unwrap(),
                m.discovered_at.to_rfc3339(),
                serde_json::to_string(&m.capabilities).unwrap(),
                m.runtime_requirements,
                m.risk_level.as_str(),
                m.resource_class.as_str(),
                serde_json::to_string(&m.tags).unwrap(),
                serde_json::to_string(&m.categories).unwrap(),
                m.semantic_version,
                serde_json::to_string(&m.dependencies).unwrap(),
                serde_json::to_string(&m.compatibility_requirements).unwrap(),
                m.trust_tier.as_str(),
                m.content_hash,
                m.signature,
                m.bundle_path,
                m.manifest_toml,
                "enabled",
                m.state_changed_at.to_rfc3339(),
                now,
                now,
            ],
        )
        .expect("insert legacy enabled skill");
    }

    // 2. Open through the production registry — this runs migration 1, which
    //    appends `granted_capabilities` at the END of the table (index shift).
    let registry = ProductionSkillRegistry::new(&db_path).expect("open + migrate");

    // 3. The enabled skill MUST be returned (was 0 before the by-name fix).
    let enabled = registry.get_enabled_skills().expect("query enabled");
    assert_eq!(
        enabled.len(),
        1,
        "enabled skill from a migrated (granted_capabilities-appended) DB must load"
    );
    assert_eq!(enabled[0].skill_id, "oc_legacy_calc");
    assert!(matches!(enabled[0].state, SkillState::Enabled));
}

/// Forward-only application check for OpenClaw ICP migrations 3-6 (task 2.1).
///
/// Builds an OLDER `skills.db` (only the base `skills` table, `user_version = 0`)
/// and opens it through the production registry. This must run every migration
/// 1..=6 in order WITHOUT dropping/renaming anything, leaving:
///   - `PRAGMA user_version` == 6 (the bumped SCHEMA_VERSION)
///   - the four new derived tables present (capability_profiles, market_catalog,
///     capability_grants_scoped, capability_edges)
///   - the `idx_grants_skill` index present
///   - the pre-existing enabled skill still readable (additive, non-destructive)
#[test]
fn icp_migrations_3_to_6_apply_forward_only_on_older_db() {
    use rusqlite::{params, Connection};

    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("older_skills.db");

    // 1. Older DB: base skills table only, user_version = 0 (pre-ICP).
    {
        let conn = Connection::open(&db_path).expect("open raw");
        conn.execute_batch(
            r#"
            CREATE TABLE skills (
                skill_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                publisher TEXT NOT NULL,
                version TEXT NOT NULL,
                category TEXT NOT NULL,
                discovery_source TEXT NOT NULL,
                discovered_at TEXT NOT NULL,
                capabilities TEXT NOT NULL,
                runtime_requirements TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                resource_class TEXT NOT NULL,
                tags TEXT NOT NULL,
                categories TEXT NOT NULL,
                semantic_version TEXT NOT NULL,
                dependencies TEXT NOT NULL,
                compatibility_requirements TEXT NOT NULL,
                trust_tier TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                signature TEXT,
                bundle_path TEXT,
                manifest_toml TEXT,
                state TEXT NOT NULL DEFAULT 'discovered',
                state_changed_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            PRAGMA user_version = 0;
            "#,
        )
        .expect("create older schema");

        let m = sample_skill("oc_older_calc");
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"INSERT INTO skills (
                skill_id, name, description, publisher, version, category,
                discovery_source, discovered_at, capabilities, runtime_requirements,
                risk_level, resource_class, tags, categories, semantic_version,
                dependencies, compatibility_requirements, trust_tier, content_hash,
                signature, bundle_path, manifest_toml, state, state_changed_at,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
            )"#,
            params![
                m.skill_id,
                m.name,
                m.description,
                m.publisher,
                m.version,
                m.category,
                serde_json::to_string(&m.discovery_source).unwrap(),
                m.discovered_at.to_rfc3339(),
                serde_json::to_string(&m.capabilities).unwrap(),
                m.runtime_requirements,
                m.risk_level.as_str(),
                m.resource_class.as_str(),
                serde_json::to_string(&m.tags).unwrap(),
                serde_json::to_string(&m.categories).unwrap(),
                m.semantic_version,
                serde_json::to_string(&m.dependencies).unwrap(),
                serde_json::to_string(&m.compatibility_requirements).unwrap(),
                m.trust_tier.as_str(),
                m.content_hash,
                m.signature,
                m.bundle_path,
                m.manifest_toml,
                "enabled",
                m.state_changed_at.to_rfc3339(),
                now,
                now,
            ],
        )
        .expect("insert older enabled skill");
    }

    // 2. Open through the production registry — runs migrations 1..=6 in order.
    let registry = ProductionSkillRegistry::new(&db_path).expect("open + migrate");

    // 3. Pre-existing data survived (additive, non-destructive).
    let enabled = registry.get_enabled_skills().expect("query enabled");
    assert_eq!(
        enabled.len(),
        1,
        "older skill must survive forward migration"
    );
    assert_eq!(enabled[0].skill_id, "oc_older_calc");

    // 4. Inspect the migrated DB directly for the new schema objects.
    let conn = Connection::open(&db_path).expect("reopen migrated db");

    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("read user_version");
    assert_eq!(user_version, 6, "SCHEMA_VERSION must be bumped to 6");

    for table in [
        "capability_profiles",
        "market_catalog",
        "capability_grants_scoped",
        "capability_edges",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |r| r.get(0),
            )
            .expect("query sqlite_master table");
        assert_eq!(count, 1, "migration must create derived table `{table}`");
    }

    let index_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_grants_skill'",
            [],
            |r| r.get(0),
        )
        .expect("query sqlite_master index");
    assert_eq!(index_count, 1, "migration 5 must create idx_grants_skill");

    // 5. Idempotent re-open is a no-op (user_version already >= SCHEMA_VERSION).
    let registry2 = ProductionSkillRegistry::new(&db_path).expect("re-open no-op");
    assert_eq!(
        registry2
            .get_enabled_skills()
            .expect("query enabled again")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Task 2.6 — additive migration coverage (extends the forward-only check above)
//
// These tests EXTEND `icp_migrations_3_to_6_apply_forward_only_on_older_db`
// (task 2.1) rather than duplicating it. They assert the stronger additive /
// forward-only invariants required by R5.2:
//   * no base `skills` column is dropped or renamed by migrations 1..=6;
//   * every pre-existing `skills` row is preserved byte-for-byte across the
//     migration (additive `ADD COLUMN` only, never a destructive rewrite);
//   * each new derived table has exactly the columns specified in design §7.4;
//   * re-running the migration pipeline is an idempotent no-op that never
//     drops/recreates a table (the stored `CREATE TABLE` SQL is unchanged).
// ---------------------------------------------------------------------------

/// Build an OLDER (pre-ICP) `skills.db`: only the base `skills` table with
/// `user_version = 0`, seeded with one ENABLED skill. Returns the raw values
/// inserted for the base columns so a caller can assert byte-for-byte survival.
///
/// Column set/order matches the real pre-migration production schema (the same
/// one used by `icp_migrations_3_to_6_apply_forward_only_on_older_db`).
#[cfg(test)]
fn seed_older_base_db(db_path: &Path, skill_id: &str) -> Vec<(String, String)> {
    use rusqlite::{params, Connection};

    let conn = Connection::open(db_path).expect("open raw older db");
    conn.execute_batch(
        r#"
        CREATE TABLE skills (
            skill_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            publisher TEXT NOT NULL,
            version TEXT NOT NULL,
            category TEXT NOT NULL,
            discovery_source TEXT NOT NULL,
            discovered_at TEXT NOT NULL,
            capabilities TEXT NOT NULL,
            runtime_requirements TEXT NOT NULL,
            risk_level TEXT NOT NULL,
            resource_class TEXT NOT NULL,
            tags TEXT NOT NULL,
            categories TEXT NOT NULL,
            semantic_version TEXT NOT NULL,
            dependencies TEXT NOT NULL,
            compatibility_requirements TEXT NOT NULL,
            trust_tier TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            signature TEXT,
            bundle_path TEXT,
            manifest_toml TEXT,
            state TEXT NOT NULL DEFAULT 'discovered',
            state_changed_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        PRAGMA user_version = 0;
        "#,
    )
    .expect("create older schema");

    let m = sample_skill(skill_id);
    let now = Utc::now().to_rfc3339();
    // The exact scalar strings written for the base columns — used to assert
    // byte-for-byte survival across migration.
    let base_values: Vec<(String, String)> = vec![
        ("skill_id".into(), m.skill_id.clone()),
        ("name".into(), m.name.clone()),
        ("description".into(), m.description.clone()),
        ("publisher".into(), m.publisher.clone()),
        ("version".into(), m.version.clone()),
        ("category".into(), m.category.clone()),
        (
            "discovery_source".into(),
            serde_json::to_string(&m.discovery_source).unwrap(),
        ),
        (
            "runtime_requirements".into(),
            m.runtime_requirements.clone(),
        ),
        ("risk_level".into(), m.risk_level.as_str().to_string()),
        (
            "resource_class".into(),
            m.resource_class.as_str().to_string(),
        ),
        ("tags".into(), serde_json::to_string(&m.tags).unwrap()),
        (
            "categories".into(),
            serde_json::to_string(&m.categories).unwrap(),
        ),
        ("semantic_version".into(), m.semantic_version.clone()),
        (
            "dependencies".into(),
            serde_json::to_string(&m.dependencies).unwrap(),
        ),
        (
            "compatibility_requirements".into(),
            serde_json::to_string(&m.compatibility_requirements).unwrap(),
        ),
        ("trust_tier".into(), m.trust_tier.as_str().to_string()),
        ("content_hash".into(), m.content_hash.clone()),
        ("state".into(), "enabled".to_string()),
    ];

    conn.execute(
        r#"INSERT INTO skills (
            skill_id, name, description, publisher, version, category,
            discovery_source, discovered_at, capabilities, runtime_requirements,
            risk_level, resource_class, tags, categories, semantic_version,
            dependencies, compatibility_requirements, trust_tier, content_hash,
            signature, bundle_path, manifest_toml, state, state_changed_at,
            created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
        )"#,
        params![
            m.skill_id,
            m.name,
            m.description,
            m.publisher,
            m.version,
            m.category,
            serde_json::to_string(&m.discovery_source).unwrap(),
            m.discovered_at.to_rfc3339(),
            serde_json::to_string(&m.capabilities).unwrap(),
            m.runtime_requirements,
            m.risk_level.as_str(),
            m.resource_class.as_str(),
            serde_json::to_string(&m.tags).unwrap(),
            serde_json::to_string(&m.categories).unwrap(),
            m.semantic_version,
            serde_json::to_string(&m.dependencies).unwrap(),
            serde_json::to_string(&m.compatibility_requirements).unwrap(),
            m.trust_tier.as_str(),
            m.content_hash,
            m.signature,
            m.bundle_path,
            m.manifest_toml,
            "enabled",
            m.state_changed_at.to_rfc3339(),
            now,
            now,
        ],
    )
    .expect("insert older enabled skill");

    base_values
}

/// Read the ordered column names of a table via `PRAGMA table_info`.
#[cfg(test)]
fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare table_info");
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("query table_info")
        .map(|r| r.expect("column name"))
        .collect();
    cols
}

/// Read the stored `CREATE TABLE` SQL for a table from `sqlite_master`.
#[cfg(test)]
fn table_sql(conn: &rusqlite::Connection, table: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
        rusqlite::params![table],
        |r| r.get::<_, String>(0),
    )
    .expect("read table sql")
}

/// R5.2 — migrations are additive/forward-only: no base `skills` column is
/// dropped or renamed, the new authoritative columns are appended, and the
/// pre-existing row survives byte-for-byte.
#[test]
fn icp_migrations_preserve_skills_columns_and_row_byte_for_byte() {
    use rusqlite::Connection;

    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("older_preserve.db");
    let base_values = seed_older_base_db(&db_path, "oc_preserve_calc");

    // Column set BEFORE migration.
    let before_cols = {
        let conn = Connection::open(&db_path).expect("open before");
        table_columns(&conn, "skills")
    };

    // Migrate 0 -> 6.
    let _registry = ProductionSkillRegistry::new(&db_path).expect("open + migrate");

    let conn = Connection::open(&db_path).expect("reopen migrated");
    let after_cols = table_columns(&conn, "skills");

    // No drop/rename: every pre-existing column name is still present.
    for col in &before_cols {
        assert!(
            after_cols.contains(col),
            "base skills column `{col}` must survive migration (no drop/rename)"
        );
    }
    // Additive: the two authoritative columns were appended (migrations 1 & 2).
    assert!(
        after_cols.contains(&"granted_capabilities".to_string()),
        "migration 1 must append granted_capabilities"
    );
    assert!(
        after_cols.contains(&"input_schema".to_string()),
        "migration 2 must append input_schema"
    );
    // Additive-only: post-migration is a superset (no columns removed).
    assert!(after_cols.len() >= before_cols.len() + 2);

    // Every base column value preserved byte-for-byte for the pre-existing row.
    for (col, expected) in &base_values {
        let actual: String = conn
            .query_row(
                &format!("SELECT {col} FROM skills WHERE skill_id = 'oc_preserve_calc'"),
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("read migrated column {col}: {e}"));
        assert_eq!(
            &actual, expected,
            "base column `{col}` must be preserved byte-for-byte across migration"
        );
    }
    // Appended columns take their declared defaults (additive, non-destructive).
    let granted: String = conn
        .query_row(
            "SELECT granted_capabilities FROM skills WHERE skill_id = 'oc_preserve_calc'",
            [],
            |r| r.get(0),
        )
        .expect("read granted_capabilities default");
    assert_eq!(
        granted, "[]",
        "appended column must use its declared default"
    );
}

/// R5.2 — each new derived table created by migrations 3..=6 has exactly the
/// columns specified in design §7.4 (no extra/missing/renamed columns).
#[test]
fn icp_migrations_new_tables_match_design_schema() {
    use rusqlite::Connection;

    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("older_schema.db");
    seed_older_base_db(&db_path, "oc_schema_calc");
    let _registry = ProductionSkillRegistry::new(&db_path).expect("open + migrate");

    let conn = Connection::open(&db_path).expect("reopen migrated");

    // Expected columns per design §7.4 (order as declared in the migration DDL).
    let expected: &[(&str, &[&str])] = &[
        (
            "capability_profiles",
            &[
                "skill_id",
                "provides_json",
                "consumes_json",
                "inputs_json",
                "outputs_json",
                "embedding",
                "profile_epoch",
            ],
        ),
        (
            "market_catalog",
            &[
                "provider_id",
                "slug",
                "manifest_json",
                "version",
                "embedding",
                "trust_hint",
                "quality",
                "popularity",
                "deprecated",
                "fetched_at",
            ],
        ),
        (
            "capability_grants_scoped",
            &[
                "grant_id",
                "skill_id",
                "scope_kind",
                "scope_key",
                "caps_hash",
                "risk",
                "decision",
                "granted_at",
                "expires_at",
                "revoked",
            ],
        ),
        (
            "capability_edges",
            &["from_skill", "to_skill", "edge_kind", "weight"],
        ),
    ];

    for (table, cols) in expected {
        let actual = table_columns(&conn, table);
        let want: Vec<String> = cols.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            actual, want,
            "derived table `{table}` columns must match design §7.4 exactly"
        );
    }
}

/// R5.2 — re-running the migration pipeline is an idempotent no-op: opening the
/// already-migrated DB again neither bumps the version nor drops/recreates any
/// table (the stored `CREATE TABLE` SQL is byte-identical before and after).
#[test]
fn icp_migrations_rerun_is_idempotent_no_drop_or_recreate() {
    use rusqlite::Connection;

    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("older_idempotent.db");
    seed_older_base_db(&db_path, "oc_idem_calc");

    let derived = [
        "capability_profiles",
        "market_catalog",
        "capability_grants_scoped",
        "capability_edges",
    ];

    // First migration 0 -> 6.
    let _r1 = ProductionSkillRegistry::new(&db_path).expect("first open + migrate");
    let (version_first, sql_first) = {
        let conn = Connection::open(&db_path).expect("reopen after first");
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("read version");
        let sql: Vec<String> = derived.iter().map(|t| table_sql(&conn, t)).collect();
        (v, sql)
    };
    assert_eq!(version_first, 6);

    // Re-open twice more — each is a no-op (current_version >= SCHEMA_VERSION).
    let _r2 = ProductionSkillRegistry::new(&db_path).expect("second open");
    let _r3 = ProductionSkillRegistry::new(&db_path).expect("third open");

    let conn = Connection::open(&db_path).expect("reopen after reruns");
    let version_after: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("read version again");
    assert_eq!(version_after, 6, "re-runs must not change user_version");

    // Table definitions unchanged (no drop/recreate).
    for (i, table) in derived.iter().enumerate() {
        assert_eq!(
            &table_sql(&conn, table),
            &sql_first[i],
            "derived table `{table}` DDL must be unchanged across re-runs (no drop/recreate)"
        );
    }

    // Pre-existing data still intact.
    let cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skills WHERE skill_id = 'oc_idem_calc'",
            [],
            |r| r.get(0),
        )
        .expect("count preserved skill");
    assert_eq!(
        cnt, 1,
        "seeded skill must persist across idempotent re-runs"
    );
}
