# KRIA MEMORY ARCHITECTURE — DEFINITIVE BLUEPRINT

**Status:** Original design rationale (the "why"). Implementation is complete + hardened — see the verified-status banner below.
**Date:** July 12, 2026
**Audience:** Engineering team, before writing any code
**Nature:** META architecture — explains WHY / WHAT / WHEN / WHERE, not HOW

> This document supersedes and merges all prior research and review documents.
> It is the definitive blueprint for KRIA's cognitive memory system for the next 10+ years.

---

## ⓘ IMPLEMENTATION STATUS (verified — DOC-1, updated after the stabilization pass)

This file is the **original design rationale**, preserved for the "why". The
memory system has since been **implemented and hardened**; a few concrete
decisions below evolved during implementation. For the **current, verified
state** always trust the code plus:

- `.kiro/specs/memory-upgrade/SESSION_HANDOVER.md` — live batch status + how to re-verify.
- `.kiro/specs/memory-upgrade/STABILIZATION_BIBLE.md` — issue catalogue, fixes, and the Architecture Lock Criteria.

As-built corrections (do NOT treat the stale text below as current):

- **Engine complete + hardened** — the 37-task engine plus an 11-batch hardening pass are done (single-laptop production-ready). No longer "pre-implementation".
- **Vector backend is in-process HNSW, not LanceDB** — vectors live in SQLite (`mem_vectors`, durable authority) fronted by an in-process HNSW ANN index (`memory/stores/ann_vectors.rs::AnnVectorStore`, crate `hnsw_rs`) behind the `VectorStore` trait, with a brute-force fallback for tiny partitions. LanceDB was not adopted (single-binary, local-first). [H2]
- **Enrichment queue is bounded + durably recoverable** — bounded `mpsc` wake channel (`try_send` backpressure drops the wake, never the data); durability + crash recovery from the durable event log + consumer cursor + boot/periodic catch-up; depth via `MemorySystem::pending_enrichment_depth()`. [R1/R2]
- **One ingestion pipeline** — cold-start import, the RAG tool, and the desktop ingest command all route through `MemorySystem::ingest_document`. [M3]
- **One memory-API contract** — `memory/contract.rs` is the single canonical surface; the server's Axum routes are thin adapters over it; server live changes stream over SSE `GET /memory/events`. [API-1/UI-1]
- **Safety as-built** — the deterministic injection wall covers all untrusted provenance (`Import`/`Library`/external); cold-start does content-level secret scanning (labelled + entropy) before import; tool-outcome writes are salience-gated. [S1/S2/M5]

---

## 0. How To Read This Document

This is not a coding guide, API spec, or schema definition. It is the reasoning
an engineering team absorbs before implementation. It captures decisions, the
evidence behind them, the tradeoffs accepted, and the risks acknowledged.

Sections 1-4 define the vision and mental model.
Sections 5-14 define the architecture (the spine, storage, cognition).
Sections 15-22 define the operational concerns (integration, privacy, failure, scale).
Sections 23-26 define technology decisions, risks, roadmap, and requirement audit.

---

## 1. Executive Summary

KRIA Memory is a **local-first cognitive memory system** for an intelligent
desktop assistant — not a chatbot history store. It must remember, understand,
organize, reason, consolidate, forget intelligently, and improve over years,
while remaining transparent, private, and fully user-controlled.

**The core architectural decisions:**

1. **Event sourcing** — an append-only event log is the immutable source of truth. All else is derived and rebuildable.
2. **Memory Write Policy Engine** — the single mandatory gateway through which every subsystem writes. Nothing bypasses it.
3. **Storage: SQLite + LanceDB** — SQLite for relational + graph (adjacency tables + CTEs) + FTS5; LanceDB for vectors. Both embedded, both append-friendly, both local-first.
4. **Trait-based storage ports** — GraphStore/VectorStore/RelationalStore/EventStore abstracted so backends swap without touching callers.
5. **Cognitive layer** — dreaming (consolidation), reflection, truth maintenance, and progressive compression (episode → skill → rule) turn storage into intelligence.
6. **Memory Modes** — Permanent / Temporary / Workspace / Incognito / Read-only, always visible, enforced at the write gate.
7. **Truth Maintenance** — staleness classes + evidence tracking + contradiction resolution ensure KRIA never confidently relies on outdated knowledge.

**Requirements coverage: 28/30 fully, 2/30 partial (multimodal + 3D UI, both deliberately deferred).**

**Architecture confidence: 7.5/10 pre-implementation** (raised from an honest 6.5 after
the Red Team pass in Sections 27-31 resolved five foundational contradictions).
A higher score is not claimable until Phase 1-2 are built and benchmarked — no
unvalidated architecture earns 9/10. See Section 31 for the go/no-go rationale.

> **Read Sections 27-31 before implementation.** They contain the Red Team
> resolutions that override any earlier optimistic phrasing in this document.
> Where earlier sections conflict with Section 27, Section 27 wins.

---

## 2. Vision & Principles

KRIA Memory optimizes for **lifelong intelligence, not conversation history**.

**Non-negotiable principles:**

| Principle | Meaning | Enforcement |
|---|---|---|
| Local-first | All data on-device; no cloud dependency for core function | Embedded stores only |
| Privacy-first | User owns and controls everything | Memory modes + deletion + export |
| Offline-first | Works with no network, no LLM | Graceful degradation (Section 18) |
| Deterministic-when-possible | Prefer rules over LLM; LLM only for genuine ambiguity | Write Policy Engine fast-path |
| Transparent | Every recall is explainable | Provenance + debug API (Section 19) |
| Correctness over convenience | Never confidently wrong | Truth Maintenance (Section 12) |
| Extensible without redesign | New types/DBs/models slot in | Trait ports + event log |
| Quality over quantity | Store what matters, not everything | Write filtering (Section 5) |

**The mental model:** Memory is KRIA's **cognitive backbone**, not a database it
queries. Every subsystem reads from and writes to it; the memory system actively
thinks (consolidates, reflects, revises) between interactions.

---

## 3. Memory Taxonomy

KRIA distinguishes memory types along the **Experience Compression Spectrum**
(arxiv:2604.15877): raw experience compresses progressively into reusable knowledge.

```
Raw Event (1×) → Episode (5-20×) → Skill/Procedure (50-500×) → Rule (1000×+)
```

| Type | Purpose | Compression | Persistence |
|---|---|---|---|
| **Working** | Current turn state (goal, memo cache) | none | Ephemeral (per-turn) |
| **Short-Term** | Current session recall | 1× | Session |
| **Episodic** | "What happened when" | 5-20× | Months → summary permanent |
| **Semantic** | Facts, knowledge | varies | Long-term, decay-governed |
| **Procedural** | Reusable workflows/skills | 50-500× | Permanent unless harmful |
| **Goal** | Active/completed/recurring/ambitions | — | Life history |
| **Reflection** | Meta-observations, lessons | high | Permanent |
| **Failure** | What went wrong + recovery | — | Permanent (learning) |
| **Reasoning Trace** | WHY a decision was made | — | Long-term (self-improvement) |
| **World Model** | Verified facts about environment | — | Confidence-governed |
| **User Profile** | Preferences, habits, skills | — | Permanent, evolving |
| **Capability (CKB)** | Tool/skill success stats | — | Permanent |
| **Workspace/Project** | Git, build, project context | — | Project-scoped |
| **Desktop Context** | Ambient activity (filtered) | high | Episodic promotion only |
| **Library** | Ingested documents/knowledge | — | Reference (no decay) |

Each type's full lifecycle rules are unified in Section 13.

---

## 4. Memory Classification

Every memory carries **orthogonal classification axes** (not a single class):

| Axis | Values | Determined by |
|---|---|---|
| **Retention** | temporary / session / long-term / permanent | Write Policy (rules + importance) |
| **Epistemic** | verified / inferred / uncertain | Source + verification history |
| **Origin** | factual / observation / preference | Source type |
| **Scope** | personal / project / workspace / global | Context at write time |
| **Visibility** | private / public (to plugins/agents) | Sensitivity policy |
| **Sensitivity** | public / private / secret | Content classifier |
| **Staleness class** | immutable / slow / volatile / permanent | Content type (Section 12) |

Classification is **deterministic wherever possible** (fast-path rules, <1ms).
LLM assists ONLY for ambiguous cases (borderline importance, entity resolution,
multi-fact statements, unclear contradictions).

---

## 5. THE MEMORY WRITE POLICY ENGINE (The Spine)

**This is the single most important component.** Every subsystem — Planner,
Reasoner, OpenClaw, Library, Reflection, Execution — writes ONLY through this
gate. Nothing touches storage directly. This is what makes memory governable,
auditable, private, and safe.

### Responsibilities (in strict order)

```
1. MODE CHECK        — Is writing allowed in current memory mode?
                       (Incognito → reject all; Temporary → session-only; etc.)
2. QUALITY FILTER    — Is this worth storing? (reject noise: failed retries,
                       cancelled actions, debugging spam, transient errors)
3. CLASSIFICATION    — Assign type + all classification axes
4. DEDUPLICATION     — Vector similarity check; if duplicate → update existing
5. CONTRADICTION     — Does this conflict with existing memory? (flag/resolve)
6. CONFIDENCE        — Assign confidence from source reliability
7. PROVENANCE        — Attach source event, derived_from chain, evidence
8. EXPIRATION        — Assign staleness class + valid_from/valid_until
9. OWNERSHIP         — Namespace + owner_id + sensitivity
10. SECURITY SCAN    — Injection/poisoning detection (OWASP ASI06)
11. BUDGET CHECK     — Write budget + LLM budget (batch if over budget)
12. COMMIT           — Append event → derive memory → update indexes atomically
```

### Why this must be mandatory

- **Governance:** Every write answers: why store / what type / confidence / evidence / expiration / source / owner / subsystem / safe-to-delete / needs-confirmation
- **Security:** One choke point to defend against poisoning, not N scattered ones
- **Modes:** Incognito/Temporary enforcement is impossible if subsystems write directly
- **Quality:** Central filter prevents memory bloat (quality > quantity)
- **Auditability:** Every write is logged with its reasoning
- **LLM budgeting:** Batches extraction/classification LLM calls; degrades gracefully when LLM down
- **Deduplication:** Prevents the same fact stored 50 times

### Additional responsibilities identified

- **Write batching:** Buffer low-priority writes, flush on idle (reduces I/O + LLM calls)
- **Confirmation routing:** sensitivity=secret or high-impact → queue for user approval
- **False-promotion guard** (arxiv:2607.02579): refuse to write a "learned rule" from correlated/insufficient evidence — prevents the agent teaching itself wrong lessons

**Design rule:** If a subsystem needs to store something, it emits a
`WriteCandidate` to the engine. The engine decides. The subsystem never assumes
the write happened — it is best-effort and policy-governed.

---

## 6. Memory Modes

Modes are enforced at the Write Policy Engine and always visible in the UI.

| Mode | Writes | Retrieval | Reflection/Consolidation | Use case |
|---|---|---|---|---|
| **Permanent** (default) | All allowed (policy-governed) | Full | Yes | Normal operation |
| **Temporary Chat** | Session-only; purged at session end | Full during session | No | Sensitive one-offs |
| **Incognito** | None (RAM only, never persisted) | Session RAM only | No | Maximum privacy |
| **Workspace** | Only project/workspace-scoped; personal facts rejected | Workspace + global | Workspace-scoped | Focused project work |
| **Library-only** | Only document ingestion | Library + retrieval | Library extraction only | Research sessions |
| **Read-only** | No writes at all | Full | No | Reviewing without changing memory |
| **Guest** | None persisted; isolated namespace | Public/global only | No | Someone else using the machine |
| **Developer** | All + verbose provenance logging | Full + debug annotations | Yes | Building/debugging KRIA |
| **Benchmark/Test** | Isolated test namespace | Test-scoped | On-demand | Evaluation harness |
| **Safe Mode** | Deterministic-only (no LLM writes) | Vector+FTS only | No | LLM unavailable/untrusted |
| **Research Mode** | Aggressive extraction + gap tracking | Full + proactive | Enhanced | Deep-dive learning |

**Switching:** Mode is per-session, user-switchable mid-session. A switch emits a
boundary event so the timeline reflects the transition. Downgrading to
Incognito mid-session does NOT retroactively delete already-written memories
(user must explicitly forget those).

**UI mandate:** The current mode is ALWAYS displayed. Users never wonder whether
they are being remembered.

---

## 7. Storage Architecture

### The Decision: SQLite + LanceDB (two engines, both embedded, both append-friendly)

```
┌──────────────────────────────────────────────────────────────┐
│  TIERED BY ACCESS PATTERN (not by data type)                  │
├──────────────────────────────────────────────────────────────┤
│  HOT   (RAM, <1ms)   Working memory, recent context, caches    │
│  WARM  (SQLite,<10ms) Events, memories, graph, goals, FTS5     │
│  COLD  (LanceDB,<50ms) All embeddings + archived vectors       │
│  ARCHIVE (filesystem) Library documents, backups, models       │
└──────────────────────────────────────────────────────────────┘
```

**SQLite owns:** event log (append-only), memories, episodes, goals, preferences,
reflections, reasoning traces, capabilities, failures, knowledge gaps, entities +
relationships (graph as adjacency tables), FTS5 keyword indexes.

**LanceDB owns:** all embeddings (memory content, library chunks, episodes),
partitioned by embedding model version. Disk-native IVF-PQ index.

**Filesystem owns:** original library files, encrypted backups, ONNX models.

### Why This Pairing Is Correct (evidence)

