# KRIA Core Orchestrator + LLM Runtime Architecture

Production architecture handbook for KRIA's bounded cognition orchestration runtime.

This document explains how KRIA turns a user prompt into routed cognition, model calls,
tool execution, verification, recovery, and final user-facing output. It is intentionally
architecture-first: the focus is not "how to call an LLM", but how KRIA controls LLMs
inside a desktop operations runtime without giving raw model output unchecked authority.

Updated against the current KRIA working tree on 2026-05-27.

## Reader Contract

This handbook is written for both newcomers and engineers:

- **Plain-language model first:** each major concept starts with the human idea before
  introducing runtime names such as `AgentLoop`, `ModelRouter`, or `LlmBackend`.
- **Current implementation is source-backed:** sections before **Future Runtime Roadmap**
  describe behavior represented in the current KRIA source tree, with real file references.
- **Operational interpretation is labeled by context:** diagrams explain how the code works
  at runtime; they are not separate product promises.
- **Future work is isolated:** improvements, desired hardening, and long-term direction live
  in **Section 20. Future Runtime Roadmap** and are not described as current behavior.
- **When in doubt, source wins:** if prose and code ever disagree, treat the linked source
  file as authoritative and update this document.

---

## 1. Executive Overview

KRIA Core Orchestrator is the intelligence traffic controller inside KRIA. It decides
when a model is needed, which model/provider should be used, which tools are visible,
how context is assembled, how tool calls are parsed, how execution is bounded, and when
the system must pause for policy, verification, or human approval.

KRIA is not a chatbot wrapper. A chatbot wrapper forwards text to a model and displays
whatever comes back. KRIA uses the model as one bounded reasoning component inside a
larger operational runtime.

```text
User Prompt
    |
    v
TurnAdmission + TurnGate
    |
    v
Intent / Operation / Resource Plan
    |
    v
Prompt Compiler + Context Injectors + Tool Router
    |
    v
ModelRouter / FailoverRouter
    |
    v
LlmBackend
    |        local llama.cpp / Ollama
    |        OpenAI-compatible APIs
    |        Anthropic
    |        Gemini
    |        OpenRouter
    v
Model Response
    |
    v
Tool Call Parser + Deterministic Fallbacks
    |
    v
ExecutionGate + PolicyEngine + HITL/DecisionStore + Isolation + ToolRegistry
    |
    v
Tool Execution + Verification + Recovery
    |
    v
ResultSynthesizer / Final Response
```

### What Problem It Solves

Desktop cognition is messy. A user can ask for code editing, browser work, filesystem
operations, package installation, current information, or GUI automation in one sentence.
The orchestrator solves four problems at once:

| Problem | KRIA Runtime Answer |
| ------- | ------------------- |
| Models are probabilistic | Use policy, verifiers, routing, and deterministic fallbacks around them |
| Providers differ | Normalize every provider behind `LlmBackend` and `ProviderRegistry` |
| Context can explode | Use prompt sections, token ledgers, payload shaping, and trimming |
| Tool execution is risky | Route tools through policy, HITL, audit, isolation, and verification |
| GUI workflow expectations can be lost | Resolve semantic workflow frame, fidelity, mode, contract, and verifier authority before substrate planning |

### Hybrid Intelligence Philosophy

KRIA is local-first, not local-only. It prefers local execution and local inference when
that is sufficient, but can route to cloud providers when configured and useful. Local
models protect privacy and offline continuity. Cloud models provide larger context,
stronger reasoning, or vision/tool capability when the local runtime is degraded.

```text
                 +--------------------------+
                 | KRIA Orchestration Core  |
                 +------------+-------------+
                              |
        +---------------------+----------------------+
        |                                            |
        v                                            v
+-------------------+                    +------------------------+
| Local Cognition   |                    | Cloud Cognition        |
| llama.cpp/Ollama  |                    | OpenAI/Anthropic/etc.  |
| offline/private   |                    | larger/capable/fallback|
+-------------------+                    +------------------------+
        |                                            |
        +---------------------+----------------------+
                              |
                              v
                  Bounded Tool Execution
```

### Primary Source Files

- `crates/kria-core/src/agent/loop_engine/mod.rs`
- `crates/kria-core/src/llm/mod.rs`
- `crates/kria-core/src/llm/model_router.rs`
- `crates/kria-core/src/llm/failover.rs`
- `crates/kria-core/src/llm/provider/*`
- `crates/kria-core/src/llm/local.rs`
- `crates/kria-core/src/llm/cloud.rs`
- `crates/kria-core/src/agent/prompt_compiler.rs`
- `crates/kria-core/src/llm/budget.rs`

---

## 2. Core Orchestration Philosophy

KRIA's orchestration layer exists because raw model intelligence is not operational
authority. The model may propose, explain, classify, or generate. The runtime decides
what is allowed to run.

### Bounded Cognition

Bounded cognition means every intelligence step has limits:

- bounded tool visibility,
- bounded context injection,
- bounded output payloads,
- bounded retry loops,
- bounded execution authority,
- bounded memory retrieval,
- bounded fallback behavior.

```text
Unbounded Agent Pattern                 KRIA Pattern
-----------------------                 ------------
LLM decides tools                       Runtime routes tools
LLM loops until done                    max_tool_rounds caps turns
LLM sees all memory                     bounded retrieval/context blocks
LLM decides safety                      PolicyEngine + HITL decide safety
LLM claims success                      verifier/synthesizer ground success
```

### Orchestration Authority Chain

```text
User intent
    |
    v
TurnGate / Intent classifiers
    |
    v
Prompt + tool catalog
    |
    v
LLM suggestion
    |
    v
Tool parser / deterministic fallback
    |
    v
Tool availability gate
    |
    v
ExecutionGate
    |
    v
PolicyEngine
    |
    +--> BLACK: block
    +--> RED: HITL approval
    +--> GREEN/YELLOW: execute if permitted
    |
    v
Isolation + ToolRegistry handler
    |
    v
ExecutionVerifier / observable completion
    |
    v
Grounded final response
```

The LLM never outranks policy, tool availability, HITL, verifier authority, or runtime
cancellation. This is the central design choice that keeps KRIA from becoming a recursive
autonomy system.

### Prompt Authority Hierarchy

KRIA treats prompt material as layered, not flat:

| Layer | Authority | Example |
| ----- | --------- | ------- |
| Runtime invariants | Highest | tool format, safety, no invented tool outputs |
| Policy and tool catalog | High | available tools, GUI-last policy |
| Operational context | Medium | live desktop state, workflow continuation |
| User prompt | Goal authority | requested task |
| Tool/OCR/web payloads | Evidence only | search results, command output, DOM/OCR |

### Why KRIA Avoids Naive LLM Architectures

| Anti-pattern | Why It Is Avoided |
| ------------ | ----------------- |
| Autonomous recursive prompting | Hard to bound, audit, cancel, or verify |
| Agent swarms | Adds coordination failure without operational authority |
| Screenshot-only reasoning | Fragile, slow, and blind to semantic state |
| Vector-memory everywhere | Retrieval contamination and context bloat |
| Direct raw model execution | No policy, no verification, no reliable recovery |
| "LLM as OS" | Confuses reasoning with execution authority |

---

## 3. Full Orchestrator Runtime Architecture

The orchestrator is not one file. It is a runtime assembly centered on `AgentLoop` and
connected to model routing, prompt construction, memory, policy, tools, GUI cognition,
and provider abstractions.

```text
+--------------------------------------------------------------------------------+
|                                KRIA AgentLoop                                  |
|             crates/kria-core/src/agent/loop_engine/mod.rs                      |
+--------------------------------------------------------------------------------+
    |              |              |              |              |              |
    v              v              v              v              v              v
TurnGate       Prompt        ModelRouter     ToolRegistry    Policy/HITL   Verifiers
Intent         Compiler      FailoverRouter  MCP/Local/API   Audit         Completion
Planning       Context       LlmBackend      Execution       Isolation     Synthesis
```

