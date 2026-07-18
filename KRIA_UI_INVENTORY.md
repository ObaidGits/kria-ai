# KRIA UI/UX — Complete Reverse-Engineering Inventory

> READ-ONLY audit. Source of truth for the upcoming A→Z UI/UX redesign.
> Everything below is derived from source code only (`ui/src/**` + `crates/kria-desktop/src/**`). No redesign, no proposals — current state only.
> Stack: **SolidJS 1.9 + TypeScript + Vite 6 + Tauri v2**. Styling: hand-written CSS (11 stylesheets) + heavy inline styles. Charts: **no chart.js in views** (chart.js is a dependency but visualizations are CSS width-% bars). Markdown: `marked` + `highlight.js` + `DOMPurify`.

---

## 0. Two Application Surfaces

KRIA ships **two independent frontends from one codebase**, split at runtime in `ui/src/index.tsx` by path:

| Surface | Entry | Runtime | Transport | Purpose |
|---|---|---|---|---|
| **Desktop app** | `App.tsx` | Tauri v2 webview | `invoke()` + Tauri events | The primary product (all features) |
| **Mobile PWA** | `mobile/MobileApp.tsx` | plain phone browser at `/m` | HTTP + WebSocket + WebRTC (never `invoke`) | Remote prompt-control + remote desktop |

The service worker (`ui/index.html`) is registered **only** for `/m` outside Tauri; everywhere else it aggressively unregisters/clears caches (documented "permanent blank-screen fix" for a MIME-mismatch bug).

Root error boundary (`index.tsx`) wraps both surfaces with `<BootError>` (crash screen: message + stack + Retry + Reload).

---

## 1. COMPLETE APPLICATION MAP

### 1.1 Desktop routes (hash router in `App.tsx`)

`AppRoute = "home" | "dashboard" | "vm-management" | "settings" | "tasks" | "capabilities" | "memory"`

| Hash | Route | Renders | Status |
|---|---|---|---|
| `#/` (fallback) | home | `<ChatView>` if `currentEnvironment()==="assistant"`, else `<PromptLabView>` | Live |
| `#/dashboard` | dashboard | Ironclad "Runtime Status" strip + sub-tabs (overview/operations/n8n/forensics) + lazy `<TestRunnerDashboard>`, `<AnalyticsDashboard>`, `<N8nDashboard>` | Live |
| `#/vm-management` | vm-management | VM strip + lazy `<DeviceMatrix>` | Live |
| `#/tasks` | tasks | `<TasksView>` | Live |
| `#/capabilities` | capabilities | `<CapabilitiesView>` (CPP, 10 tabs) | Live |
| `#/memory` | memory | lazy `<MemoryWorkspace>` (13 tabs) | Live |
| `#/settings` | settings | thin strip ("Open Settings Panel"/"Back to Home") + opens `<SettingsModal>` | Live |

Navigation is a flat top nav bar (7 buttons: Home, Dashboard, VM Management, Tasks, Capabilities, Memory, Settings). Per-route `<ErrorBoundary>` isolates render crashes ("Reload view" / "Back to Home").

### 1.2 Modals / overlays mounted at App root
- `SettingsModal` (`showSettings()`) — 21-tab settings hub
- `HitlModal` (`showHitl()`) — binary approval alertdialog
- `DecisionActionCenter` (always mounted; collapsible bottom-corner queue)
- `VoiceOverlay` (`voiceActive()`) — full-screen voice
- `VoiceOnboarding` (`showVoiceOnboarding()`) — 3-step wizard
- `AddTargetModal` / `EditTargetModal` (VM enrollment)
- `SetupWizard` (first-boot, gates the whole app until provisioning complete)
- Keyboard-shortcuts overlay (`showShortcuts()`, Ctrl+K)
- Toast container (module-level `addToast`, 4s auto-dismiss)

### 1.3 In-content panels (not modals)
- `WorkflowProgress` + `GuiCognitionPanel` + `ImageProgressChip` (inside ChatView)
- `ToolChoice` low-confidence modal (inside ChatView)
- Capabilities modals: Descriptor Viewer, Approval modal, Run-result toast
- Memory: `MemoryOnboarding` (cold-start wizard), `MemoryFeedbackBar`

### 1.4 Mobile PWA screens (`/m`)
- `MobilePairing` (unpaired gate)
- `MobileChat` (agent chat over WS)
- `RemoteDesktopView` (WebRTC screen view/control)
- Settings inline block (forget-token)

### 1.5 Dead / orphaned / hidden pages (proven below in §10)
- `views/CapabilityGraphView.tsx` — orphaned (no import/route)
- `views/CapabilityManagerView.tsx` — orphaned
- `views/ExecutionLogsView.tsx` — orphaned
- `views/PermissionManagerView.tsx` — orphaned
- `views/openclawIcpTypes.ts` — dead-by-transitivity (only the 4 orphaned views use it)
- `components/QuarantineQueue.tsx` — never rendered (but its store loader runs on startup)
- `components/PermissionModal.tsx` (standalone) — never imported (superseded by inline modal in SkillMarketplace)
- `components/ExecutiveDashboard.tsx` — never rendered (store plumbing still live)
- `components/PlanVisualization.tsx` — never rendered (store plumbing still live)
- `components/N8nDiagnosticsPanel.tsx` — never imported
- `components/N8nWorkflowBrowser.tsx` — dead re-export shim of `N8nWorkflowHub`
- `N8nWorkflowManagementPanel` `view="advanced"` — defined but never mounted (hub only uses `view="profiles"`)

---

## 2. NAVIGATION MAP

### 2.1 Primary nav (desktop)
- **Top bar** (`.modern-topbar`): title "KRIA" + assistant status detail; right side chips: status dot, status label, `Routing {mode}`, `{n} MCP online`, `{n} alerts`.
- **Nav bar** (`.modern-nav`): Home · Dashboard · VM Management · Tasks · Capabilities · Memory · Settings.
- **Bottom status bar** (`.modern-statusbar`): status dot+label, Core detail, MCP count, Routing, Theme.
- **Sidebar** (`SessionSidebar`): collapsible; environment tabs (Assistant / Prompt Lab), quick actions (New Chat, Temporary chat, Configure Assistant), search, grouped session list.

### 2.2 Secondary nav (tabs within pages)
- Dashboard sub-tabs: Overview / Operations / n8n / Forensics
- Capabilities tabs (10): Providers, Browser, Marketplace, Generate, Discovery, Execution Monitor, Quarantine, Evolution, Approval Center, Timeline
- Memory tabs (13): Explorer, Timeline, Goals, Planning, Reasoning, Research, Causal, Library, Knowledge Graph, Cognition, Cold Start, Metrics, Health
- Settings: 5 layers (basic/workflow/integrations/advanced/developer) × 21 tabs
- n8n hub tabs (5): Connect / Health / Ready to Run / Add from n8n / Run History
- Analytics tabs (6): overview/tests/mcp/memory/config/tools

### 2.3 Keyboard shortcuts (global, `App.tsx`)
| Key | Action |
|---|---|
| `Ctrl+,` | Open settings |
| `Ctrl+N` | New session (navigate home + `createSession`) |
| `Ctrl+Shift+V` | Toggle voice |
| `Ctrl+K` | Toggle shortcuts overlay |
| `Escape` | Close shortcuts overlay |
| `Enter` / `Shift+Enter` | Send / newline (ChatView) |
| `/` | Slash-command menu (ChatView) |
| Space/Enter hold | Push-to-talk (ChatView, when `voiceMode==="push_to_talk"`) |

### 2.4 Command palette
**None exists.** There is no command palette in the current app (slash commands in chat are the closest: `/clear`, `/session`, `/voice`, `/settings`).

### 2.5 Context menus
No custom right-click context menus. Interactions are buttons, `<details>`, double-click (session rename), and long-press (mobile remote desktop).

### 2.6 System tray
Tauri tray events consumed: `tray:toggle-voice`, `tray:open-settings`.

### 2.7 localStorage-persisted UI state
`kria_theme`, `kria_environment`, `kria_assistant_session_id`, `kria_prompt_lab_session_id`, `kria_telegram_bot_info`, `kria_manual_tool_mode`, `kria_developer_mode`, `kria_voice_onboarded`, `kria_control_panel_expanded`, `kria_fleet_matrix_visible`, `kria_wizard_complete`, `kria_assistant_frontend_prefs`, `kria_labs_frontend_prefs`, `kria_mcp_catalog`, `kria.n8n.metadataEnrichmentPrivacyAccepted.v1`, `kria_mobile_server`, `kria_mobile_token`.
---

## 3. PAGE-BY-PAGE BREAKDOWN

### 3.1 Home — ChatView (`ChatView.tsx`)
- **Purpose**: primary assistant chat.
- **Layout**: `.chat-toolbar` (session title + `ExportDropdown`) → `.chat-messages` list → `WorkflowProgress` + `GuiCognitionPanel` + thinking row + `ImageProgressChip` → `.chat-input-form`.
- **Input**: auto-grow textarea (max 150px), Enter=send / Shift+Enter=newline, voice button (🎤/🔊), push-to-talk button (conditional), attach (📎, multi-file), Stop/Send.
- **Slash commands**: `/clear`, `/session`, `/voice`, `/settings` (arrow-nav menu).
- **Tool-choice bar**: `<select>` over manual tool modes (auto, n8n, openclaw, gui_cognition, image_generation, gmail, calendar, github, filesystem, docker, browser, slack); shows Tool Mode / Routing / Selection Source.
- **Attachments**: audio→`voice_transcribe_uploaded_audio`; documents→pending file chips; images→preview; paste + drag/drop.
- **States**: empty welcome card, thinking dots (label varies by tool mode), degradation banner (critical), GPU-swap alert.
- **Tauri commands** (via store): `send_message`, `send_manual_tool_message`, `send_document_message`, `send_image_message`, `voice_transcribe_uploaded_audio`, `create_session`, `start_voice`/`stop_voice`/`voice_ptt_release`, `cancel_turn`, `cancel_gui_cognition_turn`, `read_local_image`.
- **UX issues**: `/clear` has no explicit store handler (relies on backend); low-confidence tool-choice modal is nested inside chat.

### 3.2 Home — PromptLabView (`PromptLabView.tsx`)
- **Purpose**: tool-locked lab to test tools without normal tool-guessing.
- **Controls**: App Lock select (colab/gmail/drive/docs/sheets/calendar/slides/forms), Tool Lock select (static per-app list; dynamic for colab from `colabStatus().capabilities.discovered_tools`), Strategy select (routed_within_lock / direct).
- **Commands**: `send_lab_message`, `cancel_turn("prompt_lab")`, `get_colab_tier_status`.
- Reached via home route when `currentEnvironment()==="prompt_lab"` (sidebar tab).

### 3.3 MessageBubble (`MessageBubble.tsx`, ~1400 lines)
- Roles: user / assistant / system / tool. Assistant content = sanitized markdown (DOMPurify allowlist); user = plain text.
- Code blocks: header + copy + large-block preview cap (>80k chars / >400 lines → head 220 / tail 80).
- Tool-call block (`ToolCallBlock`): status icon, args preview, metric badges; **specialized result cards** for `search_news`, `searxng_search`/`web_search`, `fetch_article`, `gw_*` (Google, trust badges + locked links), `generate_image` (lazy base64 via `read_local_image`, retry w/ backoff). GUI workflow tool calls render `GuiWorkflowViewer`.
- Error tool calls: context-aware Retry button.
- Feedback: "Wrong tool" / "Try differently" → `submit_turn_feedback`; plus `MemoryFeedbackBar`.
- Full-screen image preview overlay; task-step progress; recovery-options panel.

### 3.4 SessionSidebar (`SessionSidebar.tsx`)
- Collapsible; environment tabs; temporary-chat banner; quick actions; debounced search (`search_sessions`); grouped session list (📌 Pinned / Today / Yesterday / Previous 7 Days / Older / Archived) via `groupSessionsByRecency`.
- Row: click→switch, double-click→rename, pin/archive/rename/delete inline.
- Feature flags `CHAT_FLAGS` (all default true): coherentSessions, reuseEmpty, search, organize, temporary.
- Commands: `list_sessions`, `search_sessions`, `switch_session`, `create_session`, `set_session_pinned`, `set_session_archived`, `set_session_temporary`, `rename_session`, `delete_session`.

### 3.5 Dashboard (Ironclad Runtime Status)
- Header actions: Refresh, Collapse/Expand (persisted), Tests toggle, Analytics toggle (overview only), Forensics.
- **Overview**: QoS traffic dot + Ready/Leased counts; expanded cards Fleet Health, Adaptive QoS (p95/SLO), Recovery FSM.
- **Operations**: soft/hard reset controls (hard requires typing `HARD RESET`).
- **n8n**: lazy `N8nDashboard` (only when expanded).
- **Forensics**: record count + top-10 entries (severity, summary, category, source, "last gasp" badge, evidence `<pre>`).
- Toggled panels: `TestRunnerDashboard`, `AnalyticsDashboard`.
- Commands: `get_ironclad_status`, `get_ironclad_forensics`, `request_ironclad_soft_reset`, `request_ironclad_hard_reset`.

### 3.6 VM Management + DeviceMatrix (`DeviceMatrix.tsx`)
- Presentational fleet matrix (pure props, data via `useDeviceStatus` heartbeat hook).
- Table: Device, Mode, State, Health (CSS bar), Latency, Failures, Docker, Test, Actions (Run Docker Evals, Terminal, Edit ✏️, Delete 🗑️ w/ confirm).
- Focused terminal pane (WebSocket, caps 500 lines) + alerts list.
- `useDeviceStatus`: SSE `/api/fleet/events`, terminal WS `/api/fleet/terminal`, heartbeat POST `/api/fleet/leases/{id}/heartbeat` (15s + jitter). NOT Tauri — raw fetch/EventSource/WebSocket to controller base URL, **no auth header**.
- `AddTargetModal` → `register_new_target`; `EditTargetModal` → `update_target`; delete → `delete_target`.

### 3.7 TasksView (`TasksView.tsx`, `#/tasks`)
- Stat cards (Open/In progress/Urgent/Overdue/Done today/Blocked); add-task row; quick actions (📅 Plan my day, "I finished…"); today's plan; task list (priority dot, status select, edit/delete); Reminders section (recurrence once/daily/weekly/monthly, snooze/cancel).
- Commands: `task_list`, `task_stats`, `task_add`, `task_update_status`, `task_delete`, `task_edit`, `task_complete`, `plan_my_day`, `reminder_list`, `reminder_set`, `reminder_snooze`, `reminder_cancel`.

### 3.8 CapabilitiesView (`#/capabilities`) — see §5 CPP feature; 10 tabs, 25 `cpp_*` commands.
### 3.9 MemoryWorkspace (`#/memory`) — see §5 Memory feature; 13 tabs, 46 wired `memory_*` commands.
### 3.10 SettingsModal — see §4.

### 3.11 Empty / loading / error state coverage (cross-cutting)
Most views implement explicit empty ("No results…"), loading ("Loading…"/spinner), and error banners. ResourceDashboard deliberately shows honest "awaiting data / collecting telemetry / no decisions journaled yet" placeholders (HRA is shadow-mode). HitlModal/DecisionActionCenter do **not** surface errors on failed invoke.

---

## 4. SETTINGS BREAKDOWN (SettingsModal.tsx, ~3500 lines)

### 4.1 Config write architecture
| Path | Command | Notes |
|---|---|---|
| Granular patch | `patch_config {section, field, value}` | preferred; diffs draft vs persisted per leaf |
| Full blob | `update_settings {settings}` | fallback |
| NL command | `config_prompt {prompt}` | gated by backend env `KRIA_CONFIG_PROMPT_CONTROL=1`; "Undo last" supported |
| Load | `get_settings` | |
| Schema | `get_config_schema` | per-field `risk`, `restart_required`, `env_locked`, `env_lock_var`, `prompt_changeable`, `secret` |
| History | `get_config_history {limit}` | hash-chained audit ledger |

`<FieldBadge>` renders per-control chips: `🔒 env-locked`, `⟳ restart`, `⚠ caution` (yellow), `⚠ high-risk` (red/black). Global notices for env-locked and pending-restart fields.

### 4.2 Layers → tabs (21 tabs)
- **Basic**: Models(llm), Voice, Safety, Search, Appearance(ui), Assistant, Briefing
- **Workflow**: Automation, GUI Automation
- **Integrations**: MCP Services, Telegram, Mobile & Remote, n8n, Google, Colab, Skill Marketplace
- **Advanced**: Labs, Hardware, Knowledge
- **Developer**: Ironclad, Developer

### 4.3 Per-tab controls (config keys)
- **Models**: `ProviderSettings` (see §8 providers) + Generation Defaults: `llm.temperature`, `llm.max_tokens`, `llm.context_window`.
- **Voice**: `voice.enabled`, `voice.mode` (push_to_talk/continuous/wake_word), `voice.mic_device` + `voice.follow_system_default_mic`, `voice.tts_voice`, `voice.language`, `voice.noise_suppression_mode`, `voice.vad_silence_ms`, `voice.energy_threshold`, `voice.partial_update_ms`, `voice.confidence_threshold`.
- **Safety**: `safety.hitl_timeout_secs`, `safety.rollback_retention_hours`, `safety.tool_timeout_secs`, `safety.max_concurrent_tools`, `safety.emergency_mode`.
- **Appearance**: `ui.theme`, `ui.language`, `ui.high_contrast`, `ui.reduce_motion`, `ui.font_scale` (applied to DOM attrs immediately).
- **Assistant** (FRONTEND-ONLY, `kria_assistant_frontend_prefs`): persona, response detail, 4 toggles. No backend.
- **Labs** (FRONTEND-ONLY PROTOTYPE): 5 module toggles + hardcoded mock MCP skill catalog (`gmail-ops`, `calendar-orchestrator`, `docs-briefing-kit`, `ops-sentinel`). Dead mock data.
- **Search**: `search.engine` (duckduckgo/searxng), `search.searxng_url`.
- **MCP Services**: live list, filter/group/paginate; `toggle_mcp_server`, `restart_mcp_server_runtime`, `remove_mcp_server`, `add_mcp_server` (trust level GREEN/YELLOW/RED), `reconcile_mcp_runtime`.
- **Telegram**: bot token, allowed chat IDs, auto-start; `test_telegram_connection`, `update_telegram_config`, `start_telegram_mcp`, `stop_telegram_mcp`.
- **Mobile & Remote**: `MobileRemotePanel` (see §5 Mobile).
- **n8n**: `N8nSettings` (see §8).
- **Automation**: system health; scheduled tasks (add/remove); macros (delete); workflows (delete). Commands `get_health`, `list_scheduled_tasks`, `add_scheduled_task`, `remove_scheduled_task`, `list_macros`, `delete_macro`, `list_workflows`, `delete_workflow`.
- **GUI Automation** (RFC 008): master switch (`set_gui_automation_enabled`), status poll (`get_gui_automation_status`), **developer readiness-bypass toggle** (`get/set_gui_cognition_readiness_bypass`, TEST MODE), action-backend panel, service status (vision sidecar / uinput daemon), safety-anchor doc.
- **Hardware**: `ResourceDashboard`; detected hardware (`get_hardware_info`); tier override `hardware.tier`; GPU policy `orchestrator.gpu_autoscale`, `orchestrator.cuda_reserve_mb`, `orchestrator.vram_volatility_cap_mb`.
- **Briefing**: `BriefingBuilder` (`get_briefing_config`/`set_briefing_config`).
- **Google**: status + connect/disconnect/set-account/reconcile/restart; capabilities grid (Gmail/Calendar/Drive/Docs/Sheets/Slides/Forms/Meet); OAuth via events `gw:connected`/`gw:error`/`gw:notice`.
- **Colab**: status + connect/disconnect/set-notebook; discovered tools list.
- **Ironclad** (developer): status + forensics + advanced config (`get/update_ironclad_config`: `high_recovery_slo_ms`, `lease_ttl_ms`, `heartbeat_grace_ms`, `quarantine_cooldown_ms`, `max_normalized_hash_distance`) + soft/hard reset.
- **Knowledge**: RAG list (`list_knowledge_base`); Memory master toggle (`get/set_memory_enabled`); Clear all chat sessions (`clear_all_chat_sessions`).
- **Developer**: Developer Mode toggle (localStorage only, no backend) — reveals debug panels app-wide.
- **Skill Marketplace**: `OpenClawSettings` + `SubstrateStatus` + `SkillMarketplace` (see §5 OpenClaw).

