# 🚀 KRIA MASTER ROADMAP (v2 — Engineering Edition)

## From AI Assistant → Productivity Platform → Desktop Intelligence System → Personal Operating Layer

> **What changed in v2:** This version converts the vision into an *engineering* roadmap.
> Every phase now has (a) concrete, shippable milestones, (b) a clear **recommended
> open-source / free tool stack** per integration (trusted sources only), (c) an honest
> **current completion %** based on a real audit of the codebase, and (d) safety, cost,
> latency and metrics treated as first-class concerns — not afterthoughts.

---

# 🌟 KRIA VISION

Most AI assistants help users perform individual tasks:

```text
User → Prompt → Response
```

KRIA's goal is to become an **intelligent operating layer** that understands user goals,
the active workspace, files, apps, meetings, emails, and workflows — and then executes
through the most appropriate system automatically.

```text
User Goal
   │
   ▼
KRIA Intelligence Layer  ──►  Execution Router
   │                              │
   ├── Productivity Systems       ├── GUI Cognition   (desktop apps)
   ├── Document Intelligence      ├── Browser Cognition (web)
   ├── Communication Hub          ├── MCP Integrations (APIs / SaaS)
   ├── Workflow Automation        ├── OpenClaw         (sandboxed code)
   └── Workspace Intelligence     └── Native Tools     (local OS)
```

```text
User stops managing tools.   User manages goals.   KRIA manages execution.
```

---

# � THE UNIQUE ANGLE (why KRIA is different)

Be honest about the market. Raycast AI, Microsoft Copilot, Rewind, Rabbit, and the
ChatGPT desktop app all chase pieces of this. **None** combine all five of KRIA's moats:

| Moat | KRIA | Typical competitor |
|------|------|--------------------|
| **Local-first & private** (data never leaves device) | ✅ core principle | ❌ cloud-bound |
| **One Execution Router** across GUI + browser + API + sandboxed code | ✅ unified | ❌ single channel |
| **Voice-native**, hands-free, wake-word | ✅ full pipeline | ⚠️ bolt-on |
| **GUI Cognition** — drives *any* desktop app, including legacy/closed software | ✅ vision-driven | ❌ API-only |
| **Open & extensible** — skills as sandboxed containers, MCP, skill-from-prompt | ✅ ClawHub + MCP | ⚠️ closed marketplace |

**Sharpen the positioning for each audience (single most important goal):**

- **Laymen** → *"Just tell it what you want."* Zero config. File cleanup, document
  toolkit, "prepare me for tomorrow", reminders, voice control. The assistant that
  does the boring computer chores.
- **Developers** → *"Your local pair that touches the whole machine."* Repo-aware,
  ephemeral dev envs, test→fix loops, PR review — all on-device, no code leaves.
- **Designers** → *"Describe it, iterate it."* Local image generation (ComfyUI),
  asset organization, batch export/convert, reference research.
- **Business / working professionals** → *"From inbox to outcome."* Morning briefing,
  unified tasks, document generation, meeting prep, repeatable workflow automation.

> **The wedge:** *A private, voice-native assistant that can actually operate your
> computer — not just chat about it — and that you can extend without trusting a cloud.*
> Double down on **Execution Router + local privacy + voice + GUI cognition**.
> Treat the dozens of read-only SaaS integrations as commodity (let MCP/n8n carry them).

---

# 📊 CURRENT STATE — HONEST AUDIT

Completion % is based on a code audit (functional vs partial vs stub). "Functional"
means wired end-to-end; "partial" means code exists but depends on missing pieces;
"stub" means scaffolding only.

| Subsystem | % | Status | What remains |
|-----------|----|--------|--------------|
| **Safety / HITL / audit / rollback** | **90%** | Functional | GREEN/YELLOW/RED/BLACK tiers, policy gate, blacklist all present |
| **MCP client** | **90%** | Functional | Solid client + server manager; depends on external MCP servers installed |
| **n8n integration** | **90%** | Functional | Heaviest-built; needs a running n8n instance + licensing review (see below) |
| **OpenClaw substrate** | **85%** | Functional | Pool, resolver, transpiler, audit ledger present; needs live ClawHub catalog |
| **Image generation (ComfyUI)** | **85%** | Functional | Orchestrator + cloud fallback; needs local ComfyUI install |
| **GUI Cognition v2** | **85%** | Functional | Loop engine, planner, OmniParser sight, uinput hands; needs model + daemon at runtime |
| **Chat infrastructure** | **80%** | Functional (desktop) | Multi-session + SQLite persistence + streaming; **server WS chat loop not wired** |
| **Observability** | **80%** | Functional | Tracing, traces, health, telemetry; no Prometheus/OTel export |
| **Fleet / remote execution** | **80%** | Functional | QEMU, signed leases, QoS, snapshots; needs more cloud backends |
| **Voice pipeline** | **75%** | Partial | Pipeline complete, but **STT/TTS run via CLI fallback** — native bindings + real AEC missing |
| **Browser cognition** | **75%** | Functional | Native CDP control; needs Chrome+CDP, broader browser support |
| **Memory / RAG** | **75%** | Partial | SQLite store + RAG; **embeddings fall back to hashing**, LLM fact-extraction is a placeholder |
| **Google Workspace** | **55%** | Partial | Tools registered & visible to LLM, but **calls require external `google-workspace-mcp`**; no in-repo backend |
| **Task engine / reminders** | **55%** | Partial | Reminders fire but are **in-memory** (lost on restart); scheduler is interval-only, not durable |
| **Telegram** | **75%** | Functional | Real polling bridge through the agent loop |
| **Secrets / credential vault** | **25%** | Stub | Only `.env` + 0600 token file; **no encrypted vault, no rotation** |
| **OAuth (Google/MS/GitHub)** | **20%** | Stub | No real OAuth flow; Google delegated to external MCP |
| **GitHub integration** | **5%** | Absent | Only routing keywords & eval fixtures; no API client |
| **Slack / Teams / WhatsApp** | **~5%** | Absent | Routing keywords + URL schemes only; reachable via browser fallback |

### ⚠️ The two structural gaps to fix first

1. **No real OAuth + no encrypted secrets vault.** Every first-party (non-MCP)
   Google / Microsoft / GitHub integration is blocked until this exists. This is
   *foundational* and currently the weakest link.
2. **The Python sidecar layer described in docs is largely absent on disk**
   (only `sidecars/kria-vision/` exists). Google Workspace backend and voice
   streaming lean on processes that aren't there — they depend on external MCP/CLI.
   Decide per-integration: **own it in-repo** or **formally depend on an external MCP**.

---

# 🧰 RECOMMENDED TECH STACK PER DOMAIN

All picks are **open-source / free** and from trusted sources (official vendor repos,
widely-adopted crates on crates.io/lib.rs, or high-adoption community projects). Where a
currently-used tool has a stronger alternative, both are listed with a recommendation.

### Authentication & Secrets (the foundation — build this first)