### Runtime Layers

```text
Layer 7: User-facing stream events
         Token, Plan, ToolStart, ToolEnd, RecoveryOptions, ApprovalRequired, Done

Layer 6: Result synthesis and semantic completion
         ResultSynthesizer, observable completion, verifier outcomes

Layer 5: Execution authority
         PolicyEngine, HITL, AuditLogger, run_isolated, ToolRegistry

Layer 4: Tool orchestration
         Tool routing, tool schema filtering, fallback calls, payload shaping

Layer 3: LLM orchestration
         Prompt compiler, ModelRouter, FailoverRouter, LlmBackend

Layer 2: Operational context
         PSDG, desktop awareness, workflow continuation, document RAG

Layer 1: Turn admission and intent
         TurnAdmission, TurnGate, intent compilers, operation classification
```

### Major Systems

| System | Main Files | Responsibility |
| ------ | ---------- | -------------- |
| AgentLoop | `agent/loop_engine/mod.rs` | ReAct-style orchestration, streaming events, tool rounds |
| ModelRouter | `llm/model_router.rs` | Select local/cloud backend by routing mode and vision needs |
| FailoverRouter | `llm/failover.rs` | Deterministic primary/fallback FSM |
| Provider abstraction | `llm/mod.rs`, `llm/provider/*` | Normalize chat, streaming, tools, vision, health |
| Prompt compiler | `agent/prompt_compiler.rs`, `agent/prompts.rs` | Deterministic prompt sections and legacy operating prompt |
| Context budget | `llm/budget.rs`, `llm/tokenize.rs` | Token ledger, pressure, context windows |
| Local runtime | `llm/local.rs`, `llm/orchestrator/*` | llama.cpp server, VRAM-aware local inference |
| Cloud runtime | `llm/cloud.rs`, `llm/provider/openai.rs` | OpenAI-compatible request/stream handling |
| Tool orchestration | `tools/registry.rs`, `mcp/payload_shaper.rs` | Tool schemas, handlers, compact LLM payloads |
| Policy/HITL | `safety/policy.rs`, `safety/hitl.rs`, `safety/audit.rs` | Safety tiering, approval, audit trail |
| Execution gate | `agent/execution_gate.rs`, `agent/collaborative_decision.rs` | Preflight, execution authority, policy, resource leases, and durable decisions. |
| GUI semantic workflow | `agent/semantic_workflow.rs`, `execution_mode_reasoner.rs`, `workflow_intent_contract.rs` | Deterministic GUI workflow/fidelity/mode/contract metadata before substrate execution. |
| GUI cognition | `agent/gui_wiring.rs`, `agent/gui_planner.rs`, `tools/gui_automation.rs` | Desktop automation integration |
| PSDG/context | `agent/psdg/*`, `agent/desktop_awareness/*` | Live desktop facts and workflow state |
| Recovery | `agent/workflow_continuation/*`, failure classifiers in `AgentLoop` | Interruption and failure handling |

### Provider Abstraction Overview

The core trait is `LlmBackend` in `crates/kria-core/src/llm/mod.rs`.

```rust
#[async_trait]
pub trait LlmBackend: Send + Sync {
    fn model_label(&self) -> &str;
    fn capabilities(&self) -> ModelCapabilities;
    fn is_configured(&self) -> bool;
    async fn chat(&self, messages: &[ChatMessage], tools: Option<&[ToolSchema]>,
        temperature: f32, max_tokens: u32) -> anyhow::Result<LlmResponse>;
    async fn chat_stream(&self, messages: &[ChatMessage], tools: Option<&[ToolSchema]>,
        temperature: f32, max_tokens: u32) -> anyhow::Result<UnifiedStream>;
    async fn health_check(&self) -> anyhow::Result<ProviderStatus>;
}
```

This interface is the contract that lets the orchestrator treat llama.cpp, Ollama,
OpenAI-compatible APIs, Anthropic, Gemini, and OpenRouter as interchangeable cognition
backends while keeping execution control outside the provider.

---

## 4. End-to-End Prompt Lifecycle

Every prompt becomes a turn. A turn may be conversational, operational, GUI-driven,
tool-driven, or clarification-oriented.

```text
Prompt
  |
  v
TurnAdmission: accept, queue, supersede, cancel stale work
  |
  v
TurnGate: classify operation, confidence, resource plan
  |
  v
Tool routing: visible mounted tools + semantic/router filters + tier gates
  |
  v
Prompt assembly: system sections + tool schemas + context + memory
  |
  v
Backend selection: ModelRouter or FailoverRouter
  |
  v
LLM response: text and/or native function calls
  |
  v
Tool parser: native calls first, text fallback second, deterministic fallback last
  |
  v
Semantic GUI workflow metadata when the turn is GUI-eligible
  |
  v
ExecutionGate / Policy / HITL / audit / isolation
  |
  v
Tool results shaped for LLM and full payload streamed to UI
  |
  v
Verification, continuation, synthesis
  |
  v
Done or RecoveryOptions or ApprovalRequired or Error
```

### Example 1: "open code and write a program to print pascal triangle and run it and show output"

```text
User prompt
  |
  v
TurnGate: operation ~= Automate / ExecuteCode / Write
  |
  v
Intent compiler: complex multi-verb GUI/code workflow
  |
  v
Semantic workflow metadata:
  - task family = Coding
  - app anchor = IDE required
  - fidelity = WorkflowStageFidelity
  - mode = HybridWorkflow
  - contract = VisibleCodingWorkflow
  |
  v
Tool routing: IDE/GUI/code tools become visible
  |
  v
Prompt compiler injects desktop context if PSDG has active workspace
  |
  v
Model call only if deterministic routing cannot produce a known workflow
  |
  +--> deterministic path: IdeCodeRunWorkflow
  +--> fallback path: LLM-generated tool/workflow proposal, still policy-gated
  |
  v
ExecutionVerifier checks observable effects:
  - file/content exists
  - runner script exists
  - IDE launch is observed by process evidence
  - command ran
  - output contains Pascal triangle-like lines
  |
  v
ResultSynthesizer reports observed output, not merely "done"
```

Participating files:

- `agent/loop_engine/mod.rs`
- `agent/turn_gate.rs`
- `agent/intent_compiler_rule.rs`
- `agent/intent_compiler_llm.rs`
- `agent/gui_substrate_planner.rs`
- `agent/gui_planner.rs`
- `agent/ide_cognition.rs`
- `tools/gui_automation.rs`
- `tools/developer.rs`
- `tools/exec.rs`
- `agent/execution_verifier_bounded.rs`
- `agent/result_synthesizer.rs`

Failure branches:

| Failure | Runtime Behavior |
| ------- | ---------------- |
| VSCode opens but does not focus | GUI/focus verification fails; workflow may recover or ask user |
| LLM emits no tool call | TurnGate fallback can inject a deterministic tool call |
| Visible terminal launch unavailable | Runner uses structural fallback and appends a disclosure marker to the output artifact |
| Code runs but output hidden | semantic completion may fail; final response should not claim output was shown |
| local model unavailable | ModelRouter/FailoverRouter may route fallback or return clear unavailable message |

### Example 2: "open youtube and play latest song from my playlist"

```text
Prompt
  |
  v
TurnGate: Browser/GUI automation, live/personal ambiguity
  |
  v
Tool routing:
  - browser_search/open_url/browser cognition if available
  - GUI tools only as last resort if browser substrate unavailable
  - browser/media governance marks private playlist/account state as HITL-required metadata
  |
  v
Prompt context:
  - current date if "latest" matters
  - browser/desktop context from PSDG if relevant
  |
  v
Execution:
  - open YouTube
  - navigate/search playlist
  - handle auth/session state if already logged in
  |
  v
HITL/clarification:
  - if playlist identity is ambiguous
  - if private account access is unavailable
  - if user action is needed for login/media permission
```

