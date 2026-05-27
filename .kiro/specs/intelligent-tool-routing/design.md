# Intelligent Tool Routing — Design

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    User Intent                              │
└────────────────────────┬────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│              Intent Classification Engine                   │
│  (Classify: LOCAL_QUERY | EXTERNAL_QUERY | EXECUTION_REQ)  │
└────────────────────────┬────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│           Execution Context Resolver                        │
│  (Resolve: substrate, capabilities, availability)          │
└────────────────────────┬────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│         Capability-Aware Tool Router                        │
│  (Select: tool + substrate + fallback strategy)            │
└────────────────────────┬────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│          Tool Invocation Validator                          │
│  (Validate: schema, capabilities, availability)            │
└────────────────────────┬────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│              Tool Execution                                 │
│  (Execute on selected substrate with fallback ready)       │
└────────────────────────┬────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│          Verifier Feedback Integration                      │
│  (Validate output, update routing feedback, log decision)   │
└─────────────────────────────────────────────────────────────┘
```

## Component Design

### 1. Intent Classification Engine

**Purpose:** Classify user intent into semantic categories to guide routing decisions.

**Input:** User prompt + execution context

**Output:** Intent classification + confidence + reasoning

**Classification Categories:**

```rust
enum IntentCategory {
    LocalQuery,           // Answerable from local context
    ExternalQuery,        // Requires external knowledge
    ExecutionRequest,     // User asking for action
    CapabilityQuestion,   // Asking about system capabilities
    ConversationalMeta,   // Greetings, meta-interaction
    FreshnessRequired,    // Explicitly needs current data
}
```

**Logic:**

```
IF prompt matches conversational patterns (greetings, meta)
  → CONVERSATIONAL_META
ELSE IF prompt asks about capabilities ("can you", "do you support")
  → CAPABILITY_QUESTION
ELSE IF prompt has temporal markers ("today", "latest", "current")
  → FRESHNESS_REQUIRED
ELSE IF prompt is imperative/task-oriented ("do X", "help me")
  → EXECUTION_REQUEST
ELSE IF prompt references local data ("my files", "my documents")
  → LOCAL_QUERY
ELSE IF prompt is factual/research-oriented ("find info about", "research")
  → EXTERNAL_QUERY
ELSE
  → EXECUTION_REQUEST (default)
```

**Determinism:** Same prompt → same classification (no randomness)

**Latency:** <10ms (simple pattern matching + keyword detection)

---

### 2. Execution Context Resolver

**Purpose:** Resolve available substrates, capabilities, and resource constraints.

**Input:** User intent + system state

**Output:** Available substrates + capabilities + constraints

**Substrate Types:**

```rust
enum Substrate {
    LocalHost,           // User's machine
    RemoteSSH,           // Remote SSH target
    DockerContainer,     // Docker container
    VMGuest,             // Virtual machine
}

struct SubstrateCapabilities {
    can_execute_shell: bool,
    has_network: bool,
    has_filesystem: bool,
    has_gpu: bool,
    has_gui: bool,
    is_isolated: bool,
}
```

**Resolution Logic:**

```
FOR each available substrate:
  1. Check if substrate is online/available
  2. Resolve capabilities (shell, network, filesystem, GPU, GUI)
  3. Check resource constraints (CPU, memory, disk)
  4. Store in available_substrates list

RETURN available_substrates sorted by:
  1. Local-first (LOCAL_HOST preferred)
  2. Capability match (substrates supporting required capabilities)
  3. Resource availability (substrates with sufficient resources)
