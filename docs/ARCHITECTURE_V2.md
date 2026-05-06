# K.R.I.A. Architecture V2: Edge-Resident Autonomous Intelligence

<Deficiency_Scalability_Analysis>

## Why The Previous Architecture Failed The JARVIS Test

The earlier V2 draft described useful pieces, but it did not impose enough authority boundaries for a continuous autonomous system. A JARVIS-class edge assistant cannot rely on "mostly coordinated" subsystems. It needs one planner, one resource arbiter, one memory write authority, explicit cancellation, and bounded admission everywhere.

### Dual Planner Failure

A dual-planner system creates race conditions because `TurnGate` and `AgentLoop` can both interpret the same user turn, mount tools, select resources, and trigger fallback behavior. In the current codebase, `AgentLoop` already performs semantic routing, regex intent classification, tool narrowing, synthetic package/Colab workflows, fallback tool-call injection, policy hints, and execution retries. Adding `TurnGate` without removing top-level planning from `AgentLoop` would create two brains competing over the same turn.

For an autonomous agent, this is not cosmetic. One planner might classify a request as tool-only while the other escalates to L1 reasoning. One might decide ComfyUI needs the GPU while the other starts a vision path. One might cancel a stale turn while the other continues executing synthetic tool calls. This destroys predictability and makes safety audits impossible.

The fix is strict planner authority: `TurnGate` is the sole top-level planner, and `AgentLoop` becomes a bounded ReAct/tool executor and policy enforcer.

### Uncalibrated Confidence Failure

Uncalibrated confidence scores break the illusion of intelligence because the system starts acting certain about the wrong thing. The current code already uses heuristic confidence constants from `IntentRouter`; an ONNX L0 classifier would add another numeric output that may look scientific but remain operationally untrusted unless constrained.

A local assistant should feel fast and competent, not erratically confident. Low-confidence routing must not produce speculative execution. It should trigger clarification, deterministic fallback, or L1 escalation. Confidence is an admission signal, not an authority signal.

The fix is a pragmatic pipeline: deterministic guards first, existing FastEmbed semantic routing second, optional ONNX classifier third, validator last. No action is authorized by L0 confidence.

### Cancellation Tree Failure

Missing cancellation trees create zombie work. A user-level "stop" must not merely stop UI streaming while Python, ComfyUI, MCP, subprocess tools, or LLM generation continue burning CPU, RAM, VRAM, file handles, or network sockets.

The current system has per-session cancellation, but cancellation does not consistently propagate into all owned child work. Tool cancellation can return early while an isolated spawned task continues until timeout. Sidecar requests can time out while their pending response slot remains registered. Image generation can continue unless ComfyUI receives `/interrupt`.

On a 6GB GPU and 16GB RAM edge machine, zombie tasks are fatal. They cause stale actions, leaked memory, VRAM fragmentation, and false resource availability.

The fix is a per-turn cancellation tree with mandatory child tokens for L0, L1, tools, sidecar, MCP, and image generation.

### Modularity Failure

Hard-coupled dependencies prevent the system from scaling beyond today's hardware. ComfyUI, llama-server, FastEmbed, ONNX, sidecar processors, and OS tools are all valid current choices, but V2 must not bake any one engine into the architecture as a permanent shape.

The 6GB RTX 4050 is today's constraint, not the final design horizon. V2 must support future image backends, future local classifiers, future multimodal inputs, future multi-agent delegation, and eventually multi-GPU orchestration without rewriting the control plane.

The fix is trait-first architecture: `Planner`, `RouterClassifier`, `GpuLeaseManager`, `ImageBackend`, `MemoryManager`, `ExecutionEngine`, and `ResourceTelemetry` boundaries must be explicit.

</Deficiency_Scalability_Analysis>

## 1. Purpose

K.R.I.A. V2 defines the control-plane architecture for a local-first autonomous assistant constrained today to:

- NVIDIA RTX 4050 Laptop GPU with 6GB VRAM.
- Approximately 16GB system RAM.
- Local llama-server L1 inference.
- CPU/Rust reflex path for low-latency OS tasks.
- Python sidecar for preprocessing and controlled execution.
- ComfyUI as the current production image backend.