### 4.4 Environment variables surfaced in UI
`KRIA_ACTIVE_PROVIDER`, `KRIA_ACTIVE_MODEL`, `KRIA_LLM_MODE` (LLM selection locks), `KRIA_CONFIG_PROMPT_CONTROL` (config-prompt gate), `KRIA_N8N_API_KEY`, `KRIA_N8N_SIGNING_SECRET`, `KRIA_N8N_BASIC_AUTH_USER`, plus any `env_locked` schema field (dynamic). Secret file paths: `~/.google-mcp/credentials.json`, `~/.kria/secrets/*`.

### 4.5 Hidden / developer / experimental settings
- Developer Mode (localStorage) gates GUI Cognition dev accordion, debug banners, hashes, probe timings.
- Ironclad + Developer tabs only visible in `developer` layer.
- GUI Cognition readiness-bypass = highest-risk toggle (relaxes runaway guards, resets on restart).
- Emergency Mode disables all tools.
- Labs + Assistant tabs are entirely frontend-only (no backend).

---

## 5. FEATURE INVENTORY

| Feature | Pages/Components | Backend commands | Status |
|---|---|---|---|
| **Chat** | ChatView, MessageBubble, SessionSidebar | send_message, send_manual_tool_message, sessions.* | Finished |
| **Prompt Lab** | PromptLabView | send_lab_message | Finished |
| **Voice** | VoiceOverlay, VoiceOnboarding | start/stop_voice, voice_ptt_release, voice_v2_*, voice_transcribe_* | Finished; wake-word "test" in onboarding is descriptive only |
| **Memory / RAG** | MemoryWorkspace (13 tabs), MemoryGraph, MemoryOnboarding, MemoryFeedbackBar | 46 wired `memory_*` | Finished; 5 commands unused (recall, update, resolve_entities, consolidate, graph_neighbors) |
| **Knowledge Graph** | memory/MemoryGraph | memory_graph_* | Finished (hand-rolled SVG force layout) |
| **Cold Start** | MemoryOnboarding + Memory Cold Start tab | memory_cold_start_* | Finished |
| **Goals / Planning / Reasoning / Causal** | Memory tabs | memory_goals/plans/reasoning/causal_* | Finished |
| **Cognition (dream/reflect/etc.)** | Memory Cognition tab | memory_reflect/run_dream/run_active_learning/run_self_improvement/run_entity_extraction | Finished (results discarded to toast) |
| **CPP (Capability Platform)** | CapabilitiesView (10 tabs) | 25 `cpp_*` | Finished; cpp_authorize + cpp_job_submit unused |
| **OpenClaw skills** | OpenClawSettings, SubstrateStatus, SkillMarketplace | openclaw_*, clawhub_* | Live subset; ICP views orphaned |
| **n8n workflows** | N8nWorkflowHub (5 tabs), management panel, settings | ~90 `*_n8n_*` | Partially wired; advanced registry view + many commands unreachable |
| **GUI Cognition** | GuiCognitionPanel, guiCognitionSession | cancel_gui_cognition_turn + `gui_cognition:event` | Finished (event-sourced, ~50 event types) |
| **GUI Automation (HTN)** | GuiWorkflowViewer | trigger_kill_switch + gui-workflow-* events | Finished (separate subsystem from GUI Cognition) |
| **HITL approval** | HitlModal | approve_action, deny_action | Finished (binary) |
| **Interaction Decisions** | DecisionActionCenter | resolve/resume/execute/cancel/continue interaction decision | Finished (multi-option) |
| **Fleet / VM / Ironclad** | DeviceMatrix, Add/EditTargetModal, dashboard | ironclad_*, register/update/delete_target + fleet SSE/WS | Finished |
| **Test Runner** | TestRunnerDashboard | start/stop/get_test_run_state, list_test_* | Finished (hardcoded dev VM defaults) |
| **Analytics** | AnalyticsDashboard (6 tabs) | get_analytics_dashboard | Finished (`mcp_failure_history` typed but unrendered) |
| **Executive Controller** | ExecutiveDashboard | get_executive_snapshot, cancel_executive_task | Component ORPHANED; store live |
| **Structured Planner** | PlanVisualization | intelligence:plan/step_result/goal_verification events | Component ORPHANED; store live |
| **Quarantine (tools)** | QuarantineQueue | list/approve/reject_quarantined_tool | Component ORPHANED; loader runs on startup |
| **Resource Authority (HRA)** | ResourceDashboard | get_hra_diagnostics + resource:hra_* events | Live (shadow/advisory) |
| **Tasks / Reminders / Briefing** | TasksView, BriefingBuilder | task_*, reminder_*, plan_my_day, briefing | Finished |
| **Telegram / Google / Colab** | Settings tabs | telegram/google/colab commands | Finished |
| **MCP management** | Settings MCP tab | mcp_* | Finished |
| **Provisioning / Setup** | SetupWizard | provisioning_* | Finished |
| **Model providers** | ProviderSettings | providers.* | Finished (4 commands unused) |
| **Mobile PWA / Remote Desktop** | mobile/* + MobileRemotePanel | mobile_gateway_*, remote_desktop_* | Finished; QR pairing unfinished, tool_* frames unhandled |
| **Export** | ExportDropdown | save_export_file, open_html_for_print | Finished |
| **i18n** | stores/i18n | — | Partial (7 langs, but UI strings mostly hardcoded English) |
| **Themes** | Settings Appearance + `applyTheme` | ui.theme (localStorage `kria_theme`) | Finished (dark/light) |
---

## 6. COMPONENT INVENTORY

### 6.1 Component catalogue (`ui/src/components/` + `views/` + `mobile/`)

| Component | Used where | Reusable? | Notes |
|---|---|---|---|
| ChatView | home route | no (page) | primary chat |
| MessageBubble | ChatView + PromptLab | yes | huge (~1400 lines); markdown + tool cards |
| SessionSidebar | App root | no | session mgmt |
| ToolCallBadge | MessageBubble | yes | execution-source badge |
| ImageProgressChip | ChatView | yes | image-gen progress |
| ExportDropdown | ChatView toolbar | yes | txt/md/pdf export |
| WorkflowProgress | ChatView | yes | substrate/HTN telemetry (backend-inert — see §10) |
| WorkflowSuggestionCard | n8n hub | yes | routing suggestion |
| GuiCognitionPanel | ChatView | yes | 2-layer (layman + dev accordion) |
| GuiWorkflowViewer | MessageBubble | yes | RFC-007 HTN viewer + kill switch |
| HitlModal | App root | no (singleton) | binary approval |
| DecisionActionCenter | App root | no (singleton) | multi-option decision queue |
| PlanVisualization | — | yes | **ORPHANED** |
| VoiceOverlay | App root | no | full-screen voice |
| VoiceOnboarding | App root | no | 3-step wizard |
| SetupWizard | App root | no | first-boot |
| SettingsModal | App root | no | 21-tab hub |
| ProviderSettings | Settings→Models | yes | universal provider system |
| OpenClawSettings | Settings→Marketplace | yes | substrate config |
| SubstrateStatus | Settings→Marketplace | yes | live container health (3s poll) |
| SkillMarketplace | Settings→Marketplace | yes | local+remote skills (has own inline PermissionModal) |
| PermissionModal (standalone) | — | yes | **ORPHANED** (duplicate) |
| QuarantineQueue | — | yes | **ORPHANED** |
| MemoryWorkspace | memory route | no (page) | 13 tabs |
| memory/MemoryGraph | Memory tab | yes | SVG force graph |
| memory/MemoryOnboarding | Memory Cold Start | yes | wizard |
| MemoryFeedbackBar | MessageBubble | yes | per-answer grounding feedback |
| AnalyticsDashboard | Dashboard toggle | yes | 6 tabs |
| ExecutiveDashboard | — | yes | **ORPHANED** (virtualized log, well-built) |
| ResourceDashboard | Settings→Hardware | yes | HRA shadow telemetry |
| DeviceMatrix | VM Management | yes | fleet matrix (pure props) |
| AddTargetModal / EditTargetModal | VM Management | yes | SSH enrollment |
| TestRunnerDashboard | Dashboard toggle | yes | E2E test command center |
| TasksView | tasks route | no (page) | task board |
| BriefingBuilder | Settings→Briefing | yes | briefing config |
| BootError | index.tsx root | yes | crash screen |
| N8nDashboard | Dashboard n8n tab | no | pass-through → N8nWorkflowHub |
| N8nWorkflowHub | via N8nDashboard | no | real container (5 tabs) |
| N8nWorkflowBrowser | — | — | **DEAD re-export shim** |
| N8nSettings | Settings→n8n + hub Connect | yes | connection wizard |
| N8nWorkflowManagementPanel | hub Add-from-n8n | yes | huge (~2751 lines); `advanced` view unmounted |
| N8nWorkflowCard | hub | yes | workflow card |
| N8nRunTimeline / N8nRunProgress / N8nEvidenceViewer | hub Runs | yes | run monitoring |
| N8nDiagnosticsPanel | — | — | **ORPHANED** |
| MobileRemotePanel | Settings→Mobile | yes | desktop-side gateway control |
| mobile/MobileApp, MobileChat, MobilePairing, RemoteDesktopView, RdToolbar, RdKeyboardBar | `/m` PWA | no | separate surface |

### 6.2 Duplicate / redundant UI
- **Two approval UIs**: `HitlModal` (binary) vs `DecisionActionCenter` (multi-option lifecycle) — no shared abstraction.
- **Two GUI-automation UIs**: event-sourced `GuiCognitionPanel` (guiCognition types/events) vs RFC-007 `GuiWorkflowViewer` (guiAutomation types/events) — separate type files, possible drift.
- **Two PermissionModals**: dead standalone `PermissionModal.tsx` vs live inline modal inside `SkillMarketplace` (mismatched `clawhub_install_skill` arg shapes).
- **Two n8n hubs**: `N8nWorkflowBrowser` is a dead shim of `N8nWorkflowHub`.
- **Dashboards**: Analytics + Executive + Resource + Ironclad strip overlap on system-telemetry surfacing.

---

## 7. DATA FLOW

### 7.1 Desktop pattern
```
UI event → store action → invoke("cmd", args) → Tauri (Rust command) → kria-core
                                                                            ↓
UI ← store signal ← Tauri event listener ← emit (stream token / status / progress)
```
- Central store `stores/app.ts` owns most state + all Tauri event listeners (`initListeners`).
- Streaming is per-scope (`agent` + `prompt_lab` prefixes), routed by `session_id` into per-session buckets (`sessionRuntime.ts`) so background chats keep streaming.
- Resilience: `invokeWithTimeout` (6s), session-hydration retry/backoff, 60s "thinking" watchdog, sequential prompt queue, rAF-batched intelligence events.
- Subsystem stores: `memory.ts`, `n8n.ts`, `provisioning.ts`, `guiCognitionSession.ts`, `workflowSession.ts`, `i18n.ts`, `mobile/mobileStore.ts`.

### 7.2 Live event channels (Tauri `listen`)
Chat stream (`agent:*`/`prompt_lab:*`: token/thinking/done/approval_required/tool_choice_required/tool_call/tool_result/recovery_options/task_step), `interaction_decision:created`, `runtime:status`, `config-changed`, `agent:stage`, `tray:*`, `image:*`, `workflow:telemetry`, `gui_cognition:event`, `n8n:*`, voice `voice:*`, orchestrator `orchestrator:*`, `resource:hra_*`, `colab:status`, `ironclad:*`, intelligence (`executive:*`, `policy_gate:*`, `quarantine:*`, `intelligence:*`), `fleet:target_deleted`/`updated`, `llm-runtime:apply`, `gw:*`, `provisioning:*`.

### 7.3 Mobile / fleet non-Tauri flow
- Mobile PWA: `MobileClient` WS `/ws`, `pairDevice` POST `/api/mobile/pair/complete`, remote-desktop control `/api/remote-desktop/*`, WebRTC signaling `/rd-signal`.
- Fleet heartbeat (`useDeviceStatus`): SSE `/api/fleet/events`, WS `/api/fleet/terminal`, POST heartbeat — raw browser APIs to controller base URL.

### 7.4 Example workflow — send a chat message
`ChatView.send` → `appStore.sendMessage(text)` → `invoke("send_message"|"send_manual_tool_message")` → backend agent loop → emits `agent:token`/`agent:tool_call`/`agent:done` → `initListeners` routes by session_id into bucket → `messages()` memo updates → MessageBubble re-renders.

---

## 8. TAURI COMMAND INVENTORY (authoritative, from `main.rs` invoke_handler)

Grouped by subsystem. ~230 commands total.

- **Chat**: send_message, send_manual_tool_message, send_lab_message
- **Sessions**: get_session_history, create_session, list_sessions, switch_session, delete_session, clear_all_chat_sessions, rename_session, auto_rename_session, search_sessions, set_session_pinned, set_session_archived, set_session_temporary, get_memory_enabled, set_memory_enabled
- **Memory** (52 defined): memory_search, memory_recall*, memory_reason, memory_health, memory_metrics, memory_remember, memory_update*, memory_verify, memory_forget, memory_hard_delete, memory_resolve_entities*, memory_record_feedback, memory_reflect, memory_consolidate*, memory_run_dream, memory_run_active_learning, memory_run_self_improvement, memory_run_entity_extraction, memory_library_list/ingest/delete, memory_timeline, memory_meta, memory_goals_list, memory_goal_create, memory_goal_set_status, memory_plans_analytics, memory_plans_for, memory_reasoning_analytics, memory_reasoning_history, memory_causal_effects_of/causes_of/chains, memory_graph_centrality/communities/neighbors*/relationships/search/predict_links/create_relationship, memory_explain, memory_backup, memory_restore, memory_health_report, memory_reasoning_replay, memory_cold_start_status/preview/import/cancel/set/complete  (*=wrapped but no UI caller)
- **App/HITL/decisions**: cancel_request, cancel_turn, cancel_executive_task, submit_turn_feedback, approve_action, deny_action, list_interaction_decisions, resolve/resume/execute_resolved/cancel_interaction_execution/check_continuation_after/continue_after_decision_execution/cancel_continuation/cancel_interaction_decision/replay_interaction_decisions, get_health, get_runtime_diagnostics, get_settings, patch_config, get_config_schema, get_config_history, config_prompt, list_audio_devices, update_settings, list_models
- **Voice**: start_voice, stop_voice, get_voice_status, voice_v2_speak, voice_v2_abort, voice_ptt_release, voice_v2_status, voice_turn_diagnostics, voice_transcribe_audio_file, voice_transcribe_uploaded_audio
- **Image/Doc**: send_image_message, send_document_message
- **MCP**: list_mcp_servers, reconcile_mcp_runtime, add_mcp_server, remove_mcp_server, toggle_mcp_server, restart_mcp_server_runtime
- **Telegram**: get/update_telegram_config, start/stop_telegram_mcp, test_telegram_connection
- **Mobile gateway**: mobile_gateway_status/start/stop, mobile_begin_pairing, mobile_list_devices, mobile_revoke_device, remote_desktop_status†, remote_desktop_kill, get/set_mobile_config (†=redundant; UI uses embedded status)
- **Automation**: list/add/remove_scheduled_task, list_macros, start/stop_macro_recording‡, delete_macro, list_workflows, delete_workflow (‡=no UI)
- **Media**: save_export_file, open_html_for_print, read_local_image, save_uploaded_image, get_session_media
- **n8n** (~90): see §5; store wires ~60, ~13 backend commands have no UI caller (apply_n8n_workflow_update_after_confirmation, backup_n8n_workflow, continue_n8n_workflow_authoring_operation, continue_n8n_workflow_crud_operation, create_or_update_n8n_workflow_draft, delete_n8n_workflow, dry_run_n8n_workflow_validation, get_n8n_workflow_crud_operations, reject_n8n_workflow_draft, rollback_n8n_workflow_authoring_update, rollback_n8n_workflow_backup, test_n8n_code_input_aware_copy, validate_n8n_workflow_draft); 6 more wired-but-no-component-caller (route_n8n_chat_prompt, analyze_n8n_workflow_authoring_request, generate_n8n_workflow_draft_plan, get_n8n_workflow_authoring_sessions, preview_n8n_workflow_update_diff, cleanup_n8n_workflow_draft)
- **Colab**: get_colab_tier_status, connect/disconnect_colab_tier, set_colab_selected_notebook
- **Google**: get_google_workspace_status, set_google_workspace_account, connect/disconnect_google_workspace
- **Briefing**: get/set_briefing_config
- **Tasks**: task_list/add/update_status/delete/stats/edit/complete, reminder_list/set/snooze/cancel, plan_my_day
- **Runtime/Ironclad**: get_orchestrator_status, get_hra_diagnostics, register_new_target, delete_target, update_target, get_ironclad_status, get_ironclad_forensics, request_ironclad_soft/hard_reset, get/update_ironclad_config
- **Test runner**: start/stop_test_run, get_test_run_state, list_test_history/docker_containers/test_targets, read/delete_test_report, delete_all_test_logs
- **Analytics**: get_analytics_dashboard
- **Provisioning**: get_provisioning_state, start_provisioning, complete_provisioning, set_provisioning_backend, run_provisioning_step, get_provisioning_diagnostics, get_hardware_profile
- **OpenClaw**: clawhub_list_skills*, clawhub_search_skills, clawhub_fetch_remote_skills, clawhub_install/uninstall/toggle_skill, openclaw_substrate_status/restart, openclaw_get/update_settings, install/uninstall_skill_bundle*, openclaw_generate_skill*, openclaw_recommend_skills*, openclaw_capability_manager◊, openclaw_execution_logs◊, openclaw_capability_graph◊, openclaw_list_grants◊, openclaw_revoke_grant◊, openclaw_get/set_developer_mode◊ (*=no caller; ◊=only orphaned views call it)
- **CPP**: cpp_status, cpp_list_providers, cpp_discover, cpp_catalog, cpp_recommend, cpp_quarantined, cpp_release_quarantine, cpp_health, cpp_proposals, cpp_proposal_apply/undo, cpp_get/set_autonomy, cpp_synthesis_preview, cpp_synthesize, cpp_discovery_status/scan, cpp_jobs, cpp_job_submit*, cpp_job_control, cpp_descriptor, cpp_list_grants, cpp_revoke_grant, cpp_authorize*, cpp_approve, cpp_execute, cpp_timeline (*=no caller)
- **GUI automation**: get_gui_automation_status, set_gui_automation_enabled, get/set_gui_cognition_readiness_bypass, get_grounding_status, cancel_gui_cognition_turn
- **Providers**: list_providers, get_active_provider*, get_active_llm_runtime, get_llm_runtime_apply_status, set_active_llm_selection, switch_provider*, switch_model*, test_provider_connection_cmd, test_provider_config*, discover_provider_models, upsert_provider, remove_provider, get_provider_types (*=no caller; superseded by set_active_llm_selection)
- **Workflow** (kria-desktop/commands/workflow.rs): workflow_hitl_respond, workflow_cancel, workflow_continuation, workflow_runtime_status — all `#[allow(dead_code)]`, **not registered in invoke_handler**, only referenced as TODO stubs in `workflowSession.ts`.