```

**Determinism:** Same system state → same substrate list (deterministic ordering)

**Latency:** <20ms (query system state, resolve capabilities)

---

### 3. Capability-Aware Tool Router

**Purpose:** Select tool + substrate + fallback strategy based on intent and capabilities.

**Input:** Intent classification + execution context + tool registry

**Output:** Routing decision (tool + substrate + fallback)

**Routing Decision Structure:**

```rust
struct RoutingDecision {
    primary_tool: ToolName,
    primary_substrate: Substrate,
    fallback_tool: Option<ToolName>,
    fallback_substrate: Option<Substrate>,
    reasoning: String,
    local_first_applied: bool,
}
```

**Routing Logic:**

```
MATCH intent_category:

  CONVERSATIONAL_META:
    → DENY retrieval
    → Use model knowledge + local context
    → Fallback: None (no external call needed)

  CAPABILITY_QUESTION:
    → DENY retrieval
    → Use tool registry + capability manifest
    → Fallback: None (no external call needed)

  LOCAL_QUERY:
    → Route to local tools (file_read, rag_search, memory_query)
    → Substrate: LOCAL_HOST
    → Fallback: Try alternative local tool

  FRESHNESS_REQUIRED:
    → Route to web_search or news_search
    → Substrate: LOCAL_HOST (network available)
    → Fallback: Use local knowledge if retrieval fails

  EXTERNAL_QUERY:
    → Route to web_search
    → Substrate: LOCAL_HOST (network available)
    → Fallback: Use local knowledge if retrieval fails

  EXECUTION_REQUEST:
    → Evaluate if execution requires external data
    → IF external data needed: route to web_search first, then execute
    → IF local data sufficient: route to execution tool directly
    → Substrate: Resolve based on execution target (host/remote/docker/vm)
    → Fallback: Try alternative substrate or local execution

FOR each routing decision:
  1. Validate tool capabilities ⊆ substrate capabilities
  2. Check tool availability on substrate
  3. Validate tool schema
  4. Select fallback tool/substrate
  5. Log routing decision with reasoning
```

**Local-First Policy:**

```
IF query is answerable from local context (files, memory, knowledge base):
  → Skip external retrieval
  → Use local tools only
  → Log: "Query answerable locally; skipping external retrieval"

IF query requires freshness (temporal markers):
  → Allow external retrieval
  → Log: "Freshness required; allowing external retrieval"

IF query is about external entities/events:
  → Allow external retrieval
  → Log: "External knowledge required; allowing external retrieval"

DEFAULT:
  → Prefer local-first
  → Only escalate to external if local attempt fails
```

**Determinism:** Same intent + context → same routing decision

**Latency:** <30ms (decision tree traversal + capability matching)

---

### 4. Tool Invocation Validator

**Purpose:** Validate tool invocations before execution to prevent misuse.

**Input:** Routing decision + tool parameters

**Output:** Validation result (PASS | FAIL with reason)

**Validation Checks:**

```
1. Tool Existence:
   → Check if tool exists in tool registry
   → FAIL if tool not found

2. Capability Match:
   → Check if tool_capabilities ⊆ substrate_capabilities
   → FAIL if substrate doesn't support tool

3. Schema Validation:
   → Check if parameters match tool schema
   → FAIL if schema mismatch

4. Availability Check:
   → Check if tool is available on substrate
   → FAIL if tool is disabled/unavailable

5. MCP Validation (if MCP tool):
   → Check if MCP server is running
   → Check if tool is registered with server
   → Check if payload is compatible
   → FAIL if MCP server offline or tool not registered

6. Safety Policy Check:
   → Check if tool invocation passes safety policy
   → FAIL if tool is blacklisted or requires HITL approval

RETURN: PASS or FAIL with detailed reason
```

**Latency:** <20ms (schema validation + capability checking)

---

### 5. Verifier Feedback Integration

**Purpose:** Integrate verifier feedback into routing decisions to improve accuracy over time.

**Input:** Tool execution result + verifier feedback

**Output:** Updated routing feedback + decision log

**Feedback Structure:**

```rust
struct VerifierFeedback {
    tool_name: ToolName,
    substrate: Substrate,
    execution_success: bool,
    output_correctness: bool,
    reason: String,
    timestamp: DateTime,
}

struct RoutingFeedback {
    decision_id: UUID,
    tool_name: ToolName,
    substrate: Substrate,
    success_rate: f32,  // 0.0-1.0
    failure_count: u32,
    last_failure: Option<DateTime>,
}
```

**Feedback Logic:**

```
AFTER tool execution:
  1. Verifier evaluates output correctness
  2. Store feedback: (tool, substrate, success/failure, reason)
  3. Update routing feedback: success_rate, failure_count
  4. IF tool fails on substrate: mark combination as risky
  5. IF tool succeeds: reinforce routing decision

