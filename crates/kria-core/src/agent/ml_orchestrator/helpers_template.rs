// crates/kria-core/src/agent/ml_orchestrator/helpers_template.rs
//
// Python helper code injected into every cell by the orchestrator.
// The LLM never writes this — it just calls job_paths.* and job_progress.*.

/// Generate the Python helper code that gets prepended to every cell.
/// Placeholders {JOB_ID}, {HOT_ROOT}, {COLD_ROOT}, {DATASET_PATH}, {STATUS_FILE}
/// are replaced by the orchestrator before execution.
pub const KRIA_HELPERS_TEMPLATE: &str = r#"
# === KRIA ORCHESTRATOR HELPERS (auto-injected, do not edit) ===
import os, json, time, hashlib

class JobPaths:
    def __init__(self, job_id, hot_root, cold_root, dataset_path):
        self.job_id = job_id
        self.hot_root = hot_root
        self.cold_root = cold_root
        self._dataset_path = dataset_path

    def phase_dir(self, phase_name):
        return os.path.join(self.hot_root, phase_name)

    def cold_phase_dir(self, phase_name):
        return os.path.join(self.cold_root, phase_name)

    def input(self, from_phase, filename):
        return os.path.join(self.hot_root, from_phase, filename)

    def output(self, phase_name, filename):
        return os.path.join(self.hot_root, phase_name, filename)

    def dataset_path(self):
        return self._dataset_path

    def safe_save_model(self, model, relative_path):
        full_path = os.path.join(self.hot_root, relative_path)
        os.makedirs(os.path.dirname(full_path), exist_ok=True)
        torch.save(model.state_dict(), full_path)
        print(f"KRIA_SAVED: {relative_path}")

    def safe_load_model(self, model_class, relative_path, **kwargs):
        full_path = os.path.join(self.hot_root, relative_path)
        model = model_class(**kwargs)
        model.load_state_dict(torch.load(full_path, map_location="cpu"))
        return model

    def makedirs(self, relative_path):
        full_path = os.path.join(self.hot_root, relative_path)
        os.makedirs(full_path, exist_ok=True)
        return full_path

    def copy(self, src_relative, dst_relative):
        import shutil
        src = os.path.join(self.hot_root, src_relative)
        dst = os.path.join(self.hot_root, dst_relative)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copy2(src, dst)
        return dst

class JobProgress:
    def __init__(self, status_file):
        self._status_file = status_file
        self._latencies = []
        self._last_report = None

    def report(self, progress=0.0, metrics=None, error=None):
        now = time.time()
        if self._last_report is not None:
            self._latencies.append(now - self._last_report)
            if len(self._latencies) > 100:
                self._latencies = self._latencies[-100:]
        self._last_report = now
        tmp = self._status_file + ".tmp"
        with open(tmp, "w") as f:
            json.dump({
                "state": "running", "progress": progress,
                "metrics": metrics or {}, "error": error,
                "pid": os.getpid(), "heartbeat_ts": now, "timestamp": now,
                "batch_latencies_p95": self._p95(),
            }, f)
        os.replace(tmp, self._status_file)

    def complete(self, metrics=None):
        self.report(progress=1.0, metrics=metrics)
        tmp = self._status_file + ".tmp"
        with open(tmp, "w") as f:
            json.dump({
                "state": "completed", "progress": 1.0,
                "metrics": metrics or {}, "pid": os.getpid(),
                "heartbeat_ts": time.time(), "timestamp": time.time(),
            }, f)
        os.replace(tmp, self._status_file)

    def fail(self, error):
        tmp = self._status_file + ".tmp"
        with open(tmp, "w") as f:
            json.dump({
                "state": "failed", "error": str(error), "pid": os.getpid(),
                "heartbeat_ts": time.time(), "timestamp": time.time(),
            }, f)
        os.replace(tmp, self._status_file)

    def _p95(self):
        if not self._latencies:
            return 0.0
        s = sorted(self._latencies)
        return s[min(int(len(s) * 0.95), len(s) - 1)]

job_paths = JobPaths("{JOB_ID}", "{HOT_ROOT}", "{COLD_ROOT}", "{DATASET_PATH}")
job_progress = JobProgress("{STATUS_FILE}")
# === END KRIA HELPERS ===
"#;

/// Fill in the placeholders in the helpers template.
pub fn render_helpers(
    job_id: &str,
    hot_root: &str,
    cold_root: &str,
    dataset_path: &str,
    status_file: &str,
) -> String {
    KRIA_HELPERS_TEMPLATE
        .replace("{JOB_ID}", job_id)
        .replace("{HOT_ROOT}", hot_root)
        .replace("{COLD_ROOT}", cold_root)
        .replace("{DATASET_PATH}", dataset_path)
        .replace("{STATUS_FILE}", status_file)
}