| Need | Recommended | Source / why | Notes |
|------|-------------|--------------|-------|
| OAuth2 flows (Google/MS/GitHub) | **`oauth2`** crate (ramosbugs/oauth2-rs) | Most-used, strongly-typed Rust OAuth2 client | PKCE + refresh token support built in |
| OS-native secret storage | **`keyring`** crate (keyring-rs) | Wraps macOS Keychain, Windows Credential Manager, libsecret | Use first; falls back to file vault if no keyring |
| Encrypted file vault (fallback) | **`aes-gcm`** + **`argon2`** (RustCrypto) | Audited RustCrypto primitives | AES-256-GCM + Argon2id; zeroize secrets in memory with `zeroize` |
| Token lifecycle | custom thin layer | — | Auto-refresh, rotation, expiry tracking |

> **Decision:** Build a `kria-core/src/auth/` module: `oauth2` for flows, `keyring` for
> storage (encrypted `aes-gcm` vault as portable fallback). This unblocks Gmail, Calendar,
> GitHub, Microsoft — *everything* downstream.

### Productivity Integrations (Gmail, Calendar, Drive, GitHub…)

| Integration | Fast path (MCP, now) | First-party path (later) | Trusted source |
|-------------|----------------------|--------------------------|----------------|
| Google Workspace | **`google-workspace-mcp`** / `orieg/gws-connector` / `MarkusPfundstein/mcp-gsuite` | Direct REST via `oauth2` + `reqwest` in a `gworkspace` module | GitHub (community, active) |
| Gmail (focused) | **`GongRzhe/Gmail-MCP-Server`** | same | GitHub (popular, auto-auth) |
| GitHub | **official GitHub MCP server** (github/github-mcp-server) | `octocrab` crate (first-party Rust GitHub client) | github.com (official) / crates.io |
| Microsoft 365 | Microsoft Graph MCP / Graph REST | `oauth2` + Graph REST | Microsoft official |

> **Strategy:** Ship via **MCP first** (already 90% wired) to get value fast, then
> selectively bring high-frequency paths (Gmail read, Calendar) **in-repo** with `octocrab`
> /direct REST for speed, offline resilience, and to remove the external-process dependency.

### Workflow Automation (re-evaluate n8n)

| Engine | License | Strength | Verdict for KRIA |
|--------|---------|----------|------------------|
| **n8n** (current) | fair-code (Sustainable Use) | Huge node catalog, mature | ⚠️ **Embedding/OEM into a product needs a commercial agreement** (reportedly costly) and requires visible n8n branding. Keep as an **optional external** engine the user runs — do not embed/rebrand. |
| **Activepieces** | MIT (community core) | Friendliest UX, closest n8n migration, REST/WebSocket triggers | ✅ **Recommended embeddable alternative** when KRIA needs to ship automation in-product |
| **Windmill** | AGPLv3 | Rust runtime, code-first, Postgres queues, very fast | ✅ Strong for **developer / code-first** workflows; AGPL → keep as separate service |
| Native KRIA scheduler | yours | Already have automation/workflow engine | ✅ For simple scheduled agent tasks, prefer native (no extra dependency) |

> **Decision:** (1) Keep n8n support but treat it as a **user-supplied external** engine
> (avoid the licensing trap). (2) Add **Activepieces** as the recommended *embeddable* option.
> (3) For simple "every Friday do X" automations, use the **native scheduler** (below) — no
> heavyweight engine needed.

### Durable Scheduling & Reminders (fix the in-memory gap)

| Need | Recommended | Source | Why |
|------|-------------|--------|-----|
| Desktop-grade durable task queue | **`taskmill`** | crates.io | SQLite-backed, **survives crashes**, priority + preemption, resource-aware — perfect for a desktop assistant |
| Cron expression parsing | **`cron`** / `croner` crate | crates.io | Standard cron syntax |
| Postgres durable step-functions (server/fleet) | **`underway`** (maxcountryman) | GitHub | If KRIA-server needs distributed durable jobs |

> **Decision:** Replace the in-memory reminder/scheduler with a **SQLite-backed durable
> queue** (reuse the existing `memory/store.rs` SQLite DB). Reminders and scheduled agent
> tasks must survive restarts. This single fix turns "reminders" from demo to product.

### Voice (remove the CLI fallback)

| Need | Recommended | Source | Notes |
|------|-------------|--------|-------|
| STT native bindings | **`whisper-rs`** (Strada-Technologies fork — active) or **`whispercpp`** crate | GitHub / lib.rs | Replace shell-out to whisper.cpp binary with in-process FFI; enables true streaming |
| TTS native bindings | **`piper-rs`** (already vendored) / `piper1-rs-sys` | vendored / lib.rs | Finish native path |
| Acoustic Echo Cancellation | **WebRTC APM** (`webrtc-audio-processing`) | open source | Replace the current AEC placeholder |
| VAD | Silero VAD / WebRTC VAD (already wired) | — | Keep |

### Memory / RAG (fix the hash fallback)

| Need | Recommended | Source | Notes |
|------|-------------|--------|-------|
| Local embeddings | **`fastembed`** (v4+, ONNX) | Anush008/fastembed-rs | Already chosen — **ensure the real ONNX model loads**; remove silent hash fallback |
| Vector index | existing in-repo / **HNSW** (`hnsw_rs`) | crates.io | Scale beyond linear scan as corpus grows |
| Code-aware RAG | **tree-sitter** chunking (you have it) + fastembed | — | Symbol-level chunks for the developer audience |
| Reranking | fastembed cross-encoder | fastembed-rs | Improves retrieval precision |

### Browser Cognition

| Need | Recommended | Source | Notes |
|------|-------------|--------|-------|
| Native control (current) | CDP via `reqwest` + `tokio-tungstenite` | in-repo | Keep — already 75% and fast |
| Cross-browser / fallback | **Playwright MCP** (Microsoft, official) | github.com (official) | Optional MCP for robustness across browsers |
| Autonomous web agent (research) | **browser-use** / **Stagehand** | open source (100k+ ⭐ combined) | Run **inside an OpenClaw container** for safety |

### GUI Cognition

| Need | Current | Notes |
|------|---------|-------|
| Screen parse | OmniParser (`sight_omniparser.rs`) | Keep; ensure model bundled/downloaded |
| Input | uinput daemon (`kria-uinput-daemon`) | Keep |
| Reasoning | local LLM brain (`llm_brain.rs`) | Keep — this is a genuine moat |

---

# 🏗 PHASED ROADMAP (with achievable steps)

Each phase lists **milestones** as small, shippable steps. Every milestone has a clear
**deliverable** (demo-able), the **tech** to use, and the **current %**. A phase is "done"
when all its milestones ship *and* pass their exit metric.

---

## 🌱 PHASE 0 — FOUNDATION (unblocks everything) — **~45% done**

**Mission:** A stable, secure platform every future capability depends on.

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 0.1 | **Encrypted secrets vault** | `keyring` + `aes-gcm` + `argon2` + `zeroize` | `kria-core/src/auth/vault.rs`: store/get/rotate any credential, encrypted at rest | 25% |
| 0.2 | **OAuth2 flow engine** | `oauth2` crate | Google + GitHub + Microsoft authorize → token → refresh, tokens land in vault | 20% |
| 0.3 | **Unified integration trait** | — | One `Integration` trait (connect / status / capabilities) for MCP, OAuth, n8n | 30% |
| 0.4 | **Server chat parity** | Axum WS + loop_engine | Wire the real agent loop into `kria-server/src/ws.rs` (today it only echoes a welcome) | 40% |
| 0.5 | **Observability export** | `tracing` + OTel exporter | Optional Prometheus/OTel export for metrics | 80% |