DURING routing:
  1. Check routing feedback for tool/substrate combination
  2. IF success_rate < 50%: prefer alternative tool/substrate
  3. IF success_rate > 90%: reinforce routing decision
  4. IF no feedback available: use default routing logic

BOUNDED LEARNING:
  → Max 100 feedback entries per tool/substrate combination
  → Feedback expires after 7 days (recalibrate)
  → No unbounded learning or AGI-style adaptation
```

**Determinism:** Feedback-aware routing is deterministic (same feedback → same decision)

---

## Data Flow

### Scenario 1: Local Query (No External Retrieval)

```
User: "Summarize my project files"
  ↓
Intent Classification: LOCAL_QUERY
  ↓
Execution Context: LOCAL_HOST available, filesystem accessible
  ↓
Tool Router: Route to file_read + rag_search
  ↓
Validator: PASS (tools available on LOCAL_HOST)
  ↓
Execution: Read files locally, generate summary
  ↓
Verifier: Output correct, execution successful
  ↓
Log: "Local query executed successfully; no external retrieval needed"
```

### Scenario 2: Freshness-Required Query (External Retrieval)

```
User: "What's the latest news on AI?"
  ↓
Intent Classification: FRESHNESS_REQUIRED (temporal marker "latest")
  ↓
Execution Context: LOCAL_HOST available, network enabled
  ↓
Tool Router: Route to web_search (freshness required)
  ↓
Validator: PASS (web_search available on LOCAL_HOST)
  ↓
Execution: Retrieve latest news via web_search
  ↓
Verifier: Output current, execution successful
  ↓
Log: "Freshness required; web_search executed successfully"
```

### Scenario 3: Execution Request with Substrate Ambiguity

```
User: "Run this script"
  ↓
Intent Classification: EXECUTION_REQUEST
  ↓
Execution Context: LOCAL_HOST available, REMOTE_SSH available
  ↓
Tool Router: Ambiguous substrate; ask clarification
  ↓
Clarification: "Run on local host or remote SSH?"
  ↓
User Response: "Remote SSH"
  ↓
Tool Router: Route to shell_exec on REMOTE_SSH
  ↓
Validator: PASS (shell_exec available on REMOTE_SSH)
  ↓
Execution: Execute script on remote SSH
  ↓
Verifier: Script executed, output captured
  ↓
Log: "Execution request routed to REMOTE_SSH; script executed successfully"
```

### Scenario 4: Tool Failure with Fallback

```
User: "Find information about quantum computing"
  ↓
Intent Classification: EXTERNAL_QUERY
  ↓
Execution Context: LOCAL_HOST available, network enabled
  ↓
Tool Router: Route to web_search (primary), fallback to local knowledge
  ↓
Validator: PASS (web_search available)
  ↓
Execution: web_search invoked
  ↓
Execution Fails: Network timeout
  ↓
Fallback: Use local knowledge base
  ↓
Verifier: Output from local knowledge, execution successful
  ↓
