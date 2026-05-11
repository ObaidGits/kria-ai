// crates/kria-core/src/agent/ml_orchestrator/sync_cell.rs
//
// Generates Python code for atomic artifact sync from hot SSD to Drive.
// Protocol: .tmp → fsync → os.replace → manifest.json

/// Generate a Python sync cell that atomically copies artifacts from hot to cold storage.
pub fn generate_sync_cell(
    job_id: &str,
    phase_dir: &str,
    hot_root: &str,
    cold_root: &str,
    artifacts: &[&str],
) -> String {
    let artifact_list = artifacts
        .iter()
        .map(|a| format!("\"{}\"", a))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"import os, json, shutil

HOT  = os.path.join("{hot_root}", "{phase_dir}")
COLD = os.path.join("{cold_root}", "{phase_dir}")
os.makedirs(COLD, exist_ok=True)

artifacts = [{artifact_list}]
manifest = {{"phase": "{phase_dir}", "job_id": "{job_id}", "artifacts": []}}

for name in artifacts:
    src = os.path.join(HOT, name)
    dst = os.path.join(COLD, name)
    tmp = dst + ".tmp"
    if not os.path.exists(src):
        print(f"WARNING: {{name}} not found at {{src}}, skipping")
        continue
    shutil.copy2(src, tmp)
    fd = os.open(tmp, os.O_RDONLY)
    os.fsync(fd)
    os.close(fd)
    os.replace(tmp, dst)
    stat = os.stat(dst)
    manifest["artifacts"].append({{"name": name, "size_bytes": stat.st_size}})
    print(f"  synced: {{name}} ({{stat.st_size}} bytes)")

mp = os.path.join(COLD, "manifest.json")
mp_tmp = mp + ".tmp"
with open(mp_tmp, "w") as f:
    json.dump(manifest, f, indent=2)
fd = os.open(mp_tmp, os.O_RDONLY)
os.fsync(fd)
os.close(fd)
os.replace(mp_tmp, mp)

print(f"✓ Synced {{len(manifest['artifacts'])}} artifacts to Drive")
"#,
        job_id = job_id,
        phase_dir = phase_dir,
        hot_root = hot_root,
        cold_root = cold_root,
        artifact_list = artifact_list,
    )
}

/// Generate a periodic checkpoint sync cell (mid-training).
pub fn generate_checkpoint_sync(
    job_id: &str,
    hot_root: &str,
    cold_root: &str,
    phase_dir: &str,
    checkpoint_files: &[&str],
) -> String {
    generate_sync_cell(job_id, phase_dir, hot_root, cold_root, checkpoint_files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_cell_contains_atomic_protocol() {
        let code = generate_sync_cell(
            "j1",
            "04_train",
            "/hot",
            "/cold",
            &["model.pth", "status.json"],
        );
        assert!(code.contains(".tmp"));
        assert!(code.contains("os.fsync"));
        assert!(code.contains("os.replace"));
        assert!(code.contains("manifest.json"));
        assert!(code.contains("model.pth"));
        assert!(code.contains("status.json"));
    }

    #[test]
    fn sync_cell_handles_missing_files() {
        let code = generate_sync_cell("j1", "01_setup", "/hot", "/cold", &["missing.json"]);
        assert!(code.contains("skipping"));
    }
}