### 8.1 Commands with NO UI caller (summary)
Memory: recall, update, resolve_entities, consolidate, graph_neighbors.
CPP: cpp_authorize, cpp_job_submit.
OpenClaw: clawhub_list_skills, openclaw_generate_skill, openclaw_recommend_skills, install_skill_bundle, uninstall_skill_bundle (+ 7 ICP commands only reachable via orphaned views).
Providers: get_active_provider, switch_provider, switch_model, test_provider_config.
Automation: start_macro_recording, stop_macro_recording.
n8n: 13 (see §5) + 6 wired-no-component.
Workflow: all 4 (not registered).
Mobile: remote_desktop_status (redundant).
---

## 9. CURRENT DESIGN LANGUAGE

- **Styling approach**: split between 11 CSS files (`base.css`, `global.css`, `modern-layout.css`, `theme-shell.css`, `messages.css`, `memory.css`, `n8n.css`, `providers.css`, `devices.css`, `setup-wizard.css`, `mobile.css`) and **heavy inline `style={{}}` objects** (CapabilitiesView, SkillMarketplace, SubstrateStatus, QuarantineQueue, PlanVisualization, ExecutiveDashboard, ResourceDashboard, mobile — mostly inline). This is the single biggest consistency problem.
- **Theme**: `data-theme` = dark/light (localStorage `kria_theme`, applied pre-paint in index.html). CSS variables (`--bg-primary`, `--text-muted`, `--accent`, `--danger`, `--surface-1/2`, `--radius`, `--radius-sm`) used by the CSS-file components; inline-styled components hardcode hex colors instead (e.g. `#6366f1`, `#16a34a`, `#dc2626`) → theme does not reach them.
- **Color semantics**: green=ok/healthy/verified, amber/yellow=warning/community, red=error/danger/high-risk, blue/indigo=active/accent, purple=predicted/generated. Risk tiers GREEN/YELLOW/RED/BLACK.
- **Typography**: system-ui sans; monospace for logs/hashes/JSON/timeline. No type scale tokens; sizes hardcoded (10–20px).
- **Spacing/grid**: ad-hoc flex/grid with px gaps; `.modern-*` classes for shell; CSS `grid-template-columns: repeat(auto-fill, minmax(260px,1fr))` for card grids.
- **Iconography**: emoji glyphs (🎤 🔊 📎 📌 🕶 🧠 🔴 ⚠ ✓ ✗) + a few inline SVGs (mic/speaker/stop). No icon library.
- **Animations**: waveform bars, pulsing status dots, CSS width transitions on bars, spinner. `ui.reduce_motion` DOM attr exists.
- **Accessibility**: partial — `role`/`aria-*` present on modals (alertdialog/dialog), toolbars, status regions; `aria-live` on thinking/voice; focus-trap on HitlModal; high-contrast + reduce-motion + font-scale toggles. Inline-styled views have weaker a11y. Full WCAG compliance unverified (requires assistive-tech testing).
- **Responsive**: desktop-fixed for most; mobile PWA is touch-first (≥44px targets, orientation handling, pinch/pan). Desktop views not designed for narrow widths.
- **Interaction model**: click + `<details>` disclosure + tabs + toasts. No drag-drop except chat file attach and remote-desktop gestures. No command palette.

---

## 10. UI HEALTH REPORT (with proof)

### 10.1 Dead pages / orphaned components (proof = grep found no import/render outside self)
- `views/CapabilityGraphView.tsx`, `views/CapabilityManagerView.tsx`, `views/ExecutionLogsView.tsx`, `views/PermissionManagerView.tsx` — Task-13 OpenClaw ICP views; no route, no import. Their commands (`openclaw_capability_manager`, `openclaw_execution_logs`, `openclaw_capability_graph`, `openclaw_list_grants`, `openclaw_revoke_grant`, `openclaw_get/set_developer_mode`) are therefore unreachable.
- `views/openclawIcpTypes.ts` — dead by transitivity.
- `components/QuarantineQueue.tsx` — never rendered; `appStore.loadQuarantinedTools()` still runs on startup wasting a `list_quarantined_tools` call with no surface.
- `components/PermissionModal.tsx` — never imported; stale `clawhub_install_skill` arg shape.
- `components/ExecutiveDashboard.tsx` — never rendered (grep: only self-def + export); backing store + `get_executive_snapshot` + `executive:*` events live.
- `components/PlanVisualization.tsx` — never rendered (only self + TestPrompts.txt mention); backing `intelligence:*` events live.
- `components/N8nDiagnosticsPanel.tsx` — never imported (Stage-3 readiness, dead-letter drilldown, callback URL have no live UI).
- `components/N8nWorkflowBrowser.tsx` — dead re-export shim.
- `N8nWorkflowManagementPanel` `view="advanced"` — never mounted (registry/import/danger-zone unreachable).

### 10.2 Backend-inert UI (looks functional, does nothing)
- `stores/workflowSession.ts`: `respondToHitl`, `cancelActiveWorkflow`, `executeContinuation` are TODO stubs (`// TODO: invoke(...)`) — only optimistic local state. `WorkflowProgress` buttons in ChatView bottom out here → **HITL/cancel/continuation for the substrate workflow are not wired end-to-end**. `workflow_*` commands exist in Rust but are `#[allow(dead_code)]` and unregistered.

### 10.3 Dead code / unused data
- 5 unused `memory_*` wrappers; 4 unused provider commands; ~19 unused n8n commands; 2 unused cpp commands; 5 unused openclaw commands; 2 unused automation (macro recording) commands.
- Unrendered typed fields: `Metrics.tool_outcomes.seen`, `MemoryHealthReport.outbox_pending`, `AnalyticsDashboard mcp_failure_history`, `LinkPrediction.shared_neighbors`, `GraphHit.path/distance`.
- `remoteDesktopApi`: `VideoEncoder` allows vp9/h264 but every preset hardcodes vp8; `hostOnly` unused outside tests.
- `rdState.ts`: control-plane events (`request/request_ok/request_fail/confirm_fail`) + tags `requesting`/`awaiting_approval` never dispatched (view uses its own `viewPhase`); `WATCHDOG_MS`/`isTransient` reference an **unimplemented** transient-state watchdog.
- `lib/displayResponse.ts` not imported by chat views (underused).
- `mobileClient.cancel()` unused; `tool_start`/`tool_end` frames unhandled in MobileChat.

### 10.4 Inconsistent / debt
- Styling split (CSS files vs inline hardcoded hex) — theme doesn't reach inline-styled views.
- Two approval UIs, two GUI-automation UIs, two PermissionModals, two n8n hub entrypoints.
- i18n scaffolding present (7 locales) but UI strings hardcoded English → translation coverage effectively partial.
- Stale docs: MobileApp "noVNC" comment (actually WebRTC/portal); VoiceOnboarding wake-word "test" not implemented; comment drift in SettingsModal tab labels.
- QR pairing generated backend-side (`qr_payload`/`expires_at`) but never displayed or scanned.
- Hardcoded dev VM in TestRunnerDashboard (192.168.122.240 / user obaid).
- Fleet endpoints called without auth header from the frontend.

### 10.5 Unreachable but not dead
- Setup wizard external-backend "Test Connection" bypasses Tauri (raw `fetch`).

---

## 11. CURRENT WORKFLOWS (step-by-step)

- **Chat turn**: type → send → store `sendMessage` → `invoke(send_message)` → stream `agent:token` into bucket → thinking watchdog armed → `agent:tool_call`/`tool_result` render cards → `agent:done` drains prompt queue → optional feedback bar.
- **Voice turn**: toggle (button/Ctrl+Shift+V/wake) → `start_voice` → VoiceOverlay shows FSM (wake_listening→listening→transcribing→thinking→speaking) driven by `voice:*` events → mic meter from `voice:mic_level` → PTT hold sends `voice_ptt_release`.
- **Memory search**: Explorer tab → query → `memory_search` → results list + retrieval trace → select → detail + `memory_explain` → feedback/verify/forget/hard-delete.
- **Cold start**: Memory Cold Start (or auto-open MemoryOnboarding) → consent per source (filesystem/git/workspace/shell) → `memory_cold_start_preview` → select → `memory_cold_start_import` → `memory_cold_start_complete`.
- **Knowledge graph**: Graph tab → `memory_graph_centrality`+`communities` → SVG force layout → click node → `memory_graph_relationships`+`predict_links` → materialize predicted link → `memory_graph_create_relationship`.
- **Capability run**: Capabilities→Browser → discover (`cpp_discover`) → Run → `cpp_execute` → if needs_approval → approval modal → `cpp_approve(scope)` → re-run → result toast + `cpp_timeline`.
- **Capability synthesis**: Generate tab → goal → `cpp_synthesis_preview` → `cpp_synthesize` → filtered synthesis log.
- **n8n run**: hub Ready-to-Run → routing prompt (`suggest_n8n_workflows`) → WorkflowSuggestionCard → prepare input (`prepare_n8n_workflow_input`) → Run (`invoke_n8n_workflow_from_ui`) → Runs tab timeline (`n8n:*` events) → HITL resume (`resume_n8n_waiting_execution`) → evidence viewer.
- **n8n onboarding**: Add-from-n8n → sync (`discover_n8n_runtime_profile_drafts`) → Prepare with AI (privacy gate → `enrich_n8n_runtime_profile_draft`) → input-aware/code/file copy flows → Save & register (`save_n8n_profile_as_workflow_draft`) → Approve.
- **GUI cognition turn**: manual tool mode gui_cognition (or auto) → `gui_cognition:event` envelopes reduce into panel (observe→plan→resolve→safety→[HITL]→execute→verify→[recovery]) → layman summary + dev accordion → Stop → `cancel_gui_cognition_turn`.
- **HITL approval**: `agent:approval_required` → HitlModal (risk tone, GUI-proposal vs generic args) → Approve (`approve_action`) / Deny (`deny_action`, Escape=deny).
- **Interaction decision**: `interaction_decision:created` → DecisionActionCenter → resolve option → resume → execute → verify step → continue.
- **Fleet enroll**: VM Management → Add Target → `register_new_target` → SSE fleet events → DeviceMatrix live status.
- **First boot**: SetupWizard: Welcome → Hardware detection (`start_provisioning`) → Backend choice (local/external) → Model download (`run_provisioning_step`) → Sidecar setup → Verification → `complete_provisioning`.
- **Mobile pair + remote**: desktop MobileRemotePanel → start gateway → generate code → phone `/m` MobilePairing (`pairDevice`) → MobileChat (WS) or RemoteDesktopView (request→confirm HITL → WebRTC session).

---

## 12. FRONTEND ARCHITECTURE

- **Folder structure**: `ui/src/{App.tsx, index.tsx, components/, views/, mobile/, stores/, lib/, hooks/, types/, utils/, locales/, styles/}`.
- **Routing**: hash-based, manual (`routeFromHash`/`hashForRoute`) — no router library. Mobile split by pathname in index.tsx.
- **State management**: SolidJS signals + `createStore`; central `stores/app.ts` (very large — sessions, chat, voice, settings, mcp, health, ironclad, tasks, intelligence, etc.) + domain stores.
- **Context hierarchy**: no SolidJS `createContext` providers — stores are module singletons imported directly (`appStore`, `memoryStore`, `n8nStore`, etc.).
- **Lazy loading**: `DeviceMatrix`, `TestRunnerDashboard`, `AnalyticsDashboard`, `N8nDashboard`, `MemoryWorkspace`, and both root surfaces (`App`, `MobileApp`).
- **Theme system**: `data-theme` attr + CSS vars + localStorage; `applyTheme()` in store.
- **Command system**: `invoke()` wrapped per-store-action; `invokeWithTimeout` guard. No generic command registry/palette.
- **Event system**: Tauri `listen` centralized in `initListeners` (app.ts) + local listeners in guiCognitionSession, n8n store, several components.
- **Live updates**: streaming buckets (sessionRuntime), polling intervals (health 12s, ironclad 10s, executive 5s, MCP 4s, substrate 3s, capabilities timeline/jobs 3s, n8n 5s, mobile status 4s, fleet heartbeat 15s).
- **Memory live**: browser `CustomEvent("kria-memory-live")` bridge + Tauri `memory://changed`.
- **Testing**: Vitest specs co-located (stores, lib, mobile pure modules, a few components).

---

## 13. PAGE COMPLEXITY REPORT

| Page/Component | Complexity | Why |
|---|---|---|
| SettingsModal | Very Complex | 21 tabs, 5 layers, 3 write paths, schema-driven badges, ~3500 lines |
| N8nWorkflowManagementPanel | Very Complex | ~2751 lines, input-aware/code/file copy flows, lifecycle, authoring, danger zone |
| MemoryWorkspace | Very Complex | 13 tabs, 46 commands, graph, cold-start, cognition |
| app.ts store | Very Complex | central hub, ~all state + listeners + resilience machinery |
| GuiCognitionPanel + session store | Very Complex | ~50 event types, 2-layer render, heavy sanitization |
| CapabilitiesView | Complex | 10 tabs, 25 commands, 3 modals, polling |
| MessageBubble | Complex | markdown + many specialized tool cards + image lazy-load |
| N8nWorkflowHub | Complex | 5 tabs, routing/authoring/prepared-input orchestration |
| N8nSettings | Complex | connection wizard + managed docker + secrets |
| RemoteDesktopView + rd* modules | Complex | WebRTC FSM, reconnect, view transform, input |
| App.tsx | Complex | routing, dashboard, fleet derivation, listeners |
| AnalyticsDashboard | Medium | 6 tabs, read-only |
| ResourceDashboard | Medium | 6 views, shadow telemetry |
| DeviceMatrix + useDeviceStatus | Medium | matrix + SSE/WS hook |
| TestRunnerDashboard | Medium | config + logs + history |
| TasksView | Medium | tasks + reminders + plan |
| ProviderSettings | Medium | provider CRUD + apply |
| DecisionActionCenter | Medium | nested decision state machine |
| SetupWizard | Medium | 6 screens |
| SessionSidebar | Medium | grouping + search + CRUD |
| ChatView | Medium | input + attachments + panels |
| PromptLabView | Simple | locked chat |
| VoiceOverlay / VoiceOnboarding | Simple | presentational |
| HitlModal | Simple | binary modal |
| BriefingBuilder / OpenClawSettings / SkillMarketplace / SubstrateStatus | Simple | forms/cards |
| ExportDropdown / BootError / ImageProgressChip / ToolCallBadge / MemoryFeedbackBar | Simple | small utilities |
| Mobile PWA (MobileApp/Chat/Pairing) | Simple–Medium | thin screens over transport modules |

---

## 14. UI REDESIGN READINESS (classification only, no design)

- **Keep as-is (core)**: ChatView, MessageBubble, SessionSidebar, MemoryWorkspace, CapabilitiesView, VoiceOverlay, SetupWizard.
- **Should merge**: the telemetry dashboards (Ironclad strip + AnalyticsDashboard + ExecutiveDashboard + ResourceDashboard) overlap → one observability surface. HitlModal + DecisionActionCenter (two approval UIs) → one approval surface. GuiCognitionPanel + GuiWorkflowViewer (two GUI-automation UIs) → one.
- **Should split**: SettingsModal (21 tabs is a mega-modal) and N8nWorkflowManagementPanel (~2751 lines) are over-loaded.
- **Redundant / removable**: N8nWorkflowBrowser (shim), standalone PermissionModal, N8nDiagnosticsPanel, the 4 orphaned OpenClaw ICP views, QuarantineQueue (unless wired), openclawIcpTypes.
- **Could become workspaces**: Memory, Capabilities, n8n hub (already multi-tab; natural workspace candidates).
- **Could become overlays/inspectors**: MessageBubble tool-result cards, Descriptor Viewer, run-result toasts, forensics detail.
- **Could become command-palette actions**: slash commands, quick nav, "new session", "toggle voice", provider switch, config_prompt (already NL) — no palette exists today.
- **Needs wiring before/with redesign**: workflowSession HITL/cancel/continuation stubs; QuarantineQueue/ExecutiveDashboard/PlanVisualization (built, unmounted); n8n advanced registry view.

---

## 15. 2D vs 3D CLASSIFICATION (classification only — WHY)

| Surface | Classification | Why (from current data shape) |
|---|---|---|
| Chat / PromptLab | Traditional 2D | linear message stream; text-first |
| Voice | HUD / immersive overlay | already a full-screen state-driven overlay with waveform; ambient/hands-free |
| Settings | Traditional 2D | dense forms; needs precise controls |
| Memory Explorer/Timeline/Goals/etc. | Traditional 2D + inspector | lists + detail pane |
| Knowledge Graph | Visualization (2D graph, optional 3D) | already node-link SVG force layout; natural graph viz |
| Causal | Visualization | cause→effect chains are graph-shaped |
| Planning / Reasoning | Traditional 2D | ordered traces/steps |
| Capabilities (Browser/Timeline/Evolution) | Traditional 2D + inspector | catalog rows + descriptor inspector; timeline feed |
| CPP Timeline / Execution Monitor | Visualization / feed | event streams |
| n8n workflows | Visualization (node graph) + 2D | workflows are DAGs; currently cards only |
| Dashboard / Analytics / Executive / Resource | HUD / dashboard | metrics + live telemetry |
| System Monitor (Ironclad/Fleet) | Spatial workspace / HUD | fleet of targets + terminal + alerts; spatial candidate |
| DeviceMatrix | 2D table / spatial | grid of machines |
| GUI Cognition / GuiWorkflowViewer | Inspector / step timeline | sequential cognition steps + screenshots |
| HITL / Decisions | Floating panel / inspector | transient approval prompts |
| Tasks / Briefing | Traditional 2D | lists/forms |
| Remote Desktop (mobile) | Immersive (video canvas) | already full-screen WebRTC video with zoom/pan |
| Setup Wizard | Traditional 2D | stepper |

---

## 16. FINAL APPLICATION BLUEPRINT (master summary)

| Metric | Count | Notes |
|---|---|---|
| Desktop routes/pages | 7 | home, dashboard, vm-management, tasks, capabilities, memory, settings |
| Mobile PWA screens | 3 | pairing, chat, remote-desktop (+ settings block) |
| Root-mounted modals/overlays | ~10 | Settings, HITL, Decisions, Voice×2, Add/Edit target, SetupWizard, shortcuts, toasts |
| In-content panels | ~8 | WorkflowProgress, GuiCognition, ImageProgress, tool-choice, 3 Capabilities modals, MemoryOnboarding |
| Tabbed sub-surfaces | 5 areas | Dashboard(4), Capabilities(10), Memory(13), Settings(21), n8n(5), Analytics(6) |
| Stores | 8 | app, memory, n8n, provisioning, guiCognitionSession, workflowSession, i18n, mobileStore |
| SolidJS contexts/providers | 0 | module-singleton stores instead |
| Reusable components | ~40 | (see §6) |
| Dead/orphaned components | 8 | +2 dead type/shim files |
| Backend-inert UI | 1 subsystem | workflowSession HITL/cancel/continuation stubs |
| Tauri commands (registered) | ~230 | (see §8) |
| Commands with no UI caller | ~40 | (see §8.1) |
| Backend integrations | ~12 | llama.cpp/cloud LLM, MCP, n8n, Google Workspace, Colab, Telegram, ComfyUI (image), OpenClaw/Docker, fleet/SSH, mobile gateway/WebRTC, memory/SQLite, sidecar |
| Distinct workflows | ~16 | (see §11) |
| Hidden/dev-gated surfaces | ~5 | developer-mode panels, Ironclad, readiness-bypass, Labs, Assistant (frontend-only) |
| Dead/unfinished pages | 8 orphaned + advanced n8n view + QR pairing | |
| Experimental pages | Labs tab (mock), Assistant tab (frontend-only), CPP Generate/Discovery/Execution Monitor (Waves 9–11) | |
| Settings tabs | 21 (5 layers) | |
| CSS files / styling | 11 files + heavy inline | biggest consistency debt |
| Feature modules | ~30 | (see §5) |
| i18n locales | 7 | en/es/de/fr/zh/ar/hi (underused) |
| Charts | CSS bars only | chart.js dependency unused in views |