Log: "web_search failed (network timeout); fallback to local knowledge executed successfully"
```

---

## Implementation Strategy

### Phase 1: Intent Classification Engine
- Implement intent classification logic (decision tree)
- Add pattern matching for conversational/meta prompts
- Add temporal marker detection for freshness-required queries
- Test with 100+ sample prompts

### Phase 2: Execution Context Resolver
- Implement substrate detection (host, remote SSH, Docker, VM)
- Implement capability resolution (shell, network, filesystem, GPU, GUI)
- Add resource constraint checking
- Test with various system configurations

### Phase 3: Capability-Aware Tool Router
- Implement routing decision logic (decision tree)
- Implement local-first policy enforcement
- Implement fallback strategy selection
- Add routing decision logging

### Phase 4: Tool Invocation Validator
- Implement schema validation
- Implement capability matching
- Implement MCP validation
- Add validation error reporting

### Phase 5: Verifier Feedback Integration
- Implement feedback storage (SQLite)
- Implement feedback-aware routing
- Add bounded learning logic
- Test feedback loop with multiple executions

### Phase 6: Integration & Testing
- Integrate all components into agent loop
- Add end-to-end tests
- Add performance benchmarks
- Add audit logging

---

## Key Design Decisions

1. **Decision Trees Over Neural Networks:** Routing logic uses deterministic decision trees, not learned models, to ensure predictability and auditability.

2. **Local-First Default:** Queries answerable locally never trigger external retrieval; external calls are explicit and justified.

3. **Bounded Learning:** Verifier feedback is bounded (max 100 entries per tool/substrate, 7-day expiry) to prevent unbounded adaptation.

4. **Fast Routing:** Routing decisions complete in <100ms to support real-time execution.

5. **Deterministic Ordering:** Substrate and tool selection is deterministic (same input → same output) for reproducibility.

6. **Fallback-Ready:** Every routing decision includes fallback strategy to enable graceful recovery.

7. **Capability-First:** Tools can only run on substrates that support their capabilities; mismatches are caught early.

8. **Auditable:** All routing decisions are logged with reasoning for transparency and debugging.

---

## Correctness Properties

1. **Local-First Preservation:** Queries answerable locally never trigger external retrieval
2. **Capability Safety:** Tools never run on substrates that don't support them
3. **Deterministic Routing:** Same intent + context → same routing decision
4. **Fallback Availability:** Every routing decision has fallback strategy
5. **Bounded Cognition:** Routing logic is free of unbounded reasoning
6. **Verifier Integration:** Feedback improves routing without unbounded learning
7. **Substrate Clarity:** Substrate selection is explicit and justified
8. **Tool Validation:** All tool invocations are validated before execution

---

## Edge Cases & Handling

| Edge Case | Handling |
|-----------|----------|
| All substrates unavailable | Report unavailability with recovery strategy |
| Tool not available on any substrate | Suggest alternative tool or manual execution |
| Ambiguous execution target | Ask clarifying question; default to LOCAL_HOST |
| Network unavailable for web_search | Fallback to local knowledge base |
| MCP server offline | Report unavailability; suggest alternative tool |
| Tool schema validation fails | Reject invocation with error message |
| Verifier feedback contradicts routing | Log discrepancy; investigate routing logic |
| Routing decision timeout (>100ms) | Use default routing logic; log timeout |

---

## Performance Targets

| Component | Target Latency |
|-----------|----------------|
| Intent Classification | <10ms |
| Execution Context Resolution | <20ms |
| Tool Routing | <30ms |
| Tool Validation | <20ms |
| Total Routing Decision | <100ms |

---

## Monitoring & Observability

**Metrics to Track:**

1. Routing decision latency (p50, p95, p99)
2. Tool invocation success rate by tool/substrate
3. Fallback invocation frequency
4. Local-first policy adherence (% of queries using local tools)
5. Substrate distribution (% of executions on host/remote/docker/vm)
6. Verifier feedback accuracy (% of feedback that improves routing)

**Logging:**

- Every routing decision logged with: intent, context, selected tool, substrate, reasoning, fallback
- Every tool invocation logged with: tool name, substrate, parameters, result, execution time
- Every fallback logged with: reason, fallback tool/substrate, result
- Every verifier feedback logged with: tool, substrate, success/failure, reason

---

## Testing Strategy

**Unit Tests:**
- Intent classification (100+ sample prompts)
- Execution context resolution (various system configurations)
- Capability matching (tool/substrate combinations)
- Routing decision logic (decision tree coverage)
- Fallback strategy selection

**Integration Tests:**
- End-to-end routing (intent → execution)
- Fallback execution (primary failure → fallback success)
- Verifier feedback loop (execution → feedback → improved routing)
- Substrate switching (local → remote → docker)

**Property-Based Tests:**
- Determinism: Same input → same output
- Local-first: Local queries never trigger external retrieval
- Capability safety: Tools never run on unsupported substrates
- Fallback availability: Every decision has fallback
- Bounded cognition: Routing completes in <100ms

**Performance Tests:**
- Routing latency benchmarks
- Substrate resolution performance
- Capability matching performance
- Verifier feedback storage/retrieval performance
