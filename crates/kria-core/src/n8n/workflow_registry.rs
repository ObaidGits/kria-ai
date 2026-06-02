use super::types::N8nWorkflowConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const N8N_WORKFLOW_REGISTRY_SCHEMA_VERSION: &str = "kria.n8n.workflow_registry.v1";
pub const N8N_WORKFLOW_REGISTRY_MIGRATED_SOURCE: &str = "migrated_from_toml";
pub const N8N_WORKFLOW_REGISTRY_UI_SOURCE: &str = "kria_ui";
pub const N8N_WORKFLOW_REGISTRY_AUTHORING_SOURCE: &str = "kria_authoring";
pub const N8N_WORKFLOW_REGISTRY_ROLLBACK_SOURCE: &str = "kria_rollback";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nWorkflowRegistryRecord {
    #[serde(flatten)]
    pub workflow: N8nWorkflowConfig,
    pub source: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct N8nWorkflowRegistryStore {
    pub schema_version: String,
    pub updated_at_ms: u64,
    pub workflows: Vec<N8nWorkflowRegistryRecord>,
}

impl Default for N8nWorkflowRegistryStore {
    fn default() -> Self {
        Self {
            schema_version: N8N_WORKFLOW_REGISTRY_SCHEMA_VERSION.into(),
            updated_at_ms: now_ms(),
            workflows: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum N8nWorkflowRegistryStoreError {
    #[error("failed to read workflow registry: {0}")]
    Read(#[from] io::Error),
    #[error("failed to parse workflow registry: {0}")]
    Parse(serde_json::Error),
    #[error("failed to serialize workflow registry: {0}")]
    Serialize(serde_json::Error),
    #[error("duplicate workflow id '{0}'")]
    DuplicateWorkflow(String),
}

pub fn default_workflow_registry_store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".kria")
        .join("n8n")
        .join("workflow_registry.json")
}

pub fn load_workflow_registry_store_at(
    path: &Path,
) -> Result<N8nWorkflowRegistryStore, N8nWorkflowRegistryStoreError> {
    if !path.exists() {
        return Ok(N8nWorkflowRegistryStore::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(N8nWorkflowRegistryStoreError::Parse)
}

pub fn save_workflow_registry_store_at(
    path: &Path,
    store: &N8nWorkflowRegistryStore,
) -> Result<(), N8nWorkflowRegistryStoreError> {
    ensure_unique_workflow_ids(store.workflows.iter().map(|record| &record.workflow))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }

    let mut next = store.clone();
    next.schema_version = N8N_WORKFLOW_REGISTRY_SCHEMA_VERSION.into();
    next.updated_at_ms = now_ms();
    let content =
        serde_json::to_string_pretty(&next).map_err(N8nWorkflowRegistryStoreError::Serialize)?;
    fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn workflow_registry_records(
    store: &N8nWorkflowRegistryStore,
) -> Vec<N8nWorkflowRegistryRecord> {
    let mut records = store.workflows.clone();
    records.sort_by(|a, b| a.workflow.workflow_id.cmp(&b.workflow.workflow_id));
    records
}

pub fn workflow_registry_workflows(store: &N8nWorkflowRegistryStore) -> Vec<N8nWorkflowConfig> {
    workflow_registry_records(store)
        .into_iter()
        .filter(|record| !record.workflow.is_archived_or_deleted())
        .map(|record| record.workflow)
        .collect()
}

pub fn workflow_registry_archived_workflows(
    store: &N8nWorkflowRegistryStore,
) -> Vec<N8nWorkflowConfig> {
    workflow_registry_records(store)
        .into_iter()
        .filter(|record| record.workflow.is_archived_or_deleted())
        .map(|record| record.workflow)
        .collect()
}

pub fn upsert_workflow_registry_record(
    store: &mut N8nWorkflowRegistryStore,
    workflow: N8nWorkflowConfig,
    source: impl Into<String>,
) -> Result<(), N8nWorkflowRegistryStoreError> {
    let workflow_id = workflow.workflow_id.trim().to_string();
    ensure_unique_workflow_ids(
        store
            .workflows
            .iter()
            .filter(|record| record.workflow.workflow_id != workflow_id)
            .map(|record| &record.workflow)
            .chain(std::iter::once(&workflow)),
    )?;

    let now = now_ms();
    if let Some(existing) = store
        .workflows
        .iter_mut()
        .find(|record| record.workflow.workflow_id == workflow_id)
    {
        existing.workflow = workflow;
        existing.source = source.into();
        existing.updated_at_ms = now;
    } else {
        store.workflows.push(N8nWorkflowRegistryRecord {
            workflow,
            source: source.into(),
            created_at_ms: now,
            updated_at_ms: now,
        });
    }
    store.updated_at_ms = now;
    Ok(())
}

pub fn delete_workflow_registry_record(
    store: &mut N8nWorkflowRegistryStore,
    workflow_id: &str,
) -> bool {
    let before = store.workflows.len();
    store
        .workflows
        .retain(|record| record.workflow.workflow_id != workflow_id);
    let removed = store.workflows.len() != before;
    if removed {
        store.updated_at_ms = now_ms();
    }
    removed
}

pub fn migrate_toml_workflows_to_registry_store(
    store: &mut N8nWorkflowRegistryStore,
    workflows: &[N8nWorkflowConfig],
) -> Result<usize, N8nWorkflowRegistryStoreError> {
    if workflows.is_empty() || !store.workflows.is_empty() {
        return Ok(0);
    }
    ensure_unique_workflow_ids(workflows.iter())?;
    let now = now_ms();
    store.workflows = workflows
        .iter()
        .cloned()
        .map(|workflow| N8nWorkflowRegistryRecord {
            workflow,
            source: N8N_WORKFLOW_REGISTRY_MIGRATED_SOURCE.into(),
            created_at_ms: now,
            updated_at_ms: now,
        })
        .collect();
    store.updated_at_ms = now;
    Ok(store.workflows.len())
}

pub fn migrate_missing_toml_workflows_to_registry_store(
    store: &mut N8nWorkflowRegistryStore,
    workflows: &[N8nWorkflowConfig],
) -> Result<usize, N8nWorkflowRegistryStoreError> {
    if workflows.is_empty() {
        return Ok(0);
    }

    ensure_unique_workflow_ids(store.workflows.iter().map(|record| &record.workflow))?;
    ensure_unique_workflow_ids(workflows.iter())?;

    let mut registry_ids = store
        .workflows
        .iter()
        .map(|record| record.workflow.workflow_id.clone())
        .collect::<HashSet<_>>();
    let now = now_ms();
    let mut migrated = 0usize;

    for workflow in workflows {
        if registry_ids.insert(workflow.workflow_id.clone()) {
            store.workflows.push(N8nWorkflowRegistryRecord {
                workflow: workflow.clone(),
                source: N8N_WORKFLOW_REGISTRY_MIGRATED_SOURCE.into(),
                created_at_ms: now,
                updated_at_ms: now,
            });
            migrated += 1;
        }
    }

    if migrated > 0 {
        store.updated_at_ms = now;
    }

    Ok(migrated)
}

pub fn migrate_toml_workflows_to_registry_at(
    path: &Path,
    workflows: &[N8nWorkflowConfig],
) -> Result<(N8nWorkflowRegistryStore, usize), N8nWorkflowRegistryStoreError> {
    let mut store = load_workflow_registry_store_at(path)?;
    let migrated = migrate_toml_workflows_to_registry_store(&mut store, workflows)?;
    if migrated > 0 {
        save_workflow_registry_store_at(path, &store)?;
    }
    Ok((store, migrated))
}

pub fn registry_has_workflow_parity(
    store: &N8nWorkflowRegistryStore,
    workflows: &[N8nWorkflowConfig],
) -> bool {
    let registry_ids = store
        .workflows
        .iter()
        .map(|record| record.workflow.workflow_id.as_str())
        .collect::<HashSet<_>>();
    workflows
        .iter()
        .all(|workflow| registry_ids.contains(workflow.workflow_id.as_str()))
}

fn ensure_unique_workflow_ids<'a>(
    workflows: impl IntoIterator<Item = &'a N8nWorkflowConfig>,
) -> Result<(), N8nWorkflowRegistryStoreError> {
    let mut ids = HashSet::new();
    for workflow in workflows {
        let id = workflow.workflow_id.trim();
        if !ids.insert(id.to_string()) {
            return Err(N8nWorkflowRegistryStoreError::DuplicateWorkflow(id.into()));
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::n8n::N8nWorkflowStatus;
    use tempfile::tempdir;

    fn workflow(id: &str) -> N8nWorkflowConfig {
        N8nWorkflowConfig {
            workflow_id: id.into(),
            display_name: id.into(),
            status: N8nWorkflowStatus::Approved,
            credential_requirements: vec!["gmail.readonly".into()],
            ..Default::default()
        }
    }

    #[test]
    fn empty_registry_loads_cleanly() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workflow_registry.json");
        let store = load_workflow_registry_store_at(&path).unwrap();
        assert!(store.workflows.is_empty());
    }

    #[test]
    fn toml_workflows_migrate_when_registry_empty() {
        let mut store = N8nWorkflowRegistryStore::default();
        let migrated =
            migrate_toml_workflows_to_registry_store(&mut store, &[workflow("one")]).unwrap();
        assert_eq!(migrated, 1);
        assert_eq!(store.workflows[0].workflow.workflow_id, "one");
        assert_eq!(
            store.workflows[0].source,
            N8N_WORKFLOW_REGISTRY_MIGRATED_SOURCE
        );
    }

    #[test]
    fn existing_registry_wins_over_toml() {
        let mut store = N8nWorkflowRegistryStore::default();
        upsert_workflow_registry_record(&mut store, workflow("existing"), "test").unwrap();
        let migrated =
            migrate_toml_workflows_to_registry_store(&mut store, &[workflow("legacy")]).unwrap();
        assert_eq!(migrated, 0);
        assert_eq!(store.workflows.len(), 1);
        assert_eq!(store.workflows[0].workflow.workflow_id, "existing");
    }

    #[test]
    fn missing_toml_workflows_can_be_backfilled_without_overwriting_registry() {
        let mut store = N8nWorkflowRegistryStore::default();
        let mut existing = workflow("existing");
        existing.display_name = "Registry Version".into();
        upsert_workflow_registry_record(&mut store, existing, "test").unwrap();

        let mut legacy_existing = workflow("existing");
        legacy_existing.display_name = "Legacy Version".into();
        let migrated = migrate_missing_toml_workflows_to_registry_store(
            &mut store,
            &[legacy_existing, workflow("missing")],
        )
        .unwrap();

        assert_eq!(migrated, 1);
        assert_eq!(store.workflows.len(), 2);
        let existing_record = store
            .workflows
            .iter()
            .find(|record| record.workflow.workflow_id == "existing")
            .unwrap();
        assert_eq!(existing_record.workflow.display_name, "Registry Version");
        let missing_record = store
            .workflows
            .iter()
            .find(|record| record.workflow.workflow_id == "missing")
            .unwrap();
        assert_eq!(missing_record.source, N8N_WORKFLOW_REGISTRY_MIGRATED_SOURCE);
    }

    #[test]
    fn duplicate_migrated_ids_fail_safely() {
        let mut store = N8nWorkflowRegistryStore::default();
        let error = migrate_toml_workflows_to_registry_store(
            &mut store,
            &[workflow("dup"), workflow("dup")],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            N8nWorkflowRegistryStoreError::DuplicateWorkflow(_)
        ));
        assert!(store.workflows.is_empty());
    }

    #[test]
    fn registry_save_read_delete_roundtrip_preserves_metadata_without_secrets() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workflow_registry.json");
        let mut store = N8nWorkflowRegistryStore::default();
        upsert_workflow_registry_record(&mut store, workflow("mail"), "test").unwrap();
        save_workflow_registry_store_at(&path, &store).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("gmail.readonly"));
        assert!(!content.contains("api_key"));
        assert!(!content.contains("signing_secret"));
        assert!(!content.contains("oauth_token"));

        let mut loaded = load_workflow_registry_store_at(&path).unwrap();
        assert_eq!(workflow_registry_workflows(&loaded).len(), 1);
        assert!(delete_workflow_registry_record(&mut loaded, "mail"));
        assert!(workflow_registry_workflows(&loaded).is_empty());
    }

    #[test]
    fn archived_registry_workflows_are_hidden_from_runnable_catalog() {
        let mut store = N8nWorkflowRegistryStore::default();
        let active = workflow("active");
        let mut archived = workflow("archived");
        archived.archived = true;
        archived.archived_at_ms = now_ms();
        archived.archived_reason = "test archive".into();

        upsert_workflow_registry_record(&mut store, active, "test").unwrap();
        upsert_workflow_registry_record(&mut store, archived, "test").unwrap();

        let runnable = workflow_registry_workflows(&store);
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].workflow_id, "active");

        let archived = workflow_registry_archived_workflows(&store);
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].workflow_id, "archived");
    }
}