The architecture must provide:

- zero-latency reflexes for deterministic local operations,
- GPU reservation for deep cognition and image generation,
- strict safety and resource authority in Rust,
- modular traits for future engines and modalities,
- bounded concurrency suitable for edge reliability.

Cloud APIs, multi-GPU scheduling, and multi-agent federation are future-compatible design targets, not required V2 runtime assumptions.

## 2. Non-Negotiable Invariants

- Rust owns planning, safety, memory authority, resource allocation, and audit boundaries.
- `TurnGate` is the only top-level planner.
- `AgentLoop` is not allowed to allocate hardware, decide global route class, or override TurnGate resource plans.
- No GPU-consuming component runs without a `GpuLease`.
- L0 cannot authorize actions, tools, memory writes, or GPU allocation.
- Tool execution always flows through `ToolRegistry`, `PolicyEngine`, HITL where required, and audit.
- Python sidecar, ComfyUI, MCP servers, and OS tools are execution engines only.
- Memory writes go through `MemoryManager`; `MemoryStore` is private persistence.
- Every turn has a root cancellation token and bounded admission.
- Telemetry reconciliation, not logical state alone, decides GPU recovery and degraded mode.

## 3. Component Map

```text
UI / Voice / Telegram / Server Transport
        |
        v
TurnAdmission
  |-- per-session turn gate
  |-- bounded queue
  |-- stale-turn invalidation
        |
        v
TurnGate  (SOLE TOP-LEVEL PLANNER)
  |-- deterministic guards
  |-- FastEmbed semantic router
  |-- optional ONNX classifier
  |-- validator
  |-- resource planner
        |
        v
Resource Plane
  |-- GpuLeaseManager
  |-- ResourceTelemetry / ResourceSnapshot
  |-- L1 Residency Manager
  |-- ImageBackend registry
        |
        v
Execution Plane
  |-- AgentLoop ReAct executor
  |-- ToolRegistry
  |-- PolicyEngine / HITL / Audit
  |-- Python sidecar
  |-- MCP clients
  |-- OS subprocess tools
        |
        v
MemoryManager
  |-- facts
  |-- turns
  |-- media
  |-- snippets
  |-- document chunks
  |-- preferences
```

## 4. Structured Intent Taxonomy

The previous flat enum is rejected. V2 uses a structured intent envelope so new modalities and compute backends can be added without breaking routing logic.

```rust
pub struct IntentEnvelope {
    pub modality: Modality,
    pub operation: Operation,
    pub hazard_hint: HazardHint,
    pub compute: ComputeClass,
    pub confidence: f32,
    pub source: IntentSource,
}
```

### 4.1 Modality

```rust
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    File,
    Screen,
    Mixed,
    Unknown,
}
```

### 4.2 Operation

```rust
pub enum Operation {
    Converse,
    Read,
    Search,
    RetrieveMemory,
    Write,
    Send,
    Delete,
    ExecuteCode,
    ExecuteShell,
    Automate,
    GenerateImage,
    AnalyzeImage,
    AnalyzeFile,
    Schedule,
    ConfigureSystem,
    Cancel,
    Clarify,
    Refuse,
}
```

### 4.3 Hazard Hint

```rust
pub enum HazardHint {
    Green,
    Yellow,
    Red,
    Black,
    Unknown,
}
```

`HazardHint` is only a routing hint. Final risk classification and authorization always belong to `PolicyEngine`.

### 4.4 Compute Class

```rust
pub enum ComputeClass {
    ReflexRust,
    ToolOnly,
    SidecarCpu,
    L1Text,
    L1Vision,
    ImageGpu,
    MixedPipeline,
    ClarifyOnly,
    RefuseOnly,
}
```

### 4.5 Intent Source

```rust
pub enum IntentSource {
    DeterministicGuard,
    FastEmbedSemanticRouter,
    OnnxClassifier,
    UserClarification,
    Fallback,
}
```

### 4.6 L0 Output Contract

L0 remains intentionally minimal. It emits a compact classifier result, and the validator translates that result into the structured `IntentEnvelope`.

Permitted L0 output:

```json
{"intent":"generate_image","confidence":0.91}
```

