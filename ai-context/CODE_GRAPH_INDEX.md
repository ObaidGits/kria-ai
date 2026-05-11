# KRIA Code Graph Index

> **Generated:** 2026-05-11
> **Nodes:** 9,878 | **Edges:** 15,636 | **Communities:** 622
> **Source:** `graphify-out/GRAPH_REPORT.md`

---

## Purpose

This document provides a structured index of KRIA's code graph for AI/LLM navigation. Use this to quickly locate modules, understand dependencies, and trace execution flows.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        KRIA Workspace                            │
├─────────────────────────────────────────────────────────────────┤
│  kria-core (Sovereign Core)                                      │
│  ├── agent/        → ReAct loop, routing, planning               │
│  ├── tools/        → 60+ native tools                            │
│  ├── safety/       → Policy, HITL, audit, rollback               │
│  ├── llm/          → Model router, orchestrator                  │
│  ├── openclaw/     → Skill substrate, container pool             │
│  ├── memory/       → SQLite store, RAG, embeddings               │
│  ├── voice/        → STT, TTS, VAD, wake word                    │
│  ├── image/        → ComfyUI, cloud fallback                     │
│  ├── mcp/          → MCP client, tool bridge                     │
│  └── infra/        → Health, events, pools, supervisor           │
├─────────────────────────────────────────────────────────────────┤
│  kria-desktop (Tauri Runtime)                                    │
│  └── commands/     → Modular Tauri command handlers              │
├─────────────────────────────────────────────────────────────────┤
│  kria-server (Axum API)                                         │
│  └── routes/       → HTTP/WebSocket handlers                     │
├─────────────────────────────────────────────────────────────────┤
│  ui (SolidJS Frontend)                                          │
│  ├── stores/       → app.ts, provisioning.ts, i18n.ts           │
│  └── components/   → ChatView, HitlModal, VoiceOverlay, etc.     │
└─────────────────────────────────────────────────────────────────┘
```

---

## Key Communities (Code Clusters)

### Core Agent Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **Agent Loop** | 0.02 | `AgentStageEvent`, `applyEvent()`, `approveAction()` | ReAct execution loop |
| **Turn Gate** | 0.03 | `TurnGate`, `ResourcePlan`, `TurnAdmission` | Intent classification |
| **Routing** | 0.04 | `domain_router`, `semantic_router`, `OODChecker` | Request routing |

### Tool Execution Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **Tool Registry** | 0.05 | `ToolRegistry`, `ToolDef`, `ToolHandler` | Tool management |
| **Safety Policy** | 0.06 | `PolicyEngine`, `RiskLevel`, `HITLGateway` | Risk classification |
| **Audit** | 0.07 | `AuditLedger`, `AuditEntry`, `HMACSigner` | Action logging |

### LLM Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **Model Router** | 0.04 | `ModelRouter`, `LocalClient`, `CloudClient` | Model dispatch |
| **Orchestrator** | 0.05 | `GPUWatchdog`, `VRAMBudget`, `TierStrategy` | Hardware management |
| **Server Manager** | 0.06 | `LlamaServer`, `ChildGuard`, `ServerState` | llama.cpp lifecycle |

### OpenClaw Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **Container Pool** | 0.08 | `ContainerPool`, `ContainerHandle`, `PoolState` | Docker management |
| **Skill Registry** | 0.07 | `SkillRegistry`, `SkillManifest`, `TrustTier` | Skill management |
| **Capability Resolver** | 0.06 | `CapabilityResolver`, `BM25Index`, `DenseRetriever` | Skill matching |

### Voice Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **STT** | 0.05 | `WhisperModel`, `TranscriptionResult`, `VAD` | Speech-to-text |
| **TTS** | 0.04 | `PiperModel`, `SynthesisResult`, `PlaybackSink` | Text-to-speech |
| **Wake Word** | 0.03 | `WakeDetector`, `AudioCapture`, `WakeEvent` | Hands-free activation |

### Memory Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **SQLite Store** | 0.06 | `MemoryStore`, `Conversation`, `Fact` | Persistence |
| **RAG** | 0.05 | `RAGEngine`, `DocumentChunk`, `RetrievalResult` | Document Q&A |
| **Embeddings** | 0.04 | `VectorIndex`, `FastEmbed`, `SimilaritySearch` | Semantic search |

### Frontend Communities

| Community | Cohesion | Key Nodes | Purpose |
|-----------|----------|-----------|---------|
| **App State** | 0.02 | `assistantMessages`, `assistantIsThinking`, `sessions` | Central state |
| **Chat UI** | 0.03 | `ChatView`, `MessageBubble`, `ToolResult` | Chat rendering |
| **HITL Modal** | 0.04 | `HitlModal`, `ApprovalRequest`, `approveAction()` | Approval UX |
| **Voice Overlay** | 0.03 | `VoiceOverlay`, `VoiceState`, `transcriptionText` | Voice UX |

---

## Entry Points

### Desktop Runtime Entry

```
crates/kria-desktop/src/main.rs
  └── run_app()
      ├── init_app_state()     → AppState initialization
      ├── register_commands()  → Tauri command registration
      └── run_tauri()          → Start Tauri runtime
```

### Agent Turn Entry

```
crates/kria-desktop/src/commands/chat.rs
  └── send_message()
      └── AgentLoop::run()
          ├── TurnGate::admit()        → Intent classification
          ├── ModelRouter::route()     → Model selection
          ├── ToolRegistry::execute()  → Tool calls
          └── MemoryStore::persist()    → Save conversation
```

### Voice Pipeline Entry

```
crates/kria-desktop/src/commands/voice.rs
  └── start_voice_session()
      └── VoicePipeline::start()
          ├── AudioCapture::start()    → Microphone input
          ├── VAD::process()           → Voice activity
          ├── STT::transcribe()        → Whisper
          ├── AgentLoop::run()         → Process text
          └── TTS::synthesize()        → Piper output