**Exit metric:** A user can connect Google + GitHub once; tokens persist encrypted and
auto-refresh; no plaintext secrets on disk.

---

## 🟢 PHASE 1 — DAILY INFORMATION ASSISTANT — **~80% done** ✅ core shipped

**Mission:** Be useful every single morning.

**Status:** Gmail/Calendar/GitHub tooling + Morning Briefing all implemented and unit-tested.
Live end-to-end is gated on user credentials (Google connect via existing MCP flow; GitHub
needs `GITHUB_PERSONAL_ACCESS_TOKEN` + Docker). Calendar availability engine has 7 tests;
briefing is config-driven via the Briefing Builder UI (Phase 1.5).

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 1.1 | **Gmail (read/search/summarize)** | `google-workspace-mcp` (`gw_gmail_*`) | `gw_gmail_inbox/search/read` functional; LLM summarizes. Write actions in Phase 1.5 | **90%** |
| 1.2 | **Calendar (today / free slots / conflicts)** | `gw_calendar_*` + `tools/availability.rs` | `gw_calendar_today/search` + `gw_calendar_availability` ("when am I free", conflicts) — pure engine, 7 tests | **90%** |
| 1.3 | **GitHub (PRs / issues / CI)** | official GitHub MCP (`config/mcp_servers.json`) | Tools auto-discovered (list PRs/issues/CI/notifications). Needs PAT+Docker; no first-party `octocrab` yet | **70%** |
| 1.4 | **Morning Briefing** | `gw_morning_briefing` (config-driven) + Briefing Builder UI | One result: Gmail + Calendar + conflicts + GitHub + Tasks; user-customisable sections | **75%** |

**Exit metric:** Morning Briefing renders across sources, customisable per-section. ✅
**Not yet met:** "< 3s parallel fetch + cached" (current fetch is sequential, no cache) and
**auto-run at a user-set time** (schedule is configurable in UI but background auto-delivery
not yet wired — see Phase 1.5 remaining).

**Remaining for 100%:** parallel+cached briefing fetch; auto-delivery scheduler wiring;
first-party Gmail/GitHub REST (offline/no-MCP); live e2e validation with real credentials.

---

## 🟦 PHASE 1.5 — PRODUCTIVITY WRITE-ACTIONS + PERSONALIZATION — **complete (backend + frontend)** ✅

**Mission:** Turn read-only Google into full write actions + a user-personalised briefing.

**Status (backend + Tauri + frontend, fully verified):**
- **Gmail write:** `gw_gmail_draft_create` (draft-only + formatted preview + draft_id, YELLOW),
  `gw_gmail_send_draft` (send by id, RED+HITL), `gw_gmail_send_bulk` (≤50 recipients, RED+HITL).
  `gw_gmail_send`/`gw_gmail_reply` already existed.
- **Calendar write:** `gw_calendar_update` (reschedule/edit, YELLOW+HITL) — joins existing create/delete.
- **Multi-account:** `gw_account_switch` + optional `account` param on new write tools.
- **Configurable briefing:** `crate::briefing` (`BriefingConfig` + `BriefingStore` in `kria.db`);
  `gw_morning_briefing` now **config-driven** (gmail query/max, calendar window, github tool,
  tasks filter, per-section enable). Tauri commands `get_briefing_config` / `set_briefing_config`.
- Tools mounted (ambient/admin) + policy tiers set.
- **Frontend:** `BriefingBuilder.tsx` (Settings → "Briefing" tab) for per-section/schedule editing;
  Gmail/GitHub connect UI already existed (Google tab).
- **Verification:** kria-core + kria-desktop compile; tests — briefing config 5, gmail preview 1,
  + 14 google_workspace (Rust) and UI `tsc` + vitest (160 existing + 4 new) + `vite build`, all green.

**Remaining:** unified multi-account fan-out (query 2 accounts at once); calendar update
delete+recreate fallback; scheduled auto-briefing *delivery* wiring; chat-header account picker;
live e2e with real Google/GitHub credentials.

---

## 🔵 PHASE 2 — WORK MANAGEMENT LAYER — **~95% done** ✅ core + intelligence upgrade

**Mission:** Understand the user's workload.

**Status:** Core engine shipped in `crates/kria-core/src/tasks/` (`store.rs`, `priority.rs`,
`scheduler.rs`) + tools in `tools/tasks.rs`. SQLite-backed (`kria.db`), 16 unit tests passing.
Tools: `task_add`, `task_list`, `task_update_status`, `task_next`, `task_stats`,
`reminder_set`, `reminder_list` (all GREEN, ambient). Durable reminder scheduler armed at
desktop startup (30s poll). *Implementation note: used `rusqlite` directly (mirrors
`MemoryStore`) instead of `taskmill` — no new dep, consistent pattern.*

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 2.1 | **Unified Task Engine** | `tasks/store.rs` (SQLite, `kria.db`) | Persistent task queue (manual now; Gmail/Calendar/GitHub source field ready) — CRUD, priority-ordered list, `task_next` | **90%** |
| 2.2 | **Priority Engine** | `tasks/priority.rs` (deterministic rules) | Urgent / Important / Blocked / Waiting / Normal + score | **100%** ✅ |
| 2.3 | **Durable reminders** | `tasks/scheduler.rs` (SQLite poll loop) | Reminders **survive restart**; overdue fire on boot — fixes in-memory loss | **100%** ✅ |
| 2.4 | **Productivity analytics** | `productivity_stats()` + `task_stats` tool | Open/in-progress/blocked/done, overdue, done-today, urgent/important counts | **85%** |

**Exit metric:** "Today's priorities" reflects live state across sources; reminders fire
after an app restart. ✅ (reminders restart-durable; task queue live & priority-ordered)

**Remaining for 100%:** auto-import adapters (Gmail/Calendar/GitHub → tasks, composing Phase 1
tools) and wiring `task_stats` into the analytics dashboard. (Frontend **Tasks view** —
board + stats + durable reminders ✅ shipped, with `task_*`/`reminder_*` Tauri commands.)

### Intelligence upgrade (shipped — discussed in chat)
Took Phase 2 from "task DB" toward "true assistant" — backend + frontend, fully verified:
- **Natural-language time** (`tasks/nl_time.rs`, `interim` crate) → reminders/tasks accept
  "tomorrow 5pm" / ISO; Hinglish via LLM path.
- **Recurring reminders** (`tasks/recurrence.rs`) → daily / weekly@day / monthly@n / every-Nm;
  scheduler reschedules next occurrence on fire.
- **Edit / snooze / cancel** → `update_task`, `snooze_reminder`, `cancel_reminder` + tools/commands/UI.
- **Daily planning** (`tasks/planner.rs`) → greedy fit active tasks into free slots (reuses
  Phase 1.2 availability); `plan_my_day` tool/command + "Plan my day" UI panel.