Forbidden L0 output:

- tool arguments,
- shell commands,
- file paths,
- memory writes,
- safety approvals,
- GPU instructions.

## 5. Single Planner Authority

`TurnGate` is an orchestrator, not a god object or monolithic container. It owns top-level planning authority by delegating to independent modules with narrow responsibilities:

- `TurnAdmission` accepts, rejects, queues, supersedes, or cancels incoming turns.
- `IntentPipeline` runs deterministic guards, FastEmbed semantic routing, optional ONNX classification, and validation.
- `PlanCompiler` converts an `IntentEnvelope` into an immutable `ResourcePlan`.
- `ResourceScheduler` requests leases and schedules scarce resources.
- `ExecutionDispatch` hands the approved plan to `AgentLoop` or a direct reflex executor.

`TurnGate` owns:

- turn classification,
- top-level operation selection,
- compute class,
- resource plan,
- GPU lease request,
- L1 wake/sleep decision,
- image backend selection,
- stale-turn invalidation.

`AgentLoop` owns:

- ReAct rounds inside an approved plan,
- tool-call parsing,
- tool execution through `ToolRegistry`,
- policy checks,
- HITL coordination,
- tool result shaping,
- final response synthesis.

`AgentLoop` must not:

- decide whether a turn is image generation versus L1 reasoning,
- independently allocate GPU resources,
- mount broad tool sets outside the TurnGate plan,
- override compute class,
- launch ComfyUI or L1 lifecycle changes directly.

Migration rule:

Current routing logic inside `AgentLoop` should be gradually moved into `TurnGate` or narrowed to executor-local behavior.

## 6. Routing Pipeline

The routing pipeline order is strict.

```text
1. Deterministic Guards
2. Existing FastEmbed Semantic Router
3. Optional ONNX Classifier
4. Validator
5. ResourcePlan
```

### 6.1 Deterministic Guards

These run first and can short-circuit:

- cancel / stop / interrupt,
- approval continuation,
- explicit slash command,
- direct safe local command,
- blacklisted request,
- empty prompt,
- attachment type detection,
- active turn supersession.

### 6.2 Existing FastEmbed Semantic Router

The current router using `multilingual-e5-small` remains the primary semantic domain router.

It provides:

- domain narrowing,
- tool category candidates,
- OOD detection,
- multi-domain detection,
- lexical modality/destructive hints.

### 6.3 Optional ONNX Classifier

The ONNX classifier is optional and in-process.

It is used only for:

- compute class hints,
- ambiguous routing resolution,
- cheap top-level operation classification.

It must run behind a bounded CPU worker and must not block Tokio runtime workers.

### 6.4 Validator

Validator responsibilities:

- reject malformed classifier output,
- clamp confidence,
- apply confidence thresholds,
- merge deterministic, semantic, and classifier signals,
- produce `IntentEnvelope`,
- enforce safety invariants,
- trigger clarification or L1 escalation when confidence is low.

Low confidence behavior:

```text
green + obvious deterministic path -> proceed
ambiguous tool/action path -> clarify
high-value reasoning path -> escalate to L1
dangerous path -> refuse or HITL preparation
```

No academic calibration is required for V2. Use pragmatic thresholds and a regression corpus.

## 7. ResourcePlan

`TurnGate` converts `IntentEnvelope` into a `ResourcePlan`.

```rust
pub enum ResourcePlan {
    ReflexRust,
    ToolOnly,
    SidecarCpu,
    L1Text {
        residency: L1ResidencyRequirement,
    },
    L1Vision {
        visual_budget: VisionBudget,
    },
    ImageGeneration {
        backend: ImageBackendId,
        l1_policy: L1ImagePolicy,
    },
    MixedPipeline {
        stages: Vec<ResourceStage>,
    },
    Clarify,
    Refuse,
}
```

ResourcePlan is immutable for the lifetime of a turn except through explicit replanning after tool results or user clarification.

## 8. GpuLeaseManager

The GPU lease state machine is intentionally small.