Important architectural point: the LLM does not directly "drive YouTube". It proposes
or selects a browser/GUI route. The tool substrate performs the operation, and completion
depends on observable browser/media state.

### Example 3: "fix compile errors in current Rust project"

```text
Prompt
  |
  v
TurnGate: ExecuteCode / Write / IDE-aware workflow
  |
  v
PSDG context injection:
  - IDE workspace
  - active file
  - terminal cwd
  |
  v
Tool routing:
  - shell/developer/file tools
  - IDE cognition when useful
  |
  v
Execution graph:
  cargo check
      |
      v
  parse diagnostics
      |
      v
  edit focused files
      |
      v
  cargo check again
      |
      v
  final verification
```

The orchestrator uses the model for diagnosis and edit planning, but verification is
deterministic: compiler output determines whether the fix succeeded.

Failure branches:

- If workspace cannot be inferred, the system asks one targeted clarification.
- If edits require broad refactors, workflow continuation stores progress and blockers.
- If the model suggests unsafe shell changes, policy/preflight can block execution.

### Example 4: "summarize latest Jira tickets and create implementation plan"

```text
Prompt
  |
  v
TurnGate: live external data + planning/summarization
  |
  v
Tool routing:
  - Jira/MCP connector if mounted
  - browser or web only if no API/MCP route is available
  |
  v
If connector missing:
  HITL/clarification: ask for connection/configuration path
  |
  v
If data available:
  payload_shaper compacts ticket fields for LLM
  |
  v
LLM synthesis:
  summarize grounded tickets
  group by priority/component
  create implementation plan
```

This workflow demonstrates provider separation. A cloud model may be chosen for larger
ticket context or stronger synthesis, while the actual Jira access remains a tool/MCP
operation with bounded payload shaping. The model must not invent tickets.

---

## 5. Prompt Compilation + Context Engineering

KRIA has both a legacy prompt builder and a typed prompt compiler. The typed compiler
exists because string-marker prompt rewriting is fragile at scale.

### Typed Prompt Sections

`agent/prompt_compiler.rs` defines `StructuredPrompt` with deterministic section order:

```text
identity
tools_catalog
system_state
live_fact_mode
user_context
execution_context
session_summary
tool_call_format
```

Each section has a priority:

| Priority | Behavior |
| -------- | -------- |
| 0 | Always included, truncated if necessary |
| 1 | Included if budget allows |
| 2 | Optional, included only with meaningful remaining budget |

```text
StructuredPrompt
  |
  +--> priority 0 sections: identity, tools, format
  |
  +--> priority 1 sections: state, live facts, user/execution context
  |
  +--> priority 2 sections: summaries
  |
  v
AssembledPrompt { text, included_sections, omissions, budget_chars }
```

### Tool Catalog as Runtime Control

The tool catalog is not just documentation for the model. It is a control surface. It
includes only routed tools for the current turn and tags execution modes:

```text
API > MCP > CLI > Browser > GUI
```

This preference is also enforced after model output. In `AgentLoop`, if the model chooses
a browser/GUI last-resort tool while a better API/MCP/CLI alternative is available, the
runtime injects an execution redirect instead of blindly executing the GUI path.

### Context Budgeting

`llm/budget.rs` provides `ContextBudgets` and `TurnTokenLedger`.

```text
Context Window
  |
  +-- system_reserve
  +-- response_reserve
  +-- tool_result_budget
  +-- turn_tool_budget
  +-- history_char_budget
  +-- max_routed_tools
```

The budget system is deterministic. It uses provider tokenizer APIs when available
(`llm/tokenize.rs`) and falls back to a `chars / 4` heuristic.

### Context Trimming

`llm/mod.rs` includes `trim_messages_for_context`, which applies increasingly aggressive
trimming attempts. Tool results are capped through:

- `TOOL_RESULT_MAX_CHARS = 3000`
- `LLM_TOOL_RESULT_TOKEN_BUDGET = 1024`
- `LLM_TURN_TOOL_BUDGET = 4096`

### PSDG Context Injection

`agent/psdg/context_injector.rs` injects live desktop context only when useful:

| Operation | Inject PSDG? |
| --------- | ------------ |
| Automate | Yes |
| ExecuteShell / ExecuteCode | Yes |
| Write | Yes |
| Clarify | Yes |
| Converse | No |
| Search / Read | No |
| GenerateImage | No |

The context block is capped at about 800 characters. KRIA avoids dumping the entire
desktop graph into the prompt.

---

## 6. LLM Routing + Model Selection

`ModelRouter` is the first model selection boundary. It is intentionally simpler than
the higher-level orchestration around it: it selects backends based on configuration,
routing mode, and vision support.

### Routing Modes

`llm/model_router.rs` defines:

| Mode | Meaning |
| ---- | ------- |
| `Local` | Prefer local backend |
| `Colab` | Route to cloud/Colab-style backend where configured |
| `Gemini` | Route to Gemini/cloud backend |
| `External` | Use externally configured cloud backend |

```text
ModelRouter::route(intent)
  |
  +-- RoutingMode::Local    -> LocalBackend
  +-- RoutingMode::Colab    -> CloudBackend or local fallback
  +-- RoutingMode::Gemini   -> CloudBackend
  +-- RoutingMode::External -> CloudBackend
```

Vision routing is separate:

```text
route_vision()
  |
  +-- local vision backend available and runtime supports images -> local vision
  +-- otherwise cloud vision backend if configured
```

### Capability-Aware Provider Layer

The provider layer defines normalized model capabilities in
`llm/provider/capabilities.rs`:

- chat completion,
- streaming,
- tool calling,
- vision,
- embeddings,
- JSON mode,
- reasoning,
- audio,
- system messages.

This capability model exists in the current provider layer. Today, the core router is
still largely configuration-driven, with explicit vision routing and provider
health/failover behavior around it.

### Model Selection Decision Map

```text
Need image input?
  |
  +-- yes --> route_vision()
  |             |
  |             +-- local vision ready?
  |             +-- cloud vision configured?
  |
  +-- no --> route("chat")
                |
                +-- FailoverRouter attached?
                |       |
                |       +-- primary serving -> primary
                |       +-- primary failed  -> fallback
                |
                +-- else ModelRouter mode
```

---

## 7. Local LLM Runtime

The local runtime is more than an HTTP client. It manages an L1 inference service backed
by llama.cpp, including process lifecycle, VRAM budgeting, model loading, streaming
cancellation, and degraded operation.

### Key Files

- `llm/local.rs`
- `llm/orchestrator/server_manager.rs`
- `llm/orchestrator/runtime.rs`
- `llm/orchestrator/strategy.rs`
- `llm/orchestrator/vram_budget.rs`
- `llm/orchestrator/vision_strategy.rs`
- `llm/orchestrator/gpu_watchdog.rs`
- `llm/orchestrator/child_guard.rs`

### L1 Runtime Contract

`llm/orchestrator/runtime.rs` defines a narrow runtime-facing control trait:

```rust
#[async_trait]
pub trait L1Runtime: Send + Sync {
    fn snapshot(&self) -> OrchestratorSnapshot;
    async fn ensure_ready(&self, reason: &str) -> Result<()>;
    async fn release_if_idle(&self, reason: &str) -> Result<bool>;
    async fn evict_to_ram(&self) -> Result<()>;
    async fn reload_to_vram(&self) -> Result<()>;
}
```

The narrow contract prevents higher layers from coupling directly to llama.cpp internals.

### Server State Machine

`LlamaServerManager` stores server state in an atomic byte:

```text
STOPPED
   |
   v
STARTING
   |
   v
READY <----+
   |       |
   v       |
SWAPPING --+
   |
   v
ERROR
```

Important behaviors:

- launches `llama-server`,
- discovers ephemeral ports,
- waits for `/health`,
- tracks GPU layers and context,
- cancels in-flight streams during model swaps,
- uses `ChildGuard` for process cleanup,
- supports router-mode load/unload endpoints when available.

### VRAM-Aware Runtime

`llm/orchestrator/strategy.rs` calculates target parameters:

```text
free VRAM
  - safety margin
  - base overhead
  - vision projector cost
  = layer/context budget
```

It produces:

- GPU layer count (`ngl`),
- context window,
- vision mode,
- degradation level.

Degradation levels:

| Level | Meaning |
| ----- | ------- |
| Full | full GPU, full context, vision available |
| ReducedContext | full GPU, reduced context |
| PartialOffload | some layers on CPU |
| HeavyOffload | heavy CPU offload |
| CpuOnly | no GPU offload |

### Vision Preflight

`llm/orchestrator/vram_budget.rs` prevents reactive OOM by estimating visual tokens before
image analysis. It computes a hard token cap from free VRAM, safety margin, KV cache cost,
and existing context usage.

```text
image dimensions
  |
  v
estimate visual tokens
  |
  v
calculate safe cap from VRAM headroom
  |
  +-- fits -> run
  +-- too large -> downscale or cap
  +-- zero cap -> fail closed
```

---

## 8. Third-Party LLM Integration Architecture

All providers are normalized behind `LlmBackend`, but their APIs differ significantly.
KRIA handles those differences in provider-specific adapters.

### Provider Map

| Provider | File | API Shape | Notes |
| -------- | ---- | --------- | ----- |
| OpenAI-compatible | `llm/provider/openai.rs` | `/chat/completions`, SSE | Used for OpenAI and compatible endpoints |
| OpenRouter | `llm/provider/openrouter.rs` | OpenAI-compatible | Delegates to `OpenAIBackend`; app attribution fields exist but wrapper currently delegates request handling |
| Anthropic | `llm/provider/anthropic.rs` | Messages API | Converts system/tools/messages to Anthropic schema |
| Gemini | `llm/provider/gemini.rs` | `generateContent` | Converts messages to Gemini `contents` and tool declarations |
| Ollama | `llm/provider/ollama.rs` | OpenAI-compatible + Ollama tags | Local provider, model listing/pull support |
| CloudBackend | `llm/cloud.rs` | OpenAI-compatible | Generic cloud backend with retries/rate limits/tool fallback |

### Provider Normalization

```text
KRIA ChatMessage / ToolSchema
    |
    +-- OpenAI adapter     -> messages + tools
    +-- Anthropic adapter  -> system + messages + input_schema
    +-- Gemini adapter     -> contents + function_declarations
    +-- Ollama adapter     -> OpenAI-compatible payload
    +-- CloudBackend       -> OpenAI-compatible payload
    |
    v
LlmResponse { content, model, usage, tool_calls }
```

### Error Normalization

`llm/provider/error.rs` classifies provider failures:

| Error Kind | Typical Source | Retry? |
| ---------- | -------------- | ------ |
| AuthFailure | 401/403 | No |
| RateLimited | 429 | Yes |
| Timeout | client timeout | Yes |
| NetworkError | connection issue | Yes |
| ContextTooLarge | 413 or provider error | Usually no, trim/retry elsewhere |
| ServiceUnavailable | 5xx | Yes |
| InvalidModel | 404/model missing | No |
| Cancelled | cancellation token | No |

### CloudBackend Tool Fallback

`llm/cloud.rs` contains a pragmatic production adaptation: if a provider rejects
tool/function calling with a 400-style error, KRIA can disable `supports_tools` and retry
without tools. This keeps chat available in degraded mode, but it means orchestration must
not assume every cloud model can function-call reliably.

---

## 9. Model Swapping + Runtime Flexibility

KRIA avoids hardcoding one model or one provider. It supports provider switching,
model switching, local server swaps, and fallback routing.

### ProviderRegistry

`llm/provider/registry.rs` owns configured provider instances:

```text
ProvidersConfig
  |
  +-- active_provider
  +-- fallback_provider
  +-- providers[]
          |
          +-- ProviderConfig
                provider_type
                endpoint
                active_model
                streaming preference
                timeout/retry/rate limit
```

It supports:

- initialization,
- backend creation,
- active backend lookup,
- provider switching,
- model switching,
- provider upsert/removal,
- connection testing,
- health snapshots.

### Failover FSM

`llm/failover.rs` wraps `ModelRouter` with deterministic provider state.

```text
Healthy
  |
  | soft failures
  v
Degraded
  |
  | hard threshold
  v
Failed
  |
  v
CoolingDown
  |
  v
Recovering
  |
  +-- probe success -> Healthy
  +-- probe fail    -> CoolingDown/Failed
```

Policy options include manual failover, failover on hard failure, and failover on any
failure. Session stickiness can keep a turn on one provider to avoid mixed-context
behavior.

### Runtime Adaptation

```text
Primary provider fails
  |
  v
FailoverRouter records failure
  |
  +-- below threshold -> keep primary
  +-- threshold hit   -> route to fallback
  |
  v
AgentLoop receives backend + is_fallback flag
  |
  v
Call result feeds FSM health state
```

---

## 10. Tool Orchestration + Execution Intelligence

KRIA does not let the LLM execute tools directly. The LLM may return function calls, but
tool execution is a runtime action.

### Tool Execution Pipeline

```text
LLM response
  |
  v
parse native tool calls
  |
  +-- none -> parse text tool-call patterns
  |
  +-- none -> deterministic TurnGate/package/Colab fallback
  |
  v
allowed_tool_names gate
  |
  v
GUI-last policy check
  |
  v
tool-specific preflight
  |
  v
PolicyEngine
  |
  v
HITL if required
  |
  v
run_isolated(handler.execute_with_context)
  |
  v
payload shaping + full payload UI stream
  |
  v
verifier + turn memory + final synthesis
```

### Runtime Tool Guards

| Guard | Purpose |
| ----- | ------- |
| Mount/tier gating | Only expose tools available in current runtime/hardware |
| Semantic router | Limit catalog to relevant tools |
| GUI-last policy | Prefer API/MCP/CLI over browser/GUI automation |
| Preflight | Check prerequisites before expensive/risky execution |
| PolicyEngine | Enforce risk tier and destructive boundaries |
| HITL | Ask user for approval on red-tier operations |
| Isolation | Run tool handlers with timeout/cancellation boundary |
| Verifier | Check whether execution actually achieved the expected state |

### Payload Shaping

`mcp/payload_shaper.rs` keeps the LLM from seeing huge raw payloads:

```text
Raw MCP/tool result
  |
  +-- keep identity fields
  +-- drop raw/html/base64/attachments
  +-- truncate long strings
  +-- cap arrays
  +-- attach __shape metadata + handle
  |
  v
Compact LLM-visible payload
```

The UI can still receive full payload chunks through `StreamEvent::ToolPayloadChunk`.
The LLM gets the bounded summary.

---

## 11. Memory + Context Runtime

KRIA uses memory as operational evidence, not as an unbounded source of truth.

### Memory Sources

| Source | Files | Use |
| ------ | ----- | --- |
| PSDG | `agent/psdg/*`, `agent/world_model/*` | Desktop state and live operational facts |
| TurnMemory | `agent/turn_memory.rs` | Per-turn completed actions and satisfaction |
| Workflow sessions | `agent/workflow_session.rs` | Checkpoint persistence across tool rounds |
| Procedural memory | `agent/procedural_memory/*` | Reusable workflow patterns |
| Document RAG | preprocessing session store, `memory/rag.rs` | Uploaded document context |
| User memory | `memory/store.rs`, `memory/manager.rs` | Preferences/facts when appropriate |

### Context Flow

```text
Operational state
    |
    +-- PSDG snapshot
    +-- active workflow
    +-- terminal cwd
    +-- IDE workspace
    +-- browser URL/title
    |
    v
Context injector
    |
    v
bounded prompt block
```

### Retrieval Contamination Prevention

KRIA prevents memory from overwhelming the turn by:

- injecting PSDG only for relevant operations,
- capping PSDG prompt blocks,
- using shaped payloads for tool results,
- truncating history under budget pressure,
- keeping full payloads outside the model context,
- treating retrieved facts as context, not authority over policy or verifier results.