### Top redesign-relevant facts
1. Styling is fractured (CSS-vars files vs inline hardcoded hex) — theming is inconsistent and inline views ignore dark/light.
2. No command palette, no router library, no context providers — flat hash nav + singleton stores.
3. Significant dead/orphaned surface: 4 OpenClaw ICP views, QuarantineQueue, ExecutiveDashboard, PlanVisualization, N8nDiagnosticsPanel, N8nWorkflowBrowser, standalone PermissionModal, n8n advanced view.
4. `workflowSession` HITL/cancel/continuation are UI-complete but backend-inert (TODO stubs; commands unregistered).
5. Duplicated concepts: 2 approval UIs, 2 GUI-automation UIs, 2 n8n hubs, 2 PermissionModals, 4 overlapping telemetry dashboards.
6. Two independent frontends (desktop Tauri + mobile PWA) share one codebase but different transports.
7. ~40 backend commands have no reachable UI (large latent capability surface).
8. Rich event-driven live-update backbone already exists (streaming buckets, ~30 event channels, polling).

---
---

# PART II — DEEP UX/ARCHITECTURE EXTENSION (Sections A–M)

> Appended READ-ONLY audit. Everything below is source-backed. Where it corrects/challenges Part I, it is flagged **[CORRECTION]** or **[NEW]**. Click counts and cognitive-load estimates are derived from the actual control graph in source, not opinion.

## Corrections & new findings vs Part I (challenge pass)
- **[CORRECTION] Two `:root` token systems, one wins.** `base.css` defines a GitHub-dark palette (`--bg-primary:#0d1117`, `--accent:#58a6ff`, `--radius:8px`), then `theme-shell.css` (imported after, via `global.css`) redefines the SAME variables (`--bg-primary:#0f1417`, `--accent:#18a57a`, `--radius:12px`) plus a full `[data-theme="light"]` set. Net: base.css token values are **largely dead** (overridden by cascade); the live design tokens are theme-shell.css. Part I §9 undercounted this.
- **[NEW] Dangling (undefined) CSS variables.** `--surface-elevated`, `--shadow-lg`, `--accent-contrast`, `--surface-muted`, `--surface`, `--warning-bg`/`--warning-text` are **referenced but never defined** in any `:root`. They are used by `DecisionActionCenter` and `HitlModal` styles (theme-shell.css) and the degradation pill (base.css) → those surfaces render with browser fallbacks (transparent/inherit). Real latent styling bug.
- **[NEW] Styling quantified**: 433 inline `style={{` blocks and **352 hardcoded hex colors vs 132 `var(--)` references** in `.tsx` (≈73% of in-component color usage is theme-blind). 2522 `class=` usages. Worst inline offenders: CapabilitiesView (109), SkillMarketplace (53), PlanVisualization (44), SettingsModal (41), QuarantineQueue (37), ExecutiveDashboard (35), TestRunnerDashboard (21), SubstrateStatus (17).
- **[NEW] CSS scale**: 11,167 lines across 11 files (base 3664, theme-shell 2158, n8n 2059, providers 907, messages 618, devices 516, setup-wizard 515, mobile 384, memory 216, modern-layout 113, global 17). Only ~18 `@media` queries total → desktop-fixed, minimal responsiveness.
- **[NEW] CSS import chain**: `index.tsx` imports `global.css` + `mobile.css`. `global.css` `@import`s base → theme-shell → setup-wizard → messages → devices → modern-layout → providers → n8n. `memory.css` is NOT in the chain (component-imported). So inline-styled views (CapabilitiesView etc.) never reference the token files at all.

---

## SECTION A — COMPLETE USER JOURNEY MAPS

Click counts are minimum clicks along the happy path (source-derived). Cognitive load and discoverability are Low/Medium/High.

### A1. First launch (brand-new machine)
- Goal: get to a working chat.
- Start: app boots → `wizardLoading` spinner → provisioning state loaded (`get_provisioning_state`).
- Path: SetupWizard Welcome → **Get Started** (`start_provisioning`) → Hardware screen (auto-detect) → **Continue** → Backend Choice → pick **Run Locally** (`set_provisioning_backend`) → **Continue** → Model Download **Start Download** (`run_provisioning_step`, progress via `provisioning:progress`) → **Continue** → Sidecar auto-runs (`run_provisioning_step`) → **Continue** → Verification (`run_provisioning_step`) → **Start Chatting** (`complete_provisioning`, sets `kria_wizard_complete`).
- End: home/ChatView.
- Clicks: ~7–9. Cognitive load: Medium. Discoverability: High (linear stepper).
- Pain points: external-backend "Test Connection" uses raw `fetch` (inconsistent); no back navigation semantics beyond stepper; failure in sidecar → "text-only mode" with Retry/Skip (acceptable). Dead end: none.

