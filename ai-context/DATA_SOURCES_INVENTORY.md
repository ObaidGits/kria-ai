# KRIA Data Sources Inventory

## Overview
This document catalogs all data sources, stores, and persistence mechanisms in KRIA. Use this as a reference when building dashboards, analytics, or debugging data flow.

---

## 1. MemoryStore (SQLite)

| Table | Schema | Purpose | Persistence |
|-------|--------|---------|-------------|
| `conversations` | id, session_id, role, content, timestamp, metadata | Chat history | Disk |
| `facts` | id, key, value, category, confidence, access_count, last_accessed, created_at, decay_factor | User facts | Disk |
| `audit_log` | id, timestamp, tool_name, params_hash, risk_level, approved, user_response, result_summary, duration_ms | Action audit | Disk |
| `preferences` | key, value, updated_at | User preferences | Disk |
| `snippets` | id, name, content, language, tags, created_at | Code snippets | Disk |
| `document_chunks` | id, doc_id, chunk_index, content, embedding, metadata | RAG chunks | Disk |
| `chat_media` | id, message_id, media_type, path, metadata | Media attachments | Disk |

**Location:** `~/.kria/kria.db`

---

## 2. VectorIndex

| Field | Type | Purpose |
|-------|------|---------|
| embeddings | Bincode | Dense vectors for semantic search |
| id_map | HashMap | Vector ID to record mapping |

**Location:** `~/.kria/vectors/`

---

## 3. WorldModelStore (SQLite)

| Table | Schema | Purpose |
|-------|--------|---------|
| `world_facts` | id, entity, attribute, value, confidence, source, timestamp | World state |
| `world_archive` | id, entity, attribute, old_value, new_value, changed_at | State history |

---

## 4. FailureAnalyzerStore (SQLite)

| Table | Schema | Purpose |
|-------|--------|---------|
| `failure_patterns` | id, pattern, frequency, last_seen, resolution_hint | Failure patterns |
| `failure_log` | id, timestamp, context, error, pattern_id | Failure events |

---

## 5. SkillCompiler Store (SQLite)

| Table | Schema | Purpose |
|-------|--------|---------|
| `playbooks` | id, name, steps, created_at | Compiled playbooks |
| `skills` | id, name, source, compiled_at | Compiled skills |

---

## 6. QuarantineRegistry (SQLite)

| Table | Schema | Purpose |
|-------|--------|---------|
| `quarantine_items` | id, path, reason, quarantined_at, expires_at | Quarantined files |

---

## 7. OpenClaw Skill Registry (SQLite)

| Table | Schema | Purpose |
|-------|--------|---------|
| `skills` | id, name, version, manifest, installed_at, trust_tier | Installed skills |
| `invocations` | id, skill_id, timestamp, params_hash, result_hash, duration_ms | Skill audit |

**Location:** `~/.kria/openclaw.db`

---

## 8. OpenClaw Audit Ledger (SQLite)

| Table | Schema | Purpose |
|-------|--------|---------|
| `audit_entries` | id, timestamp, skill_name, action, params_hash, result_hash, hmac_signature | Tamper-proof log |

**HMAC Key:** Derived from machine-id

---

## 9. ToolRegistry (In-Memory)

| Field | Type | Purpose |
|-------|------|---------|
| tools | RwLock<HashMap<String, ToolDef>> | Registered tools |
| mounts | RwLock<Vec<MountRule>> | Tool mounting rules |

---

## 10. McpServerManager (In-Memory)

| Field | Type | Purpose |
|-------|------|---------|
| servers | HashMap<String, McpServerState> | MCP server states |
| tools | HashMap<String, McpToolDef> | Discovered MCP tools |

---

## 11. GpuLeaseManager (In-Memory)

| Field | Type | Purpose |
|-------|------|---------|
| leases | Mutex<HashMap<Uuid, GpuLease>> | Active GPU leases |
| owner | Mutex<Option<GpuOwner>> | Current GPU owner |

---

## 12. ResourceSnapshot (In-Memory, Polled)

| Field | Type | Refresh |
|-------|------|---------|
| cpu_percent | f32 | 2s |
| memory_used_mb | u64 | 2s |
| memory_total_mb | u64 | 2s |
| gpu_vram_used_mb | u64 | 2s |
| gpu_vram_total_mb | u64 | 2s |
| gpu_utilization | f32 | 2s |