---

## 12. Streaming + Real-Time Runtime

Streaming exists at two levels:

1. model token streaming,
2. runtime event streaming.

### Runtime Stream Events

`AgentLoop` emits `StreamEvent` variants:

| Event | Meaning |
| ----- | ------- |
| `TurnAccepted` | turn identity admitted |
| `Token` | model/final text chunk |
| `Plan` | runtime progress/status |
| `ToolStart` | tool execution begins |
| `ToolProgress` | long-running tool heartbeat |
| `ToolPayloadChunk` | full payload chunk to UI |
| `ToolEnd` | tool result available |
| `ApprovalRequired` | HITL request is pending |
| `ApprovalResult` | user approved/denied/timed out |
| `ToolChoiceRequired` | low-confidence routing needs user choice |
| `RecoveryOptions` | deterministic recovery buttons |
| `Error` | non-recoverable error for turn |
| `Done` | final response |

### Streaming Flow

```text
chat_stream()
  |
  v
Provider SSE/native stream
  |
  v
UnifiedStream
  |
  v
Agent/UI event bridge
  |
  +-- token chunks
  +-- cancellation
  +-- final done/error
```

Local streaming has an extra constraint: model swaps cancel active streams through the
`CancellationToken` managed by `LlamaServerManager`. This prevents stale streams from
continuing after the underlying local server changes model/runtime state.

### Cancellation and Staleness

`TurnAdmission` and the loop's stale-turn checks prevent old work from writing into a
new user turn. This is a key production detail: LLM calls and tools may outlive the user
intent that triggered them unless the runtime explicitly cancels or ignores stale output.

---

## 13. Recovery + Failure Handling

KRIA recovery is split between provider recovery, tool recovery, workflow continuation,
and human collaboration.

### Provider Failure Recovery

```text
Provider call fails
  |
  v
Error classified
  |
  +-- retryable provider error -> adapter retry/backoff
  +-- context too large        -> trim/compact/retry path
  +-- primary failure          -> FailoverRouter FSM update
  +-- no backend               -> clear unavailable message
```

### Tool Failure Recovery

`AgentLoop` includes deterministic classifiers for common failures:

- fleet/SSH connectivity,
- Docker daemon not running,
- file not found,
- permission denied,
- package not found,
- shell command not found,
- network timeout.

The result can be `RecoveryOptions`, which the UI renders as action buttons.

```text
Tool error
  |
  v
classify_tool_failure()
  |
  +-- actionable -> RecoveryOptions + context message
  +-- not actionable -> tool error in history, model/synthesizer explains
```

### Malformed or Empty Model Output

If the model returns no tool call:

1. package workflows may inject required package steps,
2. Colab workflows may inject connection/bootstrap steps,
3. TurnGate fallback may inject a deterministic tool call,
4. low confidence may emit `ToolChoiceRequired`,
5. otherwise the final text is used or a generic fallback is emitted.

### Why KRIA Pauses Instead of Hallucinating Success

Visible activity is not completion. If execution evidence is missing, policy denied,
approval timed out, or verification fails, KRIA must say that. This is why tool errors
are injected into the LLM context as explicit `TOOL_ERROR`-style facts and why the result
synthesizer grounds final output in observed execution data.

---

## 14. Safety + Boundedness Engineering

Safety in KRIA is enforced at runtime, not by hoping the prompt convinces the model.

### Safety Architecture

```text
Model output
  |
  v
Tool call parsed
  |
  v
Allowed tool gate
  |
  v
PolicyEngine.evaluate_with_modality_hint()
  |
  +-- blocked          -> audit + no execution
  +-- requires approval -> HITL request
  +-- allowed          -> audit + isolated execution
```

### Risk Authority

| System | Authority |
| ------ | --------- |
| `PolicyEngine` | decides blocked/approval/allowed |
| `HitlGateway` | obtains user decision for red-tier actions |
| `AuditLogger` | records action, parameters, risk, decision |
| `run_isolated` | provides timeout/cancellation execution boundary |
| `ExecutionVerifier` | validates result after execution |
| LLM | suggests tool calls and synthesizes language only |

### Destructive Action Handling

For a prompt like "delete all files in Downloads":

```text
Prompt classified as destructive
  |
  v
Tool route selects filesystem delete/list operations
  |
  v
PolicyEngine sees destructive modality
  |
  +-- requires HITL or blocks depending exact scope
  |
  v
ApprovalRequired event
  |
  +-- denied/timeout -> no deletion, explicit failure
  +-- approved       -> execute with audit
  |
  v
Verifier checks resulting filesystem state
```

The model does not get to decide that deletion is safe.

---

## 15. Human-in-the-Loop (HITL)

HITL is not a weakness in KRIA's autonomy. It is how KRIA keeps operational trust intact
when action risk or ambiguity exceeds the runtime's authority.

### HITL Triggers

| Trigger | Example |
| ------- | ------- |
| Red-tier policy | delete many files, destructive system action |
| Ambiguous target | "delete the old project" with multiple candidates |
| Low-confidence route | several plausible tools and no clear winner |
| External account state | login required for browser/Jira/YouTube |
| Recovery branch | user must choose how to fix prerequisite failure |

### HITL Lifecycle

```text
Tool call requires approval
  |
  v
StreamEvent::ApprovalRequired
  |
  v
HitlGateway waits for user response
  |
  +-- Approved -> audit approved, execute
  +-- Denied   -> audit denied, inject TOOL_ERROR
  +-- Timeout  -> audit timeout, no execution
```

### Collaborative Workflow Model

KRIA asks humans only where human authority is meaningful: approval, credentials, private
account access, ambiguity resolution, or recovery choice. It should not ask the user to
perform steps the runtime can safely execute.

---

## 16. GUI + Browser + IDE Cognition Integration

The orchestrator integrates GUI cognition as a substrate, not as the default path.

### Integration Boundary

```text
AgentLoop
  |
  v
TurnGate / intent compiler
  |
  v
Substrate decision
  |
  +-- API/MCP/CLI available -> use it
  +-- browser semantic route -> browser cognition
  +-- IDE route             -> IDE cognition
  +-- GUI fallback          -> GUI automation substrate
```

### Why GUI Is Last Resort

GUI automation is powerful but fragile:

- focus can be stolen,
- windows can open slowly,
- Wayland may restrict synthetic input,
- visual state may not match semantic completion,
- apps can show popups or hidden panels.

KRIA therefore prefers semantic APIs, MCP connectors, and CLI tools when available.

### Browser Cognition

Browser work may involve:

- opening a URL/search,
- using browser automation tools,
- reading page state through semantic/browser substrates,
- falling back to GUI only when structured access is unavailable.

### IDE Cognition

IDE workflows combine:

- PSDG workspace context,
- active file context,
- shell/compiler feedback,
- file edits,
- GUI/IDE focus only when necessary.

The orchestrator should treat compiler output as stronger evidence than a visually open
editor window.

---

## 17. Eval + Testing Architecture

Orchestration quality requires tests that validate runtime behavior, not just schemas.

### Test Layers

```text
Unit tests
  |
  +-- prompt compiler budget behavior
  +-- payload shaping
  +-- provider error classification
  +-- failover FSM transitions
  +-- intent fallback mapping

Integration tests
  |
  +-- provider connection tests
  +-- tool routing + policy
  +-- workflow execution loops
  +-- semantic completion

Live/runtime tests
  |
  +-- GUI automation
  +-- local model server lifecycle
  +-- provider outage/fallback
  +-- destructive workflows in VM
```

### Existing Test Anchors

| Area | Files |
| ---- | ----- |
| Provider registry/adapters | `llm/provider/tests.rs`, `llm/provider/connection_test.rs` |
| Payload shaping | `mcp/payload_shaper.rs` tests |
| Intent fallback | `agent/loop_engine/tests.rs` |
| VRAM budgeting | `llm/orchestrator/vram_budget.rs` tests |
| Strategy calculation | `llm/orchestrator/strategy.rs` tests |