```rust
pub enum GpuLeaseState {
    Idle,
    Held {
        owner: GpuOwner,
        token: LeaseToken,
        turn_id: TurnId,
        deadline: Instant,
    },
    Recovering {
        owner: Option<GpuOwner>,
        reason: RecoveryReason,
    },
    Degraded {
        reason: String,
    },
}
```

### 8.1 GPU Owners

```rust
pub enum GpuOwner {
    L1Worker,
    ImageBackend(ImageBackendId),
    Vision,
    Speech,
    Maintenance,
}
```

### 8.2 Lease Rules

- L1 inference requires a lease when GPU-resident.
- ComfyUI requires a lease for generation.
- Vision GPU paths require a lease.
- GPU STT/TTS paths require a lease.
- Future image backends require a lease.
- Lease release is not trusted until telemetry reconciliation passes.

### 8.3 Lease Fairness Policy

- Foreground user turns preempt background work.
- Image and text requests evaluate FIFO within their own class.
- Maintenance tasks have the lowest priority and must yield to foreground work.

### 8.4 Recovery Rules

If a lease expires or telemetry disagrees with logical ownership:

```text
Held -> Recovering
Recovering tries owner-specific cleanup
Recovery success -> Idle
Recovery failure -> Degraded
```

`GpuLeaseManager` must never mark the GPU idle solely because a Rust guard dropped.

## 9. ResourceSnapshot And Telemetry Reconciliation

`GpuLeaseManager` consumes a reconciled `ResourceSnapshot`.

```rust
pub struct ResourceSnapshot {
    pub vram: VramSnapshot,
    pub ram: RamSnapshot,
    pub l1: L1RuntimeSnapshot,
    pub image: ImageRuntimeSnapshot,
    pub processes: Vec<ResourceProcess>,
    pub sampled_at: Instant,
}
```

The snapshot reconciles:

- NVML or `nvidia-smi` free/total VRAM,
- system RAM,
- llama-server process state,
- llama-server logical residency,
- ComfyUI process state,
- known GPU process ownership,
- active lease owner.

Admission rule:

```text
Logical state can request.
Telemetry decides.
Reconciliation confirms.
```

## 10. L1 Residency Modes

L1 has explicit residency states.

```rust
pub enum L1Residency {
    Stopped,
    Starting,
    GpuHot,
    RamHotVramCold,
    CpuResidentLegacy,
    ReloadingGpu,
    Error,
}
```

### 10.1 Preferred Mode: Router Mode RAM-Hot / VRAM-Cold

When llama-server supports Router Mode:

- `POST /v1/models/unload` drops GPU tensors.
- llama-server process remains alive.
- HTTP API remains alive.
- mmap/page cache may remain RAM-hot.
- reload uses `POST /v1/models/load`.
- slot save/restore remains best-effort.

This is the preferred Tier-B L1 image-generation handoff.

### 10.2 Legacy Fallback: Process Restart

If Router Mode is unavailable or fails:

- save slot 0 best-effort,
- stop llama-server,
- wait for VRAM release,
- respawn with CPU or GPU parameters,
- restore slot 0 best-effort.

Legacy restart is a fallback, not the default architecture.

## 11. Pluggable Image Backend

Image generation is abstracted behind a trait.

```rust
#[async_trait]
pub trait ImageBackend: Send + Sync {
    fn id(&self) -> ImageBackendId;
    fn capabilities(&self) -> ImageBackendCapabilities;
    async fn health(&self) -> ImageBackendHealth;
    async fn estimate(&self, request: &ImageRequest) -> ImageEstimate;
    async fn generate(
        &self,
        request: ImageRequest,
        ctx: ImageExecutionContext,
    ) -> Result<ImageResult, ImageError>;
    async fn cancel(&self, job_id: ImageJobId) -> Result<(), ImageError>;
    async fn release(&self) -> Result<(), ImageError>;
}
```

### 11.1 Current Production Backend: ComfyUI

ComfyUI remains the current production backend.

It is managed through:

- process lifecycle,
- REST `/prompt`,
- REST `/system_stats`,
- REST `/free?unload_models=true`,
- REST `/interrupt`,
- WebSocket progress bridge,
- workflow graph generation.

### 11.2 Future Backends

The trait boundary must allow:

- `stable-diffusion.cpp`,
- Candle-based local generation,
- remote/cloud fallback providers,
- future multi-GPU backends,
- specialized video/image modalities.

No code outside the image backend registry should depend on ComfyUI-specific workflow details.

## 12. Cancellation Tree

Every accepted turn receives a root cancellation token.

```rust
pub struct TurnCancellationTree {
    pub root: CancellationToken,
    pub l0: CancellationToken,
    pub l1: CancellationToken,
    pub tools: CancellationToken,
    pub sidecar: CancellationToken,
    pub mcp: CancellationToken,
    pub image: CancellationToken,
}
```

Canceling the root must propagate to every child.

### 12.1 Required Cancellation Effects

L0:

- abort queued classifier job if not started,
- discard result if stale.

L1:

- cancel active stream or request,
- signal llama-server stream cancellation where supported,
- reject stale completion result.

Tools:

- abort owned task,
- terminate owned subprocess where safe,
- return cancellation result to AgentLoop.

Sidecar:

- remove pending request ID,
- reject stale response,
- optionally send cancellation method when processor supports it.

MCP:

- cancel pending request,
- reject stale response,
- restart unresponsive server if required.

Image:

- call backend cancellation,
- for ComfyUI call `/interrupt`,
- cancel WebSocket listener,
- release or recover GPU lease.

## 13. Memory Write Authority

`MemoryManager` is the only semantic writer.

```rust
pub trait MemoryManager: Send + Sync {
    fn store_turn(&self, turn: ConversationTurn) -> Result<TurnId>;
    fn store_fact(&self, fact: FactWrite) -> Result<FactId>;
    fn store_media(&self, media: MediaWrite) -> Result<MediaId>;
    fn store_snippet(&self, snippet: SnippetWrite) -> Result<()>;
    fn store_document_chunks(&self, doc: DocumentIngestWrite) -> Result<DocumentId>;
    fn set_preference(&self, pref: PreferenceWrite) -> Result<()>;
}
```

`MemoryStore` becomes private persistence:

- SQLite connection,
- migrations,
- FTS tables,
- low-level queries.

No UI command, tool, sidecar bridge, Telegram bridge, RAG engine, or AgentLoop event consumer may write directly to `MemoryStore`.

## 14. Backpressure And Saturation

V2 requires bounded admission control.

### 14.1 Per-Session Turn Gate

Each session has at most one active foreground turn by default.

New turn policy:

```text
same session + active turn:
  cancel previous turn if user intent supersedes
  queue only if explicitly requested
  reject if queue full
```

### 14.2 Bounded Queues

Required queues:

- turn admission queue,
- L0 classifier queue,
- sidecar request queue,
- tool execution queue,
- image job queue,
- MCP request queue.

### 14.3 Stale-Turn Invalidation

Every async result carries `turn_id`.

If returned `turn_id` is no longer current:

- discard result,
- release resources,
- do not write memory,
- do not emit final UI state.

### 14.4 Overload Policy

Overload must degrade predictably:

```text
L0 busy -> deterministic fallback or L1
sidecar busy -> tool reports dependency busy
image queue full -> reject or offer cloud fallback
L1 busy -> queue, clarify, or cancel previous turn
GPU degraded -> L0/tool-only mode
```

## 15. Execution Plane

### 15.1 AgentLoop Executor Contract

`AgentLoop` receives:

- `TurnExecutionContext`,
- `IntentEnvelope`,
- `ResourcePlan`,
- mounted tool allowlist,
- cancellation tree,
- memory manager handle.

It executes within those bounds.

It may request replanning only through `TurnGate`.

### 15.2 Replanning Contract

`AgentLoop` cannot mutate `ResourcePlan` directly. When tool results, policy checks, missing context, or execution failures require a different plan, it emits:

```rust
pub enum ExecutorEvent {
    Replan { reason: ReplanReason },
}
```

`TurnGate` consumes `ExecutorEvent::Replan(reason)`, re-runs the required planning stages, and returns a new immutable `ResourcePlan` with the same `turn_id` unless the turn has become stale or canceled.

### 15.3 Tool Execution

Every tool execution must include:

- tool name,
- validated args,
- turn ID,
- cancellation token,
- timeout,
- safety decision,
- audit record.

Subprocess-owning tools must expose best-effort termination on cancellation.

## 16. Safety Model

The safety system remains downstream of planning and upstream of execution.

Rules:

- `HazardHint` cannot lower actual risk.
- L0 cannot authorize execution.
- L1 cannot call unmounted tools.
- Tool arguments must be validated by Rust.
- HITL approval is required for configured yellow/red paths.
- Blacklisted operations are blocked before execution.
- Audit records are mandatory for state-changing actions.

## 17. Trait Boundaries For Future Scalability

V2 must keep these boundaries explicit:

MUST IMPLEMENT NOW:

```rust
trait ImageBackend {}
trait MemoryManager {}
trait L1Runtime {}
trait ResourceTelemetry {}
```

Status (Updated 2026-05-04):

- [x] `ImageBackend`
- [x] `MemoryManager`
- [x] `L1Runtime`
- [x] `ResourceTelemetry`

DEFER FOR LATER SCALING:

```rust
trait TurnPlanner {}
trait RouterClassifier {}
trait ResourcePlanner {}
trait GpuLeaseManager {}
trait ExecutionEngine {}
trait SafetyPolicy {}
```

Future multi-agent federation should plug in as additional planners or executors behind these traits, not by bypassing TurnGate.

Future multi-GPU support should extend `GpuLeaseManager` into a device-aware lease manager, not rewrite image or L1 code.

Future continuous learning should write through `MemoryManager`, not direct database access.

## 18. IMMEDIATE CODEBASE REMEDIATIONS

These active bugs must be fixed before new V2 features are built.

Status (Updated 2026-05-04): Completed for 18.1, 18.2, 18.3, and 18.4.

### 18.1 Fix `SidecarBridge::request` Pending-ID Leak

Current problem:

- sidecar requests insert into `pending`,
- timeout returns an error,
- pending ID may remain registered.

Required fix:

- [x] remove pending ID on timeout,
- [x] remove pending ID on channel close,
- [x] reject stale late responses,
- [x] add regression tests (`drain_pending_waiters_closes_receivers`, `stale_late_response_is_rejected`).

Affected file:

- `crates/kria-core/src/sidecar/bridge.rs`

### 18.2 Fix Isolated Tool Cancellation

Current problem:

- cancellation can return early to `AgentLoop`,
- spawned isolated task may continue until timeout.

Required fix:

- [x] pass cancellation token into `run_isolated`,
- [x] abort join handle on cancellation,
- [x] add optional cleanup hook for subprocess-owning tools,
- [x] add regression tests with long-running fake tool and cleanup hook execution.

Affected files:

- `crates/kria-core/src/infra/isolation.rs`
- `crates/kria-core/src/agent/loop_engine.rs`
- subprocess-owning tools under `crates/kria-core/src/tools/`

### 18.3 Parameterize `MemoryStore::query_audit`

Current problem:

- audit query SQL is built with string formatting.

Required fix:

- [x] use parameterized rusqlite queries,
- [x] avoid interpolating `risk_level` and `session_id`,
- [x] add injection regression test (`query_audit_uses_bound_params_and_rejects_injection_payload`).

Affected file:

- `crates/kria-core/src/memory/store.rs`

### 18.4 Wire Image Cancellation To ComfyUI `/interrupt`

Current problem:

- WebSocket listener cancellation does not necessarily interrupt ComfyUI execution.

Required fix:

- [x] propagate turn image cancellation through isolated-task abort and image-tool cancellation guard,
- [x] ensure ComfyUI backend cancellation calls `/interrupt` (WS receiver-drop and token-cancel paths),
- [x] preserve existing lease recovery/restore flow around interrupted local generation,
- [x] add cancellation integration test with fake Comfy endpoint (`receiver_drop_triggers_comfy_interrupt`).

Affected files:

- `crates/kria-core/src/image/orchestrator.rs`
- `crates/kria-core/src/image/comfy.rs`
- `crates/kria-core/src/image/ws_bridge.rs`
- `crates/kria-core/src/tools/image_generation.rs`

## 19. Implementation Roadmap