---

## 13. HealthRegistry (In-Memory, DashMap)

| Field | Type | Purpose |
|-------|------|---------|
| services | DashMap<String, ServiceHealth> | Service health status |

---

## 14. EventBus (In-Memory, Broadcast)

| Event | Fields | Purpose |
|-------|--------|---------|
| FileUploaded | path, size | File upload notification |
| MessageReceived | session_id, role, content | Chat message |
| ToolCompleted | tool_name, duration, success | Tool execution |
| SidecarResult | processor, result | Sidecar response |
| VoiceTranscribed | text, confidence | STT result |
| HardwareChanged | component, change | Hardware event |
| SkillInstalled | skill_id, name | OpenClaw skill install |
| VramPressure | level, used_mb | VRAM warning |
| LlmSwapStarted/LlmSwapCompleted | model_name | Model swap events |

---

## 15. HITL Gateway (In-Memory)

| Field | Type | Purpose |
|-------|------|---------|
| pending | HashMap<Uuid, ApprovalRequest> | Pending approvals |

---

## 16. RollbackManager (Filesystem)

| Location | Purpose |
|----------|---------|
| `~/.kria/rollback/` | Backup files before destructive ops |

**Manifest:** `rollback_manifest.json`

---

## 17. EnrolledTargetRecord (JSON File)

| Field | Type | Purpose |
|-------|------|---------|
| targets | Vec<TargetMeta> | Enrolled fleet targets |

**Location:** `~/.kria/enrolled_targets.json`

---

## 18. Voice Telemetry (In-Memory, mpsc)

| Field | Type | Purpose |
|-------|------|---------|
| latency_ms | Vec<u64> | STT/TTS latency samples |
| vad_events | Vec<VadEvent> | VAD trigger log |

---

## 19. ImageOrchestrator (In-Memory)

| Field | Type | Purpose |
|-------|------|---------|
| jobs | HashMap<Uuid, ImageJob> | Active generation jobs |
| progress | HashMap<Uuid, f32> | Job progress |

---

## 20. Container Pool (In-Memory + Docker)

| Field | Type | Purpose |
|-------|------|---------|
| containers | HashMap<Uuid, ContainerHandle> | Warm containers |
| state | PoolState | Pool status |

**Docker Containers:** `kria-openclaw-substrate-*`

---

## 21. Config Files

| File | Purpose |
|------|---------|
| `config/default.toml` | Default configuration |
| `config/mcp_servers.json` | MCP server configs |
| `config/seccomp/kria-seccomp.json` | Seccomp policy |
| `~/.kria/config.toml` | User overrides |

---

## Summary: Data Source Matrix

| # | Data Source | Persistence | Refresh Rate | Priority |
|---|-------------|-------------|-------------|----------|
| 1 | MemoryStore (conversations) | SQLite (disk) | Real-time | High |
| 2 | MemoryStore (facts) | SQLite (disk) | Real-time | High |
| 3 | MemoryStore (audit_log) | SQLite (disk) | Real-time | Critical |
| 4 | OpenClaw Audit Ledger | SQLite (disk) | Real-time | Critical |
| 5 | OpenClaw Skill Registry | SQLite (disk) | On change | High |
| 6 | VectorIndex | Bincode (disk) | On change | Medium |
| 7 | ToolRegistry | In-memory | Real-time | High |
| 8 | McpServerManager | In-memory | Real-time | High |
| 9 | GpuLeaseManager | In-memory | Real-time | Critical |
| 10 | ResourceSnapshot | In-memory (polled) | 2s interval | Critical |
| 11 | HealthRegistry | In-memory (DashMap) | Real-time | High |
| 12 | EventBus | In-memory (broadcast) | Real-time | Medium |
| 13 | HITL Gateway | In-memory | Real-time | High |
| 14 | RollbackManager | Filesystem | On action | High |
| 15 | Container Pool | In-memory + Docker | Real-time | High |
| 16 | EnrolledTargetRecord | JSON file (disk) | On change | Medium |
| 17 | Config files | TOML/JSON (disk) | On startup | High |