- **Natural completion** (`tasks/matching.rs`) → "report ho gaya" fuzzy-matches + marks done.
- Tests: 35 kria-core tasks tests; UI `tsc` + vitest (164) + `vite build` — all green.

**Still deferred (needs live LLM/desktop — documented):** LLM **auto-capture** pipeline
(email/chat → action-item → task), **proactive auto-delivery** (morning push/TTS), **actionable
notification** buttons, waiting-on tracking, full RFC-5545 (`rrule`), `task_stats` → dashboard.

---

## 🟡 PHASE 3 — DOCUMENT INTELLIGENCE — **~40% done**

**Mission:** Make KRIA the document assistant (strong fit for **designers + business**).

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 3.1 | **Docs / Sheets / Drive read+write** | google-workspace-mcp → REST | Summarize, create, update, organize | 55% |
| 3.2 | **Local doc toolkit (OpenClaw skill)** | sandboxed container: PDF merge/split/sign/redact, OCR | Works fully offline, no cloud | 40% |
| 3.3 | **Spreadsheet automation skill** | pandas in OpenClaw | Clean, pivot, chart, summarize a CSV/XLSX | 30% |
| 3.4 | **Unified knowledge search** | fastembed + tree-sitter + RAG | One search across Docs/Drive/emails/local files | 60% |

**Exit metric:** "Summarize this folder of PDFs" runs locally via an OpenClaw skill with
no external API.

---

## 🟠 PHASE 4 — COMMUNICATION HUB — **~20% done**

**Mission:** Centralize communication (read first, write behind HITL).

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 4.1 | **Telegram** (done) | in-repo bridge | Read/send/notify | 75% |
| 4.2 | **Slack** | Slack MCP / official SDK | Read channels, summarize, send (HITL) | 5% |
| 4.3 | **Teams** | Microsoft Graph MCP | Messages, meetings | 5% |
| 4.4 | **WhatsApp** | WhatsApp Business API (official) | Messaging/automation (defer — heavy) | 5% |

> **Scope advice:** Ship Slack only in early scope. **Defer Teams + WhatsApp** —
> they are distractions until the router + foundation are solid.

**Exit metric:** "Summarize my Slack #eng today" works; all *send* actions pass HITL.

---

## � PHASE 4.5 — MOBILE PROMPT-CONTROL (command KRIA from your phone) — **~40% done**

**Mission:** Send a prompt from your phone and have KRIA execute it on the laptop where it
runs — using the *same* agent loop, memory, tools, and safety as the desktop. Implemented
via **two independent approaches** so the feature never depends on a single channel.

### Tech choice & justification

| Layer | Chosen tech | Source / License | Why this (not the others) |
|-------|-------------|------------------|---------------------------|
| **Approach A — Messaging** | **Telegram Bot** (existing in-repo bridge) | reqwest + Bot API | Already ~75% built; NAT/firewall bypass is free (it polls outbound), encrypted transport, works anywhere with zero infra. Fastest ROI. |
| **Approach B — Direct** | **KRIA PWA (SolidJS) → `kria-server` WebSocket** | your existing UI + Axum stack | No third-party broker, no message limits, full rich UI (streaming, files, voice). Reuses the *exact* frontend + server you already have. This is the channel-independent path that removes the Telegram dependency. |
| **Transport for B** | **Tailscale** (or self-host **Headscale**) | BSD-3 | WireGuard mesh: laptop is never exposed to the public internet; phone + laptop share a private tailnet. NAT traversal automatic. Chosen over a reverse tunnel (rathole/frp) because it needs no public port. |
| **Device auth** | **`kria-connection-control` signed leases** + short-lived tokens | in-repo | Reuse the lease/signing model you already built for fleet; per-device keys, instant revoke. Far stronger than the single localhost bearer token. |
| **Push (laptop → phone)** | **ntfy** (self-hosted) | Apache-2.0/GPL | HTTP pub-sub, official mobile apps, UnifiedPush distributor. Trivial to fire "task done / approval needed" alerts. |

> **Why two approaches:** Telegram = instant value, works on any network with no setup, but
> routes through Telegram's servers (fine for commands, not for secrets). The PWA+Tailscale
> path is fully private and rich, but needs the mesh set up once. Shipping both means a user
> always has a working channel, and sensitive work can stay on the private path.

### Milestones

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 4.5.1 | **Telegram approach hardening** | existing bridge + HITL | Per-chat allow-list, HITL approval inline, audit per message | 85% |
| 4.5.2 | **Server WS agent loop** | Axum WS + `loop_engine` (Phase 0.4) | `kria-server/src/ws.rs` runs the real agent loop, not just a welcome echo | 100% |
| 4.5.3 | **KRIA PWA shell** | SolidJS + service worker | Installable phone PWA: chat, streaming, voice note input, file push | 70% |
| 4.5.4 | **Tailscale transport + device pairing** | Tailscale/Headscale + signed leases | Phone reaches laptop over private mesh; QR-code device pairing; revoke list | 60% |
| 4.5.5 | **ntfy push integration** | ntfy HTTP POST | Long-task completion + HITL-needed alerts land on the phone | 80% |
| 4.5.6 | **Unified session continuity** | existing SQLite sessions | Phone and desktop share the *same* session history (resume either side) | 80% |
| 4.5.7 | **AI-grade PWA UI/UX** | SolidJS + Tailwind redesign | Mobile web feels like Gemini / ChatGPT apps — clean chat, streaming bubbles, input bar, attachments, mobile-first ergonomics | 30% |
| 4.5.8 | **Markdown-rendered responses** | `marked` + `dompurify` + highlight.js (reuse desktop) | Responses render formatted markdown (code blocks, lists, tables, links) in the mobile web app, matching desktop fidelity | 20% |
| 4.5.9 | **Multimedia upload & sharing** | multipart upload → sidecar processors | Upload image / audio / video / documents (PDF, DOCX, etc.) from the PWA into a chat; forwarded to the same processors the desktop uses | 15% |
| 4.5.10 | **Generated-image display in PWA** | image events surfaced to web client | Prompt-generated images (ComfyUI/cloud) render **in the mobile web app**, not only on the desktop — same image-progress + final-image flow | 10% |
| 4.5.11 | **Tool selection in PWA** | tool-choice control (reuse desktop store) | User can pick/force a tool from the mobile web app, like the desktop tool picker | 15% |
| 4.5.12 | **KRIA Settings in PWA** | settings surface over the gateway API | Core settings (model/quality, voice, briefing, account) reachable from the mobile web app | 10% |
| 4.5.13 | **Session & chat list parity** | session list/grouping (reuse desktop) | Sessions and chat history displayed in the PWA exactly as the desktop app shows them (grouped, resumable, searchable) | 20% |
| 4.5.14 | **Tailscale alternative (no third-party SSO)** | Headscale / NetBird / Nebula / plain WireGuard | Private mesh that does **not** require a Google/Microsoft sign-in on both devices; self-hosted or pre-shared-key based | 0% |
| 4.5.15 | **QR / barcode zero-type pairing** | QR scan in the PWA + desktop-generated code | Pair a device by **scanning a QR/barcode** — no manual key/token entry on mobile or desktop | 40% |
| 4.5.16 | **Prompt-driven binding & settings** | agent tools over the gateway | Pair/unpair devices and change mobile **and** desktop settings entirely from a natural-language prompt ("pair my phone", "set quality to balanced") | 5% |