### Why Declarative Tests Are Not Enough

A declarative test can prove that a tool schema exists. It cannot prove that:

- the correct backend was selected,
- context fit under model limits,
- a model produced a valid tool call,
- a GUI app focused correctly,
- an output was visible to the user,
- failover worked during an outage,
- the final answer was grounded in tool output.

KRIA needs operational cognition evals that observe complete runtime chains.

---

## 18. Real Production Failure Analysis

This section names real classes of failures visible from the architecture and existing
test/report context. The goal is to make failure modes studyable, not to hide them.

### Failure: Wrong Substrate Selection

```text
User wants app/browser action
  |
  v
LLM selects search/info tool or GUI path incorrectly
  |
  v
Runtime interception may redirect browser_search cases
  |
  +-- if redirect exists -> corrected
  +-- if no redirect     -> workflow may appear active but not satisfy user intent
```

Root cause:

- model prior can override prompt rules,
- app-open requests and information-search requests are semantically close,
- routed catalog may still contain multiple plausible tools.

Mitigation:

- TurnGate fallback hints,
- forced tool directives,
- GUI-launch interception in `AgentLoop`,
- execution-mode tags and GUI-last enforcement.

### Failure: Provider Fallback Failure

```text
Primary backend unavailable
  |
  v
FailoverRouter only helps if fallback provider is configured
  |
  +-- fallback configured -> route fallback
  +-- no fallback         -> no backend / LLM unavailable
```

Root cause:

- failover is optional,
- fallback provider may not support required capabilities,
- vision fallback requires a fallback backend with vision.

Mitigation:

- capability-aware health checks,
- explicit fallback provider configuration,
- UI-visible degraded mode,
- local preanalysis fallback for image workflows where available.

### Failure: Hidden Output Workflow

```text
Tool/app command succeeds
  |
  v
Output appears in hidden terminal/panel or not captured
  |
  v
Naive success would say "shown"
  |
  v
Semantic verifier should fail or mark incomplete
```

Root cause:

- process success is not user-visible success,
- GUI state and semantic output diverge,
- code execution may produce output outside observable substrate.

Mitigation:

- `ResultSynthesizer` uses actual tool output,
- observable completion checks human-visible outcomes,
- output-specific verifiers such as OutputContains-style checks.

### Failure: Hallucinated Completion

```text
Tool error
  |
  v
Model receives noisy/error payload
  |
  v
Model says success anyway
```

Root cause:

- LLMs may smooth over failures,
- raw payloads can obscure error fields,
- prompts alone are insufficient.

Mitigation:

- explicit `TOOL_ERROR` messages,
- result synthesizer grounded in tool result,
- deterministic replacement for non-grounded Gmail summaries,
- verifier authority.

### Failure: Context Contamination

```text
Large tool result / memory retrieval
  |
  v
Context pressure rises
  |
  v
Important current facts can be diluted by old/noisy data
```

Root cause:

- unbounded memory and raw payloads compete with task facts,
- provider context windows vary,
- tool results can be enormous.

Mitigation:

- `shape_for_llm`,
- token ledgers,
- prompt section priorities,
- message trimming,
- operation-specific PSDG injection.

---

## 19. Current Runtime Maturity Assessment

| Subsystem | Maturity | Notes |
| --------- | -------- | ----- |
| `LlmBackend` abstraction | Strong | Clean interface for chat, stream, health, tools |
| Provider adapters | Medium-Strong | Good normalization; provider quirks remain |
| ModelRouter | Medium | Stable config routing; capability/cost-aware routing is not part of the current router |
| FailoverRouter | Medium-Strong | Explicit FSM; depends on configured fallback and health probes |
| Prompt compiler | Strong foundation | Typed sections are production-minded; legacy prompt still coexists |
| Token budgeting | Strong foundation | Deterministic ledger; exact tokenizer depends on provider availability |
| Local llama.cpp runtime | Advanced but fragile | Sophisticated VRAM/swap management; hardware-specific instability remains |
| Tool orchestration | Strong | Policy, isolation, fallback, shaping, progress events |
| GUI cognition integration | Medium | Necessary but fragile under focus/Wayland/app-specific behavior |
| Verification | Medium | Architecture exists; coverage must grow across substrates |
| Memory/context | Medium | Bounded patterns exist; contamination risk requires ongoing discipline |
| Eval coverage | Improving | Needs more full-chain operational evals |

### Highest-Risk Areas

| Risk | Why It Matters |
| ---- | -------------- |
| GUI/Wayland behavior | Focus/input limitations can break visible workflows |
| Provider capability mismatch | Model may lack tools/vision/streaming despite being configured |
| Context overflow | Long workflows and tool payloads can exceed local contexts |
| Hidden-output success | App opened or command ran but user goal not semantically complete |
| Optional failover | No fallback means no resilience during provider outages |
| Prompt legacy coexistence | Two prompt construction paths increase behavioral surface |

---

## 20. Future Runtime Roadmap

### Near-Term Hardening

```text
Capability-aware routing
  |
  +-- select model by tools/vision/context/latency/cost
  |
Provider health dashboard
  |
  +-- visible primary/fallback/degraded status
  |
Verification expansion
  |
  +-- consume verifier-authority and hybrid-sync metadata as hard live completion gates
```

### Medium-Term Evolution

| Area | Direction |
| ---- | --------- |
| Local cognition | stronger local models, better quantization profiles, faster swaps |
| Prompt runtime | retire legacy string prompt assembly in favor of typed compiler |
| Provider routing | cost/latency/capability-aware model selection |
| Workflow memory | stronger procedural learning with bounded retrieval |
| GUI cognition | live enforcement of semantic workflow contracts, browser/media verifier authority, and hybrid synchronization |
| Eval runtime | provider outage harnesses, GUI live evals, long-horizon workflows |

### Long-Term Architecture

```text
Bounded local-first cognition
  |
  +-- multimodal orchestration
  +-- voice runtime
  +-- remote cognition/fleet execution
  +-- distributed provider pool
  +-- procedural workflow learning
  +-- stronger semantic verifiers
```

What should remain stable:

- LLMs propose; runtime executes.
- Policy and verifier outrank model output.
- Context remains bounded.
- GUI remains a substrate, not the default answer.
- Human approval remains mandatory for dangerous ambiguity.

---

## 21. Source File Reference Index