```

### OpenClaw Skill Entry

```
crates/kria-desktop/src/commands/openclaw.rs
  └── invoke_skill()
      └── CapabilityResolver::resolve()   → Match skill
          └── ContainerPool::acquire()    → Get container
              └── SkillHandler::execute() → Run skill
                  └── AuditLedger::record() → Log invocation
```

---

## Critical Dependencies

### Agent → Tools

```
AgentLoop
  └── ToolRegistry
      ├── FileOpsTools      → read_file, write_file
      ├── InternetTools     → web_search, fetch_url
      ├── DocumentTools     → parse_pdf, convert_doc
      ├── SystemTools       → system_info, power_control
      ├── OpenClawTools     → invoke_skill
      └── MCPTools          → mcp_call
```

### Agent → Safety

```
ToolRegistry::execute()
  └── PolicyEngine::classify()
      ├── GREEN → execute immediately
      ├── YELLOW → audit + execute
      ├── RED → HITL approval required
      └── BLACK → blocked
```

### LLM → Hardware

```
ModelRouter
  └── Orchestrator
      ├── GPUWatchdog      → VRAM monitoring
      ├── TierStrategy      → Hardware tier
      ├── VRAMBudget        → Memory allocation
      └── ServerManager     → llama-server lifecycle
```

### OpenClaw → Docker

```
ContainerPool
  ├── Docker::create_container()
  ├── Docker::exec()
  └── Docker::remove_container()
```

---

## Key Types Reference

### Agent Types

| Type | Location | Purpose |
|------|----------|---------|
| `AgentLoop` | `agent/loop_engine/mod.rs` | Main ReAct loop |
| `TurnGate` | `agent/turn_gate.rs` | Intent classification |
| `ResourcePlan` | `agent/turn_gate.rs` | Resource allocation |
| `ToolCall` | `agent/types.rs` | Tool invocation request |

### Safety Types

| Type | Location | Purpose |
|------|----------|---------|
| `RiskLevel` | `safety/policy.rs` | GREEN/YELLOW/RED/BLACK |
| `ApprovalRequest` | `safety/hitl.rs` | HITL request |
| `AuditEntry` | `safety/audit.rs` | Audit log record |
| `RollbackManifest` | `safety/rollback.rs` | Backup metadata |

### OpenClaw Types

| Type | Location | Purpose |
|------|----------|---------|
| `SkillManifest` | `openclaw/types.rs` | Skill definition |
| `SkillCapabilities` | `openclaw/types.rs` | Capability flags |
| `TrustTier` | `openclaw/types.rs` | Community/Verified/Partner/Internal |
| `ContainerHandle` | `openclaw/pool.rs` | Docker container reference |

### Memory Types

| Type | Location | Purpose |
|------|----------|---------|
| `MemoryStore` | `memory/store.rs` | SQLite persistence |
| `Fact` | `memory/facts.rs` | User fact |
| `Conversation` | `memory/manager.rs` | Chat history |
| `VectorIndex` | `memory/vectors.rs` | Embedding index |

---

## UI Component Map

| Component | Location | Purpose |
|-----------|----------|---------|
| `App` | `ui/src/App.tsx` | App shell |
| `ChatView` | `ui/src/components/ChatView.tsx` | Chat interface |
| `MessageBubble` | `ui/src/components/MessageBubble.tsx` | Message rendering |
| `HitlModal` | `ui/src/components/HitlModal.tsx` | Approval dialog |
| `VoiceOverlay` | `ui/src/components/VoiceOverlay.tsx` | Voice UI |
| `SettingsModal` | `ui/src/components/SettingsModal.tsx` | Settings |
| `FleetMatrix` | `ui/src/components/FleetMatrix.tsx` | Fleet management |
| `RemoteSkillCard` | `ui/src/components/RemoteSkillCard.tsx` | OpenClaw skill UI |
| `SkillMarketplace` | `ui/src/components/SkillMarketplace.tsx` | Skill marketplace |
| `SubstrateStatus` | `ui/src/components/SubstrateStatus.tsx` | Container pool status |

---

## Tauri Command Map

| Command | Location | Purpose |
|---------|----------|---------|
| `send_message` | `commands/chat.rs` | Send chat message |
| `start_voice_session` | `commands/voice.rs` | Start voice |
| `shutdown_runtime` | `commands/runtime.rs` | Graceful shutdown |
| `invoke_skill` | `commands/openclaw.rs` | Invoke OpenClaw skill |
| `clawhub_fetch_remote_skills` | `commands/openclaw.rs` | Fetch skill list |
| `clawhub_install_skill` | `commands/openclaw.rs` | Install skill |
| `approve_action` | `commands/app_commands.rs` | HITL approval |
| `get_system_info` | `commands/runtime.rs` | System telemetry |

---

## Related Documentation

- **Full Graph Report:** `graphify-out/GRAPH_REPORT.md` (155KB, detailed)
- **Graph JSON:** `graphify-out/graph.json` (8.7MB, machine-readable)
- **Architecture:** `docs/ARCHITECTURE.md`
- **Tools Guide:** `docs/TOOLS.md`
- **OpenClaw:** `docs/OPENCLAW.md`

---

## Usage Tips for AI/LLM

1. **Find entry points:** Search for `main.rs`, `run_app()`, `send_message()`
2. **Trace dependencies:** Follow `→` arrows in dependency chains
3. **Understand communities:** Cohesion score indicates module coupling
4. **Locate types:** Use Key Types Reference table
5. **Navigate UI:** Use Component Map for frontend queries
6. **Backend commands:** Use Tauri Command Map for IPC queries