**Exit metric:** From mobile data (off home Wi-Fi), a user sends a prompt over **both**
Telegram and the PWA path; KRIA executes on the laptop, streams the result back, and any
write/destructive step requires HITL approval shown on the phone. Losing one channel does
not break the feature.

### Pending PWA enhancements (mobile web app — bring it to desktop parity)

The mobile web app currently handles prompt → streamed text, but lags the desktop on
richness. To make the PWA a first-class client (the "Approach B" rich path), the following
are **pending**:

- **AI-app-grade UI/UX** — redesign the mobile chat to feel like Gemini / ChatGPT: clean
  message bubbles, streaming, a proper input/attachment bar, and mobile-first ergonomics.
- **Markdown-formatted responses** — the desktop renders full markdown (code blocks, tables,
  lists, links); the mobile web app must load the same formatted rendering, not raw text.
- **Multimedia sharing (upload)** — let the user upload images, audio, video, and document
  formats (PDF/DOCX/etc.) from the PWA into a chat, routed to the same Python sidecar
  processors the desktop uses (audio/document/image/web).
- **Generated-image display in the PWA** — images generated from a prompt (ComfyUI / cloud
  fallback) currently appear **only in the KRIA desktop app**; they must also stream to and
  render in the mobile web app (image-progress + final image), like the desktop.
- **Tool selection** — expose the desktop's tool-choice control in the PWA so the user can
  pick or force a specific tool from mobile.
- **KRIA Settings** — surface core settings (model/quality, voice, briefing, account) in the
  mobile web app over the gateway API.
- **Session & chat parity** — show sessions and chat history in the PWA exactly as the
  desktop does (grouped, resumable, searchable), backed by the shared SQLite sessions.

> These reuse existing desktop frontend modules (markdown renderer, tool-choice store,
> session grouping, image-progress events) — the work is wiring them through the mobile
> gateway, not rebuilding them.

### Setup, transport & pairing — pending research/enhancements

The current path works but has friction points to remove:

- **Better transport than Tailscale (drop the third-party SSO).** Tailscale requires a
  Google/Microsoft sign-in on **both** devices, which is heavy for a local-first product.
  Research a private-mesh alternative that needs no third-party identity:
  - **Headscale** — self-hosted Tailscale control server; use pre-auth keys (no Google login).
  - **NetBird** — open-source overlay (WireGuard) with its own/self-hostable IdP and setup keys.
  - **Nebula** (Slack) — certificate-based overlay mesh, no SSO at all.
  - **Plain WireGuard** — manual key exchange, simplest, no broker (pairs well with QR setup).
  - Pick based on: zero third-party login, NAT traversal, easy QR/key bootstrap, license.
- **QR / barcode setup (no manual key entry).** The desktop shows a QR encoding
  `{server URL + device token + mesh key}`; the PWA scans it (camera) and is paired in one
  step — no copy-pasting tokens or IPs on either side. (Extends 4.5.4 / 4.5.15.)
- **Prompt-driven binding & configuration.** Bind the KRIA mobile app and KRIA desktop app
  through the **agent prompt**: "pair my phone", "show the pairing code", "revoke my tablet",
  "turn on remote desktop", "set stream quality to balanced" — all device pairing and
  mobile/desktop settings reachable from natural language via agent tools, not buried in menus.

> Goal: a user sets up phone↔laptop by scanning one code, never signs into a third party, and
> can manage everything by talking to KRIA.

### Security gates (read in full)

Remote prompt-control turns the laptop into something operable from elsewhere, so the blast
radius is the whole machine. Apply these rules: keep the PWA path behind the WireGuard mesh
and never expose `kria-server` to the public internet; use per-device signed leases with
short-lived tokens and instant revocation rather than one shared token; default every remote
write/delete/install/send action to a stricter risk tier that requires explicit HITL
confirmation on the phone with a plain description of what will run; log every remote command
to the audit ledger with the originating device identity; never route secrets or sensitive
file contents through Telegram or the public ntfy instance; and wire `global_halt` so the
phone can instantly stop all activity.

**Depends on:** Phase 0.1 (vault), 0.2 (OAuth), 0.4 (server WS chat loop).

---

## 🖥️ PHASE 4.6 — REMOTE DESKTOP VIEW & TAKEOVER (see + control the live screen) — **~75% done**

**Mission:** From the phone, view the laptop's **live desktop** and control it with
touch/keyboard — Chrome Remote Desktop style — of the *same* running session KRIA operates
in. Complements 4.5: command remotely, and take over the screen directly when needed.

> **⚠️ Architecture history (read this):** The original plan (x11vnc + noVNC, RustDesk
> fallback) and a second iteration (RDP via gnome-remote-desktop + IronRDP-web /
> Guacamole) were both **abandoned after live forensic testing**. guacd/FreeRDP could not
> decode gnome-remote-desktop's NVIDIA NVENC H.264 AVC444 (black screen); IronRDP-web never
> advertised the EGFX early-capability flag grd mandates, so capability exchange failed; and
> x11vnc/noVNC cannot capture or inject input on a GNOME **Wayland** session. The shipped
> architecture below (**WebRTC + PipeWire + xdg-desktop-portal**, Chrome-Remote-Desktop
> style) bypasses RDP/grd/VNC entirely and is DE/GPU-neutral. Full decision log in
> `planning_docs/phase4_6_remote_desktop_v2_plan.md`.

### Tech choice & justification (CURRENT — shipped)

| Layer | Chosen tech | Source / License | Why this (not the others) |
|-------|-------------|------------------|---------------------------|
| **Screen capture (same session)** | **xdg-desktop-portal ScreenCast + PipeWire** | freedesktop (LGPL/MIT) | Captures the **current logged-in session via PipeWire** on **both X11 and Wayland** — the one path that satisfies "same session" + "both display servers" without a new virtual session. No grd, no NVENC/EGFX traps. |
| **Transport / codec** | **WebRTC (GStreamer `webrtcbin`, DTLS-SRTP)** | GStreamer (LGPL) | We own the codec (software **VP8** default; VP9/H264 selectable), so no dependence on the compositor's hardware encoder. Browser-native playback, low-latency, trickle ICE. |
| **Signaling** | existing KRIA token-gated **WebSocket** (`/rd-signal`) | in-repo (Axum) | Reuses the 4.5 WS infra; carries SDP/ICE **and** input JSON. Server is the **offerer** (sendonly video); the PWA answers. |
| **Input injection** | **xdg-desktop-portal RemoteDesktop** (libei) | freedesktop | Works on Wayland (where uinput/VNC input is blocked); evdev keycodes + absolute pointer + wheel. Combined ScreenCast+RemoteDesktop portal session. |
| **Client (in-browser)** | **`RTCPeerConnection` + `<video>`** in the SolidJS PWA | in-repo | No WASM RDP/VNC client, no separate app; the desktop renders to a `<video>` in the same KRIA PWA. Touch/keyboard mapped by `rdpInput`. |
| **Transport security** | **Tailscale / Headscale** (WireGuard) | BSD-3 | Stream stays inside the private mesh, bound to the tailnet only — never a public port. |