### Delivery Status (Updated 2026-05-04)

- Phase 1: Completed.
- Phase 2: Completed.
    - [x] Added per-session supersession gate (`TurnAdmission`).
    - [x] Added hierarchical `TurnCancellationTree` with `root`, `l0`, `l1`, `tools`, `sidecar`, `mcp`, and `image` tokens.
    - [x] Wired `AgentLoop` admission so a new turn supersedes and cancels the previous active turn for the same session.
    - [x] Added stale-turn fast path via `is_active(session_id, turn_id)`.
    - [x] Added stale-turn invalidation guards at round boundaries, tool execution boundaries, and final output emission in `AgentLoop`.
    - [x] Routed isolated tool execution cancellation through plane-aware child tokens (`tools`/`sidecar`/`mcp`/`image`) and moved Colab bootstrap into isolated execution.
    - [x] Added active-turn gating for async tool heartbeat callbacks to suppress stale progress events after supersession.
    - [x] Added bounded per-session queue primitives with explicit full rejection (`enqueue_turn`, `dequeue_next_turn`, `TurnAdmissionError::QueueFull`).
    - [x] Wired explicit queue-vs-supersede admission policy at runtime ingress: default supersession, explicit queue intent enqueues, queue-full emits immediate rejection.
    - [x] Propagated `turn_id` + `is_active` checks into async stream callback paths via `TurnAccepted` marker events and consumer-side stale gating (desktop chat, voice, image, Telegram).
- Phase 3: Completed.
    - [x] Added `TurnGate` scaffold with structured `IntentEnvelope` and `ResourcePlan` contracts.
    - [x] Added `AgentLoop` pre-plan handoff to `TurnGate` with per-turn planner telemetry.
    - [x] Moved pure-image turn classification and initial vision-vs-chat backend selection to `TurnGate` intent/resource-plan outputs.
    - [x] Moved reflex-cancel top-level short-circuit to `TurnGate` operation/resource-plan outputs (skip backend + tool routing for reflex cancel turns).
    - [x] Prioritized `TurnGate` `GenerateImage` operation for direct tool hinting and synthetic fallback call injection when LLM emits no tool call.
    - [x] Moved additional top-level routing authority from `AgentLoop` into `TurnGate` (tool-hint authority + fallback-hint planning for `Search`, `RetrieveMemory`, file-search `Read`, and `GenerateImage`).
    - [x] Kept `AgentLoop` executor-compatible while consuming TurnGate-owned routing/fallback hints.
- Phase 7: Completed.
    - [x] Introduced `MemoryManager` trait boundary and wired `MemoryStore` as the default implementation.
    - [x] Migrated initial write call-sites in knowledge tools (`store_fact`, `store_snippet`) to `MemoryManager`.
    - [x] Migrated Telegram turn persistence writes to `MemoryManager` (`store_turn` for user and assistant turns).
    - [x] Migrated desktop command write paths (`store_turn`, `store_media`, `set_preference`) to `MemoryManager` for chat, voice, image, upload, and session metadata flows.
    - [x] Migrated `FactManager` write paths (`store_fact`, `update_fact_access`, `delete_fact`) to `MemoryManager`.
    - [x] Migrated `RagEngine` write paths (`store_document_chunk`, `delete_document_chunks`) to `MemoryManager`.
    - [x] Migrated memory retrieval/decay and knowledge recall access updates (`update_fact_access`, `update_fact_decay`, `delete_fact`) to `MemoryManager`.
    - [x] Migrated remaining active tool/platform write call sites to `MemoryManager`.
    - [x] Made `MemoryStore` private persistence (`memory::store` module private) with explicit memory-root reexports.
    - [x] Added read/runtime contracts (`MemoryReader`, `MemoryRuntime`) and migrated active read-side callers (desktop commands, Telegram bridge, knowledge tools, and tool registry wiring) behind those contracts.