| Subsystem | File | Key Functions / Types | Purpose |
| --------- | ---- | --------------------- | ------- |
| Agent loop | `crates/kria-core/src/agent/loop_engine/mod.rs` | `AgentLoop`, `StreamEvent`, `run_stream`-style turn loop, failure classifiers | Central orchestration spine for prompt turns, tool rounds, policy, events, synthesis |
| Turn admission | `crates/kria-core/src/agent/turn_context.rs` | `TurnAdmission`, `TurnAdmissionDecision` | Accepts, queues, cancels, or supersedes user turns |
| Turn planning | `crates/kria-core/src/agent/turn_gate.rs` | `TurnGate`, `Operation`, `ResourcePlan` | Classifies operation type and fallback tool hints |
| Rule intent compiler | `crates/kria-core/src/agent/intent_compiler_rule.rs` | rule compiler types | Fast deterministic intent classification |
| LLM intent compiler | `crates/kria-core/src/agent/intent_compiler_llm.rs` | `LlmIntentCompiler` | Bounded fallback for complex GUI/multi-step intents |
| Semantic workflow | `crates/kria-core/src/agent/semantic_workflow.rs` | `SemanticWorkflowFrame`, `WorkflowFidelityResolution` | GUI workflow expectation and fidelity metadata |
| Execution mode reasoner | `crates/kria-core/src/agent/execution_mode_reasoner.rs` | `ExecutionModeReasoner`, `ExecutionModeDecision` | Deterministic structural/visible/hybrid/HITL mode selection |
| Workflow contracts | `crates/kria-core/src/agent/workflow_intent_contract.rs` | `WorkflowIntentContractRegistry`, `ContractCheck` | Declarative GUI workflow requirements |
| Verifier authority | `crates/kria-core/src/agent/verifier_authority.rs` | `VerifierAuthorityEvaluator` | Evidence authority/freshness requirements |
| Browser/media governance | `crates/kria-core/src/agent/browser_media_governance.rs` | `BrowserMediaGovernanceEvaluator` | Session/private-state governance metadata |
| Execution gate | `crates/kria-core/src/agent/execution_gate.rs` | `ExecutionGate`, `ResumeGateOutcome` | Preflight, authority, policy, resource, and decision gating |
| LLM contract | `crates/kria-core/src/llm/mod.rs` | `LlmBackend`, `ChatMessage`, `LlmResponse`, `ToolSchema`, `trim_messages_for_context` | Provider-neutral model interface and message/tool types |
| Model router | `crates/kria-core/src/llm/model_router.rs` | `ModelRouter`, `RoutingMode`, `route`, `route_vision` | Selects local/cloud/vision backend |
| Failover | `crates/kria-core/src/llm/failover.rs` | `FailoverRouter`, `ProviderFsm`, `FailoverPolicy` | Deterministic provider health and fallback routing |
| Token budget | `crates/kria-core/src/llm/budget.rs` | `ContextBudgets`, `TurnTokenLedger`, `PressureLevel` | Context windows, per-turn token ledgers, pressure signals |
| Tokenizer | `crates/kria-core/src/llm/tokenize.rs` | `count_tokens` | Provider tokenizer integration/fallback token counting |
| Local backend | `crates/kria-core/src/llm/local.rs` | `LocalBackend` | llama.cpp HTTP backend, readiness, local chat/stream handling |
| Cloud backend | `crates/kria-core/src/llm/cloud.rs` | `CloudBackend` | Generic OpenAI-compatible cloud calls, retries, tool fallback |
| L1 runtime | `crates/kria-core/src/llm/orchestrator/runtime.rs` | `L1Runtime` | Narrow lifecycle contract for managed local inference service |
| Server manager | `crates/kria-core/src/llm/orchestrator/server_manager.rs` | `LlamaServerManager`, server states | llama-server process lifecycle, port discovery, swaps, cancellation |
| VRAM strategy | `crates/kria-core/src/llm/orchestrator/strategy.rs` | `calculate_target_params`, `DegradationLevel` | GPU layer/context/vision degradation planning |
| Vision VRAM budget | `crates/kria-core/src/llm/orchestrator/vram_budget.rs` | `calculate_safe_visual_tokens`, `preflight_vision_check` | Prevents image-analysis OOM through token caps |
| Provider config | `crates/kria-core/src/llm/provider/config.rs` | `ProviderType`, `ProviderConfig`, `ProvidersConfig` | Persistent provider settings and endpoint definitions |
| Provider registry | `crates/kria-core/src/llm/provider/registry.rs` | `ProviderRegistry`, `switch_provider`, `switch_model` | Creates, switches, tests, and health-checks providers |
| Provider capabilities | `crates/kria-core/src/llm/provider/capabilities.rs` | `ProviderCapability`, `ModelCapabilities` | Normalized capability model |
| Provider errors | `crates/kria-core/src/llm/provider/error.rs` | `ProviderError`, `ProviderErrorKind` | Normalized error classification |
| Provider streaming | `crates/kria-core/src/llm/provider/streaming.rs` | `UnifiedStream` | Provider-neutral stream abstraction |
| OpenAI adapter | `crates/kria-core/src/llm/provider/openai.rs` | `OpenAIBackend` | OpenAI-compatible chat, tools, streaming, model health |
| OpenRouter adapter | `crates/kria-core/src/llm/provider/openrouter.rs` | `OpenRouterBackend` | OpenRouter as OpenAI-compatible provider wrapper |
| Anthropic adapter | `crates/kria-core/src/llm/provider/anthropic.rs` | `AnthropicBackend` | Messages API conversion and normalized tool calls |
| Gemini adapter | `crates/kria-core/src/llm/provider/gemini.rs` | `GeminiBackend` | Gemini content/tool conversion |
| Ollama adapter | `crates/kria-core/src/llm/provider/ollama.rs` | `OllamaBackend` | Local Ollama OpenAI-compatible backend |
| Prompt compiler | `crates/kria-core/src/agent/prompt_compiler.rs` | `StructuredPrompt`, `PromptSection`, `assemble` | Deterministic, priority-based prompt assembly |
| Legacy prompt | `crates/kria-core/src/agent/prompts.rs` | `build_system_prompt`, `build_planning_prompt` | Main operating prompt and planning/summarization prompts |
| Synthesis prompt | `crates/kria-core/src/agent/synthesis_prompt.rs` | `build_synthesis_prompt` | Bounded post-tool response prompt |
| Result synthesis | `crates/kria-core/src/agent/result_synthesizer.rs` | `ResultSynthesizer`, `SynthesizedResult` | Converts raw tool results into grounded user-readable responses |
| Tool registry | `crates/kria-core/src/tools/registry.rs` | `ToolRegistry`, `ToolDef`, `ToolHandler` | Registers tools, schemas, handlers, and contexts |
| Tool preflight | `crates/kria-core/src/tools/preflight.rs` | preflight validators | Deterministic validation before tool execution |
| Payload shaping | `crates/kria-core/src/mcp/payload_shaper.rs` | `shape_for_llm`, `shape_value` | Compacts large tool/MCP payloads for LLM context |
| Capability registry | `crates/kria-core/src/mcp/capability_registry.rs` | `capability_profile`, `find_better_alternative` | Execution-mode metadata and GUI-last alternatives |
| Policy | `crates/kria-core/src/safety/policy.rs` | `PolicyEngine`, `RiskLevel` | Safety tiering and destructive action governance |
| HITL | `crates/kria-core/src/safety/hitl.rs` | `HitlGateway`, `ApprovalResponse` | User approval workflow |
| Audit | `crates/kria-core/src/safety/audit.rs` | `AuditLogger`, `Decision`, `DecidedBy` | Records policy decisions and tool actions |
| Isolation | `crates/kria-core/src/infra/isolation.rs` | `run_isolated`, `ToolResult` | Timeout/cancellation boundary for tool handlers |
| Pipeline trace | `crates/kria-core/src/infra/pipeline_trace.rs` | `log_pipeline_step` | Structured runtime trace logging |
| PSDG injector | `crates/kria-core/src/agent/psdg/context_injector.rs` | `build_context_block`, `inject_into_system_prompt` | Bounded desktop context injection |
| Desktop awareness | `crates/kria-core/src/agent/desktop_awareness/mod.rs` | `DesktopAwarenessRuntime` | Unified live desktop state |
| Workflow continuation | `crates/kria-core/src/agent/workflow_continuation/mod.rs` | `WorkflowContinuationRuntime` | Bounded interruption and continuation planning |
| Execution verifier | `crates/kria-core/src/agent/execution_verifier.rs` | `ExecutionVerifier` | Verification interface |
| Bounded verifier | `crates/kria-core/src/agent/execution_verifier_bounded.rs` | bounded verifier implementation | Fail-closed semantic/tool result validation |
| Observable completion | `crates/kria-core/src/agent/observable_completion/mod.rs` | `ObservableCompletionEngine` | Checks human-visible workflow completion |
| GUI wiring | `crates/kria-core/src/agent/gui_wiring.rs` | GUI runtime wiring helpers | Connects GUI cognition into core agent runtime |
| GUI planner | `crates/kria-core/src/agent/gui_planner.rs` | GUI planning types | GUI automation planning |
| GUI substrate planner | `crates/kria-core/src/agent/gui_substrate_planner.rs` | substrate planner types | Selects GUI interaction substrate |
| Browser cognition | `crates/kria-core/src/agent/browser_cognition.rs` | browser cognition types | Browser-oriented semantic automation |
| IDE cognition | `crates/kria-core/src/agent/ide_cognition.rs` | IDE cognition types | IDE/workspace-aware operations |
| Goal tree | `crates/kria-core/src/agent/goal_tree.rs` | goal tree types | Hierarchical task decomposition |
| Workflow compiler | `crates/kria-core/src/agent/workflow_compiler.rs` | workflow compiler types | Converts user workflows into executable structure |