> **Why WebRTC + PipeWire + portal:** it is the only stack that captures the *same live
> GNOME Wayland session* AND injects input there, while letting us choose a codec the browser
> can always decode. RDP (grd) is EGFX/AVC444-mandatory and rejected every in-browser client
> we tried; VNC can't touch GNOME Wayland. RustDesk/Sunshine remain out of scope (separate
> apps, can't embed in the PWA). Same-session capture + DE/GPU neutrality + in-PWA rendering
> is the deciding combination.

### Milestones

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 4.6.1 | **Portal ScreenCast + PipeWire capture** | ashpd portal session + PipeWire fd | KRIA acquires the live session's screen node on confirm; no grd; spike validated 1920×1200 @~118fps | 100% |
| 4.6.2 | **WebRTC stream in the PWA** | `webrtcbin` (offerer) → `RTCPeerConnection` `<video>` | Phone renders the live desktop in-app over `/rd-signal`; SDP/ICE validated server-side | 90% |
| 4.6.3 | **Touch/keyboard control** | portal RemoteDesktop (libei) + `rdpInput` | Tap/scroll/type/gestures from phone drive the desktop (evdev + XKB) | 90% |
| 4.6.4 | **Session gating + lifecycle** | HITL + signed leases | Start is a high-risk HITL action; idle auto-expire; reconnect/resume; single-session; reconcile | 90% |
| 4.6.5 | **Kill switch + audit** | `global_halt` + audit ledger | One control tears down any session; connect/disconnect logged per device | 90% |
| 4.6.6 | **Production UX polish** | view transform, gestures, toolbar, reconnect, a11y | Pinch/double-tap zoom + pan, direct/trackpad modes, F-keys, granular states, auto-reconnect, fullscreen, 44px/ARIA (`.kiro/specs/remote-desktop-ux-polish`) | 85% |
| 4.6.7 | **Streaming-quality enhancement** *(pending)* | adaptive bitrate/resolution/FPS, HW encode | Quality **selector** + `getStats()` health shipped; **adaptive** ABR + hardware-accelerated encode (NVENC/VA-API) for higher fidelity/lower latency still pending | 25% |
| 4.6.8 | **Skip repeated screen-share consent** *(research)* | portal `restore_token` / split capture + input | Today GNOME pops the screen-share consent dialog **every session** (combined ScreenCast+RemoteDesktop input sessions can't persist — `PersistMode` rejected). Research a persistable **ScreenCast** session (`restore_token`) plus a separate input path (libei/uinput), or a privileged helper, so the user grants once | 5% |

**Exit metric:** A paired phone, over the private mesh, opens the KRIA PWA, switches to the
Desktop tab, sees the live laptop screen (portal+PipeWire+WebRTC), and controls it by touch —
with the session start gated by HITL, a visible on-screen indicator, and a working kill
switch. **Remaining:** real-phone E2E media validation over Tailscale (DTLS/ICE) and the
streaming-quality enhancement (4.6.7).

### Security gates (read in full)

Live screen sharing with input control is the single highest-risk capability in KRIA,
because anyone holding the session has full interactive control and can click past every
safety tier. Therefore: never expose the signaling or media path to the public internet —
keep it inside the WireGuard mesh, bound to the tailnet interface only, never `0.0.0.0`; the
PipeWire capture and portal RemoteDesktop grant are acquired only on HITL confirm and torn
down on stop/idle/halt; treat *starting* a remote-desktop session as a high-risk action that
requires HITL approval and device pairing, and show a clear on-screen indicator on the laptop
whenever a remote view is active (the GNOME screen-share indicator is always visible); log
every connect/disconnect with device identity and auto-expire idle sessions with
re-authentication to resume; keep a single kill switch (wired to `global_halt`) that instantly
tears down any session; and keep clipboard and file transfer disabled by default since they
can silently leak secrets between devices.

**Depends on:** Phase 4.5 (PWA shell + Tailscale transport + device pairing), Phase 0 auth.

---

## �🔵 PHASE 5 — WORKFLOW AUTOMATION — **~50% done**

**Mission:** Reduce repetitive work.

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 5.1 | **Native scheduled automations** | durable scheduler + agent loop | "Every Friday send the report" without any external engine | 50% |
| 5.2 | **n8n as external engine** | existing n8n integration | Execute/monitor user-hosted n8n workflows (no embedding — licensing) | 90% |
| 5.3 | **Activepieces (embeddable option)** | Activepieces (MIT) | In-product workflow builder when needed | 0% |
| 5.4 | **Workflow suggestions** | pattern detector | "You do this weekly — automate it?" | 10% |
| 5.5 | **Workflow dashboard** | existing analytics | Success/failure/runtime/logs | 40% |

**Exit metric:** A recurring task runs unattended on schedule and reports success/failure.

---

## 🟣 PHASE 6 — INTELLIGENT WORK ASSISTANT — **~20% done**

**Mission:** KRIA starts helping *proactively* (highest-trust phase — gate carefully).

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 6.1 | **Email intelligence** | local LLM + embeddings | Importance detection, auto-priority, follow-up suggestions | 20% |
| 6.2 | **Schedule intelligence** | calendar + rules | Meeting optimization, conflict resolution | 15% |
| 6.3 | **"What next?" recommender** | task engine + priority engine | Best next action from emails/meetings/deadlines | 15% |
| 6.4 | **Pattern learning** | on-device stats | Working hours, productivity, comms habits — **stored locally only** | 10% |

> **Safety note (write in full prose, no shorthand):** Proactive actions widen the blast
> radius. Any action that *sends, deletes, schedules, or modifies* external state must be
> classified at least YELLOW and routed through the existing HITL approval flow. Proactive
> *suggestions* are fine to surface automatically; proactive *execution* must remain
> opt-in per action class until trust is established. Define an explicit allow-list of
> auto-executable action classes and keep everything else gated.

**Exit metric:** KRIA suggests the next action with a visible rationale; nothing external
is mutated without HITL.

---

## 🔴 PHASE 7 — WORKSPACE INTELLIGENCE — **~30% done**

**Mission:** Understand the user's desktop (this is the hardest infra — schedule realistically).

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 7.1 | **Desktop digital twin** | `sysinfo`, D-Bus (`zbus`), window/app perception | Live model of apps/tabs/projects/files | 25% |
| 7.2 | **Workspace memory / restore** | durable SQLite state | "Continue from yesterday" reopens VS Code + tabs + terminals + context | 20% |
| 7.3 | **Continuous observer** | existing telemetry + `nvml` | Detect CPU spikes, mem leaks, build/service failures with root-cause hints | 40% |

**Exit metric:** "Continue from yesterday" restores a real working session; observer flags
a runaway process with a probable source.

---

## 🔥 PHASE 8 — HYBRID AUTOMATION LAYER (the real moat) — **~35% done**

**Mission:** Combine every execution system under one **Intelligent Execution Router**.

> This is **not one box** — it is a planning/orchestration subsystem and the single most
> valuable thing KRIA can build. Treat it as a first-class project, not a footnote.

| # | Milestone | Tech | Deliverable | Now |
|---|-----------|------|-------------|-----|
| 8.1 | **Capability registry** | existing tool registry + MCP discovery | Every executor declares what it can do + cost/risk | 50% |
| 8.2 | **Router v0 (rules)** | deterministic routing by domain | Picks GUI vs browser vs MCP vs OpenClaw vs n8n for a task | 35% |
| 8.3 | **Router v1 (LLM planner)** | local LLM + the planner you have in gui_cognition_v2 | Multi-step plan across executors with fallbacks | 25% |
| 8.4 | **Cross-executor pipelines** | workflow engine | `Drive → OpenClaw summary → Gmail draft → n8n tracking` as one goal | 20% |
| 8.5 | **Cost & latency budgeting** | router policy | Prefer local/cheap; escalate to cloud only when needed | 10% |

**Exit metric:** A single natural-language goal ("send the weekly report") executes across
≥2 executors automatically, with HITL on the send step, under a stated cost budget.

---

## 🚀 PHASE 9 — PERSONAL OPERATING SYSTEM — **~10% done**

**Mission:** Become the operating layer over existing software.

| # | Milestone | Deliverable |
|---|-----------|-------------|
| 9.1 | **Goal compiler** | "Prepare me for tomorrow's client meeting" → reads emails, checks calendar, finds docs, builds agenda, sets reminders, drafts summary |
| 9.2 | **"What should I do now?"** | Analyzes emails/meetings/deadlines/tasks → ranked next actions |
| 9.3 | **Skill-from-prompt** | User describes a task → KRIA generates + registers a new OpenClaw skill on the fly (killer differentiator, no marketplace needed) |

**Exit metric:** A multi-system goal completes end-to-end with the user only approving
gated steps.

---

# 🏆 PRIORITY IMPLEMENTATION ORDER (revised)

Ordered by **value ÷ effort**, and by what *unblocks* the most downstream work.

### NOW (foundation + first visible win)
```text
0.1 Secrets Vault  ─┐
0.2 OAuth Engine   ─┤── unblocks ALL first-party integrations
0.3 Integration trait
        ↓
1.1 Gmail  →  1.2 Calendar  →  1.3 GitHub
        ↓
1.4 Morning Briefing   (the daily "wow", parallel + cached)
        ↓
2.3 Durable Reminders  +  2.1 Unified Task Queue   (daily-return stickiness)
```

### NEXT (prove the moat early)
```text
8.1 Capability Registry → 8.2 Router v0 → 8.4 one cross-executor pipeline
4.5 Mobile Prompt-Control  (A: Telegram now → B: PWA + Tailscale)   ← high "wow", reuses stack
3.2 Local Doc Toolkit (OpenClaw)   (offline, private, demo-able)
5.1 Native Scheduled Automations
4.2 Slack (read + HITL send)
```

> **Note:** Phase **4.6 Remote Desktop View & Takeover** builds directly on 4.5's PWA +
> Tailscale + device pairing, so ship 4.5 first, then 4.6 reuses the same transport and
> auth — only the WebRTC + PipeWire + xdg-desktop-portal capture/stream layer is added on top.

### LATER
```text
3.1 Docs/Sheets/Drive write   ·   6.x Proactive intelligence (gated)
8.3 Router v1 (LLM planner)   ·   5.3 Activepieces embed
Voice native bindings (replace CLI)   ·   Embeddings real-model fix
```

### FUTURE
```text
7.x Workspace Intelligence (digital twin, restore)
9.x Personal Operating Layer (goal compiler, skill-from-prompt)
4.3 Teams · 4.4 WhatsApp
```

---

# 📏 SUCCESS METRICS (make it measurable)

A phase isn't "done" because code exists — it's done when it hits its number:

| Dimension | Target |
|-----------|--------|
| **Daily return** | User opens KRIA's briefing ≥ 5 mornings/week |
| **Goal completion** | ≥ 80% of routed multi-step goals finish without manual fixup |
| **Latency** | Morning Briefing < 3s; voice round-trip < 1.5s |
| **Privacy** | 0 secrets in plaintext; 0 user data leaves device unless explicitly cloud-routed |
| **Safety** | 100% of write/destructive actions pass risk classification + HITL |
| **Cost** | Local-first routing keeps cloud LLM spend per active user under target |

**North star:** the user stops asking *"which app do I open?"* and asks *"what do I need
to accomplish today?"* — and KRIA handles the rest.

---

# 🧭 CROSS-CUTTING ENGINEERING PRINCIPLES

1. **Foundation before features** — Vault + OAuth (Phase 0) gate everything; do them first.
2. **MCP-first, in-repo-later** — Ship integrations fast via MCP, then internalize the
   high-frequency paths for speed, offline use, and to drop external-process dependencies.
3. **Local-first, always** — Cloud is an explicit, budgeted fallback, never the default.
4. **The Router is the product** — Phase 8 is the moat. Don't bury it; staff it early.
5. **Safety scales with autonomy** — More proactivity ⇒ stricter HITL gating.
6. **Durable by default** — Reminders, schedules, and workspace state must survive restarts.
7. **Cut the breadth trap** — Defer Teams/WhatsApp/extra SaaS. Depth on the moat beats
   breadth on commodity readers.
8. **Pin trusted dependencies** — Exact versions, official/high-adoption sources only.

---

> **Bottom line:** Vision is A+. The risk was always scope and treating the Execution
> Router as an afterthought. This v2 fixes the foundation (auth/secrets), makes every
> phase shippable and measurable, picks concrete trusted open-source tools per domain,
> and re-centers the roadmap on KRIA's true moat: **a private, voice-native assistant
> that can actually operate the whole machine.**


---

# 💡 EXTRA SUGGESTIONS — missing high-value use cases & modern integrations

> Analysis layer (not yet committed milestones). Captures use cases a production
> desktop assistant should cover, gaps in the current roadmap by persona, and
> modern tool integrations (Figma, etc.). Format: **gap → why it matters → where it slots**.
> Items here reuse existing KRIA subsystems wherever noted — most are wiring, not new infra.

## A. Cross-cutting big ones (hit all three personas — prioritize)

| Use case | Why it matters | Reuses | Slot |
|----------|----------------|--------|------|
| **Meeting notetaker** | Capture mic/system audio → live transcript → summary → action-items into tasks. **Biggest single gap.** | Voice pipeline + tasks (Phase 2) | new Phase 1.x/4.x |
| **Universal local semantic search** ("Rewind"-style) | One search across files, emails, docs, screenshots, clipboard, chat history | RAG + fastembed (Memory) | expand 3.4 + 7.x |
| **Screenshot + clipboard history w/ OCR** | "Find the screenshot with the error", "what did I copy earlier" | OmniParser/OCR + memory | 7.x |
| **Notification triage** | Summarize/prioritize OS + app notifications, surface only what matters | event bus + LLM | 6.x |
| **Translation / multilingual content** | UI has 7 locales but no content translation (emails, docs, chat) | LLM + tools | 3.x/4.x |
| **First-run setup wizard + user-facing activity log** | Onboarding for non-technical users; transparency/undo UI over the audit ledger | audit ledger, HITL | Phase 0 / cross-cutting |

## B. Layman user

- **Voice-first everyday utilities** — timers, alarms, unit/currency convert, quick math, "remind me when I get home" (geofence). Reminders exist (Phase 2); ephemeral timers/alarms missing.
- **Photo & screenshot organization** — dedupe, album-by-content, OCR search.
- **Receipts / spending tracker** — snap receipt → extract → log (also designer/business).
- **File cleanup assistant** — "clean my Downloads", find big/duplicate/old files. In positioning, no milestone.
- **Guided "how do I do X on my computer"** — GUI-cognition-driven walkthroughs, framed for laymen.
- **Smart-home / device control** — optional but common layman ask (Home Assistant / Matter).
- **Family / multi-profile** on one machine.

## C. Developer / Designer

- **Codebase chat / repo Q&A** — "where is auth handled". Code-aware RAG exists (tree-sitter); no dev-facing milestone.
- **Test→fix loop & PR review** — in positioning, no milestone. OpenClaw + GUI cognition fit.
- **Ephemeral dev environments** — positioning promise; OpenClaw substrate fits.
- **Build / log / error triage** — paste stack trace → root cause; "why did my build fail". 7.3 observer partial.
- **DB query assistant** — natural-language → SQL on local DBs.
- **Dependency / security audit** — `cargo audit` / `npm audit`, license check, CVE surfacing.
- **Designer gaps** — asset batch convert/export, screenshot→code, contrast/a11y checker, brand-kit/style memory, reference research board. Only raw ComfyUI image-gen covered today.
- **Doc generation from code** — READMEs, API docs, changelogs.
- **Screen recording → GIF / tutorial.**

## D. Business Professional

- **Meeting notetaker** (see A) — #1 for this persona.
- **Smart email triage + draft replies** — 6.1 partial (20%); needs real importance ranking + reply suggestions.
- **Document generation** — proposals, contracts, reports from a brief; template/brand flow. 3.1/3.2 partial.
- **Slide / presentation generation** — absent.
- **Scheduling links (Calendly-style)** — availability engine exists (1.2); no shareable booking page.
- **CRM / follow-up tracking** — "who haven't I replied to", contact memory.
- **Expense reports / invoicing / time tracking** — absent.
- **Spreadsheet analysis** — 3.3 at 30%, under-built.
- **Reporting / dashboards from data sources** — absent.
- **E-signature flow** — only PDF-sign mentioned; no real e-sign.

## E. Modern tool & app integrations (latest ecosystem)

KRIA's reach grows fastest by integrating where users already work. Prefer **official
APIs/MCP servers**; sandbox third-party agents inside **OpenClaw**; keep all writes behind HITL.

### Design & creative
- **Figma** — read frames/components via the Figma REST API + **Dev Mode MCP server**; "summarize this file", export assets, **design→code** (frame → HTML/Tailwind/SolidJS), design-token extraction, screenshot→Figma. High value for designers + developers.
- **Canva / Adobe Express** — template-based asset generation via API.
- **Penpot** (open-source Figma alt) — self-hostable, fits local-first ethos.
- **Blender / Inkscape / GIMP** — drive via scripting (OpenClaw skill) for batch/headless asset ops.

### Developer ecosystem
- **GitHub / GitLab / Bitbucket** — beyond read: PR review, issue triage, Actions/CI status, releases (official GitHub MCP exists; add first-party `octocrab`).
- **VS Code / JetBrains** — bridge to the editor (open file, run task, apply edit) via extension or LSP.
- **Docker / Kubernetes** — container/pod status, logs, exec (read-only first; writes HITL).
- **Linear / Jira / Asana / Notion** — task + doc sync, two-way with KRIA tasks.
- **Postman / HTTP** — API request runner + collection import.
- **Sentry / Datadog / Grafana** — pull error/metric context for triage.

### Productivity & communication
- **Slack / Discord / Microsoft Teams** — read+summarize+send (HITL). Slack already scoped (Phase 4.2).
- **Notion / Obsidian / Confluence** — knowledge-base read/write + RAG indexing.
- **Google Workspace / Microsoft 365** — Docs/Sheets/Slides/Drive/Outlook (MCP now, REST later).
- **Zoom / Google Meet / Teams meetings** — join + record + notetaker hook (ties to A).
- **Calendar booking** — Cal.com (open-source) for shareable scheduling links.

### Automation & data
- **n8n / Activepieces / Windmill** — external/embeddable workflow engines (Phase 5).
- **Zapier / Make (MCP)** — reach long-tail SaaS without per-app code.
- **Local DBs (SQLite/Postgres/MySQL) + DuckDB** — NL→SQL analytics, CSV/Parquet crunching.
- **Home Assistant / Matter** — smart-home control for laymen.

### AI / model ecosystem
- **MCP server marketplace auto-discovery** — browse + install MCP servers like OpenClaw skills (same dynamic-registration pattern).
- **Local model hot-swap** — pull/switch GGUF models (llama.cpp) by prompt; per-task model routing.
- **Browser agents** (browser-use / Stagehand) — run inside OpenClaw for safe web automation.

## F. "Table-stakes" capabilities a desktop assistant must have

Baseline behaviors users expect from any serious desktop assistant — flag any that are weak:

- **Global hotkey + quick-launcher** (Spotlight/Raycast-style command bar) — instant invoke from anywhere.
- **Always-available voice wake word** + push-to-talk + barge-in (voice pipeline exists; ensure always-on path).
- **Clipboard manager** with history + "paste as plain text" + AI transform (summarize/translate/rewrite selection).
- **Selected-text actions anywhere** — select in any app → explain/translate/rewrite/summarize (GUI cognition + accessibility APIs).
- **Screenshot → ask** — region capture → "what is this / fix this error / extract text".
- **Universal file drop** — drag any file onto KRIA → summarize/convert/extract.
- **Offline-first graceful degradation** — clear UX when a cloud/optional service is down (sidecar/MCP/ComfyUI optional).
- **Cross-device continuity** — start on desktop, continue on phone (sessions shared; extend to clipboard + files).
- **Undo / dry-run** — preview destructive actions; one-click revert (rollback subsystem exists — surface it).
- **Privacy dashboard** — what data was read, what left the device, per-integration toggles.
- **Accessibility** — full keyboard nav, screen-reader labels, large-text/contrast modes, voice-only operation.
- **Local backup / export** — export all KRIA data (sessions, tasks, memory) in an open format.

## G. Quick priority pick (value ÷ effort — reuses existing stack)

1. **Meeting notetaker** (voice + tasks already built).
2. **Universal local search + screenshot/clipboard memory** (RAG + embeddings built).
3. **Email triage + draft replies** (Gmail tools built).
4. **Codebase chat + build/error triage** (tree-sitter + observer built).
5. **User activity log + undo UI** (audit + rollback built).
6. **Global quick-launcher + selected-text actions** (GUI cognition + voice built) — makes KRIA feel omnipresent.
7. **Figma design→code + asset ops** (highest-leverage modern integration for dev+design).

> **Note:** these are *suggestions/analysis*. Promote individual items into the numbered
> phases above (with tech + deliverable + %) before implementation. Most are integration/
> wiring on top of subsystems KRIA already ships — not new foundations.