- **SQLite:** 30-year track record, embedded, single-file, WAL for crash safety, FTS5 built-in, recursive CTEs for graph. Backup = copy a file.
- **LanceDB:** Rust core, **append-only at storage layer** (pairs perfectly with event sourcing), columnar Lance format (Lance 2.1 stable as of 2026), built-in versioning + time-travel (instant rollback on bad re-embedding), disk-native (doesn't need all vectors in RAM), Apache 2.0, $30M-funded active project.

**Critical synergy:** Both SQLite (WAL) and LanceDB (fragments) are append-oriented.
This aligns the entire storage layer with the event-sourcing philosophy — writes
add, they don't destroy. Time-travel and recovery become natural, not bolted-on.

### Rejected Storage Technologies (with evidence)

| Rejected | Reason |
|---|---|
| SurrealDB | BSL license, ~50MB binary, still maturing. Overkill for <50K graph nodes. |
| Oxigraph | Self-described "unstable hobby project," possible data loss |
| CypherLite / graphdblite / Uni-DB | <1 year old, no production references, single maintainers |
| Neo4j / Memgraph | JVM or server-mode, cannot embed cleanly |
| usearch | RAM-only; LanceDB supersedes (disk-native + FTS + versioning) |
| DuckDB | OLAP analytics engine, wrong workload (we need OLTP) |
| Multiple SQLite DBs | The CURRENT KRIA problem — fragmentation. Consolidate to one. |

---

## 8. Graph Architecture

### Decision: Graph lives in SQLite (adjacency tables + recursive CTEs), behind a GraphStore trait

**What belongs in the graph:** people, projects, tools/technologies, concepts —
and the typed, weighted, time-valid relationships between them.

**What NEVER belongs:** raw conversation turns, filesystem trees, audit entries,
individual memories, embeddings, ephemeral working memory.

### Why SQLite, not a dedicated graph DB

At KRIA's projected scale (10K nodes @ 5yr, 30K @ 10yr, 80K @ 20yr), SQLite
recursive CTEs perform 2-hop traversal in <5ms. Dedicated graph databases win at
millions of nodes with deep (5+ hop) traversals — KRIA needs neither. Adding one
means: new dependency, new binary weight, new backup target, new failure mode,
new corruption surface — for **zero proven benefit at this scale**.

**The GraphStore trait** makes this reversible: if KRIA ever exceeds 50K nodes
with >50ms traversal latency, OR needs PageRank/community-detection for GraphRAG
document intelligence, swap the backend implementation. Planner, retrieval, and
memory services never change.

**Precedent:** Apple Intelligence, Claude Code, and OpenAI Dreaming all achieve
knowledge relationships without a dedicated graph DB at their current scale.

---

## 9. Vector Architecture & Embeddings

### Vectors: LanceDB, partitioned by embedding-model version

Single embedding per memory (title+content combined). Multiple embeddings add
complexity without clear benefit at KRIA's scale.

### Embedding model: start with a modern multilingual ONNX model

**[UPDATED]** all-MiniLM-L6-v2 (384-dim) is aging. As of 2026, **EmbeddingGemma-300M**
(ONNX available, 100+ languages, Matryoshka dimensions 768→128) or **nomic-embed-text-v2**
match/beat OpenAI's text-embedding-3-large on retrieval while running locally on
modest hardware. Recommendation: **EmbeddingGemma-300M ONNX** for new builds
(better multilingual, Matryoshka lets you trade dim for speed).

### The Embedding Version Crisis (critical 10-year concern)

Embeddings from different models are INCOMPARABLE (cosine similarity between
MiniLM and Gemma vectors is meaningless). Design for this NOW:

```
- Every embedding stores: model_name + model_version + dimension
- LanceDB: one table per model version (embeddings_gemma_v1, embeddings_next_v2)
- On model upgrade: dual-search both tables, merge by text-level dedup
- Background worker re-embeds old memories, oldest-first, rate-limited
- NEVER drop old table until ALL memories re-embedded
- LanceDB time-travel = instant rollback if a re-embedding batch corrupts data
```

This turns embedding obsolescence from a catastrophe into a background migration.

### Matryoshka insight

EmbeddingGemma supports Matryoshka representation (truncate 768→256→128 dims
with graceful quality loss). KRIA can store full 768-dim for cold storage but
search with truncated 256-dim for speed on the hot path — a free performance lever.

---

## 10. Retrieval Architecture

### Adaptive multi-strategy fusion (no HyDE, no ColBERT, no default cross-encoder)

**Rejected after evidence:**
- HyDE: advantage collapsed to 1-4 nDCG points vs modern embeddings (2026 reruns), +25-40% latency. Not worth it.
- ColBERT: 32× storage for per-token embeddings. Overkill for personal memory.
- Cross-encoder: 50-200ms latency. Reserve for Library document QA only.

### Pipeline

```
QUERY → classify (deterministic, <5ms) → {temporal|entity|conceptual|recent|procedural}
    │
    ├── Strategy 1: LanceDB vector search (always)
    ├── Strategy 2: SQLite FTS5 keyword (always)
    ├── Strategy 3: Graph traversal (if entities present, 2-hop)
    ├── Strategy 4: Temporal filter (if time signal)
    └── Strategy 5: Goal-context filter (always — a filter, not a search)
    │
    ▼ Adaptive RRF (weights vary by query type)
    ▼ Memory Worth weighting (success/failure co-occurrence, arxiv:2604.12007)
    ▼ Importance + confidence weighting
    ▼ Staleness flag (mark possibly-stale memories)
    ▼ Namespace + sensitivity filter
    ▼ TOKEN-BUDGET fill (not top-K — fill ~800 tokens by relevance)
    ▼ Provenance annotation
    → Injected into context
```

**Multi-turn coherence:** Within a session, if topic is stable (>0.8 cosine),
KEEP previously surfaced memories pinned; only clear on topic shift. Prevents
context thrashing across related questions.

**Proactive retrieval:** Before the user asks, a salience loop (every ~10s when
idle) compares current desktop/file/time context against high-importance
memories; strong matches surface into the next turn's context.

---

## 11. Cognitive Layer (Thinking Over Memory)

This is what separates KRIA from a database. Between interactions, memory
actively processes itself. Modeled on neuroscience (hippocampal replay,
sleep consolidation) and validated by production systems (Anthropic Dreaming
May 2026, OpenAI Dreaming V3 June 2026 — accuracy 41.5%→82.8%).

### Trigger-based (NOT fixed calendar)

| Trigger | Operation | LLM? |
|---|---|---|
| Idle >30min | Micro-consolidation (decay, dedup, stats) | No |
| Session end | Session reflection + episode boundary + skill extraction | Yes |
| Idle >4h / daily | Dreaming: summarize, compress, update user/self models | Yes |
| Weekly | Deep reflection, pattern/habit detection, archival | Yes |
| Unprocessed backlog > threshold | Forced consolidation | Yes |
| After failure/success | Targeted reflection (why?) | Yes |

### Cognitive operations

Reflection · Generalization (episode→skill→rule) · Contradiction detection ·
Knowledge-gap detection · Causal reasoning (action→outcome links) · Habit
detection · Goal inference · Confidence calibration · Proactive recall ·
Self-improvement (self-model update).

### Decoupled execution (arxiv:2605.30842 CoMem)

Consolidation runs in a SEPARATE async worker, parallel to the agent loop —
never blocking user interaction. 1.4× latency benefit, no quality loss.

### Two-mode dreaming (adopt both, per lab evidence)

- **Session-oriented** (Anthropic-style): "What did I learn from this work session?"
- **User-oriented** (OpenAI-style): "What do I know about the user? What's stale?"

---

## 12. Truth Maintenance System (TMS)

**The correctness spine.** Decay ≠ truth. KRIA must never confidently rely on
outdated knowledge. No existing memory framework does this well — a differentiator.

### Staleness classes (govern re-verification, not deletion)

| Class | Re-verify after | Example |
|---|---|---|
| Immutable | Never | Name, birthday, core identity |
| Permanent | Never | World facts, math, language |
| Slow | 30 days | Employer, tech stack, team |
| Volatile | 1 hour | Build status, branch, running processes |

### Mechanisms

- **Evidence tracking:** each fact carries supporting + contradicting evidence lists
- **Evidence aging:** old evidence weakens unless re-confirmed
- **Confidence propagation:** if a fact a memory depends on is revised, dependents recompute
- **Fact supersession:** new fact with stronger evidence supersedes old; old moves to version history (never destroyed)
- **Multiple competing beliefs:** KRIA can hold "user MIGHT work at X or Y" with split confidence until resolved
- **Partial truths:** confidence < 1.0 memories flagged on retrieval, not hidden
- **Automatic revalidation:** volatile facts re-checked against source (filesystem/tool) during consolidation
- **Verification-on-retrieval:** codebase-invalidatable memories (verify_against: path) re-checked before use; if filesystem changed → demote + flag
- **Contradiction resolution protocol** (deterministic first): user-stated > recent-verified > higher Memory-Worth > else flag for user

### Conflict resolution order

```
1. User-stated beats inferred
2. More-recently-verified beats stale
3. Higher Memory-Worth (success correlation) beats lower
4. Ambiguous → keep both, surface to user for resolution
```

---

## 13. Unified Memory Lifecycle

Every memory type follows this lifecycle. Type-specific variations noted inline.

```
CREATE → VALIDATE → STORE → RETRIEVE → UPDATE → MERGE/SPLIT →
PROMOTE/DEMOTE → COMPRESS → SUMMARIZE → REFLECT → ARCHIVE →
EXPIRE → FORGET → DELETE → RESTORE → BACKUP → AUDIT
```

| Stage | Rule |
|---|---|
| CREATE | Via Write Policy Engine only. Emits an event. |
| VALIDATE | Dedup + contradiction + injection scan. |
| STORE | Append event → derive memory → index (atomic). |
| RETRIEVE | Multi-strategy; updates access_count + last_accessed (reconsolidation). |
| UPDATE | Never in-place on events; new event supersedes. Materialized view updates. |
| MERGE | Two memories → one, `derived_from: [both]`. Originals archived, not deleted. |
| SPLIT | One memory → several, each `derived_from: [original]`. |
| PROMOTE | High access + importance → permanent tier. |
| DEMOTE | Low decay + no access → archive candidate. |
| COMPRESS | Episode→skill→rule; provenance chain preserved. |
| SUMMARIZE | Old raw episodes → summary; detail archived. |
| REFLECT | Produces new meta-memories (reflections). |
| ARCHIVE | Cold tier; still queryable; embedding retained. |
| EXPIRE | Staleness-class-driven; flags for re-verification (not deletion). |
| FORGET | User command or critical decay → tombstone (reversible 30 days). |
| DELETE | Hard delete + cascade (vectors, graph edges, derived, caches). Export-before-delete for bulk. |
| RESTORE | From event log (rebuild) or backup. |
| BACKUP | Daily incremental + pre-risky-op snapshot. |
| AUDIT | Every lifecycle transition is an event — fully traceable. |

**Type-specific notes:** Episodic = immutable once closed. Procedural = never
auto-deleted. Goal = archived forever (life history). Library = no decay
(reference material). Working = never persisted (ephemeral).

---

## 14. Library System

A personal knowledge library, separate from experiential memory (different
lifecycle: documents don't decay, don't get "forgotten").

**Supports:** books, PDFs, EPUB, Word, Markdown, documentation, repositories,
source code, images, videos, audio, meeting transcripts, notes, knowledge bases,
folders, collections, datasets.

### Architecture

```
Files → Filesystem (~/.kria/library/{collection}/{sha256}/file)
Metadata → SQLite (library_items, library_chunks, collections)
Chunks → embedded → LanceDB
Extracted knowledge → entities/relationships (graph) + facts (semantic memory)
                      with provenance source="library:{item}:chunk:{idx}"
```

### Key behaviors

- **Adaptive chunking:** 512 chars dense text, 1024 code, by-section for structured docs
- **Large files:** streamed on ingest, never fully loaded into RAM
- **Deletion cascade:** delete item → chunks → vectors → filesystem file; sourced memories flagged `source_deleted` (user chooses keep-fact or cascade)
- **Contradiction:** new doc contradicting memory → surface to user; source-reliability weighting (user-stated > document > inferred)
- **Versioning:** new version appended, linked to previous; never lose old
- **Duplicate detection:** SHA-256 exact + title/author fuzzy
- **Re-indexing:** background, incremental; supports partial re-index on model upgrade
- **Cross-document reasoning:** GraphRAG-style community summaries (Microsoft pattern) for "what do my docs collectively say about X"
- **Citations:** every extracted fact traces to item + chunk + page

### Non-text modalities (schema now, implement later)

Images → CLIP/SigLIP embeddings; Audio → transcript + CLAP; Video → keyframes +
transcript. Stored in modality-partitioned LanceDB tables. Cross-modal retrieval
via shared embedding space. Schema carries `modality` + `embedding_model` from day one.

---

## 15. Subsystem Integration (Cognitive Backbone)

Memory is the backbone every KRIA subsystem reads from and writes through (via
the Write Policy Engine).

| Subsystem | Reads | Writes (via Policy Engine) |
|---|---|---|
| Intent Compiler | Preferences, past intents | — (stateless) |
| Planner | Goals, procedural skills, failures, capabilities | Reasoning traces |
| Reasoner | Semantic facts, world/user model | Inferred facts |
| Execution Engine | Tool affordances, execution patterns | Tool outcomes, events |
| Capability Platform (CKB) | (merged into memory) | Success rates, benchmarks |
| OpenClaw | Scoped read: capabilities + relevant user context | Via orchestrator only (never direct) |
| Discovery/Evolution | CKB health, benchmark trends | Proposals, decisions |
| Jobs | Job state | Job outcomes |
| Reflection Engine | Recent episodes, patterns | Reflections, meta-observations |
| Library | Document chunks | Extracted facts (provenance-tagged) |
| Desktop Context (PSDG) | Ambient state | Filtered episodic promotions |
| Workspace | Git/build/test state | Workspace observations |
| Frontend | Everything (via explain/debug API) | User edits/deletions/mode switches |
| Future Multi-Agent | Namespace-scoped shared "core" | Namespace-isolated discoveries |

**OpenClaw specifics:** Skills get read-only `SkillMemoryView` scoped to their
namespace + public "core". Skills NEVER write directly — results flow to the
orchestrator, which decides what to memorize. Each skill: `namespace: openclaw/{id}`.
Generated skills get fresh namespace but can read parent execution history from CKB.

**Multi-agent (future):** Every memory already carries `namespace` + `owner_id`.
Agents read shared "core", keep private namespaces isolated, promote discoveries
to core via approval. SQLite WAL handles concurrent reads; single writer
serializes writes (adequate for desktop scale).

---

## 16. Privacy & User Control

Users are always in control. Non-negotiable capabilities:

- **Delete** by: memory / project / workspace / document / book / plugin / date-range / subsystem
- **Forget** command (natural language) → tombstone, reversible 30 days
- **Disable** memory entirely (Read-only or Incognito mode)
- **Temporary / Workspace / Incognito** modes (Section 6)
- **Backup / Restore / Export / Import** (user owns the data, portable)
- **Consent:** sensitivity=secret writes require confirmation
- **Audit:** "what KRIA believes about you" monthly report; full provenance on any memory
- **Export-before-delete:** mandatory safety net for bulk deletions

**Never stored:** passwords/API keys/tokens (OS keychain reference only), full
file contents (path references), raw network traffic, keystroke logs, screen
recordings, incidental third-party private data, anything after "don't remember."

---

## 17. Security & Threat Model

Memory poisoning is now OWASP ASI06. MINJA attack achieves >95% injection success.
Defense-in-depth across the Write Policy Engine:

| Threat | Defense |
|---|---|
| Poisoning via conversation | Write-gate injection classifier + source tagging |
| Poisoning via documents | Doc sanitization, confidence cap for doc-sourced facts |
| Injection persisted as fact | Never store instruction-like text as facts (pattern detection) |
| Data exfiltration | Never store secrets; namespace isolation; sensitivity tags |
| Malicious embedding model | Pin model checksums, verify on load |
| Stale poisoned memory | Provenance chain → flag old low-provenance on retrieval |
| Malicious plugin | Strict namespace enforcement; plugins write ONLY to own namespace |
| False self-promotion | Refuse to write "rules" from correlated/insufficient evidence (arxiv:2607.02579) |

**Encryption:** SQLite + LanceDB at rest via OS-level encryption (LUKS/FileVault)
or optional SQLCipher; backups always encrypted (age/libsodium); sensitive fields
encrypted at column level.

---

## 18. Failure Handling

Memory must survive everything. **Core principle: raw events are always stored;
enrichment is best-effort.**

| Failure | Protection / Recovery |
|---|---|
| LLM unavailable | Degrade: heuristic extraction, formula importance, queue consolidation/reflection. Retrieval + storage unaffected. |
| Embedding unavailable | Store raw text; queue for embedding; FTS5 keyword search still works |
| Corrupted SQLite | integrity_check on startup → restore from daily backup |
| Corrupted vectors | LanceDB version rollback OR re-embed from SQLite text |
| Partial/interrupted backup | Atomic backup (temp + rename); verify checksum before marking valid |
| Power failure | WAL replay (SQLite) + append-only fragments (LanceDB) |
| Disk full | Capacity self-regulation: warn at 80%, aggressive archive at 95%, never unbounded |
| Millions of tiny writes | Write batching in Policy Engine (buffer + flush on idle) |
| Huge library ingestion | Streamed, rate-limited, background; never blocks interaction |
| Plugin crash | Isolated namespace; crash cannot corrupt core memory |
| Memory poisoning | Section 17 + provenance rollback via event log |
| Conflicting facts | TMS resolution protocol (Section 12) |
| Clock drift / timezone change | Store UTC always; display in local; events use monotonic UUID v7 ordering |
| Workspace deletion | Cascade delete workspace-scoped memories; keep global |
| Project rename | Entity alias (graph handles rename as new alias, not new entity) |
| Duplicate imports | SHA-256 dedup at Library ingest |
| Large binary files | Store reference + metadata, not content; skip embedding |

**Startup integrity check:** SQLite quick_check → LanceDB open-verify → event log
checksum tail → critical preferences present → offer repair if any fail.

---

## 19. Observability

Memory must be debuggable. Every decision explainable.

**explain_retrieval(query)** → strategies used, per-strategy results, fusion
scores, final ranking, budget allocation, what was injected, what was filtered and why.

**explain_memory(id)** → provenance chain, derived_from, contradicted_by, Memory
Worth, access history, staleness status, verification history, why stored, why not forgotten.

**memory_health_report()** → totals by type + staleness class, avg confidence,
knowledge gaps, low-worth memories, unresolved contradictions, pending LLM tasks,
disk usage, consolidation lag.

**Visibility into:** why stored / why rejected / why forgotten / why retrieved /
confidence / provenance / retrieval path / ranking / compression lineage /
reflection sources / importance / graph traversal path / vector hits.

---

## 20. Benchmarking

Measurable success criteria (no public benchmark fits a desktop assistant — build
a harness modeled on MemoryArena arxiv:2602.16313 + LongMemEval-style multi-session).

| Metric | Target |
|---|---|
| Retrieval p95 latency | <80ms @ 150K memories (5yr) |
| Startup time | <3s @ 5yr scale |
| Insertion latency | <5ms per memory |
| Graph 2-hop traversal | <5ms @ 10K nodes |
| Vector search | <20ms @ 1M vectors |
| Retrieval quality | Accuracy must not degrade as memory grows (the key test) |
| Consolidation | <5min daily @ 5yr |
| Storage efficiency | <6GB total @ 5yr |
| Compression ratio | episode 5-20×, skill 50-500×, rule 1000×+ |
| LLM cost | Batched; degrade to zero when unavailable |
| Reflection quality | Human-rated usefulness of generated insights |

**The single most important benchmark:** retrieval quality as a function of
memory-bank size. If quality degrades as KRIA remembers more, the architecture fails.

---

## 21. Long-Term Scalability

| Metric | 1yr | 5yr | 10yr | 20yr |
|---|---|---|---|---|
| Active memories | 15K | 150K | 400K | 800K |
| Vectors | 20K | 250K | 700K | 1.5M |
| Graph nodes | 500 | 10K | 30K | 80K |
| Total disk | 600MB | 6GB | 12GB | 35GB |
| Retrieval p95 | 30ms | 80ms | 120ms | 200ms |

**What breaks and when:**
- Nothing breaks at 5yr with current design.
- At 10yr+: graph MAY need dedicated engine (GraphStore trait handles swap).
- At 10yr+: tiered vector storage (hot RAM cache + cold disk) becomes worthwhile.
- Embedding model WILL change 2-3× over 10yr → version partitioning (Section 9) handles it.

**Optimizations reserved for scale:** tiered vectors, monthly LanceDB compaction,
episode summarization after 1yr, archive partition (separate cold SQLite file),
graph pruning of orphan entities.

---

## 22. Future-Proofing

The architecture supports these WITHOUT redesign (event log + trait ports + namespaces):

| Future capability | Enabled by |
|---|---|
| Multimodal (image/audio/video) | `modality` + `embedding_model` schema fields (day one) |
| Voice/vision memory | Modality-partitioned LanceDB tables |
| Multiple devices / cloud sync | Event log + CRDT-compatible design + owner_id/device_id |
| Multiple users | namespace + owner_id (already present) |
| Shared workspaces / team memory | visibility=Team namespace |
| Knowledge federation | Import/export + provenance |
| Better embedding models | Version partitioning + background re-embed |
| Future vector/graph DBs | Trait ports (swap backend, zero caller change) |
| Future LLMs / reasoning engines | LLM accessed via trait; memory is model-agnostic |
| Digital user twin / world models | Buildable from event log + graph (out of v1 scope) |
| Robotic integration | Spatial + sensor memory as new types via Write Policy Engine |

---

## 23. Technology Decisions & Tradeoffs

| Component | Choice | Confidence | Tradeoff accepted |
|---|---|---|---|
| Relational + graph + FTS | SQLite (WAL, FTS5, CTEs) | High | Graph via SQL not Cypher (uglier, but zero new dep) |
| Vectors | LanceDB | High | Newer than usearch, but disk-native + versioned + append-only |
| Embeddings | EmbeddingGemma-300M ONNX | Medium-High | Larger than MiniLM (300M vs 22M), but far better multilingual + Matryoshka |
| Inference | ONNX Runtime (`ort`) | High | — |
| Async | Tokio | High | — |
| Caches | dashmap (P1), moka (P2) | High | — |
| Parallel batch | rayon (P4) | High | Add only when batch jobs exist |
| Event log | SQLite append-only table | High | Storage growth (mitigated by archival) |
| Graph abstraction | GraphStore trait | High | Slight upfront abstraction cost |
| Consolidation | LLM dreaming (local llama.cpp) | High | LLM dependency (mitigated by degradation) |
| Governance metric | Memory Worth (2 counters) | Medium-High | Simple but proven (arxiv:2604.12007) |

**Crates by phase:** P1: rusqlite, lancedb, ort, tokio, serde, chrono, uuid,
blake3, tracing, anyhow/thiserror, dashmap · P2: moka · P4: rayon.
**Rejected:** parking_lot (near-zero contention in single-writer model).

---

## 24. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| No production validation yet | High | Build benchmark harness early (P2); dogfood aggressively |
| LanceDB relatively young | Medium | Append-only + versioned = safe; trait port allows swap |
| Consolidation quality = LLM quality | Medium | Degrade gracefully; human-rate reflection output |
| Embedding model obsolescence | Medium | Version partitioning designed day one |
| Write Policy Engine becomes bottleneck | Medium | Batching + async; it's the spine, invest in it |
| Graph outgrows SQLite | Low (10yr+) | GraphStore trait swap |
| Memory poisoning | Medium | Defense-in-depth (Section 17) |
| Scope creep (over-building cognition early) | Medium | Phased roadmap; ship storage+policy first |

---

## 25. High-Level Implementation Roadmap

Concise phases only. No sprint-level detail.

**Phase 1 — Foundation & Spine**
Core storage (SQLite unified + LanceDB), event log, **Memory Write Policy Engine**,
Memory Modes, trait-based storage ports. Migrate current 5 DBs → 1. Replace
brute-force VectorIndex → LanceDB. LLM-failure degradation paths.

**Phase 2 — Intelligence & Governance**
Multi-strategy retrieval + adaptive fusion + token budget, Truth Maintenance,
importance scoring + Memory Worth, dreaming/consolidation, deletion granularity +
undo + export, observability/debug API, benchmark harness.

**Phase 3 — Cognition & Relationships**
Graph (entities/relationships + GraphStore trait), goals (recurring/ambitions) +
temporal NL resolver, salience/attention loop, knowledge-gap tracker, episode
boundaries, progressive compression, embedding version partitioning.

**Phase 4 — Library & Knowledge**
Library ingestion + cascade + citations, document intelligence (entity/relation
extraction), GraphRAG-style cross-document summaries, cross-encoder rerank for
Library QA only, rayon batch jobs.

**Phase 5 — Multimodal & Advanced**
Multimodal embeddings (image/audio/video), memory management UI (2D graph,
merge/split, inspector), advanced dreaming.

**Phase 6+ — Horizon**
3D visualization (optional plugin), multi-device sync (CRDT), multi-agent
shared memory, digital-twin/world-model exploration.

---

## 26. Final Requirements Audit (30 Requirements)

**Legend:** ✅ Fully covered · 🟡 Partial · ❌ Missing

| # | Requirement | Status | Section |
|---|---|---|---|
| 1 | Fundamental Goals | ✅ | 1, 2 |
| 2 | Long-Term Scalability | ✅ | 21 |
| 3 | True Cognitive Memory | ✅ | 11 |
| 4 | Memory Types (all, with lifecycle) | ✅ | 3, 13 |
| 5 | Memory Classification (orthogonal axes) | ✅ | 4 |
| 6 | Memory Lifecycle (full) | ✅ | 13 |
| 7 | Retrieval Intelligence | ✅ | 10 |
| 8 | Memory & Reasoning Integration | ✅ | 15 |
| 9 | Memory Intelligence (self-improvement) | ✅ | 11, 12 |
| 10 | Entity & Relationship Understanding | ✅ | 8 |
| 11 | Temporal Intelligence | ✅ | 10, 12 (NL resolver P3) |
| 12 | Goal Awareness (recurring/ambitions) | ✅ | 3, 13 |
| 13 | Library System | ✅ | 14 |
| 14 | OpenClaw Integration | ✅ | 15 |
| 15 | Scalability & Extensibility | ✅ | 22 |
| 16 | Backup & Disaster Recovery | ✅ | 18 |
| 17 | Security & Privacy | ✅ | 16, 17 |
| 18 | Memory Management UI | 🟡 | 25 (P5); 3D deferred to P6 |
| 19 | Performance Targets | ✅ | 20 |
| 20 | Technology Selection (evidence-based) | ✅ | 23 |
| 21 | Future Readiness | 🟡 | 22 (schema-ready; multimodal impl P5) |
| 22 | Truth Maintenance | ✅ | 12 |
| 23 | Graceful LLM Degradation | ✅ | 18 |
| 24 | Cold Start Experience | ✅ | see below |
| 25 | Memory Modes | ✅ | 6 |
| 26 | Intelligent Write Filtering | ✅ | 5 |
| 27 | Memory Write Governance | ✅ | 5 |
| 28 | Deletion & Ownership | ✅ | 16, 13 |
| 29 | Explainable Memory | ✅ | 19 |
| 30 | Final Architecture Goals | ✅ | all |

### Cold Start (Req 24) — covered

First-run value in minutes: onboarding questions (name, projects, languages) +
workspace scan (Cargo.toml/package.json/.git detection → instant world model) +
optional import (Claude Code CLAUDE.md, Cursor rules, git history, shell history)
+ aggressive extraction for first 50 turns + privacy explanation + storage estimate.

### The Two Remaining Partials

- **Req 18 (Memory Management UI):** Backend fully supports it (explain/debug API, merge/split lifecycle). UI itself is Phase 5; 3D graph is Phase 6 optional plugin (2D force-directed is more useful — 3D reserved as a "wow" feature, not core).
- **Req 21 (Future Readiness — multimodal):** Schema carries `modality` + `embedding_model` from day one, so no redesign needed. Actual image/audio/video ingestion is Phase 5. This is a deliberate deferral, not a gap.

### Verdict

**28/30 fully covered, 2/30 partial (both deliberate deferrals with schema
readiness). Zero missing.** The three previously-critical gaps — Memory Write
Policy Engine, Memory Modes, and multimodal-readiness — are now closed: two
implemented in the design (Sections 5, 6), one made future-safe via schema fields.

This architecture can realistically serve as KRIA's memory system for the next
10+ years. It prioritizes production reliability, local-first operation,
extensibility, maintainability, and genuine intelligence over novelty.

**Final architecture score: 9/10.** The missing point is honest: no design
survives contact with production unchanged. Build Phase 1-2, benchmark, and let
evidence refine the rest.

---

*Definitive blueprint. Supersedes all prior memory architecture documents.*

*Key evidence: SQLite (30yr track record), LanceDB (Lance 2.1 stable, append-only,
Apache 2.0), EmbeddingGemma-300M ONNX (arxiv:2509.20354), Experience Compression
Spectrum (arxiv:2604.15877), Memory Worth (arxiv:2604.12007), MemoryArena bench
(arxiv:2602.16313), CoMem decoupled memory (arxiv:2605.30842), Anthropic Dreaming
(May 2026), OpenAI Dreaming V3 (June 2026, 41.5%→82.8%), OWASP ASI06 memory
poisoning, MINJA attack (arxiv:2601.05504), false-promotion governance
(arxiv:2607.02579). Content rephrased for compliance with licensing restrictions.*


---
---

# SECTION 27 — RED TEAM RESOLUTIONS (Known Issues 1-30)

Each issue is verified genuine, root-caused, its production failure named, and
resolved with a chosen solution that introduces no new contradiction.

## Issue 1 — Event-sourcing rebuildability contradiction

**Genuine? YES.** LLM extraction is non-deterministic; replaying events cannot
reproduce identical derived memories.

**Root cause:** The doc conflated two different things — the *event log* (raw,
deterministic truth) and *derived memories* (LLM output, non-deterministic).

**Production failure:** After corruption, "rebuild from event log" silently
produces DIFFERENT memories → user's KRIA subtly changes personality/knowledge
after every recovery. Erodes trust; undebuggable.

**Resolution (chosen):** **Derived memories are ALSO durable state, not
rebuildable artifacts.** The event log is an audit/provenance/erasure ledger, NOT
a rebuild source. Both the event log AND the derived memory tables are backed up
and restored together as a consistent snapshot. Replay is used ONLY for forensic
inspection ("what did KRIA see"), never to regenerate memories.

- Rejected alt: pin LLM + temperature=0 for determinism → still breaks on model upgrade; discard.
- Rejected alt: store extraction output alongside each event → that IS the derived table; simpler to just persist it.

**New contradiction check:** None. This clarifies roles; nothing else depends on
LLM-replay. Section 13 RESTORE updated: "Restore from backup snapshot (events +
derived state together); replay is forensic-only."

## Issue 2 — Immutable event log vs Right-to-be-forgotten

**Genuine? YES.** Immutable append-only log cannot honor GDPR Article 17 erasure.

**Root cause:** Immutability and erasure are fundamentally opposed if data is stored in plaintext.

**Production failure:** Legal non-compliance; user "forget my ex-employer" leaves
the data recoverable in the log → privacy violation, potential liability.

**Resolution (chosen): Crypto-shredding** (mature pattern — Kafka, Axon, MongoDB
CSFLE all use it). Each erasure-scoped subject (person, employer, project, session)
gets a unique encryption key. Sensitive event payloads are encrypted per-subject
key. "Forget X" = destroy X's key → ciphertext remains in the immutable log but
becomes cryptographically unreadable = effectively erased (GDPR Recital 26:
irreversibly anonymized data is out of scope).

- Key store: separate from event log (a shreddable keyring in SQLite or OS keychain).
- Rejected alt: log compaction/rewrite → breaks immutability + provenance chains.
- Rejected alt: tombstone-only → data still recoverable, fails GDPR.

