import {
  Component,
  For,
  Show,
  createEffect,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ── Types ─────────────────────────────────────────────────────────────────

interface DashboardPayload {
  timestamp_unix_ms: number;
  uptime_secs: number;
  overview: OverviewStats;
  memory: MemoryStats;
  mcp_servers: McpServerView[];
  mcp_failure_history: McpFailureHistoryEntry[];
  config: ConfigSnapshot;
  test_reports: TestReportEntry[];
  cognitive_score: CognitiveScoreView | null;
  system_health: SystemHealthSnapshot;
  orchestrator: OrchestratorView;
  colab: ColabView;
  tool_registry: ToolRegistryView;
}

interface OverviewStats {
  total_sessions: number;
  total_turns: number;
  total_facts: number;
  total_snippets: number;
  total_documents: number;
  total_tools: number;
  mcp_servers_running: number;
  mcp_servers_total: number;
}

interface MemoryStats {
  sessions: { session_id: string; turn_count: number; last_active: string }[];
  recent_facts: {
    id: number;
    text: string;
    category: string;
    source: string;
    decay_score: number;
    access_count: number;
  }[];
  snippets: string[];
  documents: { doc_name: string; doc_type: string; chunk_count: number }[];
  facts_by_category: Record<string, number>;
  facts_by_source: Record<string, number>;
}

interface McpServerView {
  name: string;
  command: string;
  enabled: boolean;
  state: string;
  tool_count: number;
  error: string | null;
  health: string;
  tags: string[];
  remediation: string | null;
  last_failure: { timestamp_unix_ms: number; state: string; reason: string } | null;
}

interface McpFailureHistoryEntry {
  server_name: string;
  failures: { timestamp_unix_ms: number; state: string; reason: string }[];
}

interface ConfigSnapshot {
  llm_routing_mode: string;
  llm_primary_model: string;
  voice_enabled: boolean;
  voice_stt_model: string;
  voice_tts_voice: string;
  safety_max_concurrent_tools: number;
  orchestrator_enabled: boolean;
  colab_enabled: boolean;
  telegram_enabled: boolean;
  memory_max_facts: number;
  memory_decay_threshold: number;
  executive_enabled: boolean;
  planner_enabled: boolean;
  uncertainty_enabled: boolean;
  skill_compiler_enabled: boolean;
  curiosity_enabled: boolean;
  browser_agent_enabled: boolean;
  hardware_tier: string;
}

interface TestReportEntry {
  filename: string;
  modified_unix_ms: number;
  mode: string;
  passed: number;
  failed: number;
  skipped: number;
}

interface CognitiveScoreView {
  zone: string;
  total_prompts: number;
  passed: number;
  failed: number;
  score_pct: number;
  top_failures: { prompt_id: string; expected: string; actual: string }[];
}

interface SystemHealthSnapshot {
  cpu_cores: number;
  ram_total_mb: number;
  vram_mb: number | null;
  vram_free_mb: number;
  gpu_name: string;
  hostname: string;
  uptime_secs: number;
}

interface OrchestratorView {
  active: boolean;
  backend: string;
  ngl: number;
  context_window: number;
  degradation: string;
  server_healthy: boolean;
  active_turns: number;
}

interface ColabView {
  state: string;
  server_name: string;
  selected_notebook: string;
  last_error: string | null;
}

interface ToolRegistryView {
  total_tools: number;
  by_category: Record<string, number>;
  by_risk_level: Record<string, number>;
}

// ── Styles ────────────────────────────────────────────────────────────────

const CARD = "background:var(--surface-3);border:1px solid var(--border);border-radius:12px;padding:14px;";
const CARD_TITLE = "font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:0.55px;color:var(--text-secondary);margin-bottom:10px;";
const STAT_BIG = "font-size:28px;font-weight:700;line-height:1;";
const STAT_LABEL = "font-size:10px;color:#666;margin-top:2px;";
const PILL = (c: string) =>
  `display:inline-block;padding:2px 8px;border-radius:99px;font-size:10px;font-weight:600;background:${c}22;color:${c};border:1px solid ${c}44;`;
const HEALTH_COLORS: Record<string, string> = {
  healthy: "#4ade80",
  degraded: "#fbbf24",
  error: "#f87171",
  disabled: "#888",
  stopped: "#888",
  running: "#4ade80",
  starting: "#fbbf24",
};
const BAR_BG = "background:rgba(255,255,255,0.10);border-radius:4px;overflow:hidden;height:8px;";
const formatUptime = (s: number) => {
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
};
const formatBytes = (mb: number) =>
  mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;

// ── Component ─────────────────────────────────────────────────────────────

const AnalyticsDashboard: Component = () => {
  const [data, setData] = createSignal<DashboardPayload | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [activeTab, setActiveTab] = createSignal("overview");
  const [autoRefresh, setAutoRefresh] = createSignal(true);

  const fetchData = async () => {
    setLoading(true);
    setError(null);
    try {
      const payload = await invoke<DashboardPayload>("get_analytics_dashboard");
      setData(payload);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => void fetchData());

  createEffect(() => {
    if (!autoRefresh()) return;
    const timer = setInterval(() => void fetchData(), 8000);
    onCleanup(() => clearInterval(timer));
  });

  const tabs = ["overview", "tests", "mcp", "memory", "config", "tools"];

  return (
    <section class="analytics-shell" style={{ "margin-top": "14px" }}>
      {/* ── Header ─────────────────────────────────────────────────── */}
      <div style="display:flex;align-items:center;gap:12px;margin-bottom:14px;">
        <h2 style="margin:0;font-size:18px;font-weight:700;">KRIA Analytics</h2>
        <Show when={data()}>
          <span style="font-size:11px;color:#666;">
            Updated {new Date(data()!.timestamp_unix_ms).toLocaleTimeString()}
          </span>
        </Show>
        <div style="margin-left:auto;display:flex;gap:8px;">
          <button class="btn-small" onClick={() => void fetchData()} disabled={loading()}>
            {loading() ? "…" : "↻ Refresh"}
          </button>
          <label style="font-size:11px;color:#888;display:flex;align-items:center;gap:4px;">
            <input type="checkbox" checked={autoRefresh()} onChange={(e) => setAutoRefresh(e.currentTarget.checked)} />
            Auto
          </label>
        </div>
      </div>

      {/* ── Tab Bar ────────────────────────────────────────────────── */}
      <div style="display:flex;gap:4px;margin-bottom:14px;overflow-x:auto;">
        <For each={tabs}>
          {(tab) => (
            <button
              style={{
                padding: "6px 14px",
                "border-radius": "6px",
                "font-size": "12px",
                "font-weight": activeTab() === tab ? "600" : "400",
                background: activeTab() === tab ? "rgba(99,102,241,0.2)" : "transparent",
                color: activeTab() === tab ? "#818cf8" : "#888",
                border: activeTab() === tab ? "1px solid rgba(99,102,241,0.3)" : "1px solid transparent",
                cursor: "pointer",
              }}
              onClick={() => setActiveTab(tab)}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          )}
        </For>
      </div>

      <Show when={error()}>
        <div style="color:#ffd8d8;padding:10px;background:rgba(239,68,68,0.22);border:1px solid rgba(248,113,113,0.5);border-radius:8px;margin-bottom:12px;">
          {error()}
        </div>
      </Show>

      <Show when={data()} fallback={<div style="color:#888;">Loading dashboard…</div>}>
        {(d) => (
          <>
            <Show when={activeTab() === "overview"}>
              <OverviewTab data={d()} />
            </Show>
            <Show when={activeTab() === "tests"}>
              <TestsTab data={d()} />
            </Show>
            <Show when={activeTab() === "mcp"}>
              <McpTab data={d()} />
            </Show>
            <Show when={activeTab() === "memory"}>
              <MemoryTab data={d()} />
            </Show>
            <Show when={activeTab() === "config"}>
              <ConfigTab data={d()} />
            </Show>
            <Show when={activeTab() === "tools"}>
              <ToolsTab data={d()} />
            </Show>
          </>
        )}
      </Show>
    </section>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tab: Overview
// ═══════════════════════════════════════════════════════════════════════════

const OverviewTab: Component<{ data: DashboardPayload }> = (props) => {
  const d = () => props.data;
  const o = () => d().overview;
  return (
    <div>
      {/* Stat cards */}
      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px;margin-bottom:16px;">
        <StatCard label="Sessions" value={o().total_sessions} icon="💬" />
        <StatCard label="Turns" value={o().total_turns} icon="🔄" />
        <StatCard label="Facts" value={o().total_facts} icon="🧠" />
        <StatCard label="Tools" value={o().total_tools} icon="🔧" />
        <StatCard label="Documents" value={o().total_documents} icon="📄" />
        <StatCard label="Snippets" value={o().total_snippets} icon="✂️" />
        <StatCard label="MCP Servers" value={o().mcp_servers_running} sub={`/ ${o().mcp_servers_total}`} icon="🔌" />
      </div>

      {/* Hardware + Orchestrator */}
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px;margin-bottom:16px;">
        <div style={CARD}>
          <div style={CARD_TITLE}>🖥 Hardware</div>
          <div style="display:grid;grid-template-columns:1fr 1fr;gap:8px;font-size:12px;">
            <div><span style="color:#666;">Host:</span> {d().system_health.hostname}</div>
            <div><span style="color:#666;">Tier:</span> {d().config.hardware_tier}</div>
            <div><span style="color:#666;">CPU Cores:</span> {d().system_health.cpu_cores}</div>
            <div><span style="color:#666;">RAM:</span> {formatBytes(d().system_health.ram_total_mb)}</div>
            <div><span style="color:#666;">GPU:</span> {d().system_health.gpu_name}</div>
            <div><span style="color:#666;">VRAM:</span> {d().system_health.vram_mb != null ? formatBytes(d().system_health.vram_mb!) : "N/A"}</div>
            <div><span style="color:#666;">Uptime:</span> {formatUptime(d().system_health.uptime_secs)}</div>
          </div>
        </div>
        <div style={CARD}>
          <div style={CARD_TITLE}>⚡ Orchestrator</div>
          <Show when={d().orchestrator.active} fallback={<div style="color:#888;font-size:12px;">Not active</div>}>
            <div style="display:grid;grid-template-columns:1fr 1fr;gap:8px;font-size:12px;">
              <div><span style="color:#666;">Backend:</span> {d().orchestrator.backend}</div>
              <div><span style="color:#666;">NGL:</span> {d().orchestrator.ngl}</div>
              <div><span style="color:#666;">Context:</span> {d().orchestrator.context_window}</div>
              <div><span style="color:#666;">Degradation:</span> <span style={PILL(d().orchestrator.degradation === "None" ? "#4ade80" : "#fbbf24")}>{d().orchestrator.degradation}</span></div>
              <div><span style="color:#666;">Healthy:</span> {d().orchestrator.server_healthy ? "✅" : "❌"}</div>
              <div><span style="color:#666;">Active Turns:</span> {d().orchestrator.active_turns}</div>
            </div>
          </Show>
        </div>
      </div>

      {/* Cognitive + Colab */}
      <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px;">
        <div style={CARD}>
          <div style={CARD_TITLE}>🧪 Cognitive Score</div>
          <Show when={d().cognitive_score} fallback={<div style="color:#888;font-size:12px;">No score data</div>}>
            {(cs) => (
              <div>
                <div style="display:flex;align-items:baseline;gap:8px;">
                  <span style={"font-size:28px;font-weight:700;line-height:1;color:" + (cs().score_pct >= 70 ? "#4ade80" : cs().score_pct >= 50 ? "#fbbf24" : "#f87171")}>
                    {cs().score_pct.toFixed(1)}%
                  </span>
                  <span style="font-size:11px;color:#666;">{cs().passed}/{cs().total_prompts} passed</span>
                </div>
                <div style={`${BAR_BG}margin-top:8px;width:100%;`}>
                  <div style={`height:100%;width:${cs().score_pct}%;background:${cs().score_pct >= 70 ? "#4ade80" : cs().score_pct >= 50 ? "#fbbf24" : "#f87171"};border-radius:4px;`} />
                </div>
              </div>
            )}
          </Show>
        </div>
        <div style={CARD}>
          <div style={CARD_TITLE}>☁️ Colab</div>
          <div style="font-size:12px;">
            <div style={`display:flex;align-items:center;gap:6px;`}>
              <span style={`width:8px;height:8px;border-radius:50%;background:${HEALTH_COLORS[d().colab.state] || "#888"};`} />
              {d().colab.state}
            </div>
            <div style="color:#666;margin-top:4px;">Server: {d().colab.server_name || "none"}</div>
            <Show when={d().colab.selected_notebook}>
              <div style="color:#666;">Notebook: {d().colab.selected_notebook}</div>
            </Show>
            <Show when={d().colab.last_error}>
              <div style="color:#f87171;margin-top:4px;font-size:11px;">⚠ {d().colab.last_error}</div>
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tab: Tests
// ═══════════════════════════════════════════════════════════════════════════

const TestsTab: Component<{ data: DashboardPayload }> = (props) => {
  const reports = () => props.data.test_reports;
  const cs = () => props.data.cognitive_score;

  return (
    <div>
      {/* Test report history */}
      <div style={CARD}>
        <div style={CARD_TITLE}>📋 Test Reports</div>
        <Show when={reports().length > 0} fallback={<div style="color:#888;font-size:12px;">No test reports found in tests-logs/</div>}>
          <div style="max-height:400px;overflow:auto;">
            <table style="width:100%;border-collapse:collapse;font-size:11px;">
              <thead>
                <tr style="color:#666;text-align:left;">
                  <th style="padding:4px 8px;">Report</th>
                  <th style="padding:4px 8px;">Mode</th>
                  <th style="padding:4px 8px;">Passed</th>
                  <th style="padding:4px 8px;">Failed</th>
                  <th style="padding:4px 8px;">Skipped</th>
                  <th style="padding:4px 8px;">Result</th>
                  <th style="padding:4px 8px;">Date</th>
                </tr>
              </thead>
              <For each={reports()}>
                {(r) => {
                  const total = () => r.passed + r.failed + r.skipped;
                  const pct = () => (total() > 0 ? (r.passed / total()) * 100 : 0);
                  return (
                    <tr style="border-top:1px solid rgba(255,255,255,0.05);">
                      <td style="padding:4px 8px;font-family:monospace;">{r.filename.replace("KRIA_TEST_REPORT_", "").replace(".md", "")}</td>
                      <td style="padding:4px 8px;">{r.mode}</td>
                      <td style="padding:4px 8px;color:#4ade80;">{r.passed}</td>
                      <td style={{ padding: "4px 8px", color: r.failed > 0 ? "#fca5a5" : "#94a3b8" }}>{r.failed}</td>
                      <td style="padding:4px 8px;color:#888;">{r.skipped}</td>
                      <td style="padding:4px 8px;">
                        <div style="display:flex;align-items:center;gap:6px;">
                          <div style={`${BAR_BG}width:60px;`}>
                            <div style={`height:100%;width:${pct()}%;background:${r.failed > 0 ? "#f87171" : "#4ade80"};border-radius:4px;`} />
                          </div>
                          <span style={{ color: r.failed > 0 ? "#f87171" : "#4ade80" }}>{pct().toFixed(0)}%</span>
                        </div>
                      </td>
                      <td style="padding:4px 8px;color:#666;">{new Date(r.modified_unix_ms).toLocaleString()}</td>
                    </tr>
                  );
                }}
              </For>
            </table>
          </div>
        </Show>
      </div>

      {/* Cognitive score detail */}
      <Show when={cs()}>
        {(score) => (
          <div style={`${CARD}margin-top:12px;`}>
            <div style={CARD_TITLE}>🧠 Cognitive Routing Score ({score().zone})</div>
            <div style="display:flex;align-items:baseline;gap:12px;margin-bottom:12px;">
              <span style={"font-size:28px;font-weight:700;line-height:1;color:" + (score().score_pct >= 70 ? "#4ade80" : "#fbbf24")}>{score().score_pct.toFixed(1)}%</span>
              <span style="font-size:12px;color:#888;">{score().passed} passed / {score().failed} failed / {score().total_prompts} total</span>
            </div>
            <Show when={score().top_failures.length > 0}>
              <div style="font-size:11px;color:#888;margin-bottom:6px;">Top routing failures:</div>
              <div style="max-height:250px;overflow:auto;">
                <For each={score().top_failures}>
                  {(f) => (
                    <div style="display:flex;gap:8px;padding:3px 0;border-bottom:1px solid rgba(255,255,255,0.04);font-size:11px;">
                      <span style="color:#f87171;min-width:60px;">{f.prompt_id}</span>
                      <span style="color:#888;">expected</span>
                      <span style="color:#818cf8;font-family:monospace;">{f.expected}</span>
                      <span style="color:#888;">got</span>
                      <span style="color:#fbbf24;font-family:monospace;">{f.actual}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tab: MCP Servers
// ═══════════════════════════════════════════════════════════════════════════

const McpTab: Component<{ data: DashboardPayload }> = (props) => {
  const servers = () => props.data.mcp_servers;
  return (
    <div>
      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:10px;">
        <For each={servers()}>
          {(s) => (
            <div style={CARD}>
              <div style="display:flex;align-items:center;gap:8px;margin-bottom:8px;">
                <span style={`width:10px;height:10px;border-radius:50%;background:${HEALTH_COLORS[s.health] || "#888"};`} />
                <strong style="font-size:13px;">{s.name}</strong>
                <span style={PILL(HEALTH_COLORS[s.health] || "#888")}>{s.health}</span>
              </div>
              <div style="font-size:11px;color:#666;margin-bottom:6px;">{s.command} { }</div>
              <div style="display:grid;grid-template-columns:1fr 1fr;gap:4px;font-size:11px;">
                <div><span style="color:#555;">State:</span> {s.state}</div>
                <div><span style="color:#555;">Tools:</span> {s.tool_count}</div>
                <div><span style="color:#555;">Enabled:</span> {s.enabled ? "✅" : "❌"}</div>
                <div><span style="color:#555;">Tags:</span> {s.tags.join(", ")}</div>
              </div>
              <Show when={s.error}>
                <div style="color:#f87171;font-size:11px;margin-top:6px;">⚠ {s.error}</div>
              </Show>
              <Show when={s.remediation}>
                <div style="color:#fbbf24;font-size:11px;margin-top:4px;">💡 {s.remediation}</div>
              </Show>
              <Show when={s.last_failure}>
                <div style="color:#888;font-size:10px;margin-top:6px;">
                  Last failure: {new Date(s.last_failure!.timestamp_unix_ms).toLocaleString()} — {s.last_failure!.reason}
                </div>
              </Show>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tab: Memory
// ═══════════════════════════════════════════════════════════════════════════

const MemoryTab: Component<{ data: DashboardPayload }> = (props) => {
  const m = () => props.data.memory;
  const catEntries = () => Object.entries(m().facts_by_category).sort((a, b) => b[1] - a[1]);
  const srcEntries = () => Object.entries(m().facts_by_source).sort((a, b) => b[1] - a[1]);

  return (
    <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px;">
      {/* Sessions */}
      <div style={CARD}>
        <div style={CARD_TITLE}>💬 Sessions ({m().sessions.length})</div>
        <div style="max-height:250px;overflow:auto;">
          <For each={m().sessions.slice(0, 30)}>
            {(s) => (
              <div style="display:flex;justify-content:space-between;padding:3px 0;border-bottom:1px solid rgba(255,255,255,0.04);font-size:11px;">
                <span style="font-family:monospace;color:#818cf8;">{s.session_id.slice(0, 20)}</span>
                <span style="color:#888;">{s.turn_count} turns</span>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* Facts by category */}
      <div style={CARD}>
        <div style={CARD_TITLE}>🧠 Facts by Category</div>
        <For each={catEntries()}>
          {([cat, count]) => (
            <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
              <span style="font-size:11px;min-width:100px;color:#888;">{cat}</span>
              <div style={`${BAR_BG}flex:1;`}>
                <div style={`height:100%;width:${Math.min(100, (count / Math.max(...catEntries().map((e) => e[1]))) * 100)}%;background:#818cf8;border-radius:4px;`} />
              </div>
              <span style="font-size:11px;color:#aaa;min-width:30px;text-align:right;">{count}</span>
            </div>
          )}
        </For>
      </div>

      {/* Facts by source */}
      <div style={CARD}>
        <div style={CARD_TITLE}>📡 Facts by Source</div>
        <For each={srcEntries()}>
          {([src, count]) => (
            <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
              <span style="font-size:11px;min-width:100px;color:#888;">{src}</span>
              <div style={`${BAR_BG}flex:1;`}>
                <div style={`height:100%;width:${Math.min(100, (count / Math.max(...srcEntries().map((e) => e[1]))) * 100)}%;background:#4ade80;border-radius:4px;`} />
              </div>
              <span style="font-size:11px;color:#aaa;min-width:30px;text-align:right;">{count}</span>
            </div>
          )}
        </For>
      </div>

      {/* Documents */}
      <div style={CARD}>
        <div style={CARD_TITLE}>📄 Documents ({m().documents.length})</div>
        <div style="max-height:250px;overflow:auto;">
          <For each={m().documents}>
            {(doc) => (
              <div style="display:flex;justify-content:space-between;padding:3px 0;border-bottom:1px solid rgba(255,255,255,0.04);font-size:11px;">
                <span>{doc.doc_name}</span>
                <span style="color:#888;">{doc.doc_type} · {doc.chunk_count} chunks</span>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* Recent facts */}
      <div style={`${CARD}grid-column:span 2;`}>
        <div style={CARD_TITLE}>📝 Recent Facts (top 20)</div>
        <div style="max-height:300px;overflow:auto;">
          <For each={m().recent_facts.slice(0, 20)}>
            {(f) => (
              <div style="padding:4px 0;border-bottom:1px solid rgba(255,255,255,0.04);font-size:11px;">
                <div style="color:#e2e8f0;">{f.text}</div>
                <div style="color:#666;margin-top:2px;">
                  {f.category} · {f.source} · decay: {f.decay_score.toFixed(2)} · accessed: {f.access_count}x
                </div>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tab: Config
// ═══════════════════════════════════════════════════════════════════════════

const ConfigTab: Component<{ data: DashboardPayload }> = (props) => {
  const c = () => props.data.config;
  const toggle = (v: boolean) => (v ? "✅" : "❌");

  return (
    <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px;">
      <div style={CARD}>
        <div style={CARD_TITLE}>🤖 LLM</div>
        <div style="font-size:12px;">
          <div><span style="color:#666;">Model:</span> {c().llm_primary_model}</div>
          <div><span style="color:#666;">Routing:</span> {c().llm_routing_mode}</div>
          <div><span style="color:#666;">Tier:</span> {c().hardware_tier}</div>
          <div><span style="color:#666;">Orchestrator:</span> {toggle(c().orchestrator_enabled)}</div>
        </div>
      </div>
      <div style={CARD}>
        <div style={CARD_TITLE}>🎤 Voice</div>
        <div style="font-size:12px;">
          <div><span style="color:#666;">Enabled:</span> {toggle(c().voice_enabled)}</div>
          <div><span style="color:#666;">STT:</span> {c().voice_stt_model}</div>
          <div><span style="color:#666;">TTS:</span> {c().voice_tts_voice}</div>
        </div>
      </div>
      <div style={CARD}>
        <div style={CARD_TITLE}>🧠 Intelligence</div>
        <div style="font-size:12px;">
          <div><span style="color:#666;">Executive:</span> {toggle(c().executive_enabled)}</div>
          <div><span style="color:#666;">Planner:</span> {toggle(c().planner_enabled)}</div>
          <div><span style="color:#666;">Uncertainty:</span> {toggle(c().uncertainty_enabled)}</div>
          <div><span style="color:#666;">Skill Compiler:</span> {toggle(c().skill_compiler_enabled)}</div>
          <div><span style="color:#666;">Curiosity:</span> {toggle(c().curiosity_enabled)}</div>
          <div><span style="color:#666;">Browser Agent:</span> {toggle(c().browser_agent_enabled)}</div>
        </div>
      </div>
      <div style={CARD}>
        <div style={CARD_TITLE}>🛡 Safety & Memory</div>
        <div style="font-size:12px;">
          <div><span style="color:#666;">Max Concurrent Tools:</span> {c().safety_max_concurrent_tools}</div>
          <div><span style="color:#666;">Max Facts:</span> {c().memory_max_facts}</div>
          <div><span style="color:#666;">Decay Threshold:</span> {c().memory_decay_threshold}</div>
          <div><span style="color:#666;">Colab:</span> {toggle(c().colab_enabled)}</div>
          <div><span style="color:#666;">Telegram:</span> {toggle(c().telegram_enabled)}</div>
        </div>
      </div>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tab: Tools
// ═══════════════════════════════════════════════════════════════════════════

const ToolsTab: Component<{ data: DashboardPayload }> = (props) => {
  const t = () => props.data.tool_registry;
  const catEntries = () => Object.entries(t().by_category).sort((a, b) => b[1] - a[1]);
  const riskEntries = () => Object.entries(t().by_risk_level).sort((a, b) => b[1] - a[1]);

  return (
    <div style="display:grid;grid-template-columns:1fr 1fr;gap:10px;">
      <div style={CARD}>
        <div style={CARD_TITLE}>🔧 Tools by Category ({t().total_tools} total)</div>
        <For each={catEntries()}>
          {([cat, count]) => (
            <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
              <span style="font-size:11px;min-width:120px;color:#888;">{cat}</span>
              <div style={`${BAR_BG}flex:1;`}>
                <div style={`height:100%;width:${Math.min(100, (count / Math.max(...catEntries().map((e) => e[1]))) * 100)}%;background:#818cf8;border-radius:4px;`} />
              </div>
              <span style="font-size:11px;color:#aaa;min-width:30px;text-align:right;">{count}</span>
            </div>
          )}
        </For>
      </div>
      <div style={CARD}>
        <div style={CARD_TITLE}>🛡 Tools by Risk Level</div>
        <For each={riskEntries()}>
          {([risk, count]) => {
            const color = risk.includes("Green") ? "#4ade80" : risk.includes("Yellow") ? "#fbbf24" : risk.includes("Red") ? "#f87171" : "#888";
            return (
              <div style="display:flex;align-items:center;gap:8px;margin-bottom:6px;">
                <span style={PILL(color)}>{risk}</span>
                <div style={`${BAR_BG}flex:1;`}>
                  <div style={`height:100%;width:${Math.min(100, (count / Math.max(...riskEntries().map((e) => e[1]))) * 100)}%;background:${color};border-radius:4px;`} />
                </div>
                <span style="font-size:12px;color:#aaa;font-weight:600;">{count}</span>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

// ═══════════════════════════════════════════════════════════════════════════
//  Shared: StatCard
// ═══════════════════════════════════════════════════════════════════════════

const StatCard: Component<{ label: string; value: number; sub?: string; icon: string }> = (props) => (
  <div style={CARD}>
    <div style="font-size:16px;margin-bottom:4px;">{props.icon}</div>
    <div style={STAT_BIG}>{props.value.toLocaleString()}{props.sub && <span style="font-size:14px;color:#666;">{props.sub}</span>}</div>
    <div style={STAT_LABEL}>{props.label}</div>
  </div>
);

export default AnalyticsDashboard;
