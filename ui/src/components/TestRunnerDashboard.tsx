import { Component, For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type RunState = {
  running: boolean;
  started_unix_ms: number | null;
  pid: number | null;
  mode: string | null;
  run_label: string | null;
  command: string | null;
};

type HistoryItem = {
  run_label: string;
  report_path: string;
  modified_unix_ms: number;
};

type LogLineEvent = {
  stream: "stdout" | "stderr";
  line: string;
  timestamp_unix_ms: number;
};

type RunFinishedEvent = {
  run_label: string;
  mode: string;
  exit_code: number;
  report_path: string | null;
  finished_unix_ms: number;
};

type RunResult = {
  passed: number;
  failed: number;
  skipped: number;
  failedSuites: string[];
  reportPath: string | null;
  exitCode: number;
};

type TestTargetItem = {
  id: string;
  label: string;
  target_type: string; // "vm" | "docker"
  host?: string | null;
  port?: number | null;
  username?: string | null;
  ssh_key_path?: string | null;
  last_verified_unix_ms?: number | null;
  state?: string | null;
};

const TestRunnerDashboard: Component = () => {
  const [mode, setMode] = createSignal("FULL");
  const [allowDestructive, setAllowDestructive] = createSignal(true);
  const DESTRUCTIVE_MODES = new Set(["DESTRUCTIVE", "FULL", "RELEASE"]);

  const handleModeChange = (value: string) => {
    setMode(value);
    if (DESTRUCTIVE_MODES.has(value)) setAllowDestructive(true);
  };
  const [snapshot, setSnapshot] = createSignal(true);
  const [resumeRunId, setResumeRunId] = createSignal("");
  const [fromZone, setFromZone] = createSignal("");
  const [fromSuite, setFromSuite] = createSignal("");
  const [selectedTargetId, setSelectedTargetId] = createSignal("auto");
  const [testTargets, setTestTargets] = createSignal<TestTargetItem[]>([]);
  const [vmHost, setVmHost] = createSignal("192.168.122.240");
  const [vmPort, setVmPort] = createSignal("22");
  const [vmUser, setVmUser] = createSignal("obaid");
  const [vmSshKey, setVmSshKey] = createSignal("~/.ssh/kria_id");
  const [vmHostkeySha256, setVmHostkeySha256] = createSignal("");
  const [dockerFallback, setDockerFallback] = createSignal(false);
  const [continueOnFailure, setContinueOnFailure] = createSignal(true);

  // Refresh test targets from backend (fleet VMs + Docker containers)
  const refreshTargets = async () => {
    try {
      const targets = await invoke<TestTargetItem[]>("list_test_targets");
      setTestTargets(targets);
    } catch (_e) {
      // Docker may not be available — that's fine
    }
  };

  // Auto-select: when targets load and selection is "auto", pick the best target
  // and populate VM config fields from it
  createEffect(() => {
    const targets = testTargets();
    if (targets.length === 0) return;

    const sel = selectedTargetId();
    if (sel === "auto") {
      // Priority: most recently verified VM → first Docker container
      const bestVm = targets.find((t) => t.target_type === "vm");
      const bestDocker = targets.find((t) => t.target_type === "docker");
      const best = bestVm ?? bestDocker;
      if (best) {
        applyTargetToFields(best);
      }
    } else {
      const match = targets.find((t) => t.id === sel);
      if (match) applyTargetToFields(match);
    }
  });

  const applyTargetToFields = (target: TestTargetItem) => {
    if (target.host) setVmHost(target.host);
    if (target.port) setVmPort(String(target.port));
    if (target.username) setVmUser(target.username);
    if (target.ssh_key_path) setVmSshKey(target.ssh_key_path);
    setDockerFallback(target.target_type === "docker");
  };

  const [runState, setRunState] = createSignal<RunState>({
    running: false,
    started_unix_ms: null,
    pid: null,
    mode: null,
    run_label: null,
    command: null,
  });
  const [history, setHistory] = createSignal<HistoryItem[]>([]);
  const [logs, setLogs] = createSignal<string[]>([]);
  const [selectedReportPath, setSelectedReportPath] = createSignal<string | null>(null);
  const [selectedReportText, setSelectedReportText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [runResult, setRunResult] = createSignal<RunResult | null>(null);
  let logContainer: HTMLDivElement | undefined;

  let unlistenLog: UnlistenFn | null = null;
  let unlistenDone: UnlistenFn | null = null;

  const refreshState = async () => {
    try {
      const state = await invoke<RunState>("get_test_run_state");
      setRunState(state);
    } catch (e) {
      setError(String(e));
    }
  };

  const refreshHistory = async () => {
    try {
      const items = await invoke<HistoryItem[]>("list_test_history", { limit: 100 });
      setHistory(items);
    } catch (e) {
      setError(String(e));
    }
  };

  const parseReportSummary = (text: string): Omit<RunResult, "exitCode" | "reportPath"> => {
    const passedMatch = text.match(/Passed:\s*(\d+)/i);
    const failedMatch = text.match(/Failed:\s*(\d+)/i);
    const skippedMatch = text.match(/Skipped:\s*(\d+)/i);
    const passed = passedMatch ? parseInt(passedMatch[1], 10) : 0;
    const failed = failedMatch ? parseInt(failedMatch[1], 10) : 0;
    const skipped = skippedMatch ? parseInt(skippedMatch[1], 10) : 0;
    const failedSuites: string[] = [];
    const lines = text.split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].includes("Status: Failed")) {
        // Walk backward to find the suite name (### heading)
        for (let j = i - 1; j >= Math.max(0, i - 5); j--) {
          const m = lines[j].match(/^###\s+(.+)/);
          if (m) {
            failedSuites.push(m[1].trim());
            break;
          }
        }
      }
    }
    return { passed, failed, skipped, failedSuites };
  };

  const readReport = async (path: string) => {
    try {
      const text = await invoke<string>("read_test_report", { reportPath: path });
      setSelectedReportPath(path);
      setSelectedReportText(text);
    } catch (e) {
      setError(String(e));
    }
  };

  const scrollLogsToBottom = () => {
    if (logContainer) {
      logContainer.scrollTop = logContainer.scrollHeight;
    }
  };

  const startRun = async () => {
    setBusy(true);
    setError(null);
    setRunResult(null);
    try {
      setLogs([]);
      await invoke("start_test_run", {
        request: {
          mode: mode(),
          allowDestructive: allowDestructive(),
          snapshot: snapshot(),
          resume: resumeRunId().trim() || null,
          fromZone: fromZone().trim() || null,
          fromSuite: fromSuite().trim() || null,
          vmHost: vmHost().trim() || null,
          vmPort: parseInt(vmPort().trim(), 10) || null,
          vmUser: vmUser().trim() || null,
          vmSshKey: vmSshKey().trim() || null,
          vmHostkeySha256: vmHostkeySha256().trim() || null,
          dockerFallback: dockerFallback(),
          targetId: selectedTargetId() !== "auto" ? selectedTargetId() : null,
          dockerContainerId: selectedTargetId().startsWith("docker:") ? selectedTargetId().replace("docker:", "") : null,
          continueOnFailure: continueOnFailure(),
        },
      });
      await refreshState();
      await refreshHistory();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stopRun = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("stop_test_run");
      await refreshState();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const deleteSelectedReport = async () => {
    const path = selectedReportPath();
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_test_report", { reportPath: path });
      setSelectedReportPath(null);
      setSelectedReportText("");
      await refreshHistory();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const deleteAllLogs = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("delete_all_test_logs");
      setLogs([]);
      setHistory([]);
      setSelectedReportPath(null);
      setSelectedReportText("");
      await refreshHistory();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  onMount(async () => {
    await refreshState();
    await refreshHistory();
    await refreshTargets();

    unlistenLog = await listen<LogLineEvent>("kria://tests/log_line", (event) => {
      const row = `[${event.payload.stream}] ${event.payload.line}`;
      setLogs((prev) => {
        const next = [...prev, row];
        return next.slice(-2000);
      });
      setTimeout(scrollLogsToBottom, 0);
    });

    unlistenDone = await listen<RunFinishedEvent>("kria://tests/run_finished", async (event) => {
      const exitCode = event.payload.exit_code;
      const marker = exitCode === 0 ? "✅ PASSED" : `❌ FAILED (exit=${exitCode})`;
      setLogs((prev) => [...prev, `--- ${marker} ---`]);
      await refreshState();
      await refreshHistory();
      if (event.payload.report_path) {
        await readReport(event.payload.report_path);
        // Parse the report for summary
        try {
          const text = await invoke<string>("read_test_report", { reportPath: event.payload.report_path });
          const summary = parseReportSummary(text);
          setRunResult({
            ...summary,
            exitCode,
            reportPath: event.payload.report_path,
          });
        } catch (_e) {
          // Report may not exist yet or be unreadable
          setRunResult({
            passed: 0,
            failed: 0,
            skipped: 0,
            failedSuites: [],
            exitCode,
            reportPath: event.payload.report_path,
          });
        }
      }
      setTimeout(scrollLogsToBottom, 0);
    });
  });

  onCleanup(() => {
    if (unlistenLog) unlistenLog();
    if (unlistenDone) unlistenDone();
  });

  createEffect(() => {
    if (!selectedReportPath() && history().length > 0) {
      void readReport(history()[0].report_path);
    }
  });

  return (
    <section class="ironclad-forensics-panel" style={{ "margin-top": "14px" }}>
      <div class="ironclad-forensics-head">
        <strong>Test Command Center</strong>
        <span>{runState().running ? "Running" : "Idle"}</span>
      </div>

      <div class="ironclad-reset-controls">
        <div class="ironclad-control-group">
          <label>Mode</label>
          <select value={mode()} onChange={(e) => handleModeChange(e.currentTarget.value)}>
            <option value="FULL">FULL</option>
            <option value="SMOKE">SMOKE</option>
            <option value="INFRA">INFRA</option>
            <option value="APPLOGIC">APPLOGIC</option>
            <option value="DESTRUCTIVE">DESTRUCTIVE</option>
            <option value="RELEASE">RELEASE</option>
          </select>
        </div>
        <div class="ironclad-control-group">
          <label>Resume Run Id</label>
          <input
            type="text"
            value={resumeRunId()}
            onInput={(e) => setResumeRunId(e.currentTarget.value)}
            placeholder="20260509_042715"
          />
        </div>
        <div class="ironclad-control-group">
          <label>From Zone</label>
          <input
            type="text"
            value={fromZone()}
            onInput={(e) => setFromZone(e.currentTarget.value)}
            placeholder="os_level / app_logic / smoke"
          />
        </div>
        <div class="ironclad-control-group">
          <label>From Suite</label>
          <input
            type="text"
            value={fromSuite()}
            onInput={(e) => setFromSuite(e.currentTarget.value)}
            placeholder="App Logic"
          />
        </div>
      </div>

      <div class="ironclad-reset-controls" style={{ "margin-top": "8px" }}>
        <div class="ironclad-control-group">
          <label>Test Target</label>
          <select value={selectedTargetId()} onChange={(e) => setSelectedTargetId(e.currentTarget.value)}>
            <option value="auto">Auto (latest VM → Docker)</option>
            <For each={testTargets()}>
              {(target) => (
                <option value={target.id}>{target.label}</option>
              )}
            </For>
          </select>
        </div>
        <div class="ironclad-control-group">
          <label>VM Host</label>
          <input
            type="text"
            value={vmHost()}
            onInput={(e) => setVmHost(e.currentTarget.value)}
            placeholder="192.168.122.240"
          />
        </div>
        <div class="ironclad-control-group">
          <label>VM Port</label>
          <input
            type="number"
            value={vmPort()}
            onInput={(e) => setVmPort(e.currentTarget.value)}
            placeholder="22"
            min="1"
            max="65535"
          />
        </div>
        <div class="ironclad-control-group">
          <label>VM User</label>
          <input
            type="text"
            value={vmUser()}
            onInput={(e) => setVmUser(e.currentTarget.value)}
            placeholder="obaid"
          />
        </div>
        <div class="ironclad-control-group">
          <label>VM SSH Key</label>
          <input
            type="text"
            value={vmSshKey()}
            onInput={(e) => setVmSshKey(e.currentTarget.value)}
            placeholder="~/.ssh/kria_id"
          />
        </div>
        <div class="ironclad-control-group">
          <label>Host Key SHA256</label>
          <input
            type="text"
            value={vmHostkeySha256()}
            onInput={(e) => setVmHostkeySha256(e.currentTarget.value)}
            placeholder="Optional: pin host key"
          />
        </div>
      </div>

      <div class="ironclad-chip-row" style={{ "margin-bottom": "10px" }}>
        <label class="ironclad-chip">
          <input
            type="checkbox"
            checked={allowDestructive()}
            onChange={(e) => setAllowDestructive(e.currentTarget.checked)}
          />
          VM destructive enabled
        </label>
        <label class="ironclad-chip">
          <input
            type="checkbox"
            checked={snapshot()}
            onChange={(e) => setSnapshot(e.currentTarget.checked)}
          />
          snapshot hooks
        </label>
        <label class="ironclad-chip">
          <input
            type="checkbox"
            checked={dockerFallback()}
            onChange={(e) => setDockerFallback(e.currentTarget.checked)}
          />
          Docker fallback
        </label>
        <label class="ironclad-chip">
          <input
            type="checkbox"
            checked={continueOnFailure()}
            onChange={(e) => setContinueOnFailure(e.currentTarget.checked)}
          />
          Continue on failure
        </label>
      </div>

      <div class="ironclad-strip-actions" style={{ "margin-bottom": "10px" }}>
        <button class="btn-secondary" disabled={busy() || runState().running} onClick={startRun}>
          Start Run
        </button>
        <button class="btn-danger" disabled={busy() || !runState().running} onClick={stopRun}>
          Stop Run
        </button>
        <button class="btn-secondary" disabled={busy()} onClick={() => void refreshHistory()}>
          Refresh History
        </button>
        <button class="btn-secondary" disabled={busy() || !selectedReportPath()} onClick={deleteSelectedReport}>
          Delete Selected Report
        </button>
        <button class="btn-danger" disabled={busy() || runState().running} onClick={deleteAllLogs}>
          Clear All Logs
        </button>
      </div>

      <Show when={error()}>
        <div class="ironclad-muted" style={{ color: "#ff8f8f" }}>{error()}</div>
      </Show>

      <Show when={runResult()}>
        {(result) => (
          <div
            style={{
              padding: "10px 14px",
              "border-radius": "8px",
              "margin-bottom": "10px",
              background: result().failed > 0 ? "rgba(239,68,68,0.15)" : "rgba(34,197,94,0.15)",
              border: `1px solid ${result().failed > 0 ? "rgba(239,68,68,0.4)" : "rgba(34,197,94,0.4)"}`,
            }}
          >
            <div style={{ "font-weight": "600", "margin-bottom": "6px", "font-size": "13px" }}>
              {result().failed > 0 ? "❌" : "✅"} Run {result().failed > 0 ? "Failed" : "Passed"}
              {result().exitCode !== 0 ? ` (exit ${result().exitCode})` : ""}
            </div>
            <div style={{ "font-size": "12px", color: "var(--text-secondary, #aaa)" }}>
              <span style={{ color: "#4ade80" }}>Passed: {result().passed}</span>
              {' · '}
              <span style={{ color: result().failed > 0 ? "#f87171" : "#aaa" }}>Failed: {result().failed}</span>
              {' · '}
              <span>Skipped: {result().skipped}</span>
            </div>
            <Show when={result().failedSuites.length > 0}>
              <div style={{ "margin-top": "6px", "font-size": "12px" }}>
                <strong style={{ color: "#f87171" }}>Failed suites:</strong>
                <For each={result().failedSuites}>{(suite) => (
                  <div style={{ "padding-left": "8px", color: "#fca5a5" }}>• {suite}</div>
                )}</For>
              </div>
            </Show>
            <Show when={result().reportPath}>
              <div style={{ "margin-top": "4px", "font-size": "11px", color: "#888" }}>
                Report: {result().reportPath}
              </div>
            </Show>
          </div>
        )}
      </Show>

      <div class="ironclad-muted" style={{ "margin-bottom": "8px" }}>
        {runState().command || "No active command"}
      </div>

      <div style={{ display: "grid", "grid-template-columns": "1fr 1fr", gap: "12px" }}>
        <div>
          <strong>Realtime Logs</strong>
          <div
            ref={logContainer}
            style={{
              "max-height": "320px",
              overflow: "auto",
              padding: "8px",
              background: "rgba(255,255,255,0.08)",
              "border-radius": "8px",
              "font-size": "11px",
              "line-height": "1.4",
              "font-family": "monospace",
            }}
          >
            <For each={logs()}>{(line) => (
              <div style={{
                color: line.startsWith("[stderr]")
                  ? "#ef4444"
                  : line.startsWith("---")
                    ? "#f59e0b"
                    : "#ffffff",
              }}>{line}</div>
            )}</For>
          </div>
        </div>
        <div>
          <strong>Run History</strong>
          <div style={{ "max-height": "120px", overflow: "auto", "margin-bottom": "8px" }}>
            <For each={history()}>
              {(item) => (
                <button class="btn-secondary" style={{ width: "100%", "margin-bottom": "6px", "text-align": "left", "font-size": "11px" }} onClick={() => void readReport(item.report_path)}>
                  {item.run_label}
                </button>
              )}
            </For>
          </div>
          <strong>Report Preview</strong>
          <div style={{ "max-height": "200px", overflow: "auto", padding: "8px", background: "rgba(0,0,0,0.25)", "border-radius": "8px", "font-size": "11px", "line-height": "1.4", "white-space": "pre-wrap" }}>
            {selectedReportText()}
          </div>
        </div>
      </div>
    </section>
  );
};

export default TestRunnerDashboard;