---

## Closing Model

The shortest accurate mental model is:

```text
KRIA uses LLMs for bounded cognition,
not for unchecked authority.

The orchestrator owns:
  context,
  routing,
  provider selection,
  tool visibility,
  policy,
  execution,
  recovery,
  verification,
  and final grounding.

The model contributes:
  language understanding,
  planning suggestions,
  code/text generation,
  synthesis,
  and structured tool-call proposals.
```

That separation is the essence of KRIA's Core Orchestrator.

---

## Vision Gap Analysis: Orchestrator, Intelligence, And Data Flow

This section compares the current orchestrator implementation against KRIA's intended
"reliable operational coworker" vision. It is deliberately direct: the system has strong
building blocks, but the current orchestration path still contains several gaps that
prevent KRIA from consistently feeling like a true desktop cognition runtime.

### Current Fit Against The Vision

| Vision Expectation | Current Fit | Evidence In Current Code |
| ------------------ | ----------- | ------------------------ |
| Local-first cognition runtime | Partial to strong | `LocalBackend`, `LlamaServerManager`, VRAM strategy, provider fallback |
| True workflow coworker | Partial | `AgentLoop`, `GoalTree`, workflow continuation exist, but many turns still behave as tool-call loops |
| Dynamic intelligence with minimal hardcoding | Partial | TurnGate/fallback/routing include many deterministic rules and special cases |
| Multi-provider cognition | Strong foundation | `LlmBackend`, `ProviderRegistry`, provider adapters |
| Tool execution safety | Strong | `PolicyEngine`, HITL, audit, isolation |
| Semantic completion | Partial | verifiers/synthesizer exist, but completion coverage is uneven |
| External system cognition via MCP | Fragile | latest report shows failing MCP prompt-output tests |
| Long-horizon continuity | Early foundation | `WorkflowSession`, `PersistentGoalRuntime`, procedural memory, PSDG |

### Highest-Impact Current Issues

| Issue | Why It Blocks The Vision | Current Signal | Implementation Direction | Expected Impact |
| ----- | ------------------------ | -------------- | ------------------------ | --------------- |
| MCP prompt-output loop is failing | External systems are central to Jira/MCP/API coworker workflows | `mcp_prompt_output_integration_tests` missing `ToolEnd` for MCP tools | Fix MCP tool invocation event lifecycle so every MCP call emits `ToolStart`, `ToolEnd`, shaped payload, and error event deterministically | External systems become debuggable and reliable instead of silently disappearing |
| Routing collision in smoke test | A coworker must choose the right operational path | Docker service status expected `execute_bash`, got `manage_service` | Decide canonical route for service status; update router expectation or tool behavior so inspect vs manage is explicit | Less surprise in system-management workflows |
| Orchestrator remains too ReAct-loop-centric | A coworker should execute explicit workflows, not just ask a model for next tool calls | `AgentLoop` is the main spine; `GoalTree`/OpGraph exist but are not always primary | Promote workflow graph execution for multi-step tasks; use model for planning, not per-step improvisation | More stable long workflows and fewer hallucinated or missing tool calls |
| Capability-aware routing is incomplete | Local/cloud/model choice should match task needs | `ModelRouter` is mostly config mode + vision branch | Add task capability profile: context size, tools, vision, code, latency, privacy, cost, local VRAM pressure | Better model choices on RTX 4050-class hardware |
| Context/data flow can still become noisy | A coworker must focus on the relevant state | prompt compiler, payload shaper, PSDG injector exist but not unified as one evidence ledger | Create a per-turn `EvidencePack` containing user goal, current state, retrieved facts, tool results, and verifier outputs with priority and provenance | Lower hallucination, cleaner final synthesis |
| Provider fallback is not full capability fallback | Fallback provider may not support same tools/vision/context | `FailoverRouter` tracks health, but capability equivalence is limited | Fallback chains should be capability-filtered, not just provider configured | Fewer degraded-mode surprises |
| Streaming does not equal workflow transparency | User needs coworker-level progress explanation | `StreamEvent` exists, but higher-level narrative depends on paths | Attach every plan/tool/verifier/recovery event to a workflow trace ID and expose "what happened / why / next" | More trust and easier debugging |

### Required Orchestrator Data Flow Upgrade

Current shape:

```text
messages + tool schemas
  -> LLM
  -> parsed tool calls
  -> tool execution
  -> tool result messages
  -> final response
```

Recommended target shape:

```text
UserGoal
  |
  v
TurnFrame
  |-- intent classification
  |-- operation type
  |-- risk/destructive modality
  |-- environment snapshot
  |-- available substrates
  |-- active workflow/goal
  |
  v
EvidencePack
  |-- prompt context
  |-- routed tools
  |-- memory facts with provenance
  |-- previous tool results
  |-- verifier checkpoints
  |
  v
ExecutionGraph
  |-- stages
  |-- tool calls
  |-- verification criteria
  |-- recovery branches
  |
  v
Bounded executor
```

Implementation steps:

1. Add a `TurnFrame` type near `agent/loop_engine` or `agent/turn_context`.
2. Move scattered state such as operation, modality, routed tools, PSDG block, direct hints,
   and expectation category into this frame.
3. Add an `EvidencePack` builder that emits both:
   - compact LLM context,
   - structured runtime evidence for verifiers/synthesizer.
4. Make the final response read from verified evidence first, model text second.
5. Require `ToolEnd` or explicit `ToolSkipped/ToolFailed` for every attempted tool.

### Orchestrator Implementation Priorities

| Priority | Change | Files To Start | Impact |
| -------- | ------ | -------------- | ------ |
| P0 | Fix missing MCP `ToolEnd` lifecycle | `agent/loop_engine/mod.rs`, `mcp/tool_bridge.rs`, `mcp/client.rs`, `tests/mcp_prompt_output_integration_tests.rs` | Restores trust in MCP/external system workflows |
| P0 | Resolve service-status routing mismatch | `agent/router.rs`, `routing/*`, `tools/system_config.rs`, `tests/test_smoke_system.rs` | Cleans routing semantics for system operations |
| P1 | Introduce `TurnFrame` / `EvidencePack` | `agent/loop_engine/mod.rs`, `agent/turn_context.rs`, `mcp/payload_shaper.rs` | Makes data flow understandable and testable |
| P1 | Promote graph execution for multi-step tasks | `agent/goal_tree.rs`, `agent/opgraph.rs`, `agent/workflow_compiler.rs`, `agent/stage_executor.rs` | Reduces fragile LLM step-by-step improvisation |
| P1 | Capability-aware model routing | `llm/model_router.rs`, `llm/provider/capabilities.rs`, `llm/failover.rs` | Better local/cloud routing and degraded-mode behavior |
| P2 | Provider capability fallback contracts | `llm/provider/registry.rs`, `llm/failover.rs` | Avoids selecting fallback models that cannot perform task |
| P2 | Trace-linked streaming | `agent/execution_transparency/mod.rs`, `agent/loop_engine/mod.rs` | Better user-facing operational transparency |

### What Success Should Look Like

For a prompt like "summarize latest Jira tickets and create implementation plan":

```text
KRIA should:
  1. identify this as external-system + planning workflow,
  2. check whether Jira/MCP connector is mounted,
  3. retrieve tickets through API/MCP rather than browser if possible,
  4. shape ticket payloads for the LLM,
  5. cite exact tickets used as evidence,
  6. produce an implementation plan,
  7. explain missing connector/auth clearly if blocked.
```

The current architecture can support this, but the MCP event lifecycle and evidence flow
must be hardened first.