**New contradiction check:** Preserves immutability (log unchanged) AND enables
erasure (key gone). Derived memories referencing the subject cascade-delete
normally (they're mutable). Consistent.

## Issue 3 — SQLite ↔ LanceDB consistency

**Genuine? YES.** Two independent stores, no shared transaction → orphaned rows/vectors.

**Root cause:** Classic dual-write problem across two embedded engines.

**Production failure:** Memory row exists but embedding missing (invisible to
vector search) OR vector exists but memory deleted (retrieval returns dangling
ID → crash or ghost result). Drift compounds over years.

**Resolution (chosen): Transactional Outbox + SQLite-as-authority.**
- SQLite is the SINGLE source of truth. LanceDB is a REBUILDABLE index (not truth).
- A write commits to SQLite in one transaction, including an `embedding_outbox` row.
- A background relay reads the outbox, writes to LanceDB, marks the row done.
- Crash between = outbox row replays (idempotent by memory_id). No lost vectors.
- Orphan sweep: periodic reconcile (SQLite memory_ids vs LanceDB ids) repairs drift.
- Because LanceDB is a rebuildable index, Issue 1's concern does NOT apply here —
  vectors are deterministically re-derivable from stored memory text.

- Rejected alt: 2-phase commit across engines → neither supports it; impossible.
- Rejected alt: write LanceDB first → orphaned vectors on SQLite failure.

**New contradiction check:** Consistent. Note the asymmetry: LanceDB vectors ARE
rebuildable (deterministic embedding of stored text); LLM-derived memories are NOT
(Issue 1). Both statements hold — different data, different determinism.

## Issue 4 — Write Policy Engine bottleneck & SPOF

**Genuine? YES.** One synchronous gate doing embed + vector dedup + contradiction
+ LLM classify + security scan per write will block; one bug breaks all writes.

**Root cause:** Conflated cheap synchronous validation with expensive async enrichment.

**Production failure:** UI stalls waiting for memory writes; or a policy bug halts
all learning silently.

**Resolution (chosen): Two-phase fast-path / slow-path split.**
- **Fast path (synchronous, <2ms, deterministic, MUST succeed):** mode check,
  namespace/ownership, security pattern scan, append raw event to log + outbox.
  This is all that blocks the caller. If enrichment later fails, the raw event
  is already safely stored.
- **Slow path (async worker pool, best-effort):** embedding, dedup, contradiction,
  classification, importance, graph update. Consumes from the event log.
- SPOF mitigation: fast path is tiny + heavily tested + has no LLM dependency.
  Slow-path failure degrades enrichment but never loses data or blocks the user.

**New contradiction check:** Aligns with "raw events always stored; enrichment
best-effort" (Section 18) and LLM degradation (Issue). Consistent.

## Issue 5 — Missing embedding step in write lifecycle

**Genuine? YES.** Section 5 step 4 dedup needs an embedding that was never generated.

**Resolution:** Embedding moves to the SLOW path (Issue 4), BEFORE dedup. Corrected
slow-path order: `embed → dedup → contradiction → classify → importance → provenance
→ graph → commit-derived`. Fast path never embeds (that's why it's <2ms).

## Issue 6 — Memory Worth credit-assignment bias

**Genuine? YES.** Hard tasks fail more AND retrieve more memories → all memories in
hard tasks unfairly penalized. Co-occurrence ≠ causation.

**Root cause:** Naive 2-counter co-occurrence ignores task difficulty + retrieval-set size.

**Production failure:** Genuinely useful memories used on hard problems get
demoted/archived → KRIA forgets exactly what it needs for its hardest work.

**Resolution (chosen): Normalized contribution, not raw co-occurrence.**
- Credit is divided across the retrieval set: each of N retrieved memories gets
  1/N of the outcome signal (not full credit) — dilutes spurious correlation.
- Difficulty-adjusted: weight the signal by task difficulty prior (hard-task
  failures penalize less).
- Require a minimum sample (≥20 observations) before Memory Worth influences
  retrieval or archival — avoids early noise.
- Memory Worth is a soft re-ranking signal + archival hint, NEVER a hard-delete trigger.

- Rejected alt: keep raw counters → proven biased.
- Rejected alt: full causal attribution (counterfactual) → too expensive per turn.

**New contradiction check:** Consistent with governance goals; softer, safer.

## Issue 7 — Continuous salience loop battery/thermal

**Genuine? YES.** Embedding + vector search every 10s all day drains laptop battery;
"only when idle" is worst — idle should let the CPU sleep.

**Root cause:** Polling model on battery-constrained hardware.

**Production failure:** Users disable KRIA because it drains battery / spins fans.

**Resolution (chosen): Event-driven, not polling; power-aware.**
- Salience triggers on MEANINGFUL context-change events (file opened, app focus
  changed, new user message) — NOT a 10s timer.
- Debounced (max once per ~60s) + coalesced.
- Power-aware: on battery below threshold, or "power saver" OS state → salience
  disabled entirely; only on-demand retrieval runs.
- Uses cached context embedding; re-embeds only when context text actually changes.

**New contradiction check:** Proactive recall still works (event-driven), now
battery-safe. Consistent with local-first/desktop constraints.

## Issue 8 — Cold-start scanning before consent

**Genuine? YES.** Scanning filesystem/git/shell on first run before consent violates privacy-first.

**Resolution (chosen): Consent-gated, opt-in, scoped.**
- First run shows an explicit consent screen BEFORE any scan: what will be read,
  where it's stored, that it's local-only, with per-source toggles (filesystem/
  git/shell/none) and folder exclusions.
- Nothing is read until the user approves. Default = minimal (onboarding questions only).
- Scan results are previewable + deletable before commit.

**New contradiction check:** Aligns with Section 16 consent model. Corrected.

## Issue 9 — Undefined importance scoring model

**Genuine? YES.** `importance>7` used everywhere; formula absent from this doc.

**Resolution (defined here):** Importance is a 0-10 score, computed deterministically
at write time, recalibrated during consolidation:
```
importance = 10 * sigmoid(
    0.30*novelty + 0.25*goal_relevance + 0.20*source_authority +
    0.15*emotional/emphasis_signal + 0.10*surprise )
```
- novelty = 1 - max_similarity_to_existing (from dedup step, free)
- goal_relevance = cosine(memory, active_goals)
- source_authority = {user_stated 1.0, tool_verified 0.8, document 0.6, inferred 0.4}
- emphasis = user markers ("important", "remember", repetition)
- surprise = contradiction with prior expectation
Deterministic (no LLM). LLM may nudge ±2 for genuinely ambiguous cases only.
Recalibrated by access frequency + Memory Worth during consolidation.

## Issue 10 — Undefined "Session" semantics

**Genuine? YES.** Episodic boundaries, crash recovery, session modes all depend on it.

**Resolution (defined):** A session is an interaction span delimited by:
- START: first user input after app launch OR after >2h inactivity gap.
- END: explicit close, app quit, OR 2h inactivity (whichever first).
- For 24/7-open KRIA: a "logical session" also rolls over at local midnight to
  bound episode size. Long active sessions are chunked into ≤4h episodes.
- Session ID is UUID v7; every event carries it. Crash = session left "open";
  startup detects open sessions <24h old and offers resume (Section 18).

## Issue 11 — Undefined decay × importance interaction

**Genuine? YES.** Both fields exist; combination unspecified.

**Resolution (defined):**
```
effective_retention = importance_weight * recency * frequency * memory_worth
decay_rate ∝ 1 / (1 + importance)     // high importance → near-zero decay
staleness_class overrides decay:
    Immutable/Permanent → decay disabled entirely (importance irrelevant)
    Slow/Volatile → decay applies, modulated by importance
```
Archival candidate when `effective_retention < archive_threshold AND
staleness_class ∈ {Slow, Volatile} AND no_access > 30d`. Importance and decay are
thus unified: importance sets the decay rate; staleness class can veto decay.

## Issue 12 — SQLite recursive graph traversal assumptions

**Genuine? YES.** `<5ms @ 10K nodes` unverified; no cycle detection (infinite recursion risk).

**Root cause:** Recursive CTEs don't index-optimize traversal well; graphs have cycles.

**Production failure:** A relationship cycle (A→B→A) causes infinite recursion →
query hang → memory subsystem freeze.

**Resolution (chosen):**
- **Cycle safety:** every traversal CTE carries a visited-set (path column) and a
  hard depth cap (default 3). Mandatory, not optional.
- **Performance honesty:** the `<5ms` claim is REMOVED as unproven. Replaced with:
  "benchmark-gated — if 2-hop traversal exceeds 25ms at target scale, activate the
  GraphStore trait swap." Bidirectional index on (source_id) and (target_id).
- **Denormalized 2-hop cache** for hot entities (user, active project) refreshed
  on graph write — avoids repeated traversal for the most common queries.

**New contradiction check:** GraphStore trait already enables the swap. Consistent.

## Issue 13 — Reflection/dreaming self-poisoning

**Genuine? YES.** LLM consolidation can hallucinate false "lessons" that then
poison retrieval. Only rule-writes were gated (false-promotion guard).

**Root cause:** Self-generated memory bypassed the same scrutiny as external input.

**Production failure:** KRIA "learns" a wrong pattern from one coincidence, then
confidently applies it for years — compounding error.

**Resolution (chosen): All reflection/dreaming output re-enters through the Write
Policy Engine as untrusted `source: self_reflection`** (NOT auto-trusted).
- Reflections require ≥N supporting episodes (evidence threshold) before promotion to rule.
- Reflection-derived memories start at capped confidence (≤0.6) and must earn
  higher confidence via Memory Worth (utility over time).
- Contradiction check runs against existing memory — a reflection that contradicts
  user-stated facts is rejected.
- Reflection-of-reflection depth capped (prevents runaway abstraction, see Issue 28-independent).

**New contradiction check:** Self-generated memory now equals external memory in
scrutiny. Closes the gap without disabling learning.

## Issue 14 — Event-log growth estimation

**Genuine? YES.** Scaling table (Section 21) counted memories/vectors but NOT events;
append-only + continuous desktop activity = potentially millions/year.

**Resolution (defined + corrected estimate):**
- Event volume estimate: ~500K-2M events/year (desktop activity dominates).
- **Tiered event log (does NOT break immutability):** recent events in hot SQLite;
  events older than 90 days rolled to compressed, still-immutable, still-append-only
  cold segments (Parquet/zstd files, one per month). Crypto-shred keys still apply.
- Cold segments are read-only archives, never mutated → immutability preserved.
- Corrected 10yr event-log size estimate: ~8-15GB compressed (added to Section 21).

**New contradiction check:** "Archival" here = moving immutable segments to cold
storage, NOT deleting/rewriting. Immutability intact. Consistent with Issue 2.

## Issue 15 — Verifiable vs non-verifiable volatile memories

**Genuine? YES.** "Re-verify volatile after 1h" assumes verifiability; "user seemed
frustrated" has no cheap re-check.

**Resolution (defined):** Split the Volatile class:
- **Volatile-Verifiable:** has a `verify_against` predicate (filesystem, tool, git).
  Auto-revalidated during consolidation.
- **Volatile-Unverifiable:** no external check (moods, ephemeral intent). These
  simply DECAY fast and are never "confirmed" — surfaced with low confidence +
  timestamp ("~2h ago you seemed…"), never asserted as current truth.

**New contradiction check:** TMS now handles both. Consistent.

## Issue 16 — Overstated multimodal readiness

**Genuine? YES.** `modality` + `embedding_model` fields do NOT make the system
multimodal; cross-modal retrieval, fusion, quality filters, graph extraction are
all text-designed.

**Resolution (honest reframing):** Downgrade claim from "schema-ready, no redesign"
to **"storage-ready, pipeline-redesign-required."** What day-one schema fields DO
buy: no destructive migration of stored data. What they DON'T buy: working
multimodal retrieval. Multimodal (Phase 5) requires: a shared/aligned embedding
space (e.g., a unified multimodal model), modality-aware fusion, and modality-aware
quality filtering. Marked explicitly as a future work-package, not a free extension.

## Issue 17 — Architecture confidence claims

**Genuine? YES.** 9/10 for an unvalidated, novel-combination architecture is overconfident.

**Resolution:** Corrected to **7.5/10 post-Red-Team** (from honest 6.5 pre). Rationale
in Section 31. No score above 8 is claimable until Phase 1-2 are benchmarked in
production. Executive Summary updated.

---

## Issues 18-30 — Resolutions (concise)

**18 — Schema evolution across years.** Resolution: additive-only schema migrations
(never drop/rename columns; add new + deprecate). A `schema_version` table + forward
migration scripts run on startup. Derived-memory schema changes trigger targeted
re-derivation from the event log (allowed — this is enrichment, not memory identity).

**19 — Embedding-model coexistence.** Resolution: already handled by version
partitioning (Section 9) + Issue 3's rebuildable-index property. Multiple model
tables coexist; dual-search during migration; background re-embed. Add: cap
concurrent model versions at 2 (current + previous) to bound complexity.

**20 — Backup compatibility across versions.** Resolution: backups carry a
`format_version` + full schema snapshot. Restore path includes forward-migration of
old backups. Backups are self-describing (schema embedded), never assume current code.

**21 — Partial/selective restore.** Resolution: backup is segmented by namespace +
time-range, enabling "restore only project X" or "restore only last week." Event log
segmentation (Issue 14) makes selective restore natural.

**22 — Multi-device sync conflicts.** Resolution (future): event log is the sync
unit. Events are content-addressed (BLAKE3) + UUID v7 ordered + carry device_id.
Merge = union of event logs, ordered by hybrid logical clock (NOT wall clock — see
Issue 29-independent on clock drift). Derived memories rebuilt per-device from merged
log. CRDT semantics for preferences (last-writer-wins per key with vector clocks).
Design-ready; implementation Phase 6.

**23 — Offline-first reconnect sync.** Resolution: each device accumulates events
offline; on reconnect, exchange missing event ranges (by UUID v7 watermark), merge,
re-derive. No central authority needed (peer-to-peer event union). Conflict on
derived state resolved by re-derivation, not manual merge.

**24 — Workspace isolation guarantees.** Resolution: namespace is enforced at the
Write Policy Engine (fast path) AND at retrieval (mandatory namespace filter).
Workspace mode rejects cross-namespace writes. Isolation is a query-time invariant,
not a convention. Add: a test-suite invariant that no retrieval returns cross-namespace
memories unless explicitly global.

**25 — Library versioning & re-indexing.** Resolution: library items are versioned
(new version = new immutable record + link to prior). Re-indexing is incremental +
resumable (checkpoint per chunk). Interrupted re-index resumes from checkpoint. Old
version's chunks/vectors retained until new version fully indexed (atomic swap).

**26 — Memory corruption recovery.** Resolution: layered. (a) SQLite integrity_check
+ restore from backup. (b) LanceDB = rebuildable index → rebuild from SQLite text.
(c) Graph = in SQLite → same protection. (d) Event log cold segments = checksummed;
corrupt segment quarantined, rest usable. Partial corruption never total loss.

**27 — Catastrophic disk/DB corruption.** Resolution: 3-2-1 backup discipline (daily
local + user-configured external + optional encrypted cloud). Backups are
self-describing + checksummed + test-restored periodically (a backup never tested is
not a backup). Worst case = roll back to last good daily snapshot (≤24h loss).

**28 — Event replay after crashes.** Resolution: replay is FORENSIC-only (Issue 1),
never for memory regeneration. Crash recovery = SQLite WAL replay + outbox drain
(Issue 3) + resume open sessions (Issue 10). Idempotent by event UUID. No
double-application (dedup on replay by event id).

**29 — Atomicity across backends.** Resolution: there is NO cross-backend atomicity
requirement anymore, BY DESIGN. SQLite is the only transactional authority (events +
memories + graph + outbox all in ONE SQLite transaction). LanceDB is a downstream
rebuildable index fed by the outbox. This collapses the distributed-atomicity problem
into a single-database local transaction. **This is the single most important
structural fix of the Red Team pass.**

**30 — Long-term observability & forensic debugging.** Resolution: the event log IS
the forensic record. Every memory links to its source event(s). `explain_memory`
walks the full chain. Add: a structured memory-audit log (separate from event log)
recording every Write Policy decision (stored/rejected/why) for debugging the policy
itself. Retention: 90 days rolling.

---
---

# SECTION 28 — INDEPENDENT ADVERSARIAL REVIEW (New Problems)

*Assume a different team built this. Try to destroy it. Findings below are NEW
(not in Issues 1-30), each with resolution.*

## N1 — Circular dependency: Write Policy Engine needs retrieval, retrieval needs memories

**Problem:** The slow-path Write Policy does dedup + contradiction, which require
RETRIEVAL (vector + graph). But retrieval reads the very store being written. On
first-ever writes (empty store) this is fine, but under concurrent load the policy
engine and the retrieval engine call each other → potential deadlock on the single
SQLite writer.

**Resolution:** Retrieval is READ-ONLY (uses SQLite read connections + LanceDB
reads — no writer lock). The Write Policy's dedup/contradiction use these read
paths. Only the final COMMIT takes the single writer. Reads never block on the
writer (WAL). No cycle at the lock level. Documented as an invariant: **the slow
path reads freely but holds the writer only for the atomic commit.**

## N2 — Consolidation vs live-write starvation on single writer

**Problem:** Dreaming/consolidation can hold long write transactions (rewriting
compressed memories). With one serialized writer, a live user write could wait
minutes behind a consolidation batch → perceptible stall.

**Resolution:** **Write prioritization + chunked background writes.** Background
consolidation writes in small batches (≤100 rows/txn), yielding the writer between
batches. Live user writes get priority via a two-queue scheduler (foreground queue
drained before background). No single background transaction exceeds ~50ms. This is
a scheduler invariant, not best-effort.

## N3 — Reflection amplification / infinite consolidation loop

**Problem:** Reflection produces memories. Consolidation processes memories,
including reflections. A reflection about reflections about reflections → unbounded
abstraction growth + compute loop + memory bloat.

**Resolution:** (a) Compression-level ceiling: level 3 (Rule) is terminal — rules
are never further compressed. (b) Reflections are excluded from being SOURCE
material for new reflections beyond one meta-level (reflection-of-reflection depth
= 1 max). (c) Consolidation is idempotent: re-running on unchanged input produces
no new memories (content-hash dedup). Loop mathematically bounded.

## N4 — Retrieval degradation as memory grows (the existential risk)

**Problem:** At 400K memories, even good ANN returns "20 relevant" but the SIGNAL
is diluted — more near-duplicates, more stale variants, more competing versions.
Precision degrades even if latency doesn't. This is THE way memory systems fail at
scale (fountaincity 2026: "retrieval returning wrong memories" kills agents at 6 months).

**Resolution:** (a) Aggressive dedup + supersession (Issue 12/TMS) keeps only the
current version retrievable; superseded versions are archived, excluded from default
retrieval. (b) Importance + Memory Worth gate the candidate pool BEFORE fusion. (c)
The mandatory benchmark (Section 20) is precisely "retrieval quality vs bank size" —
if it degrades, that gates release. **This is explicitly the #1 metric that can
fail the architecture, and it is now a release gate, not an afterthought.**

## N5 — Identity resolution failure (entity merge/split ambiguity)

**Problem:** "John from work" and "John Smith" and "JS" — are they one entity or
three? Wrong merge = conflated memories about different people (privacy + correctness
disaster). Wrong split = fragmented knowledge.

**Resolution:** Entity resolution is CONSERVATIVE by default (prefer separate
entities over wrong merge). Merges require either (a) high-confidence signal
(same email/handle) or (b) user confirmation. Merges are REVERSIBLE (entities carry
`merged_from` provenance; split restores). Never auto-merge people on name similarity
alone. Documented: **wrong-merge is worse than no-merge for people; bias toward split.**

## N6 — Goal conflict & goal explosion

**Problem:** Goal inference (Section 11) creates goals from activity. Over years →
hundreds of stale/contradictory inferred goals ("learn Rust" vs "abandoned Rust").
Proactive assistance driven by conflicting goals = noise.

**Resolution:** (a) Inferred goals start as low-confidence "candidate goals," only
promoted to active on user confirmation or repeated strong signal. (b) Goal conflict
detection (two active goals with opposing intent) surfaces to user. (c) Hard cap on
active goals (e.g., 20); beyond that, lowest-priority auto-paused. (d) Goals decay
to "paused"→"abandoned" without activity (Section 3). Prevents goal bloat.

## N7 — SSD write amplification / wear from event log + WAL

**Problem:** Append-only event log + WAL + LanceDB fragments + frequent small writes
= high write volume. On consumer SSDs over 10 years, write amplification could
matter (though modern SSDs tolerate ~600TBW+).

**Resolution:** (a) Write batching (Issue 4/N2) coalesces small writes. (b) Desktop
activity events are debounced/sampled before hitting the log (not every focus change).
(c) WAL checkpoint tuning (batch commits). Estimated write volume: <50GB/year →
negligible vs SSD endurance. Flagged as monitored, not critical. Non-issue with batching.

## N8 — Privacy leak via embeddings (embeddings are invertible)

**Problem:** Research shows text embeddings can be partially inverted to reconstruct
source text. If LanceDB vectors are less protected than SQLite (e.g., unencrypted
while SQLite is encrypted), embeddings become a side-channel leak of "forgotten" or
sensitive content.

**Resolution:** (a) LanceDB gets the SAME encryption-at-rest as SQLite (OS-level or
app-level) — never a weaker tier. (b) Crypto-shredded content's embedding is ALSO
purged on erasure (the outbox reconcile removes vectors for shredded memories). (c)
sensitivity=secret memories: embedding stored encrypted or not at all (keyword-only
retrieval for secrets). Closes the embedding side-channel.

## N9 — "Split-brain" after restore on a second device

**Problem:** User restores a backup on device B while device A keeps running. Now
two divergent event logs claim to be authoritative → split-brain on future sync.

**Resolution:** Each device has a stable device_id; event UUIDs are device-scoped +
hybrid-logical-clock ordered. Restore does NOT claim authority — it seeds device B's
log, and future sync (Issue 22/23) MERGES by event union, not overwrite. There is no
single "authoritative" copy — the merged event set is truth. Split-brain becomes a
normal merge, not a conflict. (Deferred to Phase 6 with sync, but design is sound now.)

## N10 — Timezone / DST / clock-drift corrupting temporal reasoning

**Problem:** "What did I do yesterday" breaks across DST shifts, timezone travel, or
clock drift. Wall-clock ordering of events is unreliable.

**Resolution:** (a) Store BOTH UTC instant AND originating timezone offset per event.
(b) Event ORDERING uses UUID v7 (monotonic, drift-tolerant) + a hybrid logical clock,
NEVER wall-clock comparison. (c) Temporal QUERIES ("yesterday") resolve in the user's
CURRENT local timezone against stored UTC. (d) Clock drift/backward jumps handled by
HLC (logical component increments even if wall clock goes backward). Temporal
correctness decoupled from wall-clock reliability.

## N11 — Massive document import blocking / OOM

**Problem:** User imports a 5GB repository or 2000-page PDF. Naive ingestion → OOM or
hours-long UI freeze.

**Resolution:** Library ingestion is streamed, chunked, checkpointed, rate-limited,
and fully background (Section 14). Import is a resumable JOB (survives restart). UI
shows progress. Memory-mapped reads; never load whole file. Per-file size guard
warns above threshold. Already covered; reaffirmed as hard requirement.

## N12 — Orphans everywhere (vectors, graph nodes, chunks, keys)

**Problem:** Deletions cascade imperfectly → orphaned vectors, dangling graph edges,
library chunks with no parent, crypto-keys for deleted subjects.

**Resolution:** A single periodic **reconciliation sweep** (weekly) walks every
store and repairs referential integrity against SQLite (the authority): orphan
vectors purged, dangling edges removed, parentless chunks deleted, unused keys
shredded. Because SQLite is the sole authority (Issue 29), orphan detection is a
straightforward "exists in index but not in authority" scan. Made a first-class
maintenance worker, not an afterthought.

## N13 — Confidence inflation feedback loop

**Problem:** Retrieval boosts high-confidence memories → they get used more → Memory
Worth rises → confidence rises → used even more. Rich-get-richer; a wrong-but-
confident memory becomes unkillable.

**Resolution:** (a) Confidence gains from utility are LOGARITHMIC + capped below 1.0
for non-user-stated facts (only user confirmation reaches ~1.0). (b) Periodic
"challenge" during consolidation: high-confidence unverified facts are re-checked
against sources / surfaced for user confirmation. (c) Contradiction always dents
confidence regardless of prior. Breaks the runaway loop.

## N14 — Consolidation/reflection interrupted mid-run (partial state)

**Problem:** Power loss during a dreaming run that has written 3 of 10 new
reflections + partially decayed 500 memories → inconsistent partial state.

**Resolution:** Consolidation runs as idempotent, checkpointed, resumable jobs
within SQLite transactions (per-batch atomic, N2). Interrupted run resumes from last
checkpoint; already-committed batches are correct (atomic); no partial memory is
half-written (transaction boundary). Idempotency (content-hash) means re-running the
interrupted batch is safe.

## N15 — Knowledge drift: slow accumulation of subtly-wrong consolidated facts

**Problem:** Over years, each consolidation introduces tiny summarization errors.
Compounded across 100 consolidation cycles, the "rule" layer drifts far from the
original episodes — telephone-game corruption.

**Resolution:** (a) Provenance chain (derived_from) always links rules back to source
episodes; rules are NEVER the only copy — source episodes are retained (archived, not
deleted). (b) Periodic "grounding check": sample rules, re-derive from source
episodes, flag divergence. (c) Rules carry a confidence that decays if their source
episodes are contradicted. Drift is detectable + correctable because the ground truth
(episodes) survives.

## N16 — The Write Policy Engine's security scanner is itself an LLM (attack surface)

**Problem:** If injection detection uses an LLM, the detector itself is vulnerable to
prompt injection (the malicious content is fed to the detector).

**Resolution:** Injection/poisoning detection FAST-path is DETERMINISTIC (pattern +
heuristic + structural checks — e.g., "does this fact contain imperative
instructions?"). LLM-based semantic checks, if used at all, run in the slow path with
the content clearly delimited as untrusted data (never as instructions), and their
output is advisory (flag), never an auto-execute. Deterministic gate cannot be
prompt-injected.

## N17 — No defined ownership of the "core" namespace promotion decision

**Problem:** Multi-agent + OpenClaw can propose promoting memories to shared "core."
Who approves? If auto, a malicious plugin poisons core. If always-user, it doesn't scale.

**Resolution:** Core-promotion policy: (a) user-stated facts auto-qualify; (b)
plugin/agent-proposed core writes require EITHER user approval OR a high evidence
threshold (≥N independent corroborations from trusted sources); (c) a plugin can
NEVER unilaterally write core. Ownership of the core-promotion decision belongs to the
Write Policy Engine, governed by the evidence threshold. Documented explicitly.

---

# SECTION 29 — REQUIREMENTS RE-VALIDATION (Post Red-Team)

All 30 requirements re-checked against the hardened architecture. Explicit user
requirements verified:

| Explicit requirement | Status | Where |
|---|---|---|
| Temporary chats never enter memory | ✅ | Incognito/Temporary modes, fast-path mode check (Issue 4/6) |
| Selective writes on tool failure/noise | ✅ | Quality filter (Section 5) — failures→execution log, not semantic |
| Truth Maintenance System | ✅ | Section 12 + Issues 13/15/N13/N15 hardened |
| LLM-independent degradation | ✅ | Section 18 + fast-path is LLM-free (Issue 4) |
| Cold-start onboarding WITH consent | ✅ | Issue 8 fix (consent-gated) |
| Temp-vs-permanent classification | ✅ | Section 4 + importance model (Issue 9) |
| Library: tiny → huge documents | ✅ | Section 14 + N11 (streamed/resumable) |
| Delete all memory from a library item | ✅ | Section 14 cascade + N12 reconciliation |
| Complete backup/restore | ✅ | Issues 20/21/27 (versioned, selective, tested) |
| Future cloud sync compatibility | ✅ | Issues 22/23 + N9 (event-union merge) |
| Large-scale mgmt after years | ✅ | Issue 14 (tiered log) + N4 (retrieval gate) |
| Easy backend/frontend integration | ✅ | Section 19 explain API + trait ports |
| Future 3D/graph visualization | 🟡 | Backend-ready (explain/graph API); UI Phase 5-6 |
| Extensible plugin/OpenClaw ecosystem | ✅ | Section 15 + N17 (core-promotion governance) |
| Efficient retrieval after millions | ✅ | N4 (release-gated), LanceDB disk ANN |
| Replace storage tech without rewrite | ✅ | Trait ports (Section 8, 23) |

**Requirements verdict: 28/30 fully, 2 partial (3D UI + multimodal — both deliberate
deferrals, now HONESTLY labeled per Issues 16/18).** No requirement is unaddressed.

---

# SECTION 30 — TECHNOLOGY RE-EVALUATION (Post Red-Team)

Every technology re-challenged. Verdict: **no replacements warranted; two clarifications.**

| Tech | Re-challenge result | Verdict |
|---|---|---|
| **SQLite** | Now the SOLE transactional authority (Issue 29) — its role INCREASED. 30yr proven, WAL, FTS5, CTEs. No embedded DB matches its reliability. | **KEEP (strengthened)** |
| **LanceDB** | Reclassified as a REBUILDABLE INDEX, not truth (Issue 3). This DE-RISKS it — corruption = rebuild, not data loss. Append-only aligns with design. | **KEEP (de-risked)** |
| **FTS5** | Built-in, zero-dep, proven. BM25 ranking adequate. | **KEEP** |
| **ONNX Runtime** | Standard for local inference; model-agnostic. | **KEEP** |
| **Embedding model** | EmbeddingGemma-300M (or nomic-embed-v2) over MiniLM. Note: 300M is heavier — provide a MiniLM fallback tier for low-RAM machines (Matryoshka + tiered). | **KEEP + tiered fallback** |
| **Recursive CTE graph** | Made cycle-safe + depth-capped + benchmark-gated (Issue 12). Honest about limits. GraphStore trait = escape hatch. | **KEEP (bounded)** |
| **Event log in SQLite** | Now tiered (hot SQLite + cold compressed segments, Issue 14). Immutability preserved. | **KEEP (tiered)** |
| **Tokio / dashmap / moka / rayon** | Standard, mature, permissive. | **KEEP** |
| **Backup** | Hardened: versioned, self-describing, tested, 3-2-1 (Issues 20/27). | **KEEP (hardened)** |
| **Encryption** | Extended to LanceDB (Issue N8) + crypto-shred keyring (Issue 2). | **KEEP (extended)** |
| Candidate: SurrealDB/Kuzu/etc. | Re-checked — still BSL/immature/heavy. SQLite-as-authority removes any need. | **REJECT (reaffirmed)** |

**Two clarifications, zero replacements.** The Red Team STRENGTHENED the existing
stack by clarifying roles (SQLite=authority, LanceDB=rebuildable index) rather than
swapping technologies. This is the correct outcome — mature tech, sharper roles.

---

# SECTION 31 — REVIEW HISTORY, CONFIDENCE & GO/NO-GO

## Iteration Log

| Iteration | Focus | Issues Found | Issues Fixed | Regressions Introduced |
|---|---|---|---|---|
| 1 | Known issues 1-17 | 17 | 17 | 0 (each fix regression-checked) |
| 2 | Known issues 18-30 | 13 | 13 | 0 |
| 3 | Independent adversarial (N1-N17) | 17 new | 17 | 0 |
| 4 | Requirements re-validation | 0 architectural (2 honesty relabels) | — | 0 |
| 5 | Technology re-evaluation | 0 (2 clarifications) | — | 0 |
| 6 | Regression sweep | 0 new architectural | — | converged |

**Total: 47 distinct issues identified and resolved across 6 iterations.**
Iteration 6 found only wording/clarity items → convergence reached.

## Architectural Decisions CHANGED by the Red Team

1. **SQLite is now the SOLE transactional authority** (was: two co-equal stores). LanceDB demoted to rebuildable index. — *the keystone fix.*
2. **Event log role clarified:** audit/provenance/erasure ledger, NOT a memory-rebuild source. Derived memories are durable state.
3. **Crypto-shredding** added for GDPR erasure over immutable log.
4. **Transactional outbox** added for SQLite→LanceDB consistency.
5. **Write Policy Engine split** into deterministic fast-path (<2ms, blocking) + async slow-path (enrichment).
6. **Salience loop** changed from 10s polling to event-driven + power-aware.
7. **Memory Worth** changed from raw co-occurrence to normalized, difficulty-adjusted, min-sample-gated soft signal.
8. **Reflection output** now re-enters through the Write Policy as untrusted, evidence-gated, confidence-capped.
9. **Event log tiered** (hot + cold compressed immutable segments) with corrected growth estimates.
10. **Reconciliation sweep** added as first-class maintenance (orphan repair, referential integrity).
11. **Retrieval-quality-vs-scale** made an explicit RELEASE GATE (the existential metric).

## Technologies Changed

**None replaced.** Two clarifications: LanceDB reclassified (index not truth);
embedding model gains a low-RAM MiniLM fallback tier. The stack was strengthened
by role-sharpening, not swapping.

## Remaining Known Limitations (honest)

1. **No production validation** — all confidence is theoretical until Phase 1-2 ship + benchmark.
2. **Multimodal is deferred** (Phase 5) — storage-ready, pipeline NOT built.
3. **Multi-device sync is designed but unbuilt** (Phase 6) — event-union approach is sound but unproven at scale.
4. **Consolidation/reflection quality depends on LLM quality** — mitigated by evidence-gating + grounding checks, but a weak local LLM yields weaker insights.
5. **Retrieval-quality-at-scale is the make-or-break metric** — architecturally addressed (N4) but must be proven empirically; it is the #1 release gate.
6. **3D visualization** — backend-ready, UI unbuilt (low priority).

## Confidence Score

**7.5/10 — pre-implementation, post-Red-Team.**

Justification: The architecture is now INTERNALLY CONSISTENT (the five foundational
contradictions are resolved with mature, production-proven patterns — crypto-shredding,
transactional outbox, single-authority, fast/slow split, event-union sync). It uses
only mature, permissive, actively-maintained, Rust-suitable technologies. Every known
failure mode has a defined resolution. The remaining 2.5 points are withheld
HONESTLY: no unvalidated architecture, however sound on paper, earns more until it
survives production and the retrieval-at-scale benchmark. Claiming 9/10 for unbuilt
software would be exactly the overconfidence the Red Team was chartered to destroy.

## GO / NO-GO for Implementation

**GO — with two mandatory Phase-1 gates.**

Implementation CAN safely begin because:
- The keystone risk (dual-store atomicity) is eliminated — SQLite is the sole
  authority; there is no distributed transaction to get wrong.
- The two legal/privacy blockers (GDPR erasure, embedding leak) have concrete,
  proven solutions (crypto-shredding, uniform encryption).
- The performance blocker (Write Policy bottleneck) is resolved by the fast/slow split.
- Every subsystem has defined ownership, lifecycle, and failure behavior.

**Two NON-NEGOTIABLE Phase-1 gates before building higher layers:**
1. **Prove the single-authority + outbox model** end-to-end (SQLite txn → outbox →
   LanceDB rebuild → orphan reconcile) under simulated crashes. If this is shaky,
   nothing above it is safe.
2. **Stand up the retrieval-quality-vs-scale benchmark harness EARLY** (seed with
   synthetic 500K memories). This is the metric that silently kills memory systems
   at month 6. Measure it from day one, not after launch.

If either gate fails, STOP and reconsider — but the design as hardened here gives
both a high probability of success.

---

*End of Red Team pass. This document is now internally consistent, uses only
mature/proven/permissive technologies, has a defined resolution for every known
failure mode, and honestly labels its unvalidated areas. It is ready to guide
implementation, gated by the two Phase-1 proofs above.*

*New evidence this pass: crypto-shredding for event-sourced GDPR erasure
([conduktor.io](https://www.conduktor.io/glossary/crypto-shredding-for-kafka),
[verraes.net](https://verraes.net/2019/05/eventsourcing-patterns-throw-away-the-key/),
[axoniq.io](https://developer.axoniq.io/w/protect-sensitive-data-in-an-event-sourced-application-1));
transactional outbox for dual-write consistency
([confluent.io](https://www.confluent.io/en-gb/blog/dual-write-problem/),
[designgurus.io](https://www.designgurus.io/blog/transactional-outbox-pattern));
EmbeddingGemma ([arxiv:2509.20354](https://arxiv.org/abs/2509.20354)); memory retrieval
degradation at 6 months ([fountaincity.tech](https://fountaincity.tech/resources/blog/how-to-build-and-operate-ai-agent-memory-in-2026/)).
Content rephrased for compliance with licensing restrictions.*


---
---

# SECTION 32 — ARCHITECTURE ADDITIONS (Missing Subsystems)

Eight areas identified as under-specified. Each is a genuine strengthening;
verdicts and designs below. These are additive — no prior decision is contradicted.

## 32.1 Entity Resolution Engine ⭐ (upgrade of N5)

**Verdict: Genuine gap — promote from "merge policy" to first-class subsystem.**

N5 gave a conservative merge *policy*; it did not give an *engine*. Identity
resolution ("OpenAI" = "OpenAI Inc." = "openai.com" = "the GPT company") is a
distinct, hard problem that touches people, projects, files, repos, companies,
technologies, users, and library items.

**Design (meta-level):**
```
Entity Resolution Engine (async, slow-path):
  1. CANONICALIZATION — normalize surface forms (lowercase, strip suffixes,
     canonical URLs, alias tables)
  2. BLOCKING — cheap candidate generation (name prefix, domain, embedding cluster)
     to avoid O(n²) all-pairs comparison
  3. SCORING — multi-signal match score:
        exact identifiers (email/handle/URL/repo-path) = strong
        embedding similarity = medium
        name similarity (edit distance) = weak
        co-occurrence context = medium
  4. DECISION —
        strong identifier match → auto-merge
        medium only → propose (user confirm or evidence threshold)
        name-only → NEVER auto-merge (esp. people)
  5. REVERSIBILITY — every merge records merged_from; split restores originals
```

**Invariants:** conservative by default (wrong-merge worse than no-merge for
people); all merges reversible; identifier matches beat name matches always.
Entity resolution runs on the slow path, feeds the graph. Each entity carries
`aliases[]` + `canonical_id` + `merge_provenance`.

**Failure guarded:** wrong merge conflates two people's memories (privacy breach)
→ mitigated by identifier-gating + reversibility + user confirm for medium signals.

## 32.2 Unified Background Scheduler ⭐ (critical gap)

**Verdict: Genuine critical gap.** Workers exist scattered (consolidation,
dreaming, decay, re-index, verification, re-embed, backup, cleanup, graph update,
salience, reconciliation, entity resolution). Nothing owns them collectively → they
would compete for the single SQLite writer, CPU, and battery.

**Design (meta-level): one Cognitive Scheduler owns ALL background work.**
```
Properties:
  PRIORITY CLASSES:
     P0 foreground (user-facing writes/reads) — always preempt background
     P1 integrity (reconciliation, orphan sweep, backup) — must run
     P2 enrichment (embedding, entity resolution, dedup) — timely
     P3 cognition (consolidation, dreaming, reflection) — opportunistic
     P4 maintenance (re-index, compaction, decay) — lowest
  RESOURCE AWARENESS:
     - Battery below threshold / power-saver → suspend P3, P4
     - Memory pressure high → shed caches, defer P3/P4
     - CPU/thermal high → throttle
     - On AC + idle → full-speed cognition
  COOPERATIVE + CANCELLABLE:
     - Every job is chunked, checkpointed, resumable (N14)
     - Jobs yield the writer between batches (N2)
     - Cancellable mid-run without corruption (transaction boundaries)
  SINGLE-FLIGHT:
     - No two instances of the same job run concurrently
     - Idempotent re-entry after interruption
```

**This subsumes** the N2 two-queue write scheduler and the salience power-awareness
(Issue 7) into ONE coherent scheduler. It is the runtime backbone that makes all
background cognition safe on a laptop.

## 32.3 Runtime Resource / Memory Budget Manager ⭐ (genuine gap)

**Verdict: Genuine gap.** Disk capacity is covered (Section 18); RUNTIME resources
are not. A desktop assistant must never starve the user's actual work.

**Design (meta-level): explicit runtime budgets, enforced by the Scheduler.**
```
Governed budgets (all user-configurable, sensible defaults):
  max_ram              — hot caches + working set (default e.g. 300MB)
  max_cpu_background   — % CPU for P3/P4 when on battery vs AC
  max_gpu              — embedding/inference GPU share (0 on battery by default)
  embedding_queue_max  — backpressure: if queue full, drop-to-keyword-only + defer
  vector_cache_size    — moka LRU bound
  graph_cache_size     — hot 2-hop cache bound (Issue 12)
  consolidation_budget — max wall-time per consolidation run
Enforcement: Budget Manager exposes current pressure to the Scheduler, which
throttles/sheds accordingly. Backpressure everywhere (bounded queues, never
unbounded growth). If a budget is exceeded, degrade gracefully (Section 18), never OOM.
```

## 32.4 Retrieval Feedback Loop 🟡 (strengthen Memory Worth)

**Verdict: Partially present (Memory Worth is the mechanism) — but SIGNAL CAPTURE
was undefined.** Memory Worth counts success/failure co-occurrence, but never
defined HOW it learns whether a retrieved memory was actually used/helpful.

**Design (meta-level): explicit feedback signal capture.**
```
Per retrieved memory, capture:
  surfaced?     — was it injected into context (vs filtered out)
  referenced?   — did the response/reasoning actually use it (citation trace)
  outcome?      — did the turn/task succeed
  corrected?    — did the user correct the output (strong negative signal)
  dwelled?      — (UI) did the user act on the surfaced memory
Feed into:
  - Memory Worth (normalized, difficulty-adjusted — Issue 6)
  - Confidence calibration (used+success → small boost; corrected → dent)
  - Adaptive RRF weights (which strategy produced USED results for this query type)
```

**Key addition over Memory Worth:** "referenced?" (was it actually used, not just
present) distinguishes helpful from merely-retrieved — fixes the co-occurrence
confound more directly than difficulty-adjustment alone. Retrieval improves
automatically over time; corrections are the strongest learning signal.

## 32.5 Knowledge Gap Engine 🟡 (promote existing tracker)

**Verdict: Partially present (knowledge_gaps table, Section 19) — promote to a
subsystem that produces learning goals.**

**Design (meta-level):**
```
Knowledge Gap Engine consumes:
  - Failed retrievals (searched, found nothing / low confidence)
  - Contradictions KRIA couldn't resolve
  - Low-confidence answers KRIA gave
  - Domains where Memory Worth is consistently low (KRIA's advice keeps failing)
Produces:
  - Ranked knowledge gaps by frequency × recency × goal-relevance
  - Self-model uncertainty map ("Docker: LOW, Rust: HIGH")
  - LEARNING GOALS (feeds Goal Memory): "research Docker networking"
  - Proactive clarification prompts: "I don't know your prod environment — tell me?"
```

**Integration:** gaps become candidate goals (governed like all inferred goals,
N6). Enables genuine metacognition — KRIA knows what it doesn't know, and turns
that into action. This is a differentiator, not a nice-to-have.

## 32.6 Memory API Contract ⭐ (missing public contract)

**Verdict: Genuine gap.** Only explain/debug APIs were defined. The clean public
verb-contract every subsystem depends on was implicit. Defining it now (as CONTRACT,
not implementation) prevents ad-hoc coupling later.

**The contract (WHAT, not HOW):**
```
Write path (ALL go through Write Policy Engine):
  observe(observation)      — raw perception → event log (fast path)
  remember(candidate)       — explicit store request → policy → maybe stored
  update(id, change)        — supersede (new event, old versioned)
  forget(scope)             — tombstone + crypto-shred + cascade
  verify(id)                — re-check against source (TMS)

Read path (read-only, never blocks writer):
  search(query, context)    — multi-strategy retrieval (Section 10)
  recall(scope)             — direct scoped fetch (goals, prefs, project)
  reason(query)             — retrieval + graph traversal + synthesis
  explain(id | query)       — provenance / retrieval trace (Section 19)

Cognitive path (Scheduler-governed, async):
  reflect(trigger)          — produce reflections (re-enters via policy, Issue 13)
  consolidate(scope)        — compress/merge/decay
  resolve_entities()        — entity resolution pass

Admin path:
  backup(dest) / restore(src, scope)   — versioned, selective (Issues 20/21)
  export(scope) / import(src)          — user data portability
  health() / metrics()                 — observability (Section 20, 32.8)
  set_mode(mode)                        — memory mode switch (Section 6)
```

**Every subsystem uses ONLY this contract.** No direct storage access. This is the
seam that lets storage tech swap (trait ports) without touching callers — the
contract is stable even as backends change.

## 32.7 Cognitive State (Working Context) 🟡 (forward-looking, keep minimal in v1)

**Verdict: Genuine but SCOPE-CONTROLLED.** TurnMemory covers the current turn.
Explicit focus/attention/intent/task-context/mental-workspace matters mainly once
KRIA runs autonomous multi-task workloads. Build minimal now, expand with autonomy.

**Design (meta-level):**
```
Cognitive State (ephemeral, RAM, NOT persisted as memory — but snapshot-able for
crash recovery per Issue 10/N9):
  current_focus       — what KRIA is attending to right now
  active_intent       — the goal/task currently being pursued
  task_context        — the working set for the current task (files, entities, subgoals)
  attention_stack     — when multitasking: paused contexts to resume (context switching)
  mental_workspace    — scratchpad for in-progress reasoning (not yet a memory)
```

**v1 scope:** just `current_focus` + `active_intent` + `task_context` (extends
TurnMemory). **Deferred:** attention_stack + mental_workspace become important only
with multi-agent/autonomous execution (Phase 5-6). Explicitly NOT over-built now.
Crash recovery snapshots the state (links to Issue 10 session resume).

## 32.8 Intelligence Metrics 🟡 (strengthen Section 20)

**Verdict: Section 20 had latency + retrieval-quality + reflection-quality. Missing
the INTELLIGENCE suite.** Added:

```
QUALITY (the metrics that measure whether memory is actually good):
  Recall Precision      — of retrieved memories, % actually relevant
  Recall Recall         — of relevant memories, % retrieved
  Retrieval Hit Rate    — % queries returning a used result
  Hallucination Rate    — % responses citing non-existent/wrong memory
  False Memory Rate     — % stored "facts" that are wrong (sampled audit)
  Duplicate Rate        — % near-duplicate memories (dedup health)
  Stale Memory %        — % memories past staleness threshold unverified
  Contradiction Rate    — unresolved contradictions / total facts
COGNITION:
  Goal Completion %     — inferred/tracked goals reached
  Reflection Quality    — human-rated usefulness of generated insights (sampled)
  Consolidation Gain    — compression ratio × retained-utility
  Confidence Calibration— predicted confidence vs actual correctness (ECE)
```

**The measurement that matters most (reaffirming N4):** Recall Precision as memory
grows. Track ALL of these over time; regression in any = investigate. Confidence
Calibration (Expected Calibration Error) is the honesty metric — KRIA's stated
confidence must match its actual accuracy, or "explainable" becomes "confidently wrong."

---

## Section 32 Summary — Impact on Architecture

| Addition | New subsystem? | Phase | Confidence impact |
|---|---|---|---|
| Entity Resolution Engine | Yes (was policy) | 3 | Prevents identity-conflation bugs |
| Cognitive Scheduler | Yes (unifies scattered workers) | 1-2 | **Foundational — build early** |
| Runtime Budget Manager | Yes | 2 | Laptop-viability guarantee |
| Retrieval Feedback (signal capture) | Extends Memory Worth | 2 | Self-improving retrieval |
| Knowledge Gap Engine | Yes (promotes tracker) | 3 | Metacognition differentiator |
| Memory API Contract | Contract (not new code) | 1 | **Foundational — define first** |
| Cognitive State | Extends TurnMemory (minimal v1) | 1 (min), 5 (full) | Autonomy-readiness |
| Intelligence Metrics | Extends Section 20 | 2 | Makes quality measurable |

**Two are foundational (build/define in Phase 1):** the **Cognitive Scheduler**
(without it, background jobs collide) and the **Memory API Contract** (without it,
subsystems couple to storage). The other six layer in cleanly at their phases.

**Revised roadmap note:** Phase 1 now explicitly includes the Memory API Contract
definition + a minimal Cognitive Scheduler (priority classes + writer arbitration +
battery awareness). The rest slot into Phases 2-3 as shown.

**Confidence after Section 32: still 7.5/10** — these additions close real gaps but
change nothing structural; they make the design more COMPLETE, not more PROVEN. The
2.5 withheld points remain about production validation, unchanged.

*These eight were genuine gaps (2 critical, 6 valuable). None contradicts the Red
Team resolutions; the Scheduler in fact absorbs and unifies several previously-
scattered concerns (N2 write arbitration, Issue 7 salience power-awareness, all
background workers) into one coherent runtime backbone.*


---
---

# SECTION 33 — FUTURE EVOLUTION & EXTENSIBILITY (10-20 Year Horizon)

*Internally numbered 15.1-15.13 per the evolution brief. Principle: reserve
abstractions that prevent rewrites; refuse over-engineering that adds cost without
near-term payoff. Every item is classified: **Required / Strongly Recommended /
Optional / Over-Engineering (do NOT build yet).***

## 15.1 Future AI Learning (memory as training data)

**Capable today? Partially.** The event log + reasoning traces + outcomes already
capture the raw material. What's MISSING is the metadata that makes datasets
extractable without re-labeling years later.

**Reserve NOW (cheap, prevents rewrite):** every event/memory already has outcome,
source, confidence, provenance, Memory Worth. ADD three reserved fields:
- `feedback_signal` (thumbs/correction/undo/ignored — see 15.3)
- `preference_pair_id` (link chosen-vs-rejected outputs for DPO/RLHF pairs)
- `training_eligible` (user-consented flag — training data needs explicit consent)

**Datasets become derivable views over the event log** (no separate store):
preference pairs (chosen/rejected + feedback), tool-routing (goal→tool→outcome),
planning (goal→plan→result), reflection, failure/success. All are QUERIES, not new
schemas.

**Verdict:** Reserving the 3 fields = **Strongly Recommended** (near-zero cost).
Actually training LoRA/adapters locally = **Optional** (Phase 4). Building a
training pipeline now = **Over-Engineering.**

## 15.2 Continuous Learning (self-improvement)

**Capable today? Yes, the loops exist.** Memory Worth + retrieval feedback (32.4) +
knowledge gaps (32.5) + reflection (Section 11) + prompt-variant stats already form
improvement loops for retrieval, tool selection, and planning — WITHOUT model weight
changes (in-context / non-parametric learning, per HippoRAG/ReasoningBank evidence).

**Reserve NOW:** nothing new — the feedback + Memory Worth + gap-engine metadata is
sufficient. Continuous learning is non-parametric (memory-driven) by default;
weight-based learning (15.1) is a later, optional layer.

**Verdict:** Non-parametric continuous learning = **Required** (it's what makes
memory "cognitive"). Weight-based = **Optional/Phase 4.**

## 15.3 Feedback Learning (production feedback loop)

**Capable today? The mechanism (Memory Worth) exists; the SIGNAL TAXONOMY was
under-specified.** Define it now as a first-class event type.

**Design (reserve now — Strongly Recommended):**
```
FeedbackEvent { target_memory_id | target_response_id, signal, timestamp, context }
signal ∈ {
  thumbs_up, thumbs_down,          // explicit rating
  correction(text),                // strongest positive-learning signal
  undo, cancel,                    // implicit negative
  edit(diff), overwrite,           // implicit correction
  ignored_suggestion,              // implicit negative (proactive recall missed)
  repeated_task,                   // implicit "automate this" signal
  automation_success/failure,      // outcome
}
```
Every feedback event flows through the Write Policy → updates Memory Worth
(normalized, 32.4), confidence calibration, adaptive retrieval weights, and (with
consent) becomes a preference-pair for future training (15.1).

**Verdict:** **Required.** Feedback is the single highest-value learning signal; the
event type must exist from Phase 1 even if only thumbs up/down is wired initially.

## 15.4 Multi-Agent Future (safe shared memory)

**Capable today? Design-ready (namespaces + owner_id + event log).** The event log
IS a blackboard — validated by recent research (Terrarium, Oct 2025: "blackboard as
an append-only log" for multi-agent safety/privacy; bMAS blackboard systems).

**Architecture (reserve now conceptually, build Phase 3-4):**
```
Memory tiers per agent:
  PRIVATE   — namespace: agent/{id}, isolated, not readable by others
  SHARED    — namespace: core, read-all, write-gated (evidence threshold / user, N17)
  SCRATCHPAD— ephemeral blackboard region, TTL'd, for in-flight coordination
Coordination:
  - Blackboard = append-only event region (no in-place mutation → no lock contention)
  - Agents POST hypotheses/results as events; others read
  - Ownership: each event owned by writing agent; core-promotion governed (N17)
  - No distributed locks needed (append-only + single SQLite writer arbitration, 32.2)
  - Leases: for exclusive tasks, a lease is itself an event (claim/release), TTL'd
```

**Verdict:** Namespace/owner/event-log foundation = **Required** (already present).
Blackboard scratchpad + leases = **Strongly Recommended, Phase 3.** Full autonomous
multi-agent = **Optional/Phase 4** — don't build agent orchestration now, just keep
the memory substrate ready (it is).

## 15.5 OpenClaw Evolution (hundreds-thousands of skills)

**Capable today? Mostly (CKB + namespaces).** At thousands of skills, additions needed.

**Reserve now / build as scale demands:**
```
Per-skill memory (namespace: openclaw/{skill_id}):
  usage_history, success_rate, confidence, latency  → already in CKB
  skill_embedding    → for "which skill fits this goal" retrieval (ADD, Phase 3)
  skill_relationships→ graph edges: skill→depends-on→skill, skill→similar-to→skill
  version + migration→ skill version is an entity version (15.6 pattern reused)
  trust_evolution    → Memory Worth applied to skills (reuse existing mechanism)
  retirement         → low-worth + unused skills → archived (not deleted)
```
**Skills CONSUME** memory via read-only `SkillMemoryView` (scoped, Section 15).
**Skills CONTRIBUTE** only via orchestrator → Write Policy (never direct, N17).
Skill-generated memories carry `namespace: openclaw/{id}` + provenance → deletable
as a unit if the skill is uninstalled.

**Verdict:** skill_embedding for skill-selection = **Strongly Recommended** at 100+
skills. Skill relationship graph = **Optional** (only if skills compose). Trust
evolution = **Required** (reuses Memory Worth — near-free).

## 15.6 Library Evolution (per-library operations)

**Capable today? Core yes (Section 14); per-library GRANULARITY needs first-class support.**

**Design (reserve now — Required for the Library to be truly first-class):**
Every library and library-item is a scoping unit. All operations are library-scoped:
```
delete_library(id) / forget_library(id)  → cascade: chunks + vectors + files +
                                             derived memories + crypto-shred keys (N12)
backup_library(id) / export_library(id)   → self-contained portable bundle
share_library(id)                          → export + import on another device/user
search_library(id) / visualize_library(id)→ scoped retrieval + graph view
rebuild_library(id)                        → re-index ONLY that library (resumable, N/25)
remove_all_memories_from(library_item_id)  → provenance-driven cascade (Section 14)
```
This works because provenance already tags every derived memory with its source
(`source: library:{item}:chunk:{idx}`). Per-library operations are provenance
queries + cascades — no new architecture, just first-class API surface (32.6).

**Verdict:** **Required.** The provenance tagging must be rigorous from Phase 4 or
per-library deletion (a stated requirement) silently leaks orphaned memories.

## 15.7 Workspace Intelligence (multi-repo/company/client/device)

**Capable today? Yes (namespace + scope axis).** Workspaces are a scope dimension
already in the classification axes (Section 4).

**Design:**
```
Scope hierarchy: global > company > client > workspace/repo > session
Isolation: strict by default (client A never leaks to client B — legal/privacy).
Sharing: EXPLICIT promotion to a shared scope (user-approved).
Cross-workspace knowledge: only via global scope (e.g., "I prefer tabs") or
  user-approved promotion. Technology/skill knowledge is naturally global;
  client-specific facts are strictly isolated.
Multi-OS/device: device_id + capability facts scoped per device (World Model).
```
**Verdict:** Scope hierarchy = **Required** (privacy/legal isolation between clients
is non-negotiable). Cross-workspace transfer learning = **Optional/Phase 3.**

## 15.8 Frontend Evolution (Memory UI)

**Capable today? Backend-ready (explain/debug API 32.6 + graph API).** UI is unbuilt.

**Professional-grade Memory UI should eventually provide (prioritized):**
```
TIER 1 (Required, Phase 3-4): Memory Search · Memory Explorer (browse/edit/delete)
  · Timeline · Library Explorer · Backup/Restore Wizard · Mode indicator
TIER 2 (Strongly Recommended, Phase 4-5): Knowledge Graph (2D force-directed) ·
  Goal Dashboard · Entity Explorer · Truth/Conflict Resolution UI · Import/Export
  Wizard · Version History · Memory Diff Viewer
TIER 3 (Optional, Phase 5-6): Reflection/Dreaming/Consolidation Dashboards ·
  Memory Worth + Importance Heatmaps · Feedback Dashboard · Skill Memory Viewer ·
  Merge Viewer · Learning Dashboard
TIER 4 (Optional "wow", Phase 6+): 3D Memory Graph (gimmick risk — 2D is more
  useful; build only if it earns its complexity)
```
**Every UI view is a READ over the API contract (32.6)** — the backend need not
change to add views. That's the point of defining the contract now.

**Verdict:** Backend API contract = **Required now.** Specific UI views = phased,
mostly **Optional** except search/explorer/backup (**Strongly Recommended**).

## 15.9 Backend Evolution (long-term APIs)

**Capable today? The 32.6 contract covers most.** For long-term scale + realtime UI,
add streaming + subscriptions.

**API surface (extends 32.6):**
```
CRUD + batch          → 32.6 (Required)
search / reason / graph-traverse / vector-search → 32.6 (Required)
snapshot/backup/restore/import/export/migrate/replay/audit → Sections 18/33 (Required)
Goal/Episode/Workspace/Library/Skill APIs → scoped facades (Strongly Recommended)
Learning/Reflection/Truth/Feedback APIs → 15.1-15.3, 32.5 (Strongly Recommended)
STREAMING + SUBSCRIPTIONS (realtime UI updates: "memory changed" events) →
  the event log is ALREADY a stream; expose a subscription over it (Strongly
  Recommended for reactive frontend, Phase 4)
```
**Verdict:** Event-log-as-subscription-stream = **Strongly Recommended** (nearly
free — the stream exists; just expose it). Everything else = phased facades over the
contract.

## 15.10 Plugin & Extension Ecosystem (external developers)

**Capable today? Foundation yes (namespaces + SkillMemoryView + Write Policy).**
For third-party plugins, formalize a permission-scoped capability model.

**Design (reserve now — Strongly Recommended before opening to 3rd parties):**
```
Plugin permission scopes (declared in manifest, user-approved at install):
  memory:read:own        — read its own namespace only (default)
  memory:read:public     — read shared/global public facts
  memory:suggest         — propose writes (queued, NOT applied without approval)
  memory:annotate        — add annotations (never mutate core facts)
  memory:write:own        — write to its own namespace
  memory:search           — run scoped retrieval
  memory:create_entity    — propose graph entities (resolved conservatively, 32.1)
  memory:request_deletion — request (not perform) deletion of its own data
Plugins CANNOT: write core directly, read other plugins' namespaces, delete
  user/other-plugin data, escalate scope at runtime.
Security: all plugin writes go through the SAME Write Policy Engine (fast-path
  security scan + namespace enforcement). A malicious plugin is contained to its
  own namespace and cannot poison core (N17).
```
**Verdict:** Permission-scope model = **Required BEFORE any 3rd-party plugin
ships** (security). Not needed for first-party OpenClaw skills (trusted). Build when
the ecosystem opens.

## 15.11 Cloud & Multi-Device Future (without redesign)

**Capable today? Design-ready, unbuilt.** The event log + content-addressing +
device_id + HLC ordering (N10) + crypto-shred keys make sync possible WITHOUT
redesign.

**How it works without redesign:**
```
Sync unit = events (immutable, content-addressed BLAKE3, HLC-ordered, device-tagged).
Sync = exchange missing event ranges → union → re-derive memories per device (N9).
Selective sync = by namespace/scope (sync work laptop's "client-A" scope only).
Encrypted sync = events encrypted with user keys before leaving device; crypto-shred
  keys sync separately (or stay local for zero-knowledge cloud).
Conflict resolution = event union never conflicts (append-only); derived-state
  conflicts resolved by re-derivation; preferences by CRDT (LWW + vector clock, N10/22).
Offline merge = accumulate offline, reconcile on reconnect by HLC watermark (23).
Cloud = optional encrypted event-log backup/relay; NEVER required for core function.
```
**Verdict:** The enabling primitives (content-addressed events, HLC, device_id,
crypto-shred, namespaces) = **Required to reserve now** (they cost nothing extra and
are already in the design). Actual sync implementation = **Optional/Phase 6.** No
redesign needed — this is the payoff of event sourcing.

## 15.12 Research Roadmap (what recent work should influence KRIA)

Recent research already integrated or worth adopting, mapped to where it fits:

| Research | Where it fits KRIA | Adopt? |
|---|---|---|
| HippoRAG 2 (neurobiological KG + PageRank) | Graph retrieval ranking | Strongly Recommended (Phase 3) |
| Experience Compression Spectrum | Episode→skill→rule (Section 3) | Already core |
| Memory Worth (governance) | Retrieval/archival (32.4) | Already core |
| ReasoningBank / trajectory memory | Reasoning-trace memory (Section 3) | Already core |
| Anthropic Dreaming / OpenAI Dreaming V3 | Consolidation (Section 11) | Already core |
| Terrarium blackboard (append-log MAS) | Multi-agent (15.4) | Adopt pattern (Phase 3-4) |
| MemoryArena / LongMemEval benchmarks | Benchmark harness (Section 20) | Required (Phase 2) |
| EmbeddingGemma / Matryoshka | Embeddings (Section 9) | Adopt (Phase 1) |
| GraphRAG / LazyGraphRAG (community summaries) | Library cross-doc reasoning (14) | Strongly Recommended (Phase 4) |
| DPO / non-parametric preference learning | Future training (15.1) | Optional (Phase 4) |
| Federated/personalized alignment | Multi-device learning | Over-Engineering for now |

**Nothing in recent research invalidates the architecture.** The strongest
additions (HippoRAG-style PageRank on the graph, GraphRAG community summaries for
the Library) fit cleanly into existing subsystems. No redesign triggered.

## 15.13 Final Evolution Assessment (KRIA after 10 years)

**Would the architecture support the 10-year targets?**

| Target | Supported? | Caveat |
|---|---|---|
| 100M+ memories | 🟡 Conditional | SQLite handles metadata to ~100M rows but retrieval quality (N4) is the real limit, not storage. Tiered archival + aggressive supersession required. |
| Millions of vectors | ✅ | LanceDB disk-native IVF-PQ — designed for this |
| Millions of graph edges | 🟡 | SQLite CTEs degrade here → GraphStore trait swap to dedicated engine (the escape hatch exists) |
| Thousands of books/repos | ✅ | Library is filesystem + LanceDB — scales |
| Thousands of skills | ✅ | Namespaces + CKB + skill embeddings (15.5) |
| Years of history | ✅ | Tiered event log (Issue 14) |
| Autonomous agents | ✅ | Namespace/blackboard substrate ready (15.4) |
| Continual learning | ✅ | Non-parametric loops (15.2) |
| Local model training | 🟡 | Reserved metadata (15.1); pipeline is Phase 4 |
| Personal knowledge base / search engine | ✅ | Library + retrieval + graph |
| Desktop operating intelligence | ✅ | The whole design targets this |

**Where it breaks and the fix:**
1. **100M+ memories:** the bottleneck is RETRIEVAL PRECISION, not storage. Prevented
   today by supersession + dedup + importance gating (N4) — but this is the metric
   that must be benchmarked continuously (release gate).
2. **Millions of graph edges:** SQLite CTEs will not hold. The GraphStore trait swap
   (to a mature embedded graph engine, if one matures) is the pre-built escape hatch.
   Reserving the trait now (already done) = the fix.

**Is reserving these abstractions now worthwhile? YES for:** event log,
content-addressed events, HLC, namespaces, provenance, GraphStore/VectorStore traits,
feedback event type, reserved training fields. All are near-zero cost today and
prevent rewrites. **NO (over-engineering) for:** building the training pipeline,
multi-device sync, 3D UI, or a dedicated graph DB before evidence demands them.

### Consolidated Evolution Roadmap

**Phase 1 — Required today:** Event log + content-addressed events + HLC ordering +
namespaces + provenance + GraphStore/VectorStore traits + Write Policy + Memory API
Contract + Cognitive Scheduler + Feedback event type (thumbs) + reserved training/
feedback fields + EmbeddingGemma.

**Phase 2 — Within ~2 years (Required/Strongly Recommended):** Benchmark harness
(retrieval-quality-at-scale gate) + Runtime Budget Manager + full retrieval feedback
loop + Truth Maintenance + intelligence metrics + Knowledge Gap Engine.

**Phase 3 — Within ~5 years (Strongly Recommended/Optional):** Entity Resolution
Engine + graph (+ HippoRAG PageRank) + skill embeddings + blackboard scratchpad +
cross-workspace transfer + Memory UI Tier 1-2 + event subscription stream.

**Phase 4 — Beyond 5 years (Optional):** Local model training/LoRA from reserved
metadata + GraphRAG community summaries + autonomous multi-agent + full plugin
permission ecosystem + Memory UI Tier 3.

**Phase 5+ — Horizon (Optional / evidence-gated):** Multi-device encrypted sync +
multimodal pipeline + dedicated graph engine (if edges exceed SQLite) + 3D UI.

**Explicitly DO NOT build yet (Over-Engineering):** training pipeline, cloud sync,
federated learning, 3D visualization, dedicated graph DB — reserve the abstractions,
build only when evidence demands.

---

*Section 33 verdict: the architecture is evolution-ready. Every 10-year target is
either supported today or has a pre-reserved abstraction (trait swap, event-log sync,
reserved fields) that prevents a rewrite. The two genuine long-term risks —
retrieval precision at 100M memories and graph edges in the millions — are both
guarded (benchmark release-gate + GraphStore trait). Reserving the cheap abstractions
now is worthwhile; building the heavy future systems now would be over-engineering.
Confidence unchanged at 7.5/10 pre-implementation — evolution-readiness was already
part of that assessment.*

*New evidence: Terrarium append-log blackboard for MAS safety (arxiv Oct 2025),
LLM blackboard systems (arxiv:2510.01285, 2507.01701), RLHF/DPO preference datasets
(arxiv:2504.12501), HippoRAG 2 non-parametric continual learning. Content rephrased
for compliance.*


---
---

# SECTION 34 — CORRECTED CONSTRAINTS & FINAL STACK (Authoritative Override)

> These corrected constraints override earlier "embedded-only" assumptions.
> Where earlier sections assume no-server or single-file, THIS section wins.

## 34.1 Corrected Constraints (authoritative)

1. Local-first, NOT embedded-only — local services/Docker/servers are ACCEPTABLE if they add long-term value.
2. Fully offline; cloud optional, never a dependency.
3. Must scale to future hardware without redesign.
4. Multiple storage engines acceptable; requirement is backup/restore/reliability/consistency, not one file.
5. **Licensing is HARD:** MIT / Apache-2.0 / BSD only. No GPL-copyleft, BSL, SSPL, or commercial-gated cores.
6. Rust-first, not Rust-only.
7. Multi-user/multi-device must be addable later without major redesign.
8. Privacy-first + local-first absolute.
9. Must function with no LLM / no GPU / no internet / no embeddings.
10. Priority = best long-term architecture, not fewest dependencies.
11. Efficient multi-year storage (tiering/compression/archival/bounded growth).
12. Adopt better tech now to avoid painful migration later.
13. Must compete architecturally with the best commercial assistants.

## 34.2 Final Technology Stack (post-correction)

| Layer | Technology | License | Role |
|---|---|---|---|
| Transactional authority | **SQLite** (WAL) | Public Domain | Events, memories, graph adjacency, outbox, goals, prefs — the single source of truth |
| Vector service | **Qdrant** (local service) | Apache-2.0 | Hybrid search + RRF + payload filtering + quantization; rebuildable index |
| Full-text search | **Tantivy** (embedded) | MIT | BM25/phrase/fuzzy/faceted FTS; rebuildable index |
| Graph (now) | SQLite adjacency + CTEs | Public Domain | ≤~1M edges; cycle-safe, depth-capped |
| Graph (future >1M edges) | **Dgraph** or **NebulaGraph** (local service) | Apache-2.0 | Named licensed escape hatch via GraphStore trait |
| Embeddings (floor) | ONNX in-process (MiniLM/EmbeddingGemma) | Apache-2.0 | Works when all else is down |
| Embeddings (better, optional) | Local service (Ollama/ONNX server) | Apache-2.0 | Stronger model on capable HW; degrades to floor |
| Consistency | Transactional outbox | — | SQLite txn → outbox → Qdrant + Tantivy rebuildable indexes |
| Async / caches | Tokio, dashmap, moka, rayon | MIT/Apache | Runtime + caches + batch |
| Crypto | blake3, age/libsodium | Apache/MIT/BSD | Checksums, crypto-shred, encrypted backup |

## 34.3 What Changed vs Earlier Sections & Why

| Earlier | Now | Evidence |
|---|---|---|
| LanceDB sole vector store | **Qdrant primary** (LanceDB still valid alt) | Servers allowed → Qdrant's native hybrid+filter+quant beats hand-rolled RRF; Apache-2.0, Rust. Avoids usearch→LanceDB→X migration chain (constraint 12). |
| SQLite FTS5 only | **+ Tantivy** | Deps no longer minimized (constraint 10); Tantivy = Lucene-class FTS for Library scale, MIT embedded. |
| "Wait for embedded graph to mature" | **Dgraph/NebulaGraph named escape hatch** | Servers allowed. Neo4j (GPLv3) + Memgraph (BSL) + SurrealDB (BSL) FAIL constraint 5; Dgraph/Nebula core = Apache-2.0. |
| "One unified SQLite file" | **Multi-engine, SQLite = transactional authority** | Constraint 4. Qdrant/Tantivy are rebuildable indexes via outbox — consistency preserved. |
| Embeddings in-process only | **In-process floor + optional local service** | Constraint 3 (scale to better HW) + 9 (must work without service). |

## 34.4 What Did NOT Change (survives correction)

SQLite-as-single-transactional-authority · event sourcing · transactional outbox (now
feeds 2 indexes) · crypto-shredding · Write Policy Engine (fast/slow split) · Truth
Maintenance · Dreaming · Cognitive Scheduler · trait-based storage ports · rejection of
Neo4j/Memgraph/SurrealDB (now for LICENSING, not server). The core is unchanged — only
the retrieval/index layer got stronger.

**Critical invariant preserved:** there is still exactly ONE transactional authority
(SQLite). Qdrant and Tantivy are DERIVED, REBUILDABLE indexes. This keeps the
"collapse distributed atomicity into a local transaction" property (Issue 29) intact
even with three storage engines.

---
---

# SECTION 35 — FINAL CONVERGENCE REVIEW & PRINCIPAL SIGN-OFF

## 35.1 New Issues From the Corrected (3-Engine) Stack

The move from 1 index to 2 indexes (Qdrant + Tantivy) introduces new surface area.
Each found, root-caused, fixed, regression-checked.

**C1 — Two rebuildable indexes = two outbox consumers = two drift surfaces.**
Root cause: outbox now fans out to Qdrant AND Tantivy; either can lag/fail
independently. Consequence: a memory searchable by keyword (Tantivy synced) but not
vector (Qdrant lagged), or vice versa → inconsistent retrieval. Fix: per-index outbox
cursors + independent idempotent replay + reconciliation sweep covers BOTH indexes
(extends N12). Regression check: SQLite remains authority; both indexes rebuildable
from it. No new authority. ✅

**C2 — Qdrant as a separate process = a new failure/lifecycle domain.**
Root cause: Qdrant is a local service that can be down, crash, or version-drift
independently of KRIA. Consequence: retrieval degraded if Qdrant unavailable at
startup. Fix: (a) Qdrant supervised by KRIA (start/health/restart); (b) **Tantivy
(embedded) is the degradation floor** — if Qdrant is down, keyword+graph retrieval
still works (constraint 9). (c) SQLite FTS5 retained as a second fallback below
Tantivy. Regression check: aligns with LLM-degradation philosophy (raw always works,
enrichment best-effort). ✅

**C3 — Qdrant snapshot vs SQLite backup consistency skew.**
Root cause: backing up SQLite and Qdrant at different instants → restore mismatch.
Consequence: restored vectors reference memories that don't exist yet (or vice versa).
Fix: **do NOT back up Qdrant/Tantivy at all** — back up ONLY SQLite (authority) + the
outbox cursor; rebuild indexes on restore. Indexes are derived, so this is both
simpler AND consistent by construction. Regression check: restore = SQLite snapshot →
replay outbox → indexes rebuilt to exact authority state. ✅ (This actually SIMPLIFIES
backup — a genuine win from the correction.)

**C4 — Embedding-service tier vs in-process floor produce DIFFERENT vectors.**
Root cause: the optional local embedding service may run a different model than the
in-process floor. Consequence: vectors from the two tiers are incomparable (the
version-partitioning problem, Issue 9, resurfaces per-tier). Fix: treat each embedding
tier as a distinct `model_version` in Qdrant (separate collection); dual-search during
any tier switch; never mix. Regression check: reuses existing version-partitioning
machinery. ✅

**C5 — Graph escape hatch (Dgraph) is a different data model (RDF-ish/GraphQL±).**
Root cause: migrating SQLite-adjacency → Dgraph is not a drop-in; query language
differs. Consequence: the GraphStore trait must abstract enough that the swap is real.
Fix: keep the GraphStore trait MINIMAL (add_entity/add_relationship/neighbors/
relationships_for/search_entities) — these map cleanly to both CTEs and Dgraph/Nebula.
Do NOT leak SQL or Cypher specifics through the trait. Regression check: trait already
minimal (32.1). ✅

**No further meaningful issues after C5.** Iteration converged.

## 35.2 Contradiction Audit (post-correction)

| Checked | Status |
|---|---|
| Duplicate source of truth | ✅ None — SQLite sole authority; Qdrant/Tantivy derived |
| Conflicting ownership | ✅ Write Policy Engine sole writer path |
| Transaction/consistency | ✅ Single local txn + outbox; no distributed 2PC |
| Vector/graph/FTS inconsistency | ✅ All rebuildable from authority; reconcile sweep |
| Event sourcing vs rebuild | ✅ Resolved Issue 1 — derived memory durable, log = audit |
| Backup/restore | ✅ Simplified by C3 — back up authority only |
| Crypto-shred | ✅ Key destroy; indexes purge shredded IDs on reconcile |
| Truth maintenance | 🟡 Volatile-unverifiable residual (Issue 20) — bounded, not eliminated |
| Merge/split | 🟡 Cascade still under-specified (pre-implementation blocker) |
| Multi-agent/namespace | ✅ Namespace + blackboard event region |
| Eventual consistency gap | 🟡 Index lag window exists (C1) — bounded by reconcile interval |

Two residual 🟡 items are known, bounded, and listed as blockers (35.5).

## 35.3 Architecture Decision Records (condensed)

**ADR-1 SQLite = transactional authority.** Decision: single ACID authority for
events+memories+graph+outbox. Alts: Postgres (server, heavier), multiple co-equal
stores (rejected — dual-write). Evidence: 30yr track record; collapses distributed
atomicity locally. Risk: single-writer ceiling under heavy multi-agent write. Migration:
Postgres (same SQL) if ever needed. Confidence: 95%.

**ADR-2 Qdrant = vector service.** Decision: Apache-2.0 Rust vector service, native
hybrid+filter+quant. Alts: LanceDB (embedded, valid; weaker filtering), pgvector
(coupled), usearch (RAM-only). Evidence: native RRF + payload filters match KRIA's
scope/sensitivity needs; quantization scales to future HW. Risk: separate process
lifecycle (C2). Migration: LanceDB behind VectorStore trait. Confidence: 85%.

**ADR-3 Tantivy = full-text.** Decision: MIT embedded Lucene-class FTS. Alts: SQLite
FTS5 (kept as fallback), Meilisearch (server, overkill). Evidence: BM25/faceting for
Library scale. Risk: index rebuild cost. Confidence: 85%.

**ADR-4 Graph = SQLite now, Dgraph/Nebula later.** Decision: CTEs to ~1M edges, then
Apache-2.0 graph service via trait. Alts: Neo4j (GPL — REJECTED), Memgraph (BSL —
REJECTED), Kuzu (archived — REJECTED). Evidence: license constraint eliminates mature
options; CTEs sufficient at scale. Risk: model-shift on migration (C5, mitigated by
minimal trait). Confidence: 80%.

**ADR-5 Event sourcing + outbox.** Decision: append-only log = audit/provenance/
erasure; outbox feeds indexes. Alts: dual-write (rejected). Evidence: industry-standard
(Kafka/Debezium pattern). Risk: log growth (tiered, Issue 14). Confidence: 90%.

**ADR-6 Crypto-shred for erasure.** Decision: per-subject keys, destroy to erase.
Evidence: Kafka/Axon/MongoDB CSFLE precedent; GDPR Recital 26. Risk: key-loss =
unrecoverable (intentional). Confidence: 90%.

**ADR-7 Write Policy Engine (fast/slow).** Decision: sole write path; <2ms
deterministic fast-path + async enrichment. Evidence: prevents dual-write, centralizes
governance/security/modes. Risk: logical single-authority (acceptable). Confidence: 88%.

**ADR-8 Cognitive Scheduler.** Decision: one owner for all background jobs, priority +
battery/thermal aware. Evidence: prevents writer starvation/battery drain. Confidence: 85%.

**ADR-9 Truth Maintenance.** Decision: staleness classes + evidence + supersession.
Risk: unverifiable-volatile residual. Confidence: 80%.

**ADR-10 Dreaming/consolidation.** Decision: trigger-based, LLM, evidence-gated,
re-enters via Write Policy. Evidence: Anthropic+OpenAI production. Risk: LLM-quality
dependence. Confidence: 85%.

## 35.4 Convergence Report

| Iteration | Focus | Found | Fixed | New regressions |
|---|---|---|---|---|
| 1 (Red Team, Sec 27) | 5 foundational contradictions | 17 | 17 | 0 |
| 2 (Red Team, Sec 27) | issues 18-30 | 13 | 13 | 0 |
| 3 (Independent, Sec 28) | new problems | 17 | 17 | 0 |
| 4 (Additions, Sec 32) | 8 missing subsystems | 8 | 8 | 0 |
| 5 (Evolution, Sec 33) | 10-20yr readiness | 0 arch | — | 0 |
| 6 (Corrected stack, Sec 34) | constraint correction | 5 changes | 5 | 5 new (C1-C5) |
| 7 (This review, Sec 35) | 3-engine contradictions | 5 (C1-C5) | 5 | 0 |

**Total: 55 issues found, 55 resolved. Iteration 7 introduced zero new issues → converged.**

**Convergence: ~93%.** The remaining ~7% is inherent uncertainty that ONLY
implementation + benchmarking can resolve (retrieval precision at scale, LLM-quality
dependence, merge/split cascade behavior under load).

**Confidence trajectory:** 6.5 (pre-Red-Team) → 7.5 (post-Red-Team) → **7.8**
(post-correction: Qdrant/Tantivy strengthen retrieval; C3 simplifies backup). Capped
below 8 pre-implementation — earned only by the two Phase-1 proofs.

## 35.5 Final Scores

| Dimension | Score | Note |
|---|---|---|
| Architecture | 8.5/10 | Single-authority + event sourcing + outbox is genuinely strong |
| Intelligence | 8/10 | Dreaming + TMS + compression spectrum + feedback loops |
| Reliability | 8/10 | WAL + rebuildable indexes + backup-authority-only (C3) |
| Scalability | 7.5/10 | Qdrant scales; graph escape hatch named; retrieval-precision unproven |
| Maintainability | 8/10 | Trait ports + single authority + minimal graph trait |
| Extensibility | 8.5/10 | Event log + traits + reserved fields + namespaces |
| Local-first | 9.5/10 | Everything runs local; Tantivy floor keeps FTS alive if Qdrant down |
| Privacy | 8.5/10 | Crypto-shred + encryption + consent; event-log honeypot is the caveat |
| Security | 8/10 | Deterministic write-gate + namespace isolation + OWASP ASI06 |
| Production Readiness | 7/10 | Design-complete; unbuilt + unbenchmarked |
| Future-Proofing | 8.5/10 | Named escape hatches; adopt-now-over-migrate honored |

## 35.6 MUST Fix Before Implementation (blockers)

1. **Retrieval-quality-vs-scale benchmark harness** — build FIRST, seed 500K synthetic memories. This is the metric that silently kills memory systems at 6 months. Non-negotiable.
2. **Prove single-authority + outbox → 2 indexes** under simulated crashes (SQLite txn → outbox → Qdrant + Tantivy replay → reconcile). If shaky, nothing above is safe.
3. **Fully specify merge/split cascade** — atomic behavior across SQLite + Qdrant + Tantivy + graph + Memory Worth + provenance. Currently under-specified.
4. **Wire the feedback signal-capture loop** (`referenced?` signal, 32.4) — retrieval self-improvement depends on it.
5. **Make encryption-at-rest default** — event log is a digital-life honeypot; encrypt SQLite + backups by default.

## 35.7 Can Safely Wait (later phases)

Multimodal pipeline · 3D UI · multi-device sync · Dgraph migration · local model
training · full plugin permission ecosystem · GraphRAG community summaries · advanced
dreaming. All have reserved abstractions; none blocks Phase 1-2.

## 35.8 Principal Engineer Verdict

**Would I personally build KRIA's memory on this architecture?** Yes.

**Would I approve implementation today?** Yes, gated on the 5 blockers above being
addressed in Phase 1 (benchmark + outbox proof are the two hard gates; the other three
are Phase-1-scoped work items, not research risks).

**Has it converged?** Yes — iteration 7 produced zero new architectural issues, only
the two known bounded residuals (unverifiable-volatile truth, merge/split cascade),
both of which are implementation work items, not design flaws. Further review would
yield wording, not architecture.

**Engineering evidence for the verdict:** (a) exactly one transactional authority
eliminates the distributed-consistency class of bugs; (b) all indexes rebuildable →
corruption = rebuild not data-loss; (c) backup simplified to authority-only (C3); (d)
every rejected technology rejected on documented licensing/maturity evidence, not
taste; (e) every major decision has an ADR with a named migration path; (f) the two
genuine long-term risks (retrieval precision, graph edges) both have pre-built escape
hatches (benchmark gate + GraphStore trait). The only thing standing between this and
production-grade is empirical validation — which is what Phase 1 is for.

---

# VERDICT: **APPROVED WITH MINOR CHANGES**

"Minor" because the 5 blockers are Phase-1 work items with known solutions, not
architectural redesigns. The architecture itself is converged, internally consistent,
license-clean, local-first, and competitive with the named commercial systems on its
target axis (local-first + private + offline + OS-integrated). Build Phase 1 with the
benchmark and outbox-proof as the first two deliverables; the rest follows safely.

*New evidence this section: Neo4j GPLv3 + commercial ([neo4j.com/licensing](https://neo4j.com/licensing/)),
Memgraph BSL/commercial ([memgraph.com/pricing](https://memgraph.com/pricing)), Dgraph
Apache-2.0 core ([github.com/hypermodeinc/dgraph](https://github.com/hypermodeinc/dgraph)),
Qdrant Apache-2.0 hybrid+RRF+quantization ([qdrant.tech](https://qdrant.tech/documentation/tutorials-basics/cloud-inference-hybrid-search/),
[encore.dev/pgvector-vs-qdrant](https://encore.dev/articles/pgvector-vs-qdrant)),
Kuzu archived Oct 2025, Tantivy MIT. Content rephrased for compliance.*


---
---
---

# SECTION 36 — SPEC-READY CANONICAL SPECIFICATION

> **AUTHORITATIVE.** This section is the single source of truth for spec generation.
> Where any earlier section conflicts, THIS section wins. Sections 1-35 are the
> reasoning/evidence trail; Section 36 is the settled specification. A Kiro spec
> (requirements.md / design.md / tasks.md) should be generated from this section.

## 36.0 Scope & Non-Goals

**In scope (v1 → phased):** local-first cognitive memory for a desktop assistant —
storage, write governance, retrieval, truth maintenance, consolidation/dreaming,
lifecycle, library, memory modes, privacy/erasure, backup/restore, observability,
subsystem + OpenClaw integration, benchmarking.

**Explicit non-goals (do NOT build in v1; reserve abstractions only):** multi-device
sync, cloud services, multimodal (image/audio/video) retrieval pipeline, local model
training/LoRA, 3D visualization, autonomous multi-agent orchestration, third-party
plugin marketplace. These are Phase 4-6, gated by evidence.

## 36.1 Canonical Technology Stack (FINAL — supersedes §7/§9/§23)

| Layer | Technology | License | Status |
|---|---|---|---|
| Transactional authority | SQLite (WAL, FTS5 fallback) | Public Domain | Required P1 |
| Vector index | Qdrant (local service) | Apache-2.0 | Required P1 (LanceDB acceptable alt behind trait) |
| Full-text index | Tantivy (embedded) | MIT | Required P2 (SQLite FTS5 is the P1 floor) |
| Graph (≤1M edges) | SQLite adjacency + recursive CTEs | Public Domain | Required P3 |
| Graph (>1M edges) | Dgraph / NebulaGraph (local service) | Apache-2.0 | Deferred, via GraphStore trait |
| Embeddings (floor) | ONNX in-process (EmbeddingGemma-300M / MiniLM fallback) | Apache-2.0 | Required P1 |
| Embeddings (optional) | Local service (Ollama/ONNX server) | Apache-2.0 | Optional P3 |
| Consistency | Transactional outbox → indexes | — | Required P1 |
| Runtime | Tokio; caches dashmap(P1)/moka(P2); rayon(P4) | MIT/Apache | Phased |
| Crypto | blake3 (checksums), age/libsodium (shred + backup) | Apache/MIT/BSD | Required P1 |

**Invariant I-1:** SQLite is the ONLY transactional authority. Qdrant + Tantivy are
DERIVED, REBUILDABLE indexes fed exclusively by the transactional outbox. No component
may write a durable fact except through the Write Policy Engine.

## 36.2 Component Inventory (ownership + contract)

| Component | Owns | Reads via | Writes via |
|---|---|---|---|
| Write Policy Engine | the only write path (fast+slow) | — | SQLite txn + outbox |
| Event Log | immutable audit/provenance/erasure | forensic API | append-only (fast path) |
| Memory Store (derived) | durable memory state | Retrieval | Write Policy slow path |
| VectorStore (trait→Qdrant) | embeddings index | Retrieval | outbox consumer |
| SearchStore (trait→Tantivy) | FTS index | Retrieval | outbox consumer |
| GraphStore (trait→SQLite) | entities+relationships | Retrieval | Write Policy |
| Retrieval Orchestrator | multi-strategy fusion + budget | — | read-only |
| Truth Maintenance | staleness/evidence/supersession | consolidation | Write Policy |
| Cognitive Scheduler | ALL background jobs (priority/battery) | — | arbitrates writer |
| Consolidation/Dreaming | compression + reflection | Retrieval | Write Policy (self=untrusted) |
| Entity Resolution | canonicalize/merge/split | GraphStore | Write Policy |
| Library Manager | documents + chunks + provenance | Retrieval | Write Policy |
| Memory Worth | success/failure governance | Retrieval | slow path |
| Knowledge Gap Engine | gap tracking → learning goals | Retrieval | Write Policy |
| Backup/Restore | authority-only snapshots | — | SQLite + outbox cursor |
| Observability | explain/metrics/audit | all | audit log |

**Invariant I-2:** every subsystem uses ONLY the Memory API Contract (§32.6). No direct
storage access. This is the seam that makes storage swappable.

## 36.3 Requirements → Acceptance Criteria (EARS-style, testable)

Each requirement is spec-ready with WHEN/THEN acceptance criteria a task can verify.

**R1 Write governance.** WHEN any subsystem submits a WriteCandidate, THEN it MUST
pass through the Write Policy Engine; no durable write may occur by any other path.
- AC1: attempting a direct store write outside the engine fails a test-suite invariant.
- AC2: the fast path completes in <2ms p95 and never calls an LLM.
- AC3: the raw event is persisted even if slow-path enrichment later fails.

**R2 Memory modes.** WHEN mode = Incognito, THEN zero durable writes occur; WHEN
Temporary, THEN writes purge at session end; WHEN Workspace, THEN personal-scope
writes are rejected.
- AC: mode is always queryable + surfaced; a mode-switch emits a boundary event.

**R3 Temporary chats never persist.** WHEN a chat is marked temporary/incognito, THEN
no facts/embeddings/summaries/graph edges/reflections are created.

**R4 Selective write filtering.** WHEN a tool fails/retries/cancels or emits noise,
THEN it is logged to the execution log but NOT promoted to semantic memory (unless the
user explicitly says "remember").

**R5 Truth maintenance.** WHEN a fact's staleness class threshold elapses AND source ≠
user_stated, THEN retrieval flags it "possibly stale." WHEN two facts contradict, THEN
the deterministic resolution order applies (user-stated > recent-verified > higher
Memory-Worth > else surface to user).

**R6 LLM-independent degradation.** WHEN no LLM/GPU/internet/embeddings are available,
THEN storage + keyword/graph retrieval still function; enrichment/consolidation queue.
- AC: an integration test with LLM + embedder disabled still stores and recalls.

**R7 Consent-gated cold start.** WHEN first run, THEN no filesystem/git/shell scan
occurs before explicit per-source consent; default = onboarding questions only.

**R8 Library per-item erasure.** WHEN a library item is deleted, THEN its file, chunks,
vectors, and EVERY memory whose provenance = that item are removed/flagged, and its
crypto-shred key is destroyed.
- AC: after deletion, no retrieval returns content derived from that item.

**R9 Right-to-be-forgotten.** WHEN "forget X", THEN X's crypto-shred key is destroyed
(ciphertext in the immutable log becomes unreadable) AND derived memories cascade-delete.

**R10 Backup/restore.** WHEN backup runs, THEN ONLY the SQLite authority + outbox
cursor are captured; WHEN restore runs, THEN indexes (Qdrant/Tantivy) are rebuilt to
exact authority state. Backups are versioned, self-describing, checksummed, encrypted.
- AC: crash-then-restore reproduces identical retrieval results.

**R11 Consistency (dual index).** WHEN the outbox has pending entries, THEN a
reconciliation sweep guarantees eventual convergence of both Qdrant and Tantivy to the
SQLite authority; orphans in either index are purged.

**R12 Retrieval quality at scale (RELEASE GATE).** WHEN the memory bank grows to 500K
synthetic memories, THEN Recall Precision MUST NOT degrade below the baseline
threshold. This is a hard gate; failure blocks release.

**R13 Crash safety.** WHEN power loss/OS crash occurs mid-write, THEN WAL replay +
idempotent outbox drain recover with zero authority data loss.

**R14 Merge/split atomicity.** WHEN two memories merge (or one splits), THEN the
operation is atomic across SQLite + Qdrant + Tantivy + graph + Memory Worth +
provenance; derived_from chains are preserved; the operation is reversible ≤30 days.

**R15 Feedback learning.** WHEN a feedback event (thumbs/correction/undo/edit/ignored)
occurs, THEN Memory Worth + confidence calibration + adaptive retrieval weights update.

**R16 Explainability.** WHEN any memory is recalled, THEN explain(id) returns
provenance chain, retrieval strategy, confidence, evidence, staleness, Memory Worth.

**R17 Encryption at rest (default).** WHEN memory is initialized, THEN SQLite + Qdrant
data + backups are encrypted at rest by default; secrets are keychain-referenced, never stored.

**R18 Scope isolation.** WHEN scope = client/workspace, THEN retrieval never returns
cross-scope memories unless explicitly global or user-promoted (test-suite invariant).

**R19 Resource governance.** WHEN on battery/low-power OR memory-pressure high, THEN
the Cognitive Scheduler suspends P3/P4 background jobs; foreground writes always preempt.

**R20 Bounded growth.** WHEN storage crosses 80% of budget, THEN warn; at 95%, THEN
aggressive archival; storage never grows unbounded.

## 36.4 Consolidated Edge-Case Catalog (must be handled/tested)

| # | Edge case | Required handling |
|---|---|---|
| E1 | Power loss mid-write | WAL replay + outbox idempotent drain (R13) |
| E2 | Qdrant service down at startup | Degrade to Tantivy/FTS5 + graph; supervise+restart (C2) |
| E3 | Index lag (searchable by one index not other) | Per-index outbox cursors + reconcile sweep (C1/R11) |
| E4 | Backup consistency skew | Back up authority only; rebuild indexes (C3/R10) |
| E5 | Embedding model upgrade | Version-partitioned collections + dual-search + bg re-embed |
| E6 | Two embedding tiers differ | Each tier = distinct model_version (C4) |
| E7 | LLM unavailable | Heuristic extraction; queue consolidation (R6) |
| E8 | Graph cycle | Visited-set + depth cap (mandatory) |
| E9 | Graph >1M edges | GraphStore trait swap to Dgraph/Nebula (C5) |
| E10 | Reflection self-poisoning | Self-output re-enters as untrusted, evidence-gated |
| E11 | Infinite consolidation loop | Compression-level ceiling + content-hash idempotency (N3) |
| E12 | Wrong entity merge (two people) | Conservative; identifier-gated; reversible (N5) |
| E13 | Goal explosion | Candidate goals + cap + decay to paused/abandoned (N6) |
| E14 | Confidence inflation loop | Log-capped gains + periodic challenge (N13) |
| E15 | Knowledge drift over years | Source episodes retained + grounding checks (N15) |
| E16 | Injection-vulnerable scanner | Deterministic fast-path scan; LLM advisory only (N16) |
| E17 | Orphans (vector/edge/chunk/key) | Weekly reconciliation sweep vs authority (N12) |
| E18 | Clock drift / DST / timezone | UTC + offset stored; HLC ordering, not wall-clock (N10) |
| E19 | Split-brain after restore on 2nd device | Event-union merge, no authoritative copy (N9) |
| E20 | Massive doc import (GB-scale) | Streamed, checkpointed, resumable, bg job (N11) |
| E21 | Crypto-shred key loss | Unrecoverable by design; UX warning + export-before |
| E22 | Volatile-unverifiable fact (mood) | Fast decay + low-confidence surface, never asserted (Issue 15) |
| E23 | Interrupted consolidation/reflection | Checkpointed, resumable, per-batch atomic (N14) |
| E24 | Corrupted index | Rebuild from authority (indexes are derived) |
| E25 | Corrupted SQLite | integrity_check on startup → restore from backup |
| E26 | Repository/project rename | Entity alias, not new entity |
| E27 | Workspace deletion | Cascade delete workspace-scoped; keep global |
| E28 | Duplicate library import | SHA-256 dedup at ingest |
| E29 | Writer starvation (bg vs live) | Two-queue scheduler; bg yields ≤50ms batches (N2) |
| E30 | Plugin writes to core | Namespace enforcement; plugin→own namespace only (N17) |

## 36.5 Canonical Data Model (spec-ready entities)

Authoritative entity set (consolidates §5; adds correction-era fields). All persisted
in SQLite (authority); embeddings referenced by id in Qdrant.

```
Event (append-only, immutable)
  id:UUIDv7 · ts:UTC · tz_offset · event_type · source · payload(encrypted if sensitive)
  · session_id · parent_event_id · shred_key_id · checksum:blake3 · hlc:hybrid_logical_clock

Memory (derived, mutable)
  id:UUIDv7 · content · memory_type · compression_level(0-3) · source_event_id
  · namespace · owner_id · device_id · scope(global|company|client|workspace|session)
  · confidence · importance(0-10) · access_count · decay_score
  · staleness_class(immutable|permanent|slow|volatile_verifiable|volatile_unverifiable)
  · sensitivity(public|private|secret) · state(active|promoted|compressed|archived|forgotten)
  · created_at · last_accessed · valid_from · valid_until · embedding_id · embedding_model_version
  · estimated_tokens · derived_from[] · contradicted_by[] · supports[]
  · memory_worth_success · memory_worth_failure · verify_against(optional predicate)

Episode · Goal(kind: oneshot|recurring|ambition) · Entity(aliases[],canonical_id,merged_from[])
Relationship(source,target,type,strength,valid_from,valid_until,evidence_event_id)
Reflection · Preference · ReasoningTrace · LibraryItem(sha256,collections[],version)
LibraryChunk(item_id,chunk_index,embedding_id,modality,embedding_model)
FeedbackEvent(target_id,signal,context,ts) · KnowledgeGap(query,domain,times_missed,resolved)
OutboxEntry(memory_id,index_target,op,cursor,attempts) · ShredKey(subject_id,key,status)
```

**Reserved-now fields (near-zero cost, prevent rewrites):** device_id, owner_id,
scope, modality, embedding_model_version, feedback_signal, preference_pair_id,
training_eligible, hlc. These enable multi-device, multi-user, multimodal, and future
training WITHOUT schema migration.

## 36.6 Phased Implementation Plan (spec milestones with Definition-of-Done)

**PHASE 1 — Foundation & Governance** *(the two hard gates live here)*
Deliverables: SQLite authority + event log + transactional outbox; Qdrant integration
behind VectorStore trait; ONNX embeddings (floor); Write Policy Engine (fast/slow);
Memory Modes; Cognitive Scheduler (priority + writer arbitration + battery); Memory API
Contract; feedback event type; encryption-at-rest default; **retrieval-quality-vs-scale
benchmark harness (seed 500K)**; **crash-tested outbox→index proof**.
DoD: R1,R2,R3,R6,R13,R17 pass; **R12 benchmark green**; outbox crash-recovery proven.

**PHASE 2 — Intelligence & Governance**
Deliverables: multi-strategy retrieval + adaptive RRF + token budget; Tantivy;
Truth Maintenance; importance model + Memory Worth (normalized); dreaming/consolidation
(trigger-based, evidence-gated); deletion granularity + undo + export; observability
(explain/metrics/audit); Runtime Budget Manager; **merge/split atomic cascade (R14)**.
DoD: R4,R5,R11,R14,R15,R16,R19,R20 pass; consolidation idempotent + resumable.

**PHASE 3 — Cognition & Relationships**
Deliverables: GraphStore (entities/relationships + cycle-safe CTEs); Entity Resolution
Engine; goals (recurring/ambitions) + temporal NL resolver; salience/attention loop
(event-driven, power-aware); Knowledge Gap Engine; episode boundaries; progressive
compression; embedding version partitioning; optional embedding service; Memory UI Tier 1-2.
DoD: R18 scope-isolation invariant passes; graph cycle-safe; entity merges reversible.

**PHASE 4 — Library & Knowledge**
Deliverables: Library ingestion (streamed/resumable) + per-item cascade + citations
(R8); document intelligence (entity/relation extraction); GraphRAG community summaries;
cross-encoder rerank (Library QA only); rayon batch jobs.
DoD: R8 per-item erasure passes; large-import (GB) does not OOM or block UI.

**PHASE 5+ — Horizon (reserved, evidence-gated):** multimodal pipeline · multi-device
sync (event-union) · Dgraph migration (if >1M edges) · local model training · 3D UI ·
plugin permission ecosystem · advanced dreaming.

## 36.7 Test Strategy (spec-ready)

- **Invariant tests:** I-1 (single writer), I-2 (API-only access), R18 (no cross-scope leak).
- **Property tests:** outbox idempotency; event replay determinism (indexes only);
  merge/split reversibility; decay monotonicity per staleness class.
- **Crash-injection tests:** power-loss at each write stage → recover to authority state.
- **Scale benchmark (release gate):** Recall Precision vs bank size (10K→500K), p95 latency.
- **Degradation tests:** LLM off / embedder off / Qdrant off → core still works.
- **Security tests:** injected-instruction content never stored as fact; plugin
  cross-namespace write rejected; shredded content unrecoverable + unretrievable.
- **Privacy tests:** forget(X) → zero retrieval returns X-derived content.

## 36.8 Definition of Production-Grade

KRIA memory is production-grade WHEN: all Phase 1-2 ACs pass; R12 benchmark green at
500K; crash-injection zero-loss; degradation tests pass; security + privacy tests pass;
merge/split cascade proven atomic; encryption-at-rest default on. Confidence to reach
this from current design: high (75-80%), gated only by empirical validation.

---

**Spec-generation note:** requirements.md ← §36.3 (R1-R20) + §36.4 (edge cases as
acceptance criteria). design.md ← §36.1 (stack) + §36.2 (components) + §36.5 (data
model) + reasoning from §27-35. tasks.md ← §36.6 (phases/DoD) + §36.7 (tests). Sections
1-35 are the evidence appendix; Section 36 is the contract.

*End of canonical specification. Verdict (§35): APPROVED WITH MINOR CHANGES — the 5
Phase-1 blockers (benchmark, outbox-proof, merge/split cascade, feedback capture,
default encryption) are work items with known solutions, not redesigns.*


---
---
---

# SECTION 37 — CONTRIBUTOR REFERENCE (ADRs · Invariants · Glossary · Dependency Map · Rejected Designs)

> Maintainability layer. New contributors read this first. It consolidates scattered
> decisions into stable, consistent reference material. Nothing here is new
> architecture — it is the settled record in maintainable form.

## 37.1 Document Layering (stable vs volatile)

To keep the architecture stable while implementation evolves, the doc has three layers:

```
STABLE (rarely changes)      → Architecture Invariants (37.2), ADR *decisions* (37.3),
                                component contracts (§36.2), data-model shape (§36.5)
SEMI-STABLE (versioned)      → Technology choices (§36.1) — recorded as ADRs so a swap
                                is a new ADR revision, not a doc rewrite
VOLATILE (expected to drift) → exact crate names/versions, millisecond targets, specific
                                model names, tuning constants
```
**Directive:** VOLATILE details in this doc (e.g., "EmbeddingGemma-300M", "<2ms",
"moka", "k=60") are illustrative *current* choices. They belong long-term in the Kiro
spec / Technology Decision Record, NOT the stable architecture. When they change, update
the ADR + spec, not the invariants. Flow: **Architecture → ADR → Kiro Spec → Implementation.**

## 37.2 Architecture Invariants (THE LAWS — MUST NEVER be violated)

These are non-negotiable. Any change to these = a new architecture, not a revision.

- **L1** — An immutable, append-only event log always exists.
- **L2** — SQLite is the sole transactional authority. All other stores are derived + rebuildable.
- **L3** — No subsystem writes durable state directly. Everything goes through the Write Policy Engine.
- **L4** — All persisted derived state is rebuildable EXCEPT LLM-derived memory content (which is itself durable, per Issue 1) — indexes (vector/FTS/graph) are always rebuildable from the authority.
- **L5** — Provenance is never lost; every memory traces to its source event(s); compressed memories carry `derived_from`.
- **L6** — Every memory is explainable (provenance + retrieval path + confidence).
- **L7** — Plugins/skills/agents never bypass namespace isolation; they write only to their own namespace.
- **L8** — Memory functions with no LLM / no GPU / no internet / no embeddings (degraded, never dead).
- **L9** — Erasure is honored via crypto-shredding; "forget" makes data cryptographically unreadable + cascades derived memories.
- **L10** — Reads never block on the writer; only the atomic commit holds the single writer.
- **L11** — Self-generated memory (reflection/dreaming) re-enters as untrusted through the Write Policy, same scrutiny as external input.
- **L12** — Retrieval quality must not degrade as the memory bank grows (release gate, R12).

## 37.3 Architecture Decision Records (canonical registry)

Consistent format. Supersedes the condensed ADRs in §35.3. When a decision changes,
bump the ADR revision — do not scatter the change across the doc.

**ADR-001 — SQLite as transactional authority**
- Problem: need one consistent source of truth on local hardware.
- Decision: SQLite (WAL) owns events, memories, graph adjacency, outbox, goals, prefs.
- Alternatives: Postgres (server, heavier), multiple co-equal stores (dual-write bug class).
- Pros: 30yr proven, ACID, embedded, backup=file, collapses distributed atomicity locally.
- Cons: single-writer ceiling under heavy multi-agent write.
- Risk: writer contention (mitigated by Cognitive Scheduler, L10).
- Migration: → Postgres (same SQL dialect) if ever needed.
- Confidence: 95%.

**ADR-002 — Qdrant as vector index** (revises earlier LanceDB choice)
- Problem: scalable hybrid + filtered vector search, permissive license.
- Decision: Qdrant (Apache-2.0, Rust, local service) behind VectorStore trait.
- Alternatives: LanceDB (embedded, valid alt; weaker filtering), pgvector (coupled), usearch (RAM-only).
- Pros: native hybrid+RRF, payload filtering (scope/sensitivity), quantization, scales to future HW.
- Cons: separate process lifecycle.
- Risk: service down (mitigated — Tantivy/FTS5 floor, L8).
- Migration: → LanceDB behind trait.
- Confidence: 85%.

**ADR-003 — Full-text: Tantivy (+ SQLite FTS5 floor)**
- Decision: Tantivy (MIT, embedded, Lucene-class) as primary FTS; FTS5 as P1 floor + fallback.
- Alternatives: FTS5-only (weaker), Meilisearch (server, overkill).
- Confidence: 85%.

**ADR-004 — Graph: SQLite CTEs now, Dgraph/NebulaGraph later**
- Decision: SQLite adjacency + cycle-safe CTEs to ~1M edges; Apache-2.0 graph service beyond, via GraphStore trait.
- Alternatives REJECTED: Neo4j (GPLv3 copyleft), Memgraph (BSL), SurrealDB (BSL), Kuzu (archived).
- Risk: model-shift on migration (mitigated by minimal trait, C5).
- Confidence: 80%.

**ADR-005 — Event sourcing + transactional outbox**
- Decision: append-only log = audit/provenance/erasure; outbox feeds indexes idempotently.
- Alternatives: dual-write (rejected — consistency bug class).
- Confidence: 90%.

**ADR-006 — Crypto-shredding for erasure**
- Decision: per-subject keys; destroy key to satisfy GDPR over immutable log.
- Evidence: Kafka/Axon/MongoDB CSFLE; GDPR Recital 26.
- Risk: key loss = unrecoverable (intentional). Confidence: 90%.

**ADR-007 — Write Policy Engine (fast/slow split)**
- Decision: sole write path; deterministic <2ms fast-path + async enrichment slow-path.
- Confidence: 88%.

**ADR-008 — Cognitive Scheduler**
- Decision: one owner for all background jobs; priority classes + battery/thermal/memory awareness.
- Confidence: 85%.

**ADR-009 — Truth Maintenance System**
- Decision: staleness classes + evidence tracking + supersession + deterministic conflict order.
- Risk: unverifiable-volatile residual. Confidence: 80%.

**ADR-010 — Dreaming / consolidation**
- Decision: trigger-based, LLM-driven, evidence-gated, re-enters via Write Policy (untrusted).
- Evidence: Anthropic + OpenAI production dreaming. Risk: LLM-quality dependence. Confidence: 85%.

**ADR-011 — Memory Worth (governance)**
- Decision: normalized, difficulty-adjusted, min-sample-gated success/failure signal; soft re-rank, never hard-delete.
- Confidence: 80%.

**ADR-012 — Embeddings (in-process floor + optional service)**
- Decision: ONNX in-process (works when all else down) + optional local service for stronger model; version-partitioned.
- Confidence: 85%.

**ADR-013 — Memory Modes**
- Decision: Permanent/Temporary/Incognito/Workspace/Library-only/Read-only/Guest/Developer/Benchmark/Safe/Research; enforced at Write Policy; always visible.
- Confidence: 88%.

## 37.4 Dependency Map (impact analysis)

```
                          ┌─────────────────────────────────────┐
   KRIA Subsystems        │  Intent Compiler · Planner ·         │
   (consumers)            │  Reasoner · Execution Engine ·       │
                          │  OpenClaw · Evolution/Discovery ·    │
                          │  Jobs · Library · Workspace ·        │
                          │  Frontend · (future) Multi-Agent     │
                          └───────────────────┬─────────────────┘
                                              │  (read + write)
                                              ▼
                          ┌─────────────────────────────────────┐
   Memory API Contract    │  observe/remember/search/reason/     │  ← I-2: ONLY entry
   (§32.6) — the seam     │  forget/reflect/consolidate/backup…  │
                          └───────────────────┬─────────────────┘
                    ┌─────────────────────────┼──────────────────────────┐
                    ▼ (writes)                ▼ (reads)                   ▼ (background)
        ┌───────────────────────┐  ┌────────────────────┐   ┌────────────────────────┐
        │ WRITE POLICY ENGINE   │  │ RETRIEVAL          │   │ COGNITIVE SCHEDULER     │
        │ (L3 — sole writer)    │  │ ORCHESTRATOR       │   │ (owns all bg jobs)      │
        │ fast(<2ms)+slow(async)│  │ vector+FTS+graph+  │   │ consolidation·dreaming· │
        └──────────┬────────────┘  │ temporal → RRF     │   │ decay·reconcile·entity· │
                   │               └─────────┬──────────┘   │ backup·salience         │
                   ▼ atomic txn              │ read-only     └───────────┬────────────┘
        ┌───────────────────────────────────┼───────────────────────────┘
        ▼                                    ▼
   ┌─────────────────────┐          ┌─────────────────────────────────────────────┐
   │ SQLite (AUTHORITY)  │──outbox─▶│ DERIVED INDEXES (rebuildable, L2/L4):        │
   │ events·memories·    │          │  Qdrant (vectors) · Tantivy (FTS) ·          │
   │ graph·goals·outbox  │          │  [graph in SQLite now]                       │
   └─────────┬───────────┘          └─────────────────────────────────────────────┘
             │                                    ▲
             ▼ (embeds via)                       │ (rebuild from authority)
   ┌─────────────────────┐          Filesystem: Library files · encrypted backups · models
   │ Embeddings (ONNX    │
   │ floor + opt service)│          Backup = SQLite authority + outbox cursor ONLY (C3).
   └─────────────────────┘          Restore = snapshot → replay → rebuild indexes.
```
**Impact rule:** changing an index (Qdrant/Tantivy) or embeddings affects only the
rebuild path — never the authority. Changing SQLite schema affects everything downstream
(additive-only migrations, L2). Changing the API Contract affects all consumers (version it).

## 37.5 Rejected Designs Registry (do NOT reopen)

Settled decisions with the reason. Re-litigate only with NEW evidence that invalidates
the stated reason.

| Rejected | Category | Reason rejected | Reopen only if |
|---|---|---|---|
| **Neo4j** | Graph DB | GPLv3 copyleft + commercial enterprise → fails hard licensing | it relicenses to Apache/MIT/BSD |
| **Memgraph** | Graph DB | BSL license → fails hard licensing | relicenses permissively |
| **SurrealDB** | Multi-model | BSL license + heavy binary | relicenses permissively + proven at scale |
| **Kuzu** | Embedded graph | Archived Oct 2025 (unmaintained) | active maintained fork emerges (MIT) |
| **Oxigraph** | RDF graph | Self-described unstable "hobby project" | reaches stable + optimized release |
| **DuckDB** | Storage | OLAP analytics engine; wrong (OLTP) workload | never (wrong workload class) |
| **Dedicated graph DB (now)** | Graph | Overkill at ≤1M edges; adds process/backup/failure surface | edges >1M OR 2-hop >25ms (→ Dgraph/Nebula) |
| **Multiple co-equal SQLite DBs** | Storage | Dual-write consistency bug class; the CURRENT KRIA problem | never (violates L2) |
| **HyDE** | Retrieval | Advantage collapsed to 1-4 nDCG vs modern embeddings; +25-40% latency | embeddings regress (unlikely) |
| **ColBERT** | Retrieval | 32× storage for per-token embeddings; overkill for personal memory | storage becomes free + precision-critical |
| **Cross-encoder (default path)** | Retrieval | 50-200ms latency on every recall | reserved for Library QA only (kept there) |
| **parking_lot** | Concurrency | Near-zero contention in single-writer model; marginal gain | contention profile changes materially |
| **usearch (as primary)** | Vector | RAM-only; Qdrant/LanceDB supersede (disk + filter + version) | a RAM-only niche appears |
| **Event-log-as-rebuild-source** | Event sourcing | LLM extraction non-deterministic (Issue 1) — derived memory is durable | never (contradicts determinism) |
| **Tombstone-only deletion** | Erasure | Leaves data recoverable → fails GDPR | never (crypto-shred required) |
| **Fixed-calendar consolidation** | Cognition | Real usage isn't clock-regular | never (trigger-based is superior) |

## 37.6 Glossary (contributor onboarding)

- **Event** — immutable, append-only record of something that happened; source of truth for audit/provenance/erasure (NOT for rebuilding LLM-derived memory).
- **Memory** — a derived, durable, mutable knowledge unit produced from events via the Write Policy Engine.
- **Episode** — a bounded span of related activity (a "session chapter"); immutable once closed; summarized on consolidation.
- **Session** — an interaction span (start: first input or after >2h idle; end: close/quit/2h idle; rolls at local midnight for 24/7 use).
- **Working memory** — ephemeral per-turn state (goal, memo cache, focus); never persisted (TurnMemory + Cognitive State).
- **Semantic memory** — durable facts/knowledge, decay-governed.
- **Procedural memory** — reusable workflow skills extracted from repeated successful sessions.
- **Goal memory** — persistent goals (oneshot/recurring/ambition) with progress + resumption context.
- **Reflection** — LLM-produced meta-observation over recent memory; re-enters as untrusted (L11).
- **Dreaming / consolidation** — background process that compresses (episode→skill→rule), merges, decays, and reflects.
- **Truth Maintenance (TMS)** — subsystem ensuring KRIA never confidently relies on stale/contradicted facts (staleness classes + evidence + supersession).
- **Memory Worth** — normalized success/failure co-occurrence signal governing retrieval priority + archival.
- **Importance** — 0-10 creation-time score (novelty/goal-relevance/authority/emphasis/surprise) setting decay rate.
- **Staleness class** — immutable / permanent / slow / volatile-verifiable / volatile-unverifiable; governs re-verification, not deletion.
- **Compression level** — 0 raw → 1 episode → 2 skill → 3 rule (Experience Compression Spectrum).
- **Namespace** — isolation scope (`core`, `plugin/{id}`, `openclaw/{id}`, `agent/{id}`); enforces L7.
- **Scope** — knowledge partition (global/company/client/workspace/session) for isolation + selective sharing.
- **Provenance** — the chain from any memory back to its source event(s)/library item; never lost (L5).
- **Crypto-shredding** — erasure by destroying a per-subject encryption key, making ciphertext unreadable.
- **Transactional outbox** — pattern where index updates are queued in the same SQLite txn as the write, then relayed idempotently to Qdrant/Tantivy.
- **Write Policy Engine** — the sole write gateway (mode/quality/dedup/contradiction/confidence/provenance/security/commit).
- **Cognitive Scheduler** — sole owner of background jobs; priority + battery/thermal/memory aware.
- **Capability (CKB)** — Capability Knowledge Base; per-tool/skill success stats + decisions + benchmarks.
- **Salience loop** — event-driven, power-aware proactive-recall surfacing.
- **Knowledge Gap** — a recorded "what KRIA doesn't know," feeding learning goals.
- **HLC** — Hybrid Logical Clock; drift-tolerant event ordering (not wall-clock).

---

*Section 37 is the maintainability layer. It changes when DECISIONS change (new ADR
revision), not when reasoning is added. Invariants (37.2) are the system's constitution;
everything else serves them.*


---
---

# 38. FINAL CONVERGENCE PASS (Implementation-Ready Addendum)

**Status:** Authoritative pre-implementation convergence. Additive only — no core
decision is reversed. This section resolves the last ten open critical issues, hardens
production concerns, validates future extension points, and records the convergence
report + readiness score. Where it refines an earlier statement, it *clarifies* an
invariant; it never breaks one. Vector index / full-text index are referred to
neutrally as **the vector index** (LanceDB v1; Qdrant escape hatch behind the
`VectorStore` trait) and **the FTS index** (SQLite FTS5 v1; Tantivy behind the
`SearchStore` trait) — the trait abstraction (ADR-002/003) makes the concrete choice
reversible and is preserved.

> Evidence conventions: claims that depend on external behavior cite the primary
> source inline. Content from external sources is rephrased; no verbatim quotes.

---

## 38.1 — Cross-Process Concurrency (Issue 1)

**Question.** "Single writer" was stated without defining *scope*. KRIA runs multiple
processes that can open the same DB: `kria-desktop`, `kria-server`, a future CLI,
OpenClaw skill containers, sidecars. Is single-writer enough?

**Evidence.** SQLite in WAL mode already permits many processes to open one database;
readers never block the single writer, and a **second concurrent writer receives
`SQLITE_BUSY`, not corruption** — ret/ry is handled by `PRAGMA busy_timeout`
([sqlite.org/wal](https://www.sqlite.org/wal.html);
[SQLite multi-process Q&A](https://stackoverflow.com/questions/10325683/can-i-read-and-write-to-a-sqlite-database-concurrently-from-multiple-connections)).
`BEGIN CONCURRENT` (wal/wal2) can admit multiple writers but still serializes `COMMIT`
([BEGIN CONCURRENT](https://www.sqlite.org/src/doc/begin-concurrent/doc/begin_concurrent.md)).
The known corruption case is a database on a **networked filesystem** where locking is
unreliable. *(Rephrased for licensing compliance.)*

**Resolution — the writer-leader lease contract (no redesign; SQLite does the heavy
lifting).** The invariant is refined: **single writer *per process* (L10) + exactly
one writer-leader *across processes* (new L13)**, with SQLite's own file lock as the
correctness backstop.

1. **DB is local-filesystem only (new L14).** The authority DB must live on a local
   POSIX/NTFS volume, never NFS/SMB/network mounts. Startup detects a network mount and
   refuses to open in writer mode (degrades to read-only with a clear error). This
   removes the one real corruption vector.
2. **Writer-leader lease.** A single row `writer_lease { holder_pid, holder_uuid,
   hostname, acquired_at, heartbeat_at, lease_ttl }` in the authority DB elects one
   process as the writer-leader. Acquisition uses an `IMMEDIATE` transaction (acquire
   lock now or fail fast). The leader renews `heartbeat_at` every `ttl/3`. A process
   whose lease is stale (`now - heartbeat_at > lease_ttl`, default 30 s) may steal the
   lease *only* inside an `IMMEDIATE` transaction (atomic compare-and-swap on
   `holder_uuid`), which is safe because SQLite serializes that write.
3. **Everyone else is a reader or an RPC client.** Non-leader processes:
   - **Read** directly via WAL read connections (no coordination needed — readers never
     block, L10).
   - **Write** by sending `WriteCandidate`s to the leader over a local IPC channel (the
     desktop app's existing local API / a Unix domain socket / named pipe). The leader
     runs the one Write Policy Engine. This preserves L3 (one write gate) across
     processes without two writers ever touching the DB.
4. **`SQLITE_BUSY` is still handled** (`busy_timeout`, `IMMEDIATE` transactions) as a
   belt-and-suspenders backstop even though the lease makes contention rare.
5. **Ownership by deployment topology:**
   - *Desktop-only:* `kria-desktop` is leader.
   - *Desktop + server on one host:* whichever starts first is leader; the other
     becomes an RPC client (config `memory.role = auto | leader | client`).
   - *Server headless:* `kria-server` is leader; CLI is always a client.
   - *OpenClaw skills / sidecars:* never open the DB — they already route through the
     orchestrator (§45.4 of the spec), so they are RPC clients by construction.
6. **Failure / crash recovery.** If the leader crashes, its lease expires; the next
   process to need a write steals the stale lease (atomic CAS) and replays WAL on open.
   In-flight non-leader writes that were not yet acked are retried against the new
   leader (idempotent by `content_hash`). No data loss because the fast path commits
   the raw event before ack (L-fast-path).
7. **Escape hatch.** If a future workload proves write-contention-bound (unlikely at
   desktop scale), `BEGIN CONCURRENT` on wal2 is the pre-validated upgrade — same code,
   different transaction mode — with no schema change.

**Implementation contract (exact):** `WriterLease` trait with
`try_acquire() -> LeaseState`, `heartbeat()`, `is_leader()`, `steal_if_stale()`; the
Write Policy Engine asserts `is_leader()` before opening an `AuthorityTx` and otherwise
forwards to the leader via `MemoryRpc { submit(WriteCandidate) }`. Reads never check the
lease. **Invariant impact:** L2/L10 clarified; **L13 (single cross-process
writer-leader)** and **L14 (local-FS-only authority)** added.

---

## 38.2 — Rollback, Feature Flags & Legacy Coexistence (Issue 2)

**Question.** Migration (spec §31.1) was effectively one-way. What if the new system
regresses *in production* after cutover?

**Resolution — dual-run with a runtime kill-switch (additive; no invariant change).**

1. **Feature flag `memory.engine = legacy | dual | v2`** in `kria_config.toml`,
   switchable at runtime (hot-reload via the existing `ConfigService`). Default per
   phase: P1 `dual`, exit-P4 `v2`.
2. **`dual` mode (the safety net).** Every write goes to **both** the legacy store and
   the new authority (new is source of truth for reads); a background comparator samples
   reads from both and logs divergence to a `migration_divergence` metric. This makes
   the new engine provable in production *before* legacy is retired, and makes rollback
   instantaneous.
3. **Rollback = flip the flag to `legacy`.** Because `dual` kept legacy current, rollback
   loses nothing. Reads revert to legacy immediately; the new authority is frozen
   read-only for forensics.
4. **Partial rollback.** The flag is *per subsystem read path* (e.g.
   `memory.reads.retrieval = v2`, `memory.reads.knowledge_tools = legacy`) so a single
   regressing surface can revert without a full rollback.
5. **Legacy retirement (one-way step, gated).** Legacy is archived (not deleted) only
   after: (a) `dual` divergence rate < threshold for N days, (b) all three release gates
   pass (§38.5), (c) explicit operator confirmation. Archived legacy DBs are retained for
   one release cycle so a late rollback can still re-hydrate.
6. **Data compatibility.** The additive-only schema policy (Issue 18) + string-enum
   forward-compat (spec §40) guarantee a `v2`→`legacy`→`v2` round trip never corrupts:
   legacy ignores new fields; v2 treats unknown legacy values as `Unknown(String)`.

**Trade-off.** `dual` doubles write cost during migration. Accepted: it is temporary
(P1→P4), bounded to the migration window, and is the only way to de-risk a
memory-system swap in a shipping product. **Invariant impact:** none.

---

## 38.3 — Sensitivity / PII Classification (Issue 3)

**Question.** `sensitivity ∈ {public, private, secret}` gates crypto-shred, embedding
omission, and confirmation routing — but the classifier that *assigns* it was a black
box. This is load-bearing and was under-specified.

**Resolution — a deterministic-first, LLM-refined, user-overridable classifier
(consistent with the "deterministic-when-possible" principle).**

1. **Tier 1 — deterministic detectors (fast path, <1 ms, always run, LLM-free):**
   - **Credentials/secrets → `secret` (never stored, keychain-ref only):** regex +
     entropy for API keys, tokens, private keys (`-----BEGIN`), passwords, connection
     strings, JWTs, cloud keys (AWS `AKIA…`, etc.). High-entropy string heuristic
     catches unknown formats.
   - **Financial → `secret`/`private`:** credit-card (Luhn-validated), IBAN, routing/
     account numbers.
   - **Government/health IDs → `secret`:** SSN and national-ID patterns, medical record
     patterns; medical *topic* terms → at least `private`.
   - **Personal data → `private`:** emails, phone numbers, physical addresses, DOB.
   - **Workspace/company → `private` + `scope=workspace/company`:** internal hostnames,
     repo paths, ticket ids, `*.internal` domains.
   - Detectors are a pluggable `SensitivityDetector` registry (extensible without
     touching the gate).
2. **Tier 2 — LLM refinement (slow path, optional, advisory):** for content the
   deterministic tier marks ambiguous, an LLM classifies context (e.g. "is this a real
   secret or a code sample?"). Content is passed as **untrusted data** (D-11); the LLM
   may only *raise* sensitivity, never lower it below the deterministic floor
   (fail-safe). Absent LLM → deterministic result stands (L8).
3. **Confidence & Write-Policy effect:**
   - `secret` at any confidence → **never store the value**; store a keychain reference
     + a redacted placeholder; embedding omitted (keyword-only recall). Route to
     confirmation if the user explicitly asked to remember it.
   - `private` → store, but embedding encrypted to the SQLite tier (N8), excluded from
     plugin/public scope.
   - Low-confidence `private`/`secret` → err toward the *more* private class (fail-safe
     default), flag for the monthly audit report.
4. **Manual override (user sovereignty):** the user can reclassify any memory
   (`set_sensitivity(id, class)`); the override is recorded as evidence and is sticky
   (survives re-classification). A user can also pre-declare patterns
   (`always treat X as secret`).
5. **Detector maintenance:** detector patterns are versioned data (not code) so they
   update without a release; a detector version is stamped on each memory for auditability.

**Invariant impact:** strengthens L9 (erasure) and the "never store secrets" rule;
adds no new invariant. **Trade-off:** deterministic regex has false positives (e.g.
flags a sample key) — accepted because over-classifying is the safe direction and the
user can downgrade.

---

## 38.4 — `reason()` API Contract (Issue 4)

**Question.** `reason(query)` was listed (retrieval + graph + synthesis) but, unlike
`search()`, had no defined pipeline, I/O, streaming, cancellation, or failure contract.

**Resolution — `reason()` is a thin, explainable orchestration over `search()` +
graph, with LLM synthesis strictly optional.**

```
Input:  ReasonRequest {
          query: String,
          ctx: RetrievalCtx,               // scope, session, token_budget
          mode: {Retrieve|Synthesize},     // Synthesize requires LLM; Retrieve never does
          max_hops: u8 (<=3),              // graph expansion cap (cycle-safe)
          stream: bool,
          cancel: CancellationToken,
        }
Output: ReasonResult {
          answer: Option<String>,          // None in Retrieve mode or LLM-down
          supporting: Vec<MemoryRef>,      // the evidence set (always present)
          graph_paths: Vec<Path>,          // entity paths used (explainability, L6)
          confidence: f32,                 // min(evidence confidences) * coverage factor
          trace: ReasonTrace,              // feeds explain()
          degraded: Option<DegradeReason>, // e.g. LlmUnavailable → returned evidence only
        }
```

**Pipeline:** (1) `search()` for the evidence set (all its gating/filters/staleness
apply); (2) entity-anchored graph expansion (≤max_hops, cycle-safe CTE) to pull related
facts; (3) working-memory merge (current turn goal/focus, D-18); (4) **if
`Synthesize` and LLM up** → constrained synthesis prompt with the evidence delimited as
data (never instructions), producing an answer + inline provenance; **else** → return
the ranked evidence set with `degraded = LlmUnavailable` (L8 — `reason` never *fails*
for lack of an LLM, it degrades to `search`). **Confidence** is derived from evidence
confidence × staleness × coverage, never from the LLM's self-assessment (anti-inflation,
N13). **Streaming:** when `stream=true`, evidence is emitted first (instant), then
synthesized tokens; cancellation is cooperative at each stage boundary. **Failure:** any
strategy failing degrades (partial evidence) rather than erroring; a hard store error
surfaces as `RetrievalError`/`StorageError` per §43. **Contract guarantee:** every
`reason()` answer is explainable — `trace` + `supporting` + `graph_paths` reconstruct
exactly why (L6). **Invariant impact:** none (composition of existing verbs).

---

## 38.5 — Evaluation Datasets & Release Gates (Issue 5)

**Question.** The L12 quality gate ("Recall Precision at 500K") needs reproducible,
labeled ground truth. Generation + labeling were undefined.

**Resolution — a three-tier eval corpus with deterministic generation + labeled ground
truth, in `kria-eval`.**

1. **Synthetic corpus (reproducible, seeded).** A generator seeded by a fixed RNG
   produces N memories across all types with a realistic distribution (Zipfian topic
   frequency, entity co-occurrence graphs, temporal spread, duplicate/near-duplicate
   injection, contradiction pairs, multilingual slice). Because it is seed-driven, the
   500K corpus is byte-reproducible on any machine → a stable gate.
2. **Ground-truth query set.** The generator emits queries *with* their known-relevant
   memory ids (it planted them), yielding gold labels for free — the standard technique
   for synthetic IR eval. Categories mirror the query classifier (temporal/entity/
   conceptual/recent/procedural) so per-strategy quality is measurable.
3. **Real (opt-in) corpus.** A small hand-labeled set drawn from the developer's own
   consented KRIA history (Developer/Benchmark mode) validates that synthetic realism
   tracks reality; never shipped, never in CI artifacts (privacy).
4. **Metrics + thresholds (the gate):** Recall@k, Precision@k, nDCG (ranking), MRR,
   plus latency p95 (§41 budgets), Hallucination/False-Memory rate (via planted
   distractors), Duplicate rate, Stale-served rate. **Release gate:** at 10K/100K/500K,
   nDCG@10 and Recall@20 must be **≥ the frozen baseline** (recorded at P1) minus a
   tolerance; any regression **fails CI** (L12). Latency budgets fail independently.
5. **Determinism:** the whole harness runs offline, no network, fixed seeds → identical
   numbers across runs; a 3-run median guards against machine noise.

**Invariant impact:** operationalizes L12; no new invariant.

---

## 38.6 — Model Provisioning (Issue 6)

**Question.** How does the embedding model arrive, verify, update, and roll back —
including offline/air-gapped and licensing?

**Evidence.** `EmbeddingGemma-300M` is available as ONNX
(`onnx-community/embeddinggemma-300m-ONNX`) but is licensed under the **Gemma Terms of
Use**, *not* Apache-2.0, and its activations don't support fp16 (use fp32/q8/q4)
([HF model card](https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX);
[Gemma terms](https://ai.google.dev/gemma/terms)). all-MiniLM-L6-v2 is Apache-2.0.
*(Rephrased for licensing compliance.)*

**Resolution — a model registry with pinned, checksummed, license-aware provisioning.**

1. **Model registry manifest** (`models/manifest.toml`, versioned in-repo): each entry =
   `{ model_id, embedding_model_version, url(s), sha256, dim, quantization, license,
   requires_acceptance: bool }`. `embedding_model_version` is the partition key (C4).
2. **Licensing (compliance-critical):** the default **shippable** tier is
   **all-MiniLM-L6-v2 (Apache-2.0)** — it is already in the codebase and can be bundled.
   **EmbeddingGemma is opt-in**: because it is under the Gemma Terms (not Apache-2.0),
   KRIA does **not** redistribute it; on first selection the user is shown the Gemma
   license and must accept, then KRIA downloads it from the official source. This keeps
   KRIA's distribution clean while offering the better model. (This is an additive
   refinement of §30/D-3 — MiniLM is the *default provisioned* tier; Gemma is the opt-in
   upgrade.)
3. **Verify on install + load:** SHA-256 (BLAKE3 for internal) checked against the
   manifest on download and again on load (D-3 checksum pin). Mismatch → refuse to load,
   keep prior model.
4. **Offline / air-gapped:** a `kria models import <file>` path installs a
   pre-downloaded model bundle (model + tokenizer + manifest entry) with the same
   checksum verification; no network required. The default MiniLM tier ships in the
   installer so a fresh, fully-offline install always has a working embedder.
5. **Update policy + rollback:** a new model = a new `embedding_model_version` = a new
   vector partition (never in place). The embedding-migration state machine (spec §44.5)
   dual-searches old+new and re-embeds in the background; rollback = keep serving the old
   partition and drop the new (LanceDB time-travel). Cap at 2 concurrent versions.
6. **Compatibility:** ONNX-Runtime version is pinned in `Cargo.toml`; the manifest
   records the minimum `ort` version per model (EmbeddingGemma's fp16 limitation is
   encoded as `quantization ∈ {fp32,q8,q4}` so the loader never requests fp16).

**Invariant impact:** none; strengthens L8 (offline install) + provider neutrality
(registry, not a hard-coded model).

---

## 38.7 — Portable Export / Import (Issue 7)

**Question.** Distinct from backup (which is authority-only, machine-local). Portable
export must move a user's memory across installs/platforms/versions.

**Resolution — a self-describing, versioned, encrypted portable bundle.**

1. **Format `.kmem` (a zip/tar container):** `manifest.json { format_version,
   kria_version, schema_version, embedding_model_versions, created_at, scope,
   encryption }` + `memories.jsonl` (events + derived memories + provenance +
   graph edges + evidence, as portable JSON, string-enums) + `library/` (original files,
   optional) + `vectors/` (optional — omit to force re-embed on import, the
   smaller/portable default). Vectors are *derived*, so the portable default excludes
   them; the importer re-embeds (L4).
2. **Encryption:** the bundle is encrypted with a user passphrase (age/libsodium; a
   fresh symmetric key wrapped by the passphrase). `secret`-class content is either
   excluded or re-encrypted; crypto-shred keys are **not** exported by default (export
   ⇒ new install re-derives keys), with an explicit "include keys" option for true
   device-to-device migration.
3. **Versioning / cross-version:** import runs the additive forward-migration chain
   (Issue 18/20) so an older bundle imports into a newer KRIA. A newer bundle into an
   older KRIA is refused with a clear message (no silent downgrade, spec §43
   `MigrationError::DowngradeRefused`); string-enum `Unknown` fallback prevents crashes
   on unknown values.
4. **Cross-platform:** JSON + portable file layout; no OS-specific blobs in the portable
   path (keychain refs are re-established on import per platform).
5. **Selective export/import:** by scope, namespace, collection, date range, or memory
   type (e.g. "export only my Library" or "import only project X"). **Imports pass
   through the Write Policy Engine** (not a raw DB merge) so dedup, contradiction, and
   security scanning apply — an imported bundle is untrusted (`source: import`, SI-4).
6. **Backup vs export (clarified):** backup = authority-only, encrypted, machine-local,
   for disaster recovery, rebuild-indexes-on-restore (D-12). Export = portable,
   cross-install, policy-gated on import. Different formats, different guarantees.

**Invariant impact:** none; reinforces L3 (import via the gate) + privacy.

---

## 38.8 — Key Hierarchy & Encryption Architecture (Issue 8)

**Question.** Crypto-shred `key_ref` assumed an OS keychain "where available." Headless
Linux / `kria-server` has none. The master-key management (KEK/DEK), rotation,
recovery, and shred-key relationship were under-specified.

**Resolution — a two-level KEK/DEK hierarchy with a pluggable keystore.**

```
Master Key (MK)  ── unlocks ──►  per-subject Data Encryption Keys (DEK, the shred keys)
      ▲
   wrapped by a Key-Encryption-Key (KEK) held in a KeyStore backend
```

1. **`KeyStore` trait (pluggable, provider-neutral):**
   - **`OsKeychain`** (desktop): macOS Keychain / Windows DPAPI / Linux Secret Service.
     Holds the KEK.
   - **`FileKeyStore`** (headless/server): KEK derived from a passphrase via Argon2id,
     stored wrapped in a `keyfile` with strict permissions; passphrase supplied by env/
     systemd-creds/prompt at start. For unattended servers, a TPM/HSM-backed variant is
     the documented upgrade (same trait).
   - **`EphemeralKeyStore`** (Incognito/tests): MK in RAM only, never persisted.
2. **DEK = the shred key.** Each erasure subject (person/employer/project/session/
   library-item) has a DEK, wrapped by the KEK, stored in `shred_keys` (wrapped blob) or
   referenced from the OS keychain. Sensitive payloads (events, `secret`/`private`
   content, their embeddings) are encrypted with the subject DEK.
3. **Crypto-shred = destroy the DEK.** `forget(subject)` deletes the wrapped DEK; the
   ciphertext is then unrecoverable even though the immutable event row remains (L9, L1
   both hold). Because the DEK is per-subject, shredding one subject never affects others.
4. **Rotation:** KEK rotation re-wraps all DEKs (cheap — DEKs are small, and re-wrapping
   doesn't touch ciphertext). DEK rotation re-encrypts that subject's data (background
   job, chunked). MK compromise → rotate KEK + re-wrap; data stays encrypted throughout.
5. **Recovery & key backup:** the KEK can be backed up as a passphrase-wrapped recovery
   blob (user opt-in), stored separately from data (3-2-1). **Explicit trade-off,
   flagged to the user:** if the KEK/keystore is lost and no recovery blob exists, all
   encrypted memory is unrecoverable — this is the *cost* of genuine crypto-shred
   erasure and is surfaced prominently (spec §21 export-before-delete + a first-run key
   backup prompt).
6. **At-rest tiers stay equal (N8):** the vector index and backups are encrypted to the
   same tier as SQLite; `secret` embeddings are omitted/encrypted so inversion can't
   recover shredded text.

**Invariant impact:** implements L9 concretely across all deployment topologies; adds no
new invariant. **Trade-off:** headless passphrase management is operationally heavier
than a desktop keychain — accepted, and the TPM/HSM upgrade path is documented.

---

## 38.9 — Write Amplification / Write-Storm Protection (Issue 9)

**Question.** §46 memorizes *every* tool outcome. High-frequency sources (GUI
automation loops, file watchers, screen understanding, desktop monitoring, tool spam)
could flood the write path.

**Resolution — a mandatory admission-control stage in front of the Write Policy fast
path (bounded, deterministic).**

1. **Per-source token-bucket rate limiting.** Each source (`tool:{name}`, `desktop.*`,
   `file_watcher`, `mcp:{server}`) has a token bucket (rate + burst from config).
   Over-budget events are **coalesced/sampled**, not dropped silently — the *last*
   state in a window wins (desktops/file watchers care about latest state, not every
   tick).
2. **Debounce + coalesce for state-like sources.** Ambient/observational streams
   (desktop context, file events, screen understanding) are debounced (≥ configurable
   interval, default 60 s for salience per Issue 7/N7) and coalesced by
   `(source, entity)` so 1000 file-change ticks become one "file X changed" observation.
3. **Priority admission.** User-originated writes (`TriggerProvenance::User`) bypass
   rate limits (never throttle the user); tool/ambient writes are throttled first. This
   reuses the existing provenance signal.
4. **Batching + bounded queue (existing, made mandatory).** Low-priority writes buffer
   in a bounded ring and flush on idle via the Cognitive Scheduler (P4); on overflow →
   **backpressure to sampling** (keep every Nth + always keep failures/lessons), never
   unbounded growth (spec §25, 32.3).
5. **Quality filter still applies after admission** (spec §18.2) — noise that survives
   rate-limiting is still dropped if it carries no signal.
6. **Observability:** `admission_dropped`, `admission_coalesced`, per-source write rate
   in the health report → a runaway tool is visible and tunable.

**Invariant impact:** none; protects L-bounded-growth (R20) and fast-path latency
(§41). **Trade-off:** sampling can miss a rare-but-important ambient event — mitigated
by "always keep failures + user-flagged + contradictions."

---

## 38.10 — Internationalization (Issue 10)

**Question.** KRIA ships 7 locales. Validate tokenizer, search, temporal reasoning,
Unicode, CJK, RTL, embeddings.

**Evidence.** FTS5's `unicode61` tokenizer does **not** segment CJK (no spaces between
words); the built-in `trigram` tokenizer handles CJK substring search without semantics,
and ICU / third-party (`simple`) tokenizers add real segmentation as loadable extensions
([SO: FTS5 unicode61 & CJK](https://stackoverflow.com/questions/52422437/why-sqlite-fts5-unicode61-tokenizer-does-not-support-cjkchinese-japanese-korean);
[FTS5 docs](https://www.sqlite.org/fts5.html)). *(Rephrased for licensing compliance.)*

**Resolution — locale-aware FTS + locale-correct temporal, vectors already multilingual.**

1. **Embeddings.** EmbeddingGemma is multilingual (100+ languages); MiniLM is weaker
   multilingually — so on non-English-primary installs the provisioning default nudges
   toward Gemma (opt-in, §38.6). Vector recall is therefore language-robust regardless of
   FTS. This is the primary retrieval path; FTS is the keyword floor.
2. **FTS tokenizer selection by content script (additive):** default `unicode61`
   (diacritic-folding, good for Latin/Cyrillic/Greek/Arabic word-boundary-by-space);
   **auto-switch to `trigram` for CJK-heavy content** (built-in, no extension → keeps the
   zero-dependency floor); **optional ICU/`simple` tokenizer** behind the `SearchStore`
   trait for installs that want true CJK/Thai segmentation (an extension, opt-in). The
   tokenizer choice is recorded per FTS partition so re-index is deterministic.
3. **RTL (Arabic/Hebrew):** stored as Unicode; `unicode61` segments on whitespace which
   is correct for Arabic; display direction is a frontend concern, not a storage one.
   No special storage handling needed.
4. **Temporal reasoning is locale + timezone correct (extends N10/D-15):** "yesterday",
   "الأسبوع الماضي", "上周" are parsed by a locale-aware temporal parser keyed off the
   session locale; resolution is always against stored UTC in the user's *current* tz.
   The parser is a pluggable `TemporalResolver` per locale (falls back to
   language-agnostic relative-date heuristics).
5. **Unicode hygiene:** NFC normalization on ingest (so `content_hash` dedup is
   stable across equivalent Unicode forms); casefolding is Unicode-aware.

**Invariant impact:** none; strengthens L8/L12 for non-English users. **Trade-off:**
`trigram` CJK search is substring-based (some false positives, no word semantics) —
accepted as the dependency-free floor; ICU is the opt-in upgrade.

---

## 38.11 — Production Hardening Validation (Part 3)

Each area validated; ✔ = covered by an existing section, ➕ = refined here.

| Area | Status | Where |
|---|---|---|
| Plugin/Capability API versioning | ➕ | plugins/skills call the versioned Memory API (spec §40); capability rows carry `embedding_model_version` + detector/version stamps |
| Schema / database evolution | ✔ | additive-only + `schema_version` + forward-migrate backups (Issue 18/20) |
| Memory API versioning | ✔ | spec §40 (v1 module, SemVer, `Unknown` fallback) |
| Observability / tracing / explainability / metrics | ✔ | spec §28; `explain`/`reason.trace`; intelligence metric suite |
| Benchmark harness / perf budgets | ✔➕ | §38.5 + spec §41 (CI gate) |
| Operational runbooks | ➕ | §38.14 adds a runbook index (leader failover, rollback, restore-verify, re-embed, key rotation) |
| Threat / failure / recovery model | ✔ | spec §42 threat model, §30 recovery, §33 edge catalog |
| Power-loss / crash consistency | ✔ | WAL replay + fast-path-commits-before-ack + idempotent outbox (§30) |
| Backup / restore verification | ✔ | checksum-before-valid + periodic test-restore + retrieval-parity (D-12, §44.2) |
| Health monitoring | ✔ | `health()` + `memory_health_report` (spec §28) |
| Storage growth / compaction / vacuum / fragmentation | ➕ | §38.12 adds a maintenance policy: WAL checkpoint cadence, `PRAGMA incremental_vacuum`, LanceDB compaction as P4 jobs, cold-segment roll |
| Memory pressure / battery / thermal | ✔ | Runtime Budget Manager (spec §25, 32.3) |
| Scheduler fairness | ✔ | two-queue arbiter, P0 preempts, single-flight (spec §26) |

**Storage-maintenance policy (the ➕ above, made concrete):** WAL auto-checkpoint at
1000 pages + a P4 `PASSIVE` checkpoint on idle; `PRAGMA auto_vacuum=INCREMENTAL` with a
weekly `incremental_vacuum` P4 job; LanceDB fragment compaction weekly (P4);
event cold-segment roll at 90 days (Issue 14). All are Cognitive-Scheduler jobs →
battery/thermal aware, chunked, checkpointed. Disk self-regulation (warn 80% / archive
95%) already bounds growth (R20).

---

## 38.12 — Future Evolution Extension Points (Part 4)

Validated: each future capability lands on an **existing** seam without redesign.

| Future | Extension point (already present) |
|---|---|
| Vision / voice / video / browser / IDE / email / calendar memory | `modality` field + modality-partitioned vector tables + `source: {tool}` provenance; new source, no schema change |
| Agent / multi-agent memory | `namespace` + `owner_id` + `scope` (L7); agents = namespaces; shared `core` with gated promotion |
| Neural feedback / preference learning / RLHF-DPO / fine-tune datasets | `FeedbackEvent` (D-19) + `preference_pair_id` + `training_eligible` fields reserved; export produces datasets via §38.7 selective export |
| Long-term analytics / knowledge mining / GraphRAG | event log is the analytics source; GraphRAG community summaries reserved (spec §8.8); graph behind `GraphStore` |
| Future graph / vector / embedding / reasoning / storage engines | trait ports `GraphStore`/`VectorStore`/`SearchStore`/`Embedder`/`LlmClient` (ADR-002/003/004, C5) |
| Distributed / cloud sync / multi-device / multi-user | `device_id` + HLC + content-addressed append-only events (D-15); sync = event union (N9); reserved, not built |
| Enterprise deployment | writer-leader + `KeyStore` (FileKeyStore/TPM) + namespaces + RPC clients (§38.1/§38.8) |
| Frontend Memory Explorer (2D/3D graph, timeline, relationship/knowledge explorer, conflict resolver, memory debugger, dev tools) | all are **reads** over the Memory API + `explain`/graph traversal; adding a view needs zero backend change (spec §10 contract-stability rule) |

**Conclusion:** no future item on the list requires a schema-breaking change or a core
redesign; every one maps to a reserved field or a trait. The "reserve schema now"
discipline (spec §3) is validated as sufficient.

---

## 38.13 — Adversarial Re-Review (Part 5): new issues from the fixes

Reviewing the fixes above for newly-introduced problems:

- **N-A (from §38.1): lease thrash under rapid leader crashes.** Two processes could
  fight for a stale lease. *Fix:* lease steal is an atomic CAS in an `IMMEDIATE` txn
  (SQLite serializes it) + randomized 0–500 ms backoff before a steal attempt → exactly
  one winner, no thrash.
- **N-B (from §38.2 dual mode): divergence between legacy and v2 writes.** *Fix:* v2 is
  the read source of truth in `dual`; the comparator only *observes* — it never
  reconciles into legacy, so there's no write-write race. Divergence is a metric, not a
  correctness bug.
- **N-C (from §38.3 PII): over-classification hides useful memory.** *Fix:* `secret`
  never deletes — it stores redacted + keychain-ref and remains keyword-recallable to the
  user; user can downgrade. Fail-safe direction preserved without data loss.
- **N-D (from §38.6 provisioning): install ships only MiniLM, user expects Gemma
  quality.** *Fix:* first-run + non-English detection nudges the Gemma opt-in; MiniLM is
  a correct, working floor meanwhile (L8). No silent quality cliff.
- **N-E (from §38.9 admission control): a genuinely important high-frequency event gets
  sampled out.** *Fix:* admission always keeps failures, contradictions, and
  user-flagged/`TriggerProvenance::User` events; only redundant *state* ticks are
  coalesced.
- **N-F (from §38.10 tokenizer auto-switch): mixed-language content picks the wrong
  tokenizer.** *Fix:* tokenizer is chosen per FTS partition by dominant script with a
  Latin+CJK dual-index option; vector recall (language-agnostic) is the primary path so
  FTS mis-tokenization only degrades the keyword floor, never breaks recall.
- **N-G (from §38.8 headless keys): passphrase in env is a leak surface.** *Fix:* prefer
  systemd-creds/TPM; env passphrase is the documented last resort with a startup warning;
  never logged (SI-4).

Iterating again over N-A..N-G surfaced no further material issues — improvements are now
negligible (cosmetic), so convergence is reached.

---

## 38.14 — Convergence Report (Part 6)

**Issues found → fixed (this pass):**

| # | Issue | Fix | Why correct | Trade-off | Alternatives rejected |
|---|---|---|---|---|---|
| 1 | Cross-process writers undefined | Writer-leader lease + RPC clients + local-FS-only; SQLite lock backstop (§38.1) | SQLite serializes writers safely in WAL; lease makes it explicit + gives ownership/failover | dual-role complexity | *Multi-writer BEGIN CONCURRENT now* (unneeded at scale); *file-lock only* (no ownership/failover) |
| 2 | One-way migration | `memory.engine` flag + `dual` run + per-surface partial rollback (§38.2) | dual keeps legacy current → instant lossless rollback | 2× write cost during migration | *big-bang cutover* (unrecoverable regression risk) |
| 3 | PII classifier a black box | Deterministic detectors + LLM refine (raise-only) + user override (§38.3) | deterministic floor can't be injected/degraded; fail-safe toward private | regex false positives | *LLM-only* (injectable, non-deterministic, L8-violating) |
| 4 | `reason()` undefined | Composition over `search`+graph, LLM optional, always explainable (§38.4) | degrades to search (L8); confidence from evidence not LLM (N13) | none material | *bespoke reasoning engine* (complexity, non-neutral) |
| 5 | Eval ground truth undefined | Seeded synthetic corpus w/ planted labels + opt-in real set (§38.5) | reproducible + free gold labels → stable gate | synthetic ≠ real fully | *manual labeling only* (unscalable, non-reproducible) |
| 6 | Model provisioning undefined | Registry manifest + checksum + Gemma opt-in (license) + offline import (§38.6) | MiniLM Apache floor ships; Gemma license respected; offline works | Gemma is opt-in, not default | *bundle Gemma* (Gemma Terms redistribution risk) |
| 7 | Export vs backup conflated | `.kmem` portable, encrypted, versioned, policy-gated import (§38.7) | import via Write Policy (untrusted); vectors re-embed (L4) | larger portable bundles if vectors included | *raw DB copy* (non-portable, unsafe merge) |
| 8 | Key hierarchy for headless undefined | KEK/DEK + pluggable `KeyStore` (OsKeychain/File/TPM) (§38.8) | works on desktop + server; per-subject DEK = shred key (L9) | headless passphrase ops | *keychain-only* (no server story) |
| 9 | Write-storm risk | Admission control: per-source rate-limit + debounce/coalesce + priority (§38.9) | user never throttled; bounded queues (R20) | rare ambient event sampled | *unbounded writes* (I/O + bloat) |
| 10 | i18n gaps | Multilingual embeddings primary + per-partition FTS tokenizer + locale temporal (§38.10) | vector recall language-robust; trigram CJK floor dependency-free | trigram CJK is substring-only | *unicode61-only* (broken CJK) |

**Architecture decisions changed (all additive refinements, none reversed):**
- Default *provisioned* embedder clarified to **MiniLM (Apache-2.0)**; EmbeddingGemma is
  the **opt-in** upgrade due to the Gemma Terms (refines §30/D-3 — the *preferred* model
  is unchanged; only its distribution model is clarified).
- "Single writer" clarified to **single writer per process + one cross-process
  writer-leader** (refines L2/L10).

**New sections added:** §38.1–§38.14 (this convergence pass).
**Modified sections:** none rewritten; §9/§30 (provisioning), N10 (temporal i18n) and
§46 (admission control) are *extended* by reference here.

**Invariants validated (unchanged):** L1 (append-only log), L2 (SQLite authority —
clarified, not changed), L3 (single write gate — now holds across processes via RPC),
L4 (rebuildable indexes), L5 (provenance), L6 (explainability — `reason.trace`), L7
(namespace isolation), L8 (offline/degradation — reinforced by MiniLM floor + trigram +
deterministic PII), L9 (crypto-shred — now concrete via KEK/DEK), L10 (reads don't block
writer), L11 (self-memory untrusted), L12 (retrieval quality — operationalized by
§38.5).

**Invariants added (additive, non-breaking):**
- **L13** — Exactly one cross-process **writer-leader**; all other processes are readers
  or RPC clients.
- **L14** — The authority database resides on a **local filesystem** only (never
  network mounts).

---

## 38.15 — Implementation Readiness (Part 7)

| Dimension | Score (/10) | Notes |
|---|---|---|
| Architecture | 9 | Keystone (SQLite authority) + traits proven; cross-process now defined |
| Scalability | 8 | Desktop scale validated by design; 500K gate defined but unbenchmarked until P2 |
| Reliability | 9 | WAL + outbox + reconciliation + backup-verify + leader failover |
| Maintainability | 9 | ADRs, ownership matrix, error taxonomy, additive schema |
| Extensibility | 10 | Every Part-4 future maps to a reserved field/trait; no redesign needed |
| Security | 9 | Threat model, KEK/DEK, deterministic injection + PII, namespace isolation |
| Privacy | 9 | Modes, crypto-shred (concrete), local-first, export-before-delete, consent cold-start |
| Performance | 8 | Budgets + CI gate defined; real numbers pending P1/P2 benchmark |
| Developer Experience | 9 | Explain API, dev mode, invariant/grep gates, clear contracts |
| Local-first | 10 | No mandatory service; offline install (MiniLM) + FTS floor + trigram |
| Future-proofing | 9 | Reserved schema + traits + event log; sync primitives reserved |
| **Implementation Readiness** | **9** | No architectural blocker remains |

**Remaining non-blocking items** (tune during P1/P2, not architectural): decay
half-lives + `archive_threshold`; default token budgets per consumer; rule-promotion
evidence threshold N; ICU-tokenizer packaging decision; TPM keystore variant for
enterprise. All have defined homes (config / `SearchStore` / `KeyStore`) and none can
force a redesign.

**Verdict:**

> **This architecture is ready to generate Kiro Specs and begin implementation.**

All ten critical issues are resolved additively, every invariant (L1–L14) is validated,
no core decision was reversed, and the two decisions that were *clarified* (writer-leader
scope, MiniLM-default/Gemma-opt-in) strengthen the local-first and compliance posture
without adding complexity beyond value. Build P1 (storage authority + write gate +
scheduler + writer-leader + retrieval floor), and stand up the §38.5 eval harness and
the outbox/perf gates first, as the design's blockers direct.

---

*End of Section 38 — Final Convergence Pass. This section is additive and authoritative;
where it clarifies an earlier section the clarification governs. The document is now the
implementation-ready blueprint for KRIA's memory subsystem.*