### A2. First onboarding (returning after wizard, first real use)
- Voice not onboarded → user may open VoiceOnboarding (⚙ in VoiceOverlay, or via toggling voice). 3 steps (mic test / wake word info / engines). Wake-word step is descriptive only (no real test) — **confusing** (promises a test that doesn't exist).
- Memory cold-start: MemoryWorkspace → Cold Start tab auto-opens `MemoryOnboarding` if `onboarding_complete===false`. Consent per source → preview → import → complete. Clicks: ~5–8. Discoverability: Low (buried in Memory tab).

### A3. Returning user (daily driver)
- Goal: continue/started chat. Start: home. Path: sidebar → pick session (1 click) OR **+ New Chat**. Type → Enter.
- Clicks to send: 1–2. Cognitive load: Low. Discoverability: High.
- Pain: session search is debounced but there is no global search across message content; no pinned-quick-switch keyboard shortcut.

### A4. Power user (multi-tool, manual routing)
- Goal: force a specific tool. Path: ChatView tool-choice `<select>` → pick mode (n8n/openclaw/gui_cognition/image/gmail/…) → type → send.
- Clicks: 2–3 per turn (select persists via `kria_manual_tool_mode`). Cognitive load: Medium (12 modes, meanings not explained inline). Discoverability: Medium.
- Pain: no palette; switching tool mode is a dropdown hunt; low-confidence tool-choice modal interrupts flow.

### A5. Developer
- Goal: inspect internals. Path: Settings → Developer layer → toggle Developer Mode (localStorage) → reveals GUI Cognition dev accordion, startup banners, hashes, probe timings, Ironclad tab.
- Also: Dashboard → Tests (TestRunnerDashboard), Forensics; Capabilities → Timeline/Execution Monitor.
- Clicks to dev mode: 3 (nav Settings → Developer tab → toggle). Cognitive load: High. Discoverability: Low (must know layer switch exists).
- Pain: dev surfaces scattered across Settings(Developer/Ironclad/GUI Automation), Dashboard(Forensics/Tests), Capabilities(Timeline). No single "developer console".

### A6. AI researcher (memory / reasoning)
- Goal: inspect memory graph, reasoning traces, causal chains. Path: nav Memory → tab (Explorer/Graph/Reasoning/Causal/Planning). Each tab lazy-loads its data on select.
- Clicks: 2 per subtopic. Cognitive load: High (13 tabs, dense). Discoverability: Medium.
- Pain: 13 sibling tabs, no overview/landing; cognition triggers (dream/reflect) discard results to a toast (no result surface).

### A7. Automation builder (n8n)
- Goal: run/author a workflow. Path: nav Dashboard → n8n tab (requires panel Expanded) → hub → Connect (settings) → Add-from-n8n (sync/prepare/review/save) → Ready-to-Run (route prompt → suggestion → prepare input → Run) → Run History.
- Clicks: many (10+ to author). Cognitive load: Very High (management panel ~2751 lines of controls). Discoverability: **Low** — n8n is buried under Dashboard→n8n sub-tab AND only visible when control panel is Expanded.
- Dead ends: advanced registry view unreachable; several authoring commands wired but no button.

### A8. Voice-first user
- Goal: hands-free. Path: Ctrl+Shift+V or 🎤 → VoiceOverlay full-screen → wake/PTT/continuous per `voice.mode`.
- Clicks: 1 (or 0 with wake word). Cognitive load: Low. Discoverability: Medium (shortcut not shown until Ctrl+K).
- Pain: engine/mode selection is only in Settings→Voice, not in the overlay; barge-in/PTT only displayed, configured elsewhere.

### A9. Mobile user (prompt-control)
- Goal: chat with KRIA from phone. Path (desktop): Settings→Mobile → enable → Start gateway → Generate code. Path (phone): open `/m` → MobilePairing (type server URL + code + name) → MobileChat.
- Clicks: desktop 4–5, phone 3–4. Cognitive load: Medium. Discoverability: Low (QR generated backend-side but never shown; code typed manually).
- Pain: no QR display/scan; no in-flight cancel; tool activity frames ignored; no chat reconnect FSM.

### A10. Remote desktop user
- Goal: view/control laptop screen from phone. Path (phone): MobileApp → Desktop tab → **Start remote desktop** → request → **Confirm & connect** (HITL) → WebRTC live → toolbar (keyboard/fit/quality/stats/reconnect/disconnect).
- Clicks: 2–3 to connect. Cognitive load: Medium. Discoverability: Medium.
- Pain: STUN-only (no TURN) → off-mesh may fail; Linux-only (evdev/portal); transient states have no watchdog escalation (manual Cancel only); "noVNC" stale doc.

### A11. Fleet operator
- Goal: enroll/monitor a VM. Path: nav VM Management → Add Target (SSH fields) → `register_new_target` → Show Matrix → live status via SSE → focus terminal / run docker evals / edit / delete.
- Clicks: enroll ~6 (form). Cognitive load: Medium-High. Discoverability: Medium.
- Pain: fleet endpoints hit without auth header from frontend; docker-eval requires active lease id (silent failure toast otherwise).

---

## SECTION B — SCREEN HIERARCHY

```
KRIA (index.tsx — path split)
├── Desktop app (App.tsx, gated by SetupWizard until wizard complete)
│   ├── Sidebar (SessionSidebar)
│   │   ├── Env tabs: Assistant | Prompt Lab
│   │   ├── Quick actions: New Chat · Temporary chat · Configure Assistant
│   │   ├── Search
│   │   └── Session list (📌 Pinned / Today / Yesterday / Previous 7 Days / Older / Archived)
│   ├── Top bar (status dot, label, Routing, MCP online, alerts)
│   ├── Nav: Home · Dashboard · VM Management · Tasks · Capabilities · Memory · Settings
│   ├── Home
│   │   ├── ChatView (assistant env)
│   │   │   ├── Toolbar (title + ExportDropdown → Text/Markdown/PDF)
│   │   │   ├── Messages (MessageBubble: user/assistant/system/tool + tool cards + images)
│   │   │   ├── WorkflowProgress · GuiCognitionPanel · ImageProgressChip
│   │   │   ├── Tool-choice bar + low-confidence modal
│   │   │   └── Input (textarea, slash menu, attach, voice, PTT, send/stop)
│   │   └── PromptLabView (prompt_lab env): App Lock · Tool Lock · Strategy + messages
│   ├── Dashboard (Ironclad strip)
│   │   ├── Overview (QoS, Fleet Health, Adaptive QoS, Recovery FSM) → Analytics toggle → AnalyticsDashboard(overview/tests/mcp/memory/config/tools)
│   │   ├── Operations (soft/hard reset)
│   │   ├── n8n → N8nDashboard → N8nWorkflowHub (Connect/Health/Ready-to-Run/Add-from-n8n/Run History)
│   │   ├── Forensics (records + evidence)
│   │   └── Tests toggle → TestRunnerDashboard
│   ├── VM Management → DeviceMatrix (table + terminal + alerts) + Add/Edit Target modals
│   ├── Tasks → TasksView (stats · tasks · reminders · plan-my-day)
│   ├── Capabilities → CapabilitiesView (Providers/Browser/Marketplace/Generate/Discovery/Execution Monitor/Quarantine/Evolution/Approval Center/Timeline) + Descriptor/Approval/Result modals
│   ├── Memory → MemoryWorkspace (Explorer/Timeline/Goals/Planning/Reasoning/Research/Causal/Library/Knowledge Graph/Cognition/Cold Start/Metrics/Health) + MemoryGraph + MemoryOnboarding
│   ├── Settings (modal, 5 layers)
│   │   ├── Basic: Models(ProviderSettings + gen defaults) · Voice · Safety · Search · Appearance · Assistant · Briefing(BriefingBuilder)
│   │   ├── Workflow: Automation · GUI Automation
│   │   ├── Integrations: MCP Services · Telegram · Mobile&Remote(MobileRemotePanel) · n8n(N8nSettings) · Google · Colab · Skill Marketplace(OpenClawSettings + SubstrateStatus + SkillMarketplace)
│   │   ├── Advanced: Labs · Hardware(ResourceDashboard) · Knowledge
│   │   └── Developer: Ironclad · Developer
│   ├── Root overlays: HitlModal · DecisionActionCenter · VoiceOverlay · VoiceOnboarding · Shortcuts · Toasts
│   └── ORPHANED (unmounted): CapabilityGraphView · CapabilityManagerView · ExecutionLogsView · PermissionManagerView · QuarantineQueue · ExecutiveDashboard · PlanVisualization · N8nDiagnosticsPanel · standalone PermissionModal
└── Mobile PWA (/m — MobileApp)
    ├── MobilePairing (unpaired gate)
    ├── Tabs: Chat (MobileChat) | Desktop (RemoteDesktopView + RdToolbar + RdKeyboardBar) | Settings (forget token)
```

---

## SECTION C — NAVIGATION GRAPH (arrive / leave per surface)

| Surface | How to ARRIVE | How to LEAVE | Deep link |
|---|---|---|---|
| Home/Chat | default; nav Home; Ctrl+N; sidebar session click; SetupWizard complete; "Back to Home" in error boundary/settings strip | nav to any route; open Settings | `#/` |
| PromptLab | Home route + sidebar Env tab "Prompt Lab" | switch env tab; nav away | `#/` (env-dependent, not in hash) |
| Dashboard | nav Dashboard | nav away | `#/dashboard` |
| — Analytics | Dashboard→Overview→Analytics toggle | toggle off; leave dashboard | none (toggle state) |
| — n8n hub | Dashboard→Expand→n8n tab | leave dashboard | none |
| — TestRunner | Dashboard→Expand→Tests toggle | toggle off | none |
| VM Management | nav VM Management | nav away | `#/vm-management` |
| — Add/Edit Target | VM strip "Add"/row ✏️ | Cancel/close/submit | none (modal) |
| Tasks | nav Tasks | nav away | `#/tasks` |
| Capabilities | nav Capabilities | nav away | `#/capabilities` |
| — Descriptor/Approval/Result | Browser row Inspect/Run | close/overlay-click | none |
| Memory | nav Memory | nav away | `#/memory` |
| — MemoryOnboarding | Cold Start tab (auto or toggle) | Finish/close | none |
| Settings | nav Settings; Ctrl+,; tray:open-settings; ChatView `/settings`; sidebar "Configure Assistant" | Cancel/ESC/overlay | `#/settings` (opens modal) |
| VoiceOverlay | 🎤; Ctrl+Shift+V; tray:toggle-voice; `/voice` | × close (stops voice) | none |
| VoiceOnboarding | VoiceOverlay ⚙; first voice | ×/Finish | none |
| HitlModal | event `agent:approval_required` (programmatic) | Approve/Deny/ESC | none |
| DecisionActionCenter | always mounted; event `interaction_decision:created` | collapse toggle | none |
| Shortcuts overlay | Ctrl+K | ESC/Ctrl+K | none |
| Mobile Chat/Desktop/Settings | `/m` after pairing; bottom tab nav | tab switch; forget token | `/m` |
| **Orphaned views** | **NONE** — no route, no button, no programmatic open | — | — |

**Key gaps**: PromptLab and all Dashboard sub-panels (Analytics/n8n/Tests) have **no deep link** (lost on reload). Env selection (assistant vs prompt_lab) is not encoded in the hash. No breadcrumbs anywhere.

---

## SECTION D — COMPONENT INTERACTION MAP

Format: Component → Parent · Stores · Key props/callbacks · Events listened · Commands invoked · Lazy?

- **App** → root · appStore, provisioningStore · — · listens fleet:target_deleted/updated (+ store owns the rest) · loadHealth/loadMcpServers/loadAlerts/loadIroncladStatus/Forensics · lazy-mounts DeviceMatrix/TestRunner/Analytics/N8nDashboard/MemoryWorkspace.
- **ChatView** → App · appStore, workflowSession, guiCognitionSession · — · (store listeners) · send_message/manual/document/image, voice, cancel_turn · no.
- **MessageBubble** → ChatView · appStore · props: message · — · read_local_image, submit_turn_feedback (via store) · no. Children: ToolCallBadge, GuiWorkflowViewer, MemoryFeedbackBar.
- **SessionSidebar** → App · appStore · onSessionActivated · — · sessions.* · no.
- **ExportDropdown** → ChatView · — · props messages/sessionTitle · — · save_export_file, open_html_for_print · no.
- **GuiCognitionPanel** → ChatView · (props session from guiCognitionSession) · onDismiss/onStop, developerMode · consumes gui_cognition:event (via store) · cancel_gui_cognition_turn · no.
- **WorkflowProgress** → ChatView · workflowSession · onHitlRespond/onCancel/onContinuation (→ TODO stubs) · workflow:telemetry (via store) · **none (inert)** · no.
- **HitlModal** → App · appStore · — · agent:approval_required (store) · approve_action/deny_action · no.
- **DecisionActionCenter** → App · appStore · — · interaction_decision:created (store) · resolve/resume/execute/cancel/continue interaction decision · no.
- **VoiceOverlay / VoiceOnboarding** → App · appStore · — · voice:* (store) · start/stop_voice, voice_ptt_release · no.
- **SettingsModal** → App · appStore · onClose · gw:connected/error/notice · get_settings/patch_config/config_prompt + ~all subsystem commands · no. Children: ProviderSettings, N8nSettings, MobileRemotePanel, ResourceDashboard, BriefingBuilder, OpenClawSettings, SubstrateStatus, SkillMarketplace.
- **ProviderSettings** → SettingsModal · — (self-managed) · — · llm-runtime:apply · list_providers/get_provider_types/get_active_llm_runtime/get_llm_runtime_apply_status/set_active_llm_selection/test_provider_connection_cmd/discover_provider_models/upsert_provider/remove_provider/list_models · no.
- **MemoryWorkspace** → App(lazy) · memoryStore · — · memory://changed + kria-memory-live (store) · 46 memory_* · yes. Children: MemoryGraph, MemoryOnboarding.
- **MemoryGraph** → MemoryWorkspace · memoryStore · — · kria-memory-live · graph_* commands · no.
- **CapabilitiesView** → App · — (self-managed, 20+ signals) · — · — (polling only) · 25 cpp_* · no.
- **N8nWorkflowHub** → N8nDashboard(lazy) · n8nStore · — · n8n:* (store) · ~60 n8n commands (store) · via lazy N8nDashboard.
- **N8nSettings** → SettingsModal/hub · — (self) · — · — · 12 n8n connection commands · no.
- **DeviceMatrix** → App(lazy) · — (pure props from useDeviceStatus) · onAddTarget/onEditTarget/onDeleteTarget/onRunDockerEvals/onFocusTerminal/onReconnectStreams · — · none (props) · yes.
- **useDeviceStatus** (hook) → App · signals · commanderBaseUrl/leaseId · SSE/WS/heartbeat · **raw fetch/EventSource/WebSocket** (not Tauri) · —.
- **AnalyticsDashboard / TestRunnerDashboard** → App(lazy) · — · — · TestRunner listens kria://tests/log_line + run_finished · get_analytics_dashboard / test_runner.* · yes.
- **MobileRemotePanel** → SettingsModal · — · — · — · mobile_gateway_*, remote_desktop_*, get/set_mobile_config · no.
- **Mobile RemoteDesktopView** → MobileApp · mobileStore · — · WebRTC/WS events · requestSession/confirmSession/stopSession/remoteStatus (HTTP) · no.
- **ORPHANED** (QuarantineQueue/ExecutiveDashboard/PlanVisualization/CapabilityGraphView/CapabilityManagerView/ExecutionLogsView/PermissionManagerView): reference appStore or invoke commands but have **no parent** — never rendered.

**Relationship summary**: `appStore` is a god-object hub touched by ~20 components; domain stores (memory/n8n/guiCognition/workflow/provisioning/mobile) are localized. No prop-drilling framework, no context — components import singleton stores directly.
---

## SECTION E — PAGE INVENTORY (visible UI elements)

### E1. ChatView
- Toolbar: session-title text, ExportDropdown (button + menu: Text/Markdown/PDF).
- Message list: MessageBubble items; assistant-welcome empty card; thinking row; degradation pill; GPU-swap alert (absolute, top-center pill).
- Panels: WorkflowProgress, GuiCognitionPanel, ImageProgressChip.
- Tool-mode bar: `<select>` (12 modes), routing/selection-source text; low-confidence tool-choice modal (candidate buttons + dismiss).
- Input: auto-grow textarea, slash-command menu, attach button (hidden multi-file input), voice button, PTT button (conditional), send/stop button; file chips bar; image preview bar.
- States: empty / thinking / degraded / gpu-swap.

### E2. MessageBubble (per message)
- Avatar (K/U), role label, timestamp, copy button.
- Assistant: sanitized markdown (tables scroll, code blocks w/ copy + preview cap).
- Tool call block: status icon, name, args preview, metric badges (sourceCount/confidence/freshness/region/exit_code/item_count/duration_ms/truncated), expand chevron; specialized result cards (news/web/article/google/image); raw `<details>`; error Retry button.
- Task-step list; recovery-options panel; feedback row (Wrong tool / Try differently); MemoryFeedbackBar; image thumbnails + full-screen preview overlay + Download.

### E3. SessionSidebar
- Collapse toggle, logo, new-session (+), env tabs, temporary-chat banner, quick-action buttons, search input + clear, group labels, session rows (title, pin, archive, rename inline, delete), archived toggle, footer count.

### E4. Dashboard (Ironclad strip)
- Strip header: title/subtitle, Refresh, Collapse/Expand, Tests, Analytics, Forensics buttons.
- Sub-tab buttons (Overview/Operations/n8n/Forensics).
- Overview: QoS traffic dot, chip rows (Ready/Leased/Tainted/Quarantine/Total), cards (Fleet Health, Adaptive QoS, Recovery FSM).
- Operations: reset-reason input, hard-reset confirm input, Soft Reset button, Hard Reset (danger) button.
- Forensics: record count, entry list (severity badge, summary, category, source, last-gasp badge, evidence `<pre>`), reset timestamps footer.

### E5. AnalyticsDashboard
- Header (title, "Updated" time, Refresh, Auto checkbox); tab bar (6); StatCards; hardware/orchestrator/cognitive-score/colab cards; tests table; MCP card grid; memory lists + CSS bars; config toggles (✅/❌); tools CSS bars/pills.

### E6. CapabilitiesView
- Strip header (title, subtitle, Refresh); status summary (CPP flag, providers, capabilities); tab bar (10, underline active); per-tab lists/rows; inputs (goal/query/synth-goal); autonomy select; Descriptor Viewer modal (`<details>` schema/guidance/expectations); Approval modal (Once/Session/Workspace/Always/Deny buttons); Run-result toast.

### E7. MemoryWorkspace
- Header (title, live dot + event count, Refresh all); status flash; left tab nav (13); content pane per tab (search box, remember input, result list + detail KV grid, feedback/verify/forget/hard-delete buttons, explain block, timeline, goal cards + status select, plan/reasoning/causal columns, library cards, cognition cards (5 buttons), cold-start consent cards + wizard, metrics tiles, health KV + backup/restore inputs + DistBars).
- MemoryGraph: SVG canvas, toolbar (zoom +/−, Fit, Reset, Re-layout, Show/Hide predicted, node/edge counts), inspector (label/degree/community, predicted links + Create, Pin/Hide), search box.

### E8. SettingsModal
- Left: layer buttons (5) with tab counts. Right: tab body. Global: env-lock notice, restart notice, settings-command box (Apply/Undo), change-history viewer. Per-tab: toggles/checkboxes, selects, number/text/password inputs, FieldBadges, section headers, save footer (Cancel + Save/Done).

### E9. DeviceMatrix
- Header (kicker/title, Add Device, stream-state pill + tooltip, lease chips, Reconnect); device table (Device/Mode/State/Health bar/Latency/Failures/Docker/Test/Actions); terminal pane (header, Detach, lines); alerts list.

### E10. TasksView
- H2; stat cards (6); add-task row (title, datetime, Add, active-only checkbox); quick actions (Plan my day, "I finished…"); today plan; task list (priority dot, title, due, status select, edit ✎, delete ×); reminders (message, minutes, recurrence select, Set) + reminder list (Snooze/cancel).

### E11. TestRunnerDashboard
- Header (Running/Idle); config (Mode/Resume/Zone/Suite/Target selects + VM fields + checkboxes); chip row; action row (Start/Stop/Refresh/Delete/Clear); RunResult banner; command line; realtime log pane (color-coded, auto-scroll); run-history buttons; report preview `<pre>`.

### E12. Mobile PWA
- MobilePairing: server URL / code / name inputs, error, submit.
- MobileChat: chat log, approval card (Approve/Deny), input form.
- RemoteDesktopView: banner, RdToolbar (keyboard/fit/disconnect + more: fullscreen/touch-mode/quality/stats/reconnect), stats line, start card + resume card, confirm card, live-status overlay, `<video>`, RdKeyboardBar (modifiers/Tab/Esc/arrows/F1–F12), hidden input.
- Settings: connected-server hint, forget-token button; bottom tab nav (Chat/Desktop/Settings).

---

## SECTION F — BUTTON / ACTION INVENTORY (representative, by surface)

Legend: →cmd = Tauri command; conf = confirmation; undo = reversible.

### Global / shell
- Nav buttons ×7 (Home/Dashboard/VM/Tasks/Capabilities/Memory/Settings) → route change.
- Ctrl+, / Ctrl+N / Ctrl+Shift+V / Ctrl+K / Esc (shortcuts).

### ChatView
- Send →send_message/send_manual_tool_message; Stop →cancel_turn; Voice →start/stop_voice; PTT hold →voice_ptt_release; Attach →file picker; slash `/clear`,`/session`→create_session,`/voice`→toggle,`/settings`; ExportDropdown Text/Markdown/PDF →save_export_file/open_html_for_print; tool-mode select; tool-choice candidate buttons →resend; feedback buttons →submit_turn_feedback; message copy.

### MessageBubble tool cards
- News: Open/Extract/Verify/Refresh (→resend prompts). Web/Article similar. Google: verify links. Image: Retry (new seed), Open (full-screen). Error tool: Retry (context-aware). Recovery options: action buttons →sendMessage.

### Dashboard
- Refresh →get_ironclad_status/forensics; Collapse/Expand (persist); Tests toggle; Analytics toggle; Forensics; sub-tab buttons; **Soft Reset** →request_ironclad_soft_reset; **Hard Reset (danger, conf=type "HARD RESET")** →request_ironclad_hard_reset.

### VM Management / DeviceMatrix
- Add Device →modal→register_new_target; Reconnect Streams →hook; Run Docker Evals →fetch POST (needs lease); Open/Hide Terminal; Edit ✏️ →update_target; Delete 🗑️ (conf=confirm()) →delete_target.

### Capabilities
- Refresh, Discover →cpp_discover, Recommend →cpp_recommend, Inspect →cpp_descriptor, Run/Execute →cpp_execute, Approve scope ×4 →cpp_approve, Deny standing →cpp_approve(false), Preview →cpp_synthesis_preview, Synthesize →cpp_synthesize, Scan now →cpp_discovery_scan, autonomy select →cpp_set_autonomy, Apply/Undo/Dismiss proposal →cpp_proposal_apply/undo, Release →cpp_release_quarantine, Revoke →cpp_revoke_grant, Cancel job →cpp_job_control.

### Memory
- Search →memory_search, Remember →memory_remember, Verify →memory_verify, Forget (warn) →memory_forget, Hard delete (danger) →memory_hard_delete, 👍/👎 →memory_record_feedback, Create goal →memory_goal_create, status select →memory_goal_set_status, Ingest →memory_library_ingest, Delete doc (danger) →memory_library_delete, Reflection/Dream/Active Learning/Self-Improvement/Entity Extraction →memory_reflect/run_dream/run_active_learning/run_self_improvement/run_entity_extraction, Backup →memory_backup, Restore (danger) →memory_restore, Grant/Revoke source →memory_cold_start_set, Create relationship →memory_graph_create_relationship.

### Settings (representative)
- Save →patch_config (per field) / update_settings; Apply command →config_prompt; Undo last →config_prompt; MCP Enable/Disable/Restart/Remove/Add; Telegram Test/Enable/Disconnect; Google Connect/Disconnect/Reconcile/Restart/Set account; Colab Connect/Disconnect/Set notebook/Open; GUI Automation master toggle →set_gui_automation_enabled + readiness-bypass toggle; Memory toggle →set_memory_enabled; Clear all chat sessions (conf=two-click) →clear_all_chat_sessions; Developer Mode toggle (localStorage); Ironclad Apply/Soft/Hard reset.

### Providers
- Use →set_active_llm_selection; Test →test_provider_connection_cmd; Models →discover_provider_models; Save/Save&Test/Save&Use →upsert_provider; Remove (non-active) →remove_provider.

### Mobile (desktop panel)
- Save settings →set_mobile_config; Start/Stop gateway →mobile_gateway_start/stop; Generate code →mobile_begin_pairing; Revoke →mobile_revoke_device; Kill session →remote_desktop_kill.

### Mobile PWA
- Pair →pairDevice; Send →sendChat; Approve/Deny →approve/deny; Start/Confirm/Cancel/Reconnect/Disconnect remote desktop; toolbar keyboard/fit/quality/stats/fullscreen; forget token →mobileStore.clear.

**Confirmation/undo coverage**: explicit confirms only on Hard Reset (typed phrase), Delete target (`confirm()`), Clear all sessions (two-click), n8n permanent delete (typed `DELETE <name>`), workflow archive (`window.confirm`). Most destructive memory/skill actions have **no confirm** (consistent with dev-context "data loss acceptable"). Undo exists only for: config (`config_prompt` undo), CPP proposals (undo), n8n backups/rollback (unwired). No global undo.

---

## SECTION G — DESIGN SYSTEM AUDIT

### G1. Token systems (two, cascade-resolved)
- **base.css `:root`** (mostly overridden): `--bg-primary:#0d1117 … --accent:#58a6ff --success:#3fb950 --warning:#d29922 --danger:#f85149 --radius:8px --radius-sm:4px`, fonts JetBrains Mono / system sans.
- **theme-shell.css `:root` (LIVE dark)**: `--bg-primary:#0f1417 --bg-secondary:#172027 --accent:#18a57a --accent-hover:#23bf8f --success:#3bc975 --warning:#f3b54a --danger:#f86d6d --border:rgba(149,180,199,.2) --radius:12px --radius-sm:8px`, fonts **Space Grotesk / IBM Plex Sans**, mono JetBrains. Plus ~80 semantic tokens (surface-1/2/3, sidebar/header/modal/input gradients, user/assistant bubble, glow, grid line).
- **theme-shell.css `[data-theme="light"]`**: full parallel light palette (`--bg-primary:#f4f8fb --accent:#0f8f6b …`).
- Bubble tokens: `--user-bubble-bg` (gradient), `--assistant-bubble-bg`, avatars, copy colors — theme-scoped.

### G2. Typography
- Families: Space Grotesk / IBM Plex Sans (sans), JetBrains Mono (mono) — declared but not bundled (no @font-face/webfont import found → falls back to system if fonts absent).
- Sizes: hardcoded 10–28px (10/11/12/13/14/16/18/24/28). No modular scale. Logo letter-spacing 1.4–2px. Weights 400/500/600/700/800.

### G3. Spacing / radius / elevation
- Spacing: ad-hoc px (2/4/6/8/10/12/14/16/18/20/24). No spacing scale token.
- Radius: `--radius` 12px, `--radius-sm` 8px (theme-shell); inline components hardcode 6/8/10/12/999px/50%.
- Elevation: `--elev-shadow` + a few hardcoded `box-shadow` (export menu `0 8px 24px`, dropdowns). `--shadow-lg` referenced but **undefined**.

### G4. Color usage — inconsistency quantified
- **352 hardcoded hex vs 132 `var(--)` in `.tsx`** (~73% theme-blind). CSS files use tokens well; inline-styled views use raw hex (`#6366f1`, `#22c55e`, `#ef4444`, `#dc2626`, `#2563eb`, `#e5e7eb`, `#9ca3af`, `#f59e0b`, `#a855f7`).
- Result: CapabilitiesView, SkillMarketplace, QuarantineQueue, ExecutiveDashboard, PlanVisualization, SubstrateStatus, the 4 orphaned ICP views, mobile MobileRemotePanel **do not respond to dark/light theme** and use a different accent family (indigo #6366f1 / blue #2563eb) than the app's teal (#18a57a).
- Semantic color drift: success is `#3fb950` (base) vs `#3bc975` (theme) vs `#22c55e`/`#16a34a` (inline) vs `#4caf50` (base task-step) — **4 different greens**. Danger: `#f85149`/`#f86d6d`/`#ef4444`/`#dc2626`/`#f44336`. Warning: `#d29922`/`#f3b54a`/`#f59e0b`/`#d97706`.

### G5. Components style consistency
- Buttons: `.btn-secondary`/`.btn-primary`/`.btn-danger`/`.settings-btn`/`.recovery-action-*` (CSS) coexist with inline `btnStyle()` helpers (SkillMarketplace) and raw inline buttons (Capabilities/ICP views). No single Button component.
- Cards/lists/tables: chat tool tables fully themed (color-mix); memory/n8n/providers have dedicated CSS; inline views hand-roll cards.
- Inputs/selects: themed in CSS (`--field-*`) but inline forms use raw borders (`1px solid #d1d5db`).
- Status dots/badges: multiple implementations (ironclad-traffic-dot, status-dot, risk-badge, hitl-severity, inline dots).

### G6. Motion
- Keyframes: `tool-pulse`, `pulse` (thinking), `gpu-alert-slide-in`, `status-pulse`, `fade-slide`, voice waveform, `mem-pulse`. CSS transitions on bars/buttons. `ui.reduce_motion` sets `data-reduce-motion` (honored inconsistently — inline animations ignore it).

### G7. Dark / light / accessibility / responsive
- Dark = default (`kria_theme`, applied pre-paint). Light = full token set — but only reaches CSS-token components; inline-hex views stay dark-styled in light mode (**broken light mode** for ~8 surfaces).
- A11y: modals have role/aria/focus-trap; `aria-live` on voice/thinking; high-contrast + reduce-motion + font-scale toggles exist. Inline views weaker (color-only status, low contrast grays). Full WCAG unverified.
- Responsive: ~18 `@media` total; one breakpoint `max-width:900px` (modern-topbar stacks). Desktop-fixed sidebar 300px. Mobile PWA is the only truly responsive surface.

### G8. Undefined/dead tokens
- Referenced-but-undefined: `--surface-elevated`, `--surface-muted`, `--surface`, `--accent-contrast`, `--shadow-lg`, `--warning-bg`, `--warning-text`. Used by DecisionActionCenter, HitlModal severity, degradation-pill → render with fallbacks.
- base.css `:root` values dead where theme-shell redefines them.

### G9. Inconsistency scorecard (approx)
- Color tokens vs hardcoded (in TSX): **27% tokens / 73% hardcoded**.
- Themed components vs inline-styled: ~60% CSS-class-driven / ~40% inline (by inline-block count 433).
- Accent families in use: 2 (teal #18a57a app; indigo/blue #6366f1/#2563eb inline views).
- Distinct greens/reds/ambers: 4 / 5 / 4.
- Button implementations: ≥4. Status-dot implementations: ≥5.
---

## SECTION H — INFORMATION ARCHITECTURE

### H1. Overloaded surfaces
- **SettingsModal**: 21 tabs across 5 layers in one modal — mixes end-user prefs (Appearance, Assistant) with deep infra (Ironclad, GUI Automation readiness-bypass, Hardware GPU policy) and integrations. Highest overload.
- **MemoryWorkspace**: 13 sibling tabs, no landing/overview; mixes CRUD (Explorer), analytics (Metrics/Health), background jobs (Cognition), viz (Graph), ingestion (Library, Cold Start).
- **N8nWorkflowManagementPanel**: ~2751 lines — profile enrichment, input-aware/code/file copy, lifecycle, production audit, authoring/draft, credentials, danger zone in one component.
- **Dashboard**: n8n hub nested under Dashboard→n8n (sub-tab of a sub-tab, gated on Expand) — wrong home for a major feature.

### H2. Unrelated content grouped
- Dashboard mixes fleet/Ironclad + n8n automation + test runner + analytics (4 unrelated domains).
- Settings "Skill Marketplace" tab bundles OpenClaw runtime config + substrate health + marketplace browsing.
- Settings "Knowledge" tab bundles RAG list + memory toggle + clear-all-sessions (destructive) together.

### H3. Duplicated information
- System telemetry appears in: top-bar chips, bottom status bar, Ironclad strip, AnalyticsDashboard(overview), ResourceDashboard, ExecutiveDashboard(orphan) — same health data surfaced 4–6 ways.
- Approval surfaces: HitlModal + DecisionActionCenter + GuiCognitionPanel(HITL) + n8n HITL resume — 4 approval UIs.
- MCP server status: Settings MCP tab + AnalyticsDashboard MCP tab.

### H4. Misplaced settings
- GPU policy (`orchestrator.*`) under Hardware; routing_mode under LLM but surfaced in top bar; Developer Mode is a display flag in a tab (not a global toggle); Clear-all-sessions (destructive) under Knowledge.
- Voice engine/mode selection only in Settings, not reachable from VoiceOverlay/Onboarding where the user actually is.

### H5. Cross-page workflows
- Authoring an n8n workflow spans Settings(n8n connect) → Dashboard→n8n(sync/prepare/save) → Ready-to-Run(route/run) → Run History — 3+ locations.
- Memory feedback spans ChatView (MemoryFeedbackBar) → MemoryWorkspace (Explorer verify/forget).
- Fleet spans VM Management (matrix) + Dashboard (Ironclad reset/forensics) + Settings (Ironclad config).

### H6. Hidden / hard-to-discover
- n8n hub (behind Dashboard→Expand→n8n).
- Developer surfaces (layer switch in Settings).
- Voice onboarding (⚙ inside overlay).
- Memory cold-start (Memory→Cold Start).
- Prompt Lab (sidebar env tab, no route).
- Orphaned pages (unreachable entirely).
- ~40 backend capabilities with no UI.

### H7. Where IA breaks down
- No top-level "Automations" / "Observability" / "Developer" homes — these concepts are scattered.
- No global search / command palette to jump to any feature.
- Route set (7) doesn't match feature count (~30) → many features are sub-tabs or modal-only, so URL/deep-linking can't address them.

---

## SECTION I — UX PAIN POINT ANALYSIS

Severity: Critical / High / Medium / Low. Each: evidence · why it hurts · impact.

### Critical
1. **Broken light theme on ~8 surfaces** — evidence: 352 hardcoded hex, no var() in CapabilitiesView/SkillMarketplace/QuarantineQueue/ExecutiveDashboard/PlanVisualization/SubstrateStatus/ICP views/MobileRemotePanel · why: light mode + theming don't apply · impact: inconsistent, unusable-in-light for those pages.
2. **Backend-inert workflow controls** — evidence: workflowSession HITL/cancel/continuation are TODO stubs, `workflow_*` unregistered · why: buttons look functional but do nothing · impact: silent failure, user trust.
3. **Dead/orphaned surfaces shipped** — evidence: 8 unmounted components + 2 dead files · why: maintenance burden, confusion for next dev · impact: wasted code, latent bugs (QuarantineQueue loader runs on startup with no UI).

### High
4. **No command palette / global search** — evidence: none in source · why: 30 features across 7 routes + modal tabs · impact: high navigation cost, low discoverability.
5. **n8n buried** — evidence: Dashboard→Expand→n8n sub-tab · why: a flagship automation feature is 3 levels deep · impact: undiscoverable; advanced view unreachable.
6. **Duplicate telemetry/approval UIs** — evidence: 4–6 telemetry surfaces, 4 approval surfaces · why: fragmented mental model · impact: inconsistent behavior, redundant maintenance.
7. **Settings mega-modal (21 tabs)** — evidence: SettingsModal ~3500 lines · why: end-user + infra + integrations mixed · impact: cognitive overload, mis-clicks on dangerous toggles.
8. **Dangling CSS vars** — evidence: 6 undefined tokens used by HitlModal/DecisionActionCenter · why: fallback rendering · impact: inconsistent critical (approval) UI.
9. **Semantic color drift** — evidence: 4 greens/5 reds/4 ambers · why: no shared palette in inline views · impact: status ambiguity.

### Medium
10. No deep links for PromptLab / dashboard sub-panels / env → lost on reload.
11. Voice engine/mode config not reachable from voice UI; wake-word onboarding "test" is fake.
12. Cognition (dream/reflect) results discarded to toast — no result surface.
13. Destructive memory/skill actions lack confirmation (intentional per dev-context, but risky as it scales).
14. Fleet endpoints called without auth header from frontend.
15. Manual tool mode (12 options) meanings not explained inline.
16. No breadcrumbs; deep tab structures (Memory 13, Capabilities 10) have no context indicator.
17. Error handling silent in some flows (HitlModal/DecisionActionCenter don't surface failed invoke; memory explain fails silently).

### Low
18. Stale docs ("noVNC"), comment drift in Settings tab labels.
19. Fonts (Space Grotesk/IBM Plex) declared but not bundled.
20. `lastSearchQuery` write-only; `mobileStore.clear()` leaves server URL; `mcp_failure_history` typed unrendered.
21. Minimal responsiveness (~18 @media) — desktop-only assumption.

### System-status / feedback / recovery review
- Status visibility: strong (top bar, status bar, per-panel live dots, polling).
- Progress indication: strong for chat/image/n8n/tests; weak for memory cognition jobs.
- Undo: sparse (config, CPP proposals, unwired n8n rollback). No global undo.
- Error recovery: chat has recovery-options + retry; remote desktop has reconnect FSM (but no watchdog); many inline views only show `String(e)` banner.

---

## SECTION J — FEATURE USAGE PRIORITY (source-derived, not opinion)

Ranked by: nav centrality + store/command coverage + cross-page references + integration count.

### Core (daily, central, most-referenced)
- Chat (send_message; central store; home default).
- Sessions (sidebar always present; sessions.* heavily used).
- Voice (global shortcut + overlay + ~19 store signals + events).
- Settings/config (patch_config; every subsystem funnels here).
- Memory (46 commands; own route; feedback loop into chat).
- Model providers (10 commands; gates all LLM).

### High
- Capabilities/CPP (25 commands, own route, 10 tabs).
- n8n (largest command surface ~90; but buried).
- Fleet/Ironclad (SSE/WS + ironclad_* + own route + dashboard).
- GUI Cognition (~50 event types; chat-integrated).
- HITL / Decisions (safety-critical, event-driven).

### Medium
- Tasks/Reminders/Briefing; Analytics; Test Runner; Google/Colab/Telegram/MCP integrations; Prompt Lab; Mobile prompt-control.

### Low / Rare
- Remote desktop (mobile-only, high-risk, Linux-only); Resource Authority (shadow/advisory); Export.

### Experimental
- CPP Generate/Discovery/Execution Monitor (Waves 9–11); Labs tab (mock); Assistant tab (frontend-only).

### Developer-only
- Ironclad config, GUI readiness-bypass, Forensics, Developer Mode, Test Runner.

### Hidden / Legacy / Dead
- Orphaned: CapabilityGraph/Manager/ExecutionLogs/PermissionManager views, QuarantineQueue, ExecutiveDashboard, PlanVisualization, N8nDiagnosticsPanel, N8nWorkflowBrowser shim, standalone PermissionModal, n8n advanced view.

---

## SECTION K — WORKFLOW DEPENDENCY GRAPH

```
                         ┌────────────┐
                         │  appStore  │  (god hub: sessions, chat, voice,
                         └─────┬──────┘   settings, mcp, health, ironclad,
                               │          intelligence, tasks, image)
   Chat ──────────────────────┼──────────────────────────────┐
     │ needs: LLM provider, (opt) Memory, tools               │
     ├── Voice ── needs: sidecars (STT/TTS), audio devices    │
     ├── GUI Cognition ── needs: GUI Automation enabled,      │
     │        vision sidecar, uinput daemon                   │
     ├── n8n tools ── needs: n8n runtime (managed/external)   │
     ├── OpenClaw skills ── needs: Docker substrate           │
     ├── Google/Colab/Telegram ── needs: MCP runtime          │
     └── Image gen ── needs: ComfyUI / cloud fallback, GPU
   Memory ── needs: SQLite + embeddings; feeds Chat grounding
   Capabilities/CPP ── needs: providers; permission gate
   HITL / Decisions ── gate Chat/GUI/CPP risky actions
   Fleet/Ironclad ── needs: controller (SSE/WS); QoS gates VM ops
   Settings/Provisioning ── configure ALL of the above
   Mobile PWA ── needs: kria-server gateway (reuses agent_loop)
   Remote Desktop ── needs: xdg-portal + WebRTC (Linux)
```

- **Central hubs**: `appStore` (frontend), LLM provider + Settings (config), Memory (grounding).
- **Cannot function without**: Chat→provider; Voice→sidecars; GUI Cognition→GUI Automation daemon; n8n tools→n8n runtime; OpenClaw→Docker; integrations→MCP; Mobile→server gateway.
- **Isolated / standalone**: Tasks/Reminders (SQLite only), Export (client-only), Analytics (read-only aggregate), Resource Authority (advisory), Mobile PWA (own transport, no desktop stores).
- **Optional everywhere**: Memory (toggle), MCP servers, ComfyUI, sidecars — must degrade gracefully.

---

## SECTION L — VISUALIZATION SUITABILITY (expanded)

| Surface | Current form | Data shape | Classification | Why |
|---|---|---|---|---|
| Chat / PromptLab | 2D stream | linear messages | **Traditional 2D** | text-first, sequential |
| Voice | full-screen overlay | ephemeral state | **Floating HUD / immersive** | ambient, hands-free, already overlay |
| Settings | forms | key-values | **Traditional 2D** | precise controls, dense |
| Memory Explorer | list+detail | records | **2D + inspector** | CRUD + detail |
| Knowledge Graph | SVG force layout | node-link | **Graph (2D, 3D-capable)** | already graph; degree/community/prediction |
| Causal | columns | cause→effect chains | **Graph / flow** | directed chains |
| Planning/Reasoning | ordered traces | steps/paths | **Timeline / 2D** | sequential; PlanVisualization=3-path compare |
| Memory Timeline | vertical timeline | time series | **Timeline** | temporal |
| Metrics/Health | tiles + CSS bars | aggregates | **Dashboard** | KPI tiles |
| Capabilities Browser | rows | catalog | **2D + inspector** | list + descriptor modal |
| CPP Timeline / Execution Monitor | monospace feed | event stream | **Timeline / feed** | chronological events |
| Evolution | proposals + health | list | **Dashboard / 2D** | oversight |
| n8n workflows | cards | DAGs | **Graph / canvas** | workflows are node graphs (currently only cards!) |
| n8n Run History | timeline + progress | runs | **Timeline** | run lifecycle |
| Dashboard/Analytics/Executive/Resource | strips + tiles | telemetry | **Dashboard / HUD** | live metrics |
| Fleet/Ironclad + DeviceMatrix | table + terminal + alerts | fleet | **Spatial workspace / HUD** | many machines + live streams |
| GUI Cognition / GuiWorkflowViewer | step panels | pipeline + screenshots | **Inspector / step timeline** | sequential cognition w/ evidence |
| HITL / Decisions | modal / queue | transient approvals | **Floating panel / inspector** | interrupt-driven |
| Tasks / Briefing | lists/forms | items | **Traditional 2D** | list mgmt |
| Remote Desktop (mobile) | video canvas | pixel stream | **Immersive (canvas)** | already full-screen video + zoom/pan |
| Setup Wizard | stepper | linear | **Traditional 2D** | onboarding |

- **Must stay 2D**: Settings, Tasks, forms, chat, provider config, wizard.
- **Strong graph/canvas candidates**: Knowledge Graph (already), n8n workflows (currently under-visualized as cards), Causal, Capability graph (data exists in orphaned view).
- **Dashboard/HUD candidates**: all telemetry surfaces (currently fragmented → merge).
- **Immersive**: Voice (HUD), Remote Desktop (canvas) already are.
- **Inspector/timeline**: GUI Cognition, CPP timeline, Memory timeline, n8n runs.

---

## SECTION M — REDESIGN READINESS SCORE

Scale 1 (poor/high-debt) – 5 (strong). "Difficulty" = redesign effort (1 easy – 5 hard).

| Page/Component | Arch | UX | Consistency | A11y | Nav | Responsive | Tech Debt | Difficulty |
|---|---|---|---|---|---|---|---|---|
| ChatView | 4 | 4 | 4 | 4 | 4 | 2 | 2 | 3 |
| MessageBubble | 3 | 4 | 4 | 3 | — | 2 | 3 | 4 (1400 lines) |
| SessionSidebar | 4 | 4 | 4 | 3 | 4 | 2 | 2 | 2 |
| SettingsModal | 2 | 2 | 3 | 3 | 3 | 1 | 4 | 5 (3500 lines, 21 tabs) |
| ProviderSettings | 3 | 3 | 3 | 3 | 3 | 2 | 3 | 3 |
| MemoryWorkspace | 3 | 3 | 4 | 3 | 3 | 1 | 3 | 4 (13 tabs) |
| MemoryGraph | 3 | 3 | 3 | 2 | — | 2 | 3 | 3 |
| CapabilitiesView | 3 | 3 | 1 (all inline hex) | 2 | 3 | 1 | 4 | 4 |
| Dashboard/Ironclad | 3 | 2 | 3 | 3 | 2 | 2 | 3 | 4 |
| AnalyticsDashboard | 3 | 3 | 3 | 3 | 3 | 2 | 2 | 3 |
| ResourceDashboard | 3 | 3 | 3 | 3 | 3 | 2 | 2 | 3 |
| DeviceMatrix + useDeviceStatus | 4 | 3 | 4 | 3 | 3 | 2 | 2 | 3 |
| TestRunnerDashboard | 3 | 3 | 3 | 3 | 3 | 2 | 2 | 3 |
| TasksView | 4 | 3 | 4 | 3 | 4 | 2 | 2 | 2 |
| N8nWorkflowHub | 3 | 2 | 4 | 3 | 1 | 2 | 3 | 4 |
| N8nWorkflowManagementPanel | 2 | 2 | 4 | 3 | 2 | 2 | 4 | 5 (2751 lines) |
| N8nSettings | 3 | 3 | 4 | 3 | 3 | 2 | 3 | 3 |
| VoiceOverlay/Onboarding | 4 | 4 | 4 | 4 | 3 | 3 | 2 | 2 |
| HitlModal | 4 | 3 | 3 (dangling vars) | 4 | — | 2 | 2 | 2 |
| DecisionActionCenter | 3 | 3 | 3 (dangling vars) | 3 | — | 2 | 3 | 3 |
| SetupWizard | 4 | 4 | 4 | 3 | 4 | 3 | 2 | 2 |
| SkillMarketplace/OpenClawSettings/SubstrateStatus | 3 | 3 | 1 (inline hex) | 2 | 3 | 1 | 3 | 3 |
| Mobile PWA (all) | 4 | 4 | 3 | 3 | 4 | 5 | 2 | 3 |
| RemoteDesktop stack | 4 | 4 | 3 | 3 | 4 | 5 | 3 | 4 |
| Orphaned components (×8) | — | — | 1 | 1 | 0 | 1 | 5 | 1 (delete) |

### Overall program readiness
- **Architecture**: solid (Solid signals, per-session buckets, event backbone, timeouts/watchdogs). Main risk = `appStore` god-object + module-singleton stores (no DI/context).
- **UX**: functional but fragmented (duplicate telemetry/approval UIs, buried n8n, mega-settings, no palette).
- **Consistency**: weakest axis — 73% hardcoded colors, 2 token systems, dangling vars, broken light mode on ~8 surfaces, ≥4 button styles.
- **Responsive**: weakest for desktop (18 @media); mobile PWA strong.
- **Tech debt hotspots**: SettingsModal, N8nWorkflowManagementPanel, MessageBubble, CapabilitiesView, all inline-styled/orphaned components.
- **Highest redesign difficulty**: SettingsModal, N8nWorkflowManagementPanel (both from sheer size + coupling). Easiest wins: delete orphaned components, unify tokens, extract a Button/Card/StatusDot primitive, merge telemetry dashboards.

---

## FINAL VALIDATION CHECKLIST (Part II)
- ✓ Every route documented (§1.1, C). ✓ Every page (§3, E). ✓ Every modal/overlay (§1.2–1.3, B). ✓ Every component (§6, D). ✓ Every interaction/button (§F). ✓ Every workflow (§11, A). ✓ Every user journey (§A). ✓ Every backend interaction (§8, D). ✓ Every store (§12, D). ✓ Every navigation path (§2, C). ✓ Every UX issue (§I). ✓ Every design inconsistency (§G). ✓ Redesign readiness (§M). ✓ CSS/token system quantified (§G). ✓ Orphaned/dead surfaces proven (§10, J).
- Remaining unknowns (require runtime/assistive-tech, not source): actual WCAG conformance, real render-time performance, exact backend payload schemas beyond what the frontend types mirror. Everything statically knowable from the frontend is now documented.

---
---

# PART III — CURRENT-UI STATUS BIBLE (Sections A–N)

> READ-ONLY reverse engineering. Everything below is verified against source (`ui/src/**`, `ui/src/styles/**`, `crates/kria-desktop/**`). ASCII wireframes mirror the JSX element hierarchy exactly — they are NOT redesigns. Any measurement is quoted from CSS/inline source; anything not provable is marked **UNKNOWN**. Cross-references use Part I §n / Part II §X.

## Corrections to Parts I & II (challenge pass 3)
- **[CORRECTION → Part II §G7/§I item ties]** Reduced motion IS globally enforced. `base.css:3430` has `@media (prefers-reduced-motion: reduce){ *,*::before,*::after { animation-duration:0.01ms !important; animation-iteration-count:1 !important; transition-duration:0.01ms !important; } }`. The `!important` beats inline non-important transitions too. So the earlier "honored inconsistently" note is **wrong for CSS/most inline**; only JS-driven `requestAnimationFrame` motion (MemoryGraph force sim, remote-desktop stats) is not covered by this rule. Corrected.
- **[CONFIRM] Sidebar width has two values by cascade**: `base.css .sidebar{width:260px}` then `theme-shell.css .sidebar{width:300px}` (theme wins) and `.sidebar.collapsed{width:64px}`.
- **[CONFIRM] Generic modal**: `.modal{width:500px; max-width:90vw; max-height:80vh}` on `.modal-overlay{position:fixed; inset:0; background:rgba(0,0,0,0.6); z-index:100}`. HitlModal overrides to `min(680px,94vw)`.
- **[NEW] 16 keyframe animations, 11 responsive breakpoints, ~19 z-index layers** (enumerated in §C/§D/§L/§M).

---

## SECTION A — VISUAL SCREEN INVENTORY (structural wireframes)

> Screenshots cannot be generated from source (no rendering here); the following ASCII wireframes reproduce the exact DOM/JSX hierarchy. Real screenshots = **UNKNOWN** (require running app).

### A1. Desktop shell (home / ChatView)
```
┌────────────────────────────────────────────────────────────────────────────┐
│ SIDEBAR (300px / 64px collapsed)  │ MAIN (.modern-main-shell)                │
│ ┌───────────────────────────────┐ │ ┌──────────────────────────────────────┐ │
│ │ [logo KRIA]        [◀] [＋]    │ │ │ .modern-topbar                        │ │
│ │ ┌ Assistant ┐┌ Prompt Lab ┐    │ │ │ KRIA · <status detail>   ●[label]     │ │
│ │ [🕶 temp banner (cond)]        │ │ │      [Routing x][n MCP][n alerts]     │ │
│ │ + New Chat                     │ │ ├──────────────────────────────────────┤ │
│ │ 🕶 Temporary chat              │ │ │ .modern-nav: Home Dashboard VM Tasks  │ │
│ │ Configure Assistant            │ │ │             Capabilities Memory Set   │ │
│ │ [search........][x]            │ │ ├──────────────────────────────────────┤ │
│ │ ── 📌 Pinned ──                │ │ │ (dev banners cond.)                   │ │
│ │  • session row [📌][🗄][✎][×]   │ │ │ ┌── ErrorBoundary ──────────────────┐ │ │
│ │ ── Today ──                    │ │ │ │ .chat-view                         │ │ │
│ │  • session row                 │ │ │ │  .chat-toolbar  title  [Export ▾]  │ │ │
│ │ ── Yesterday / Prev 7 / Older ─│ │ │ │  .chat-messages                    │ │ │
│ │ ▸ Archived (toggle)            │ │ │ │   [welcome card | MessageBubbles]  │ │ │
│ │                                │ │ │ │   [WorkflowProgress]               │ │ │
│ │ footer: N sessions             │ │ │ │   [GuiCognitionPanel]              │ │ │
│ └───────────────────────────────┘ │ │ │   [thinking row] [ImageChip]       │ │ │
│                                    │ │ │  tool-mode bar [select ▾] routing  │ │ │
│                                    │ │ │  [file chips][image preview]       │ │ │
│                                    │ │ │  input-row [📎][textarea][🎤][send]│ │ │
│                                    │ │ └────────────────────────────────────┘ │ │
│                                    │ │ .modern-statusbar (sticky bottom)      │ │
│                                    │ └──────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────┘
Root overlays (portal-less, conditional): HitlModal · DecisionActionCenter(fixed br)
 · VoiceOverlay · VoiceOnboarding · SettingsModal · Add/EditTargetModal · Shortcuts · Toasts(fixed)
```

### A2. Generic modal (SettingsModal / dialogs)
```
.modal-overlay (fixed inset:0, rgba(0,0,0,.6), z100, center)
  .modal (500px / 90vw, max-h 80vh, scroll)      ← Settings is wider (custom)
   ├ .modal-header  [title h2 16px]        [× close 24px]
   ├ (Settings only) layer rail | tab body
   ├ .modal-body (padding 20px)
   └ .modal-footer (right-aligned)  [Cancel] [Save/Done]
```

### A3. SettingsModal internal
```
┌ Settings ─────────────────────────────────────────────┐
│ LAYERS (rail)        │ TAB BODY                          │
│ • Basic (7)          │ [env-lock notice] [restart notice]│
│ • Workflow (2)       │ [settings-command box][Apply][Undo]│
│ • Integrations (7)   │ [change-history toggle]           │
│ • Advanced (3)       │ ── <Active Tab> ──                 │
│ • Developer (2)      │  section h4 + rows(label/control) │
│                      │  FieldBadge 🔒⟳⚠ per control      │
│                      │ ...                               │
│ [Cancel]                                   [Save/Done]   │
└────────────────────────────────────────────────────────┘
```

### A4. Dashboard (Ironclad strip)
```
.ironclad-strip
 ├ head: "Runtime Status" | [Refresh][Collapse][Tests][Analytics][Forensics]
 ├ tabs: Overview | Operations | n8n | Forensics
 ├ Overview: [●QoS] Ready n Leased n | cards[FleetHealth][AdaptiveQoS][RecoveryFSM]
 ├ Operations: [reason input][HARD RESET confirm] [Soft Reset][Hard Reset danger]
 ├ n8n: <N8nWorkflowHub> (only when Expanded)
 └ Forensics: count + entry rows(severity/summary/category/source/evidence<pre>)
[+ toggled] <AnalyticsDashboard>  <TestRunnerDashboard>
```

### A5. MemoryWorkspace
```
.mem-workspace
 ├ .mem-header  Memory   [●Live n]   [Refresh all]
 ├ .mem-status (flash)
 └ .mem-body
     ├ .mem-tabs (vertical): Explorer Timeline Goals Planning Reasoning
     │   Research Causal Library "Knowledge Graph" Cognition "Cold Start" Metrics Health
     └ .mem-content (per tab)  e.g. Explorer: [search][Search] [remember][Remember]
          .mem-list (results)      |  .mem-detail (KV grid + 👍👎 Verify Forget HardDelete + explain)
```

### A6. CapabilitiesView
```
.ironclad-strip.capabilities-view (inline-styled)
 ├ head: Capabilities / "Provider-neutral CPP"          [Refresh]
 ├ status: CPP flag ON | Providers h/n | Capabilities n
 ├ tabs(10): Providers Browser Marketplace Generate Discovery
 │            "Execution Monitor" Quarantine Evolution "Approval Center" Timeline
 └ tab body (rows/cards) + modals: Descriptor Viewer · Approval · Result toast
```

### A7. VM Management / DeviceMatrix
```
 VM strip: [Add Target][Reconnect][Show/Hide Matrix]
 <DeviceMatrix>
  head: title | [Add Device] [●stream pill] [lease chips] [Reconnect Streams]
  grid: | Device | Mode | State | Health▮ | Latency | Failures | Docker | Test | Actions |
  right: focused terminal (lines, Detach) + alerts list
```

### A8. Mobile PWA (/m)
```
[unpaired] MobilePairing: [server url][code][name] [Pair]
[paired]
 ┌ KRIA          <server url>            ┐
 │ <tab body: Chat | Desktop | Settings>│
 │  Chat: log + [approval card] + input │
 │  Desktop: banner + RdToolbar + video │
 │          + RdKeyboardBar(cond)       │
 └ [Chat] [Desktop] [Settings] (bottom) ┘
```

### A9. VoiceOverlay
```
.voice-overlay (full-screen, state class)
  .voice-overlay__card  [×][⚙]
   [waveform | mic glyph | speaker glyph]
   <state label> · PTT(cond)
   [mic meter▮] <transcript> <partial(dim)>
   <lang·conf%> <io mode·TTFA·playback> <STT ● engine · TTS ● engine> <turns ok · e2e p50>
```

Wireframes for remaining screens (TasksView, TestRunnerDashboard, AnalyticsDashboard, Providers, n8n hub, Setup Wizard) follow the element inventories in Part I §E and Part II §E; structure = header row → config/filter row → list/grid/table → footer/actions.

---

## SECTION B — PIXEL-LEVEL UI INVENTORY (per element: behavior/command/conditions/a11y)

> Full element→command mapping for all screens is in Part II §E (elements) + §F (actions) + Part I §3/§8. This section adds the missing per-element attributes (visibility conditions, disabled/loading behavior, validation, a11y) for the highest-traffic screen (ChatView) as the canonical template; other screens share the same attribute schema.

### B1. ChatView elements (canonical detail)
| Element | Type | Purpose | Command/store | Visible when | Disabled when | Validation | Loading | a11y |
|---|---|---|---|---|---|---|---|---|
| Session title | text | show current session | currentSession | always | — | — | — | none |
| Export button ▾ | button+menu | export transcript | save_export_file / open_html_for_print | always | while exporting (items) | — | items show "Exporting…" | outside-click close |
| Message bubbles | list | conversation | messages() | always | — | — | thinking row | assistant sanitized HTML |
| Welcome card | card | empty state | messages().length===0 && !isThinking() | empty only | — | — | — | — |
| Thinking row | status | in-progress | isThinking() | thinking | — | — | animated dots | role=status aria-live=polite |
| Degradation pill | badge | critical degrade | degradationLevel()==="critical" | cond | — | — | — | — |
| GPU-swap alert | pill | swap notice | isSwapping()/swapBanner() | cond | — | — | pulse dots | — |
| Tool-mode select | select | manual routing | manualToolMode / setManualToolMode | always | — | — | — | none label (visual only) |
| Tool-choice modal | modal | low-confidence pick | toolChoiceRequest / submitToolChoice | when request | — | — | — | none |
| Textarea | input | compose | inputText/setInputText | always | — | — (Enter blocked while thinking) | — | placeholder varies |
| Slash menu | menu | commands | slashCommands | inputText startsWith "/" | — | — | — | arrow-nav; no role=menu |
| Attach button 📎 | button | files | processFile | always | — | file-type sniff | — | hidden file input |
| Voice button 🎤/🔊 | button | toggle voice | toggle_voice | always | — | — | state class | title |
| PTT button | button | push-to-talk | voice_ptt_press/release | voiceMode==="push_to_talk" | — | — | active class | pointer+key hold |
| Send/Stop | button | send/cancel | send_* / cancel_turn | always | Send disabled when empty | — | Stop while thinking | — |
| File chips | chips | pending files | pendingFiles | files present | — | — | — | remove × |
| Image preview | bar | pending image | pendingImage | image present | — | — | — | remove × |

### B2. Danger / developer / hidden actions (cross-screen)
- **Danger** (styled/confirmed): Hard Reset (type "HARD RESET"), Delete target (`confirm()`), Clear all sessions (two-click), n8n permanent delete (type `DELETE <name>`), workflow archive (`window.confirm`), memory Forget/Hard-delete/Restore (no confirm), skill Uninstall (no confirm), remote_desktop_kill (no confirm).
- **Developer-only** (gated by `developerMode()` localStorage or Developer/Ironclad layer): GUI Cognition dev accordion, startup warning banners (Colab/OCR), Ironclad config, GUI readiness-bypass, Forensics detail, Test Runner.
- **Hidden/hover**: session row action buttons (`opacity:0` → `1` on `:hover`/`.editing` — pin/archive/rename/delete only appear on hover), export menu (opacity/pointer-events toggle).
- **Keyboard-only triggers**: Ctrl+, Ctrl+N Ctrl+Shift+V Ctrl+K Esc; textarea Enter/Shift+Enter; slash menu Arrow/Tab/Enter/Esc; PTT Space/Enter hold.

### B3. Elements that should be noted as inert / unbound
- WorkflowProgress HITL/cancel/continuation buttons → workflowSession stubs (no backend) — Part I §10.2.
- ExportDropdown errors → console only.
- Fleet docker-eval / heartbeat → raw fetch, no auth header.

---

## SECTION C — VISUAL DESIGN TOKEN CATALOG (complete)

### C1. Color tokens (theme-shell.css — LIVE dark, `:root`)
Backgrounds: `--bg-primary #0f1417`, `--bg-secondary #172027`, `--bg-tertiary #24323c`, `--bg-hover #2f3f4b`.
Text: `--text-primary #f4f8fb`, `--text-secondary #b2c3cf`, `--text-muted #7b919f`, `--text-strong #e4f3fa`, `--heading-soft #d6ebf4`.
Brand/accent: `--accent #18a57a`, `--accent-hover #23bf8f`, `--primary-alt #1f8ec7`, `--primary-alt-hover #37a7df`, `--brand-soft #8fd8c0`, `--brand-soft-2 #aee9d6`.
Semantic: `--success #3bc975`, `--warning #f3b54a`, `--danger #f86d6d`, `--danger-text #ffd8d8`.
Surfaces (translucent): `--surface-1 rgba(255,255,255,.02)`, `--surface-2 rgba(255,255,255,.06)`, `--surface-3 rgba(9,15,20,.72)`.
Accent soft/border: `--accent-soft rgba(24,165,122,.2)`, `--accent-border rgba(24,165,122,.5)`, `--accent-border-strong .8`, `--danger-soft rgba(248,109,109,.2)`.
Border: `--border rgba(149,180,199,.2)`.
~40 more semantic gradient tokens (sidebar/header/modal/input/welcome/user-msg/statusbar/overlay), plus bubble tokens (`--user-bubble-bg` gradient, `--assistant-bubble-bg #1c2d38`, avatars, `--msg-copy-*`).

### C2. Color tokens (base.css `:root` — mostly OVERRIDDEN/dead)
`--bg-primary #0d1117`, `--accent #58a6ff`, `--success #3fb950`, `--warning #d29922`, `--danger #f85149`, `--radius 8px`, `--radius-sm 4px`. Live only where theme-shell doesn't redefine (rare).

### C3. Light theme (`[data-theme="light"]`)
Full parallel set: `--bg-primary #f4f8fb`, `--bg-secondary #ffffff`, `--accent #0f8f6b`, `--success #1e9a5a`, `--warning #a26b00`, `--danger #cf4a4a`, `--text-primary #182833`, etc. (Only reaches CSS-token components; inline-hex views stay dark — Part II §G7.)

### C4. Undefined (dangling) tokens — referenced, never defined
`--surface-elevated`, `--surface-muted`, `--surface`, `--accent-contrast`, `--shadow-lg`, `--warning-bg`, `--warning-text`. Used by DecisionActionCenter, HitlModal severity, degradation-pill.

### C5. Typography tokens
`--font-sans "Space Grotesk","IBM Plex Sans","Segoe UI",sans-serif` (dark) / base.css uses system stack. `--font-mono "JetBrains Mono","Cascadia Code",monospace`. **No @font-face / webfont bundling found** → Space Grotesk/IBM Plex/JetBrains fall back to system if not OS-installed (UNKNOWN whether bundled at build).
Font sizes observed (px, hardcoded): 10,11,12,13,14,16,18,20,24,28. Weights: 400,500,600,700,800. Letter-spacing: 0.3–2px.

### C6. Radius scale
`--radius 12px`, `--radius-sm 8px` (theme) / `8/4px` (base). Inline: 3,4,6,8,10,12,14,999px,50%.

### C7. Elevation / shadow
`--elev-shadow 0 12px 30px rgba(0,0,0,.2)` (dark) / `rgba(16,49,71,.12)` (light). Hardcoded: export-menu `0 8px 24px rgba(0,0,0,.4)`, gpu-alert `0 10px 24px rgba(0,0,0,.24)`, modal misc. `--shadow-lg` undefined.

### C8. Z-index layer map (from CSS)
`0` app grid · `1` app-layout/overlay · `2` statusbar · `3` (2×) · `5` · `8` (gpu-swap alert, 2×) · `15` · `20` · `45` decision-action-center · `70` · `80` · `100` modal-overlay · `200` (export menu; 2×) · `210` · `220` · `250` · `300` · `1000` (SkillMarketplace inline PermissionModal) · `9999` (global.css top). **No z-index scale tokens — all ad-hoc.**

### C9. Motion tokens
No duration/easing tokens. Durations hardcoded: 0.12s,0.15s(most common,12×),0.16s(6×),0.18s,0.2s,0.25s,0.3s,0.35s,0.6s,0.8s,0.9s,1.2s,1.4s,1.5s,1.6s,1.8s,2s + 80ms/140ms/160ms/180ms. Easing: mostly `ease`, `ease-in-out`, `ease-out`, `linear`.

### C10. Breakpoints (all `@media`)
`min-width:768px` (1); `max-width:` 420,520,640,760,820,900,960,1080,1100; `prefers-reduced-motion:reduce`. Total 11 distinct queries across files — no shared breakpoint tokens.

### C11. Icon system
Emoji (no icon font/library) + a handful inline SVG (mic/speaker/stop glyphs). Icon "sizes" = font-size on emoji (10–24px) / SVG 12–18px.

---

## SECTION D — LAYOUT MEASUREMENTS (source-quoted)

| Region | Measurement | Source |
|---|---|---|
| Sidebar width | 260px (base) → **300px** (theme) ; collapsed **64px** | base/theme-shell `.sidebar` |
| Sidebar logo | 32×32 (28×28 collapsed) | base |
| New-session/toggle btn | 34×34 (theme) / 32×32 (base) | theme-shell |
| Chat message max-width | **80%** of column | base `.message` |
| Chat messages padding | 20px 24px 12px | base |
| Chat input textarea max-height | 150px (JS auto-grow) | ChatView |
| Generic modal | width **500px**, max-width 90vw, max-height 80vh | base `.modal` |
| Modal overlay | fixed inset:0, bg rgba(0,0,0,.6), z100 | base |
| HitlModal width | min(680px, 94vw) | theme-shell |
| DecisionActionCenter | fixed right:18 bottom:18, panel min(440px, 100vw−24) | theme-shell |
| Modal header | padding 16px 20px, h2 16px, close 24px | base |
| Modal body/footer | body 20px; footer 12px 20px right-aligned | base |
| Card grid | `repeat(auto-fill, minmax(260px, 1fr))` | inline (Skill/marketplace) |
| Ironclad metric row | `repeat(3, minmax(0,1fr))` | theme-shell |
| Status/latency dots | 6–10px circle | base/theme |
| Health bars | 4–6px height | base/substrate |
| Touch targets (mobile) | ≥44px (min-width:44px 3×) | mobile.css |
| Pairing code | 28px, letter-spacing 2px | global.css |
| Container | app 100vh, main flex fill, min-height:0 (scroll) | base |

Breakpoint behavior (observed):
- **≤900px**: `.modern-topbar` stacks column, subtitle max-width 100%.
- **≤760px**: manual-tool-mode-bar stacks.
- Other breakpoints (420–1100) touch n8n/providers/devices/setup-wizard grids (collapse multi-col → single).
- **Tablet/laptop/ultra-wide/foldable**: no dedicated handling → desktop layout scales fluidly; sidebar fixed 300px regardless. High-DPI/zoom/large-fonts: rem/px mix, `ui.font_scale` sets `data-font-scale` (UNKNOWN exact scaling CSS — attribute set, mapping not located). Minimum width: **UNKNOWN** (no explicit min set on shell; horizontal overflow likely below ~700px on desktop views).
---

## SECTION E — INTERACTION STATE BIBLE

Per-state coverage of interactive primitives (source: CSS pseudo-classes + component signals).

### E1. Buttons (`.btn-*`, `.settings-btn`, inline)
- Default / **:hover** (bg/border/color shift, some `translateY(-1px)`) / **:disabled** (opacity 0.5 or grey bg, `cursor:not-allowed`) / **:focus** (mostly none; only `.attach-btn:focus-within` has outline). Pressed: no distinct `:active` style. Loading: label swap ("Saving…"/"Installing…"/"Synthesizing…"). No focus-visible ring on most buttons → **keyboard focus weakly visible**.

### E2. Session row (`.session-item`)
- Default (surface-1) / **:hover** (border+surface-2, actions fade in) / **.active** (accent-soft + accent border) / **.editing** (accent-border-strong, inline input). Actions `opacity:0→1` on hover/editing only (hidden affordance).

### E3. Inputs / textarea / select
- Default (field-bg + field-border) / **:focus** (accent-border-strong + `box-shadow 0 0 0 2px accent 25-30%`). No error state styling (validation is JS toast/banner, not field-level). Disabled: browser default. Readonly: callback-preview fields read-only (no distinct style).

### E4. Status dot (`.status-dot`)
- ready (success) / **.warming** (warning + `status-pulse` 1.2s) / **.degraded** (#f59e0b) / **.disconnected** (danger). Ironclad traffic dot: green/yellow/red/gray + glow ring.

### E5. Tool call (`.tool-call`)
- **running** (accent border + `tool-pulse` 1.5s) / **done** (#4caf50) / **error** (#f44336) / **denied** (#ff9800). Expand/collapse via `<details>` (chevron ▶/▼).

### E6. Voice overlay (state machine → CSS class `voice-state-*`)
- idle / wake_listening / listening / transcribing / thinking / processing(legacy) / speaking / interrupt / busy / error. Capturing states show waveform + mic meter; wake flash class `voice-wake-flash`. PTT active badge.

### E7. GUI Cognition lifecycle (12) — Part I/II
idle/observing/planning/resolving/safety/awaiting_approval/executing/verifying/blocked/completed/failed/cancelled → badge tone success/danger/warning/active/neutral.

### E8. Remote desktop session FSM (rdState)
idle/requesting*/awaiting_approval*/connecting/negotiating/establishing/connected/reconnecting/disconnected/error (*=dead branch). Each has label + suggested action (retry/cancel/reconnect/none). Reconnect backoff [500,1000,2000,4000,4000]ms, max 5.

### E9. Keyboard / focus behavior
- **Focus trap**: HitlModal (`dialogEl.focus()` + Escape=deny). Settings dialog `role=dialog aria-modal`. Other modals: overlay-click close, no explicit trap.
- **Escape**: closes shortcuts overlay; denies HITL; closes VoiceOnboarding; closes image preview. Not universal.
- **Enter/Space**: textarea Enter=send; PTT Space/Enter hold; slash menu Enter=execute. **Tab order**: DOM order (no `tabindex` management except 4 uses; `<video tabindex=0>`, dialog `tabIndex=-1`). Arrow keys: slash menu, memory graph none. Focus order otherwise = source order.
- **Drag**: chat file drop; mobile remote pinch/pan/drag; MemoryGraph node drag; session double-click rename.

### E10. States NOT implemented anywhere
- Global **offline** state: **UNKNOWN/none** (no navigator.onLine handling found in desktop; mobile relies on WS close).
- **Retry/timeout** surfaced only in: chat recovery-options, invokeWithTimeout (silent), remote-desktop reconnect, image lazy-load backoff. No global timeout UI.
- **Pressed/`:active`**, **selected** (except session/tab), **dragged** (except above) — largely unstyled.

---

## SECTION F — ACCESSIBILITY AUDIT (WCAG 2.2 AA, source-based)

> Static audit only. Full conformance requires assistive-tech testing = **UNKNOWN**. Counts from grep across `ui/src/**/*.tsx`.

### F1. ARIA usage (counts)
`aria-label` 41 · `aria-hidden` 12 · `aria-live` 7 · `aria-pressed` 6 · `aria-modal` 6 · `aria-labelledby` 5 · `aria-current` 2 · `aria-expanded` 1 · `aria-disabled` 1 · `aria-describedby` 1. `role`: status 11, menuitem 5, note 4, dialog 3, alertdialog 3, toolbar 1, menu 1, img 1, button 1, alert 1. `tabindex/tabIndex` 4. `alt=` 7.

### F2. Findings by criterion
| WCAG area | State | Evidence |
|---|---|---|
| 1.1.1 Non-text | Partial | 7 `alt=`; emoji icons lack labels; SVG `aria-hidden` |
| 1.4.3 Contrast | **At risk** | inline greys `#6b7280`/`#9ca3af` on dark = low contrast; not verified |
| 1.4.11 Non-text contrast | At risk | status-by-color-only in several inline views |
| 1.4.12 Text spacing / 1.4.4 Resize | Partial | `ui.font_scale` exists; px-heavy sizing may clip |
| 2.1.1 Keyboard | Partial | buttons reachable; slash menu custom; MemoryGraph drag has no keyboard path |
| 2.1.2 No trap | Partial | HitlModal traps + Escape; other modals no explicit trap |
| 2.4.3 Focus order | Partial | DOM order; minimal tabindex |
| 2.4.7 Focus visible | **Fail-ish** | most buttons have no `:focus`/`:focus-visible` ring |
| 2.4.11 Focus not obscured (2.2) | UNKNOWN | not verified |
| 2.5.8 Target size (2.2) | Good on mobile (≥44px) / mixed desktop | mobile.css min-width 44px; desktop 24–34px controls |
| 3.3.1 Error identification | Partial | errors as banners/toasts, not field-level; some silent (HitlModal, memory explain) |
| 4.1.2 Name/role/value | Partial | selects lack visible labels in ChatView tool-mode; many `<div>` buttons |
| 4.1.3 Status messages | Good | `role=status` ×11 + `aria-live=polite` ×7 (thinking, voice, resource) |
| 1.3.1 Info/relationships | Partial | headings h2–h4 used; landmark roles sparse (no `<nav>`/`<main>` semantics; divs) |
| 2.3.3 Reduced motion | **Good** | global `prefers-reduced-motion` override (base.css:3430) |
| High contrast | Partial | `ui.high_contrast` sets `data-high-contrast` (mapping CSS UNKNOWN) |

### F3. Structural gaps
- No semantic landmarks (`<nav> <main> <header> <aside>`) — layout is `<div class>`. Screen-reader region navigation limited.
- Heading hierarchy inconsistent (h1 only in assistant-header; many h2/h3/h4 without h1 on routed views).
- Tables: chat tool tables use real `<table>`; DeviceMatrix/Analytics use div-grids (not `<table>` → no row/col semantics).
- Forms: inputs frequently without associated `<label for>` (label is sibling text or visual).

---

## SECTION G — USER JOURNEY EXPANSION (decision points + backend calls + waits)

> Builds on Part II §A (11 journeys). Adds decision points, backend-call count, state transitions, wait points. Click counts = min happy-path (estimate, source-derived).

| Journey | Clicks | Decision points | Backend calls (approx) | Key waits | State transitions |
|---|---|---|---|---|---|
| First launch (A1) | 7–9 | backend local/external | start_provisioning, set_provisioning_backend, run_provisioning_step×3, complete_provisioning | model download (progress events), sidecar setup | provisioning FSM 6 steps |
| Daily chat (A3) | 1–2 | pick/new session | list_sessions(startup), send_message, stream | first-token latency | session bucket stream |
| Power/manual tool (A4) | 2–3 | tool mode (12) | send_manual_tool_message | tool exec | tool-choice/thinking |
| Developer (A5) | 3 | layer→toggle | (none for dev-mode; localStorage) | — | developerMode reveal |
| Researcher/memory (A6) | 2/topic | tab (13) | memory_* per tab lazy | graph load (2 calls) | per-tab resource run |
| Automation/n8n (A7) | 10+ | connect/author/run | ~10+ n8n_* | prepare input (LLM), run (callback) | pending→accepted→callback |
| Voice (A8) | 0–1 | mode | start_voice, voice events | TTFA | voice FSM |
| Mobile pair (A9) | desk 4–5 / phone 3–4 | — | mobile_gateway_*, pairDevice | gateway start | paired gate |
| Remote desktop (A10) | 2–3 | confirm HITL | requestSession, confirmSession, WebRTC | ICE connect | rdState FSM |
| Fleet enroll (A11) | ~6 | SSH fields | register_new_target, SSE | enroll bootstrap | target state |
| Model switch | 2–3 | provider/model | set_active_llm_selection | apply (llm-runtime:apply) | apply FSM idle→switching→ready |
| Capability install | 3–5 | approve scope | cpp_recommend, cpp_execute, cpp_approve | exec | approval modal |
| Task planning | 2–4 | — | task_add / plan_my_day | — | list refresh |
| Cold start (onboarding) | 5–8 | consent per source | memory_cold_start_* | import | wizard steps |
| Failure recovery | varies | recovery option | sendMessage(action) | retry | recovery-options panel |
| Session restore | 0 | — | rehydrateSessionsAfterReady (retry/backoff) | up to 12s deadline | hydration FSM |

Cognitive load ranking (highest→lowest): n8n authoring > Settings > Memory > Capabilities > Fleet > Chat/Voice/Tasks.

---

## SECTION H — FEATURE IMPORTANCE MAP (inferred, evidence-based)

> Refines Part I §5 / Part II §J. Signals: nav prominence, command count, polling freq, event traffic, coupling.

| Feature | Commands | Poll (s) | Events | Nav depth | Rank |
|---|---|---|---|---|---|
| Chat | ~10 | — | ~10 stream | 0 (default) | Critical |
| Sessions | 15 | — | — | 0 (sidebar) | Critical |
| Settings/config | 6 core + all | — | config-changed | 1 | Critical |
| Voice | 10 | — | 16 voice:* | 0 (shortcut) | Critical |
| Memory | 52 | live-debounce | memory://changed | 1 | High |
| Providers | 13 | — | llm-runtime:apply | 2 | High |
| CPP | 25 | 3 | cpp timeline | 1 | High |
| n8n | ~90 | 5 | 10 n8n:* | 3 (buried) | High (under-surfaced) |
| Fleet/Ironclad | 12 | 10 + 15 (heartbeat) | ironclad:*, fleet:* | 1 | High |
| GUI Cognition | 6 | — | ~50 event types | in-chat | High |
| HITL/Decisions | 14 | — | interaction_decision:* | overlay | High (safety) |
| Analytics | 1 | 10 | — | 2 | Medium |
| Tasks/Reminders | 12 | — | — | 1 | Medium |
| MCP | 6 | 4 | — | 2 | Medium |
| Google/Colab/Telegram | 12 | poll on connect | gw:* | 2 | Medium |
| Test Runner | 9 | — | kria://tests/* | 2 | Medium (dev) |
| Mobile/Remote | 10 | 4 | WebRTC | 2 | Low |
| Resource Authority | 1 | event | resource:hra_* | 2 | Low (shadow) |
| Export | 2 | — | — | in-toolbar | Low |
| Executive/Plan/Quarantine | 3+ | 5 | executive:*/intelligence:* | — | Orphaned UI (live store) |
| ICP views ×4 | 7 | — | openclaw:* | — | Dead |

---

## SECTION I — INFORMATION ARCHITECTURE (per-page ownership)

> Expands Part II §H. Per page: why it exists · data in · data out · owner store.

- **Home/Chat**: converse. In: user text/files, stream tokens. Out: send_message, feedback. Owner: appStore (session buckets).
- **PromptLab**: tool testing. In: locked tool config. Out: send_lab_message. Owner: appStore.
- **Dashboard**: runtime/fleet ops + n8n + tests + analytics. In: ironclad/analytics status. Out: reset commands. Owner: appStore (ironclad) + n8nStore + local.
- **VM Management**: fleet mgmt. In: SSE targets/heartbeat. Out: register/update/delete_target. Owner: useDeviceStatus hook + appStore.
- **Tasks**: productivity. In: task/reminder lists. Out: task_*/reminder_*. Owner: appStore.
- **Capabilities**: CPP catalog/exec. In: cpp_* status/catalog/timeline. Out: cpp_execute/approve. Owner: local component signals.
- **Memory**: knowledge mgmt. In: 46 memory_* reads. Out: writes/feedback/cognition. Owner: memoryStore.
- **Settings**: configure everything. In: get_settings/schema/history + all subsystem state. Out: patch_config/config_prompt + subsystem writes. Owner: appStore + child components self-managed.
- Cross-page comms: all via singleton stores (no context); events broadcast through appStore.initListeners; memory uses a browser CustomEvent bridge; n8n/guiCognition self-listen.

---

## SECTION J — COMPONENT COMMUNICATION MAP (diagram)

```
                         Tauri backend (kria-desktop commands + events)
                                   ▲ invoke        │ emit
                                   │               ▼
   ┌───────────────────────────────────────────────────────────────┐
   │ STORES (module singletons, no Context)                         │
   │  appStore ★god hub  ── initListeners(all Tauri events)         │
   │  memoryStore  n8nStore  provisioningStore                      │
   │  guiCognitionSession  workflowSession(stubs)  i18n  mobileStore │
   └───────────────────────────────────────────────────────────────┘
        ▲ import          ▲ import            ▲ import
        │                 │                   │
   App.tsx ──renders──► routed views/pages ──renders──► leaf components
        │                                              (props + callbacks)
        ├─ ChatView ──► MessageBubble ──► ToolCallBadge/GuiWorkflowViewer/MemoryFeedbackBar
        ├─ SettingsModal ──► ProviderSettings/N8nSettings/MobileRemotePanel/
        │                     ResourceDashboard/BriefingBuilder/OpenClaw*/SkillMarketplace
        ├─ MemoryWorkspace ──► MemoryGraph/MemoryOnboarding
        └─ overlays: HitlModal/DecisionActionCenter/VoiceOverlay/VoiceOnboarding

   Streaming: agent:*/prompt_lab:* → session buckets (sessionRuntime.ts)
   Polling: health12s ironclad10s executive5s mcp4s substrate3s cpp3s n8n5s mobile4s fleet15s
   Browser event bridge: "kria-memory-live" (memory) ; WebRTC/WS (mobile, non-Tauri)
```
- **Singletons**: appStore, memoryStore, n8nStore, provisioningStore, guiCognitionSession, workflowSession, i18n, mobileStore.
- **Shared/reusable leaf**: MessageBubble, ToolCallBadge, MemoryFeedbackBar, GuiWorkflowViewer, ExportDropdown, ImageProgressChip.
- **State ownership**: chat/session/voice/settings/mcp/ironclad/tasks/intelligence = appStore; memory = memoryStore; n8n = n8nStore; GUI cognition = guiCognitionSession; mobile = mobileStore + local.

---

## SECTION K — COMPLETE SCREEN DEPENDENCY GRAPH

```
SetupWizard ──(complete)──► App shell
App shell (nav) ──► {Home, Dashboard, VM, Tasks, Capabilities, Memory, Settings}
  Home ──env──► ChatView | PromptLabView
    ChatView ──emits/opens──► HitlModal(global), GuiCognitionPanel(inline), WorkflowProgress(inline),
                              ImageProgressChip(inline), tool-choice modal(inline), ExportDropdown
  Dashboard ──sub-tab──► Overview→AnalyticsDashboard(toggle) | Operations | n8n→N8nWorkflowHub | Forensics ; Tests→TestRunnerDashboard(toggle)
    N8nWorkflowHub ──► N8nSettings, N8nWorkflowManagementPanel, N8nWorkflowCard, WorkflowSuggestionCard, N8nRunTimeline/Progress/EvidenceViewer
  VM ──► DeviceMatrix ──► AddTargetModal, EditTargetModal
  Capabilities ──► Descriptor/Approval/Result modals (inline)
  Memory ──► MemoryGraph, MemoryOnboarding(cold-start)
  Settings ──tab──► ProviderSettings | N8nSettings | MobileRemotePanel | ResourceDashboard |
                    BriefingBuilder | OpenClawSettings+SubstrateStatus+SkillMarketplace(+inline PermissionModal)
GLOBAL overlays (any screen): HitlModal, DecisionActionCenter, VoiceOverlay, VoiceOnboarding, Shortcuts, Toasts
ISOLATED (no inbound edge): CapabilityGraphView, CapabilityManagerView, ExecutionLogsView,
  PermissionManagerView, QuarantineQueue, ExecutiveDashboard, PlanVisualization,
  N8nDiagnosticsPanel, N8nWorkflowBrowser, standalone PermissionModal  [all orphaned]
Mobile PWA (separate root): MobileApp ──► MobilePairing | MobileChat | RemoteDesktopView(→RdToolbar,RdKeyboardBar)
Shared components across screens: MessageBubble (ChatView+PromptLab), ResourceDashboard (Settings), etc.
Shared stores: appStore (nearly all), memoryStore (Memory+MessageBubble feedback), n8nStore (Dashboard n8n).
```

---

## SECTION L — MOTION SYSTEM AUDIT

### L1. Keyframe animations (16 total)
`fade-slide` (view enter 0.25–0.35s), `gpu-alert-slide-in` (0.18s), `mem-pulse` (2s, memory live dot), `pulse` (1.4s, thinking dots + gpu dots), `response-loading-border` (1.8s, response bubble border), `status-pulse` (1.2s, warming dot), `stop-btn-pulse` (2s, stop button), `thinking-bounce` (1.4s), `toast-in` (0.25s), `tool-pulse` (1.5s, running tool border), `voice-bar` (0.9s, waveform bars), `voice-btn-pulse` (1.5s), `voice-overlay-in` (180ms), `voice-ring` (1.6s), `voice-wake-pulse` (0.6s ×2), `wizard-spin` (0.8s linear, spinner).

### L2. Transitions
Predominant `all/background/color/border/transform 0.12–0.3s ease` on buttons, session rows, chips, bars. Bar fills `width 0.3–0.5s ease`. Hover lift `translateY(-1px)`.

### L3. JS-driven motion (not CSS)
- MemoryGraph force simulation (`requestAnimationFrame`, alpha cooling) — **not covered by reduced-motion**.
- Remote-desktop stats poll + view transform (CSS transform via JS).
- Chat auto-scroll (rAF-throttled).

### L4. Reduced motion
Global `@media (prefers-reduced-motion: reduce)` sets all `animation-duration/transition-duration: 0.01ms !important` (base.css:3430) → disables CSS motion app-wide including inline transitions. JS rAF loops (L3) unaffected.

### L5. No motion tokens
Durations/easings all hardcoded (see §C9). No shared motion scale, no orchestration/stagger system, no spring physics (except hand-rolled graph sim).

---

## SECTION M — RESPONSIVE AUDIT

| Target | Behavior | Evidence |
|---|---|---|
| Desktop (default) | primary; sidebar 300px fixed | base/theme |
| Laptop | scales fluidly; no dedicated bp | — |
| ≤1100/1080/960/920/900 | grid columns collapse (n8n/providers/devices/setup/topbar) | @media |
| ≤820/760 | tool-mode bar + some panels stack | @media |
| ≤640/520/420 | narrow collapses (setup-wizard, small panels) | @media |
| Tablet | no explicit handling → laptop layout | — |
| Ultra-wide | no max container width on most views → content stretches full width | — |
| Foldable | UNKNOWN (no handling) | — |
| Zoom / large fonts | `ui.font_scale` attr set; exact CSS mapping UNKNOWN | SettingsModal |
| High DPI | vector/emoji scale; raster (logo png) UNKNOWN crispness | — |
| Minimum width | none set on desktop shell → overflow likely <~700px | — |
| Mobile PWA (/m) | fully responsive, touch-first, ≥44px targets, orientation + visualViewport handling | mobile.css + RemoteDesktopView |

Overflow/scroll: `.chat-messages`, `.mem-content`, `.modal` scroll internally (min-height:0 pattern). Tables (`.tool-human-readable table`) scroll-x. Desktop routed views (CapabilitiesView etc.) not tested below ~700px → **likely broken/overflow** (estimate).

---

## SECTION N — FINAL REDESIGN-READINESS REPORT + GAP ANALYSIS + COUNTS

### N1. "Could a Principal Designer rebuild from this doc alone?" — self-challenge
- **YES for**: full screen/route/modal inventory, navigation graph, component/store/event architecture, command surface, design tokens (dark+light+dangling), layout measurements, motion inventory, a11y gaps, dead/orphaned map, user journeys, IA, feature ranking.
- **Residual gaps requiring the running app or backend (documented as UNKNOWN, not blockers for UX redesign)**:
  1. Real screenshots / exact rendered pixels (this doc uses structural wireframes).
  2. `ui.font_scale` / `data-high-contrast` exact CSS mappings (attrs set; rules not located → likely minimal/UNKNOWN).
  3. Whether Space Grotesk/IBM Plex/JetBrains fonts are bundled at build (no @font-face found).
  4. Actual color-contrast ratios (needs computed-style/testing).
  5. Exact minimum viable width / overflow behavior of desktop views (needs runtime).
  6. Backend payload schemas beyond the frontend's TS mirrors.
  7. Real performance/animation smoothness.
- These do not block IA/navigation/design-system/component-library planning. They matter for pixel-fidelity replication only.

### N2. DEFINITIVE COUNTS (answers to the enumerated questions)
| Question | Count | Note |
|---|---|---|
| Pages (desktop routes) | **7** | home, dashboard, vm-management, tasks, capabilities, memory, settings |
| Workspaces (multi-tab surfaces) | **6** | Settings(21), Memory(13), Capabilities(10), Dashboard(4), n8n hub(5), Analytics(6) |
| Routes (hash) | **7** desktop + **1** mobile (`/m`) | |
| Dialogs/modals | **~12** | Settings, HITL, Add/Edit target, Descriptor, Approval, Result toast, tool-choice, MemoryOnboarding, SetupWizard, inline PermissionModal, shortcuts |
| Overlays (global) | **~6** | HitlModal, DecisionActionCenter, VoiceOverlay, VoiceOnboarding, Shortcuts, Toasts |
| Inspectors | **~4** | Descriptor Viewer, MemoryGraph inspector, GuiCognition dev accordion, decision detail |
| Floating panels | **~3** | DecisionActionCenter (fixed), GPU-swap alert, toasts |
| Drawers | **0** | none (sidebar is fixed, not a drawer) |
| Popovers | **~2** | ExportDropdown menu, RdToolbar "more" popover |
| Tooltips | few (`title=` attrs) | no custom tooltip component |
| Context menus | **0** | no right-click menus |
| Command palettes | **0** | none |
| Dashboards | **~5** | Ironclad strip, Analytics, Executive(orphan), Resource, (n8n health) |
| Feature modules | **~30** | Part I §5 |
| Reusable components | **~40** | Part II §6 |
| Stores | **8** | app, memory, n8n, provisioning, guiCognitionSession, workflowSession, i18n, mobileStore |
| Contexts/providers | **0** | module singletons |
| Event systems | Tauri `listen` (~30 channels) + 1 browser CustomEvent bridge + WebRTC/WS(mobile) + SSE/WS(fleet) | |
| Commands (registered) | **~230** | Part I §8 |
| Commands with no UI caller | **~40** | Part I §8.1 |
| Integrations | **~12** | LLM(local/cloud), MCP, n8n, Google, Colab, Telegram, ComfyUI, OpenClaw/Docker, fleet/SSH, mobile gateway/WebRTC, memory/SQLite, sidecar |
| Backend dependencies | as above | |
| UI themes | **2** | dark (default) + light; +high-contrast attr |
| Animation systems | **1** (CSS keyframes ×16 + transitions) + JS rAF (graph/scroll/stats) | |
| CSS files | **11** (+inline) | 11,167 lines |
| Design tokens | ~120 defined (theme-shell) + ~24 base + light set; **7 dangling** | §C |
| Responsive layouts / breakpoints | **11** @media queries; 1 real desktop bp (900px) | |
| Mobile screens | **3** (+settings block) | pairing, chat, remote-desktop |
| Onboarding flows | **3** | SetupWizard, VoiceOnboarding, MemoryOnboarding(cold-start) |
| User journeys documented | **16+** | §G + Part II §A |
| Orphaned/dead components | **8** (+2 dead files) | Part I §10 |
| Keyframe animations | **16** | §L1 |
| z-index layers | **~19** distinct | §C8 |

### N3. Redesign-blocking gaps: NONE remaining for UX/IA/design-system work
All statically-derivable UI facts are now captured across Parts I–III. The only outstanding items (N1 residual) require the running application or backend source and are explicitly marked UNKNOWN. This document is sufficient as the Architecture-Freeze / Current-UI-Status Bible for the redesign phase.

---
*End of Part III. Document = Parts I (inventory) + II (deep UX/architecture) + III (status bible). Source-verified, read-only, no redesign.*