- Phase 4: Completed.
    - [x] Implemented simplified `GpuLeaseManager` with `Idle`, `Held`, `Recovering`, and `Degraded` states.
    - [x] Added `ResourceSnapshot` + runtime sub-snapshots for reconciliation inputs.
    - [x] Added `ResourceTelemetry` trait boundary with orchestrator-backed reconciled snapshots.
    - [x] Wired reconciliation hooks in image and L1 orchestrators.
    - [x] Gated local image-generation paths behind lease admission.
    - [x] Gated L1 GPU residency transitions behind lease claim/release hooks.
    - [x] Gate remaining vision and speech GPU paths with lease admission.
- Phase 5: Completed.
    - [x] Formalized L1 residency states (`GpuHot`, `RamHotVramCold`, legacy CPU, reload/error states).
    - [x] Added `L1Runtime` trait boundary and implemented it on the orchestrator runtime.
    - [x] Added unload/load latency metrics in the L1 orchestrator.
    - [x] Added slot save/restore success status tracking to traces/metrics.
- Phase 6: Completed.
    - [x] Introduced `ImageBackend` trait boundary and execution context contract.
    - [x] Added `ImageBackendRegistry` with explicit default + cloud fallback slots.
    - [x] Wrapped current ComfyUI orchestrator behind the `ImageBackend` trait.
    - [x] Added future-backend placeholder identity (`SdCpp`) without replacing ComfyUI.
- Phase 8: Completed.
    - [x] Added bounded optional L0 classifier worker scaffold.
    - [x] Integrated classifier output as non-authoritative operation/compute hints in TurnGate fallback.
    - [x] Kept deterministic and existing semantic routes primary over classifier hints.
    - [x] Finalize corpus-driven threshold calibration.

### Phase 1: Remediation Before Expansion

- [x] Fix sidecar pending leak.
- [x] Fix isolated tool cancellation.
- [x] Parameterize audit query.
- [x] Wire ComfyUI `/interrupt`.

### Phase 2: Turn Admission And Cancellation Tree

- Add per-session turn gate.
- Add root `TurnCancellationTree`.
- Add stale-turn invalidation.
- Replace unbounded foreground turn admission.

### Phase 3: Planner Boundary

- Add `TurnGate`.
- Move top-level routing from `AgentLoop` into `TurnGate`.
- Keep `AgentLoop` executor-compatible during migration.
- Add structured `IntentEnvelope`.

### Phase 4: Resource Plane

- Implement simplified `GpuLeaseManager`.
- Add `ResourceSnapshot`.
- Reconcile telemetry with logical ownership.
- Gate L1, image, vision, and speech GPU paths.

### Phase 5: L1 Residency Cleanup

- Formalize `GpuHot`, `RamHotVramCold`, and legacy restart states.
- Add metrics for unload/load latency.
- Add slot save/restore status to traces.

### Phase 6: Image Backend Trait

- Introduce `ImageBackend`.
- Wrap current ComfyUI implementation.
- Keep cloud fallback behind backend registry.
- Prepare optional `sd.cpp` backend without replacing ComfyUI.

### Phase 7: Memory Authority

- Introduce `MemoryManager`.
- Make `MemoryStore` private persistence.
- Migrate desktop, tools, Telegram, RAG, and FactManager writes.

### Phase 8: Optional L0 ONNX Classifier

- Add bounded CPU worker.
- Use corpus-driven thresholds.
- Use only for operation/compute hints.
- Keep deterministic and FastEmbed routes primary.

## 20. Acceptance Criteria

V2 is accepted only when:

- one active top-level planner exists,
- every accepted turn has a cancellation tree,
- no GPU user can run without a lease,
- telemetry reconciliation can force recovery/degraded state,
- L1 Router Mode is treated as RAM-hot/VRAM-cold,
- image generation is behind `ImageBackend`,
- sidecar/tool/image cancellation is real, not cosmetic,
- memory writes pass through `MemoryManager`,
- queues are bounded,
- stale turn results cannot write memory or update UI.

## 21. Final Architecture Rule

K.R.I.A. must behave like an edge operating system for intelligence:

```text
TurnAdmission controls load.
TurnGate plans once.
GpuLeaseManager admits scarce hardware.
AgentLoop executes within plan.
PolicyEngine authorizes action.
MemoryManager writes durable knowledge.
CancellationTree kills stale work.
Telemetry decides recovery truth.
```

Anything outside those boundaries is an orchestration leak.
