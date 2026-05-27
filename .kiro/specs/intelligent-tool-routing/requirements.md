# Intelligent Tool Routing — Requirements

## Introduction

KRIA's execution cognition is bounded by its ability to route user intents to the correct tool, substrate, and execution context. Current tool selection is reactive and lacks semantic awareness of execution constraints, substrate capabilities, and fallback reasoning. This results in:

- Unnecessary web/news retrieval for local-answerable queries
- Substrate confusion (host vs. VM vs. Docker vs. remote)
- Wrong tool selection for execution targets
- Fake browser incapability responses instead of graceful fallback
- Unnecessary GUI automation when APIs exist
- MCP/tool misuse (calling tools outside their capability bounds)
- Poor execution fallback reasoning (no recovery strategy)

**Goal:** Implement deterministic, capability-aware tool routing that preserves KRIA's authoritative orchestration principles while improving execution cognition and substrate-aware decision-making.

## Glossary

| Term | Definition |
|------|-----------|
| **Tool** | Executable capability (web_search, file_read, shell_exec, etc.) |
| **Substrate** | Execution environment (local host, remote SSH, Docker container, VM) |
| **Execution Context** | Combination of substrate, capability availability, and resource constraints |
| **Capability** | Declared ability of a tool/substrate (e.g., "can_execute_shell", "has_network") |
| **Routing Decision** | Selection of tool + substrate + fallback strategy for a user intent |
| **Fallback Strategy** | Alternative execution path when primary tool/substrate is unavailable |
| **Verifier** | Component that validates tool output and execution success |
| **Orchestration Authority** | KRIA's responsibility to make deterministic, bounded execution decisions |

## Requirements

### Requirement 1: Capability-Aware Tool Selection

**User Story:** As KRIA's orchestrator, I want to select tools based on declared capabilities and execution context, so that I avoid calling tools outside their bounds and provide accurate fallback reasoning.

#### Acceptance Criteria

1. Each tool declares its capability requirements (network, filesystem, shell, GPU, etc.)
2. Each substrate declares its available capabilities (host has shell, Docker has network isolation, etc.)
3. Tool routing evaluates: `tool_capabilities ⊆ substrate_capabilities` before invocation
4. If tool cannot run on substrate, routing immediately suggests alternative substrate or fallback
5. Capability mismatch is logged with reason (e.g., "web_search requires network; Docker container has network=false")
6. No tool is invoked on a substrate that cannot support it

---

### Requirement 2: Substrate-Aware Execution Resolution

**User Story:** As KRIA's orchestrator, I want to resolve execution targets (host vs. VM vs. Docker vs. remote SSH) based on user intent and resource availability, so that I avoid substrate confusion and select the optimal execution environment.

#### Acceptance Criteria

1. User intent is classified into execution target category: LOCAL_HOST | REMOTE_SSH | DOCKER_CONTAINER | VM_GUEST
2. Routing evaluates available substrates and their capabilities
3. If user intent is ambiguous (e.g., "run this script"), routing asks clarifying question or defaults to LOCAL_HOST (local-first principle)
4. If requested substrate is unavailable, routing suggests next-best substrate with reasoning
5. Substrate selection is deterministic: same intent → same substrate choice (unless availability changes)
6. Substrate unavailability is reported with recovery strategy (e.g., "Remote SSH unavailable; falling back to local execution")

---

### Requirement 3: Local-First Execution Policy

**User Story:** As KRIA's orchestrator, I want to enforce local-first execution by default, so that I preserve privacy, reduce latency, and avoid unnecessary external calls.

#### Acceptance Criteria

1. Queries answerable from local context (files, memory, knowledge base) never trigger external retrieval
2. Web search is only invoked when: (a) freshness required, (b) external knowledge needed, (c) local context insufficient
3. GUI automation is only invoked when: (a) no API exists, (b) user explicitly requests GUI interaction
4. Remote execution is only invoked when: (a) local execution impossible, (b) user explicitly requests remote, (c) resource constraints require remote
5. Local-first decisions are logged with reasoning (e.g., "Query answerable from local RAG; skipping web search")
6. Fallback to external/remote is only triggered after local attempt fails

---

### Requirement 4: Deterministic Tool Orchestration

**User Story:** As KRIA's orchestrator, I want tool routing decisions to be deterministic and reproducible, so that execution behavior is predictable and auditable.

#### Acceptance Criteria

1. Same user intent + same execution context → same tool routing decision (deterministic)
2. Tool routing decisions are logged with: intent, context, selected tool, substrate, reasoning, fallback strategy
3. Routing logic is free of randomness, non-deterministic ordering, or probabilistic selection
4. Routing decisions can be replayed/audited from logs
5. Routing logic is bounded: no infinite loops, no unbounded search, no AGI-style reasoning
6. Routing decision time is <100ms (fast enough for real-time execution)

---

### Requirement 5: Fake Capability Prevention

**User Story:** As KRIA's orchestrator, I want to avoid fake capability responses (e.g., "I can't use a browser" when browser tools exist), so that I provide accurate fallback reasoning and preserve user trust.

#### Acceptance Criteria

1. If a tool is unavailable, routing provides reason: "web_search unavailable: network disabled in config"
2. If a tool is available but substrate doesn't support it, routing suggests alternative: "Browser automation unavailable on Docker; use API instead"
3. If a tool fails, routing provides recovery strategy: "web_search failed; falling back to local knowledge base"
4. No tool is reported as "not supported" without offering alternative execution path
5. Capability limitations are transparent: user understands why a tool can't run and what alternatives exist
6. Fallback reasoning is accurate and actionable

---

### Requirement 6: MCP/Tool Misuse Prevention

**User Story:** As KRIA's orchestrator, I want to prevent MCP and tool misuse by validating tool invocations against their schemas and capability bounds, so that I avoid errors and improve execution reliability.

#### Acceptance Criteria

1. Before invoking any tool, routing validates: (a) tool exists, (b) parameters match schema, (c) tool is available on substrate
2. If tool schema validation fails, routing rejects invocation with error message
3. If tool is called outside its capability bounds (e.g., web_search on offline substrate), routing rejects with reason
4. MCP tools are validated against server lifecycle: (a) server is running, (b) tool is registered, (c) payload is compatible
5. Tool misuse is logged with: tool name, reason for rejection, suggested alternative
6. No tool is invoked if validation fails

---

### Requirement 7: Execution Fallback Reasoning

**User Story:** As KRIA's orchestrator, I want to provide clear fallback reasoning when primary execution path fails, so that I can recover gracefully and maintain execution continuity.

#### Acceptance Criteria

1. Each tool routing decision includes fallback strategy: PRIMARY_TOOL | FALLBACK_TOOL | FALLBACK_SUBSTRATE | LOCAL_KNOWLEDGE
2. If primary tool fails, routing automatically attempts fallback without user intervention
3. Fallback reasoning is logged: "Primary: web_search failed (network timeout); Fallback: using local knowledge base"
4. Fallback chain is bounded: max 2-3 fallback attempts before reporting failure to user
5. Fallback strategy is deterministic: same failure → same fallback attempt
6. User is informed of fallback: "Using local knowledge base instead of web search"

---

### Requirement 8: Verifier-Aware Execution

**User Story:** As KRIA's orchestrator, I want to integrate verifier feedback into tool routing decisions, so that I can improve routing accuracy over time and avoid repeated failures.

#### Acceptance Criteria

1. After tool execution, verifier evaluates: (a) output correctness, (b) execution success, (c) substrate appropriateness
2. Verifier feedback is stored: tool_name, substrate, success/failure, reason
3. Routing uses verifier feedback to adjust future decisions: if tool X fails on substrate Y, avoid that combination
4. Verifier-aware routing is bounded: no unbounded learning, no AGI-style adaptation
5. Verifier feedback is logged and auditable
6. Routing can be reset/recalibrated if verifier feedback becomes stale

---

### Requirement 9: Unnecessary Web/News Retrieval Prevention

**User Story:** As KRIA's orchestrator, I want to prevent unnecessary web and news retrieval by evaluating query intent and local context sufficiency, so that I reduce substrate waste and preserve local-first principles.

#### Acceptance Criteria

1. Before invoking web_search or news_search, routing evaluates: (a) is query answerable locally?, (b) is freshness required?, (c) is external knowledge needed?
2. If query is answerable locally (files, memory, knowledge base), web_search is skipped
3. If query is conversational/meta (capability questions, greetings), web_search is skipped
4. If query requires freshness (temporal markers: "today", "latest", "current"), web_search is allowed
5. If query is about external entities/events, web_search is allowed
6. Retrieval prevention is logged: "Query answerable from local RAG; skipping web_search"

---

### Requirement 10: GUI Automation Avoidance

**User Story:** As KRIA's orchestrator, I want to avoid unnecessary GUI automation by preferring APIs and direct tool invocation, so that I improve execution speed and reliability.

#### Acceptance Criteria

1. Before invoking GUI automation tools, routing checks: (a) does API exist for this task?, (b) can direct tool accomplish this?
2. If API exists, routing uses API instead of GUI automation
3. If direct tool exists, routing uses direct tool instead of GUI automation
4. GUI automation is only invoked when: (a) no API exists, (b) no direct tool exists, (c) user explicitly requests GUI
5. GUI automation fallback is logged: "No API available; falling back to GUI automation"
6. GUI automation is marked as lower-priority execution path

---

### Requirement 11: Execution Target Clarification

**User Story:** As KRIA's orchestrator, I want to clarify ambiguous execution targets when user intent is unclear, so that I avoid substrate confusion and make deterministic routing decisions.

#### Acceptance Criteria

1. If user intent is ambiguous (e.g., "run this script"), routing asks clarifying question: "Run on local host or remote SSH?"
2. If user provides explicit target (e.g., "run on Docker"), routing uses that target
3. If user provides no target, routing defaults to LOCAL_HOST (local-first principle)
4. Clarification questions are concise and actionable
5. User response is stored in execution context for future decisions
6. Clarification is logged: "User clarified: execute on remote SSH"

---

### Requirement 12: Bounded Orchestration Cognition

**User Story:** As KRIA's orchestrator, I want to maintain bounded, deterministic cognition without AGI-style reasoning, so that execution behavior is predictable, auditable, and production-grade.

#### Acceptance Criteria

1. Tool routing logic is free of unbounded search, probabilistic selection, or open-ended reasoning
2. Routing decisions are made in <100ms (fast enough for real-time execution)
3. Routing logic is expressible as decision trees or state machines (not neural networks or learned models)
4. Routing is deterministic: same input → same output (unless availability changes)
5. Routing is auditable: all decisions are logged with reasoning
6. Routing is bounded: no infinite loops, no unbounded recursion, no AGI-style adaptation

---

## Constraints

1. **Local-First Default:** Queries answerable locally must never trigger external retrieval
2. **Capability-First:** Tools can only run on substrates that support their capabilities
3. **Deterministic:** Same intent + context → same routing decision
4. **Bounded:** No unbounded reasoning, no AGI-style adaptation, no probabilistic selection
5. **Fast:** Routing decisions must complete in <100ms
6. **Auditable:** All decisions must be logged with reasoning
7. **Verifier-Aware:** Routing must integrate verifier feedback without unbounded learning
8. **Privacy-Preserving:** Local-first execution must be default; external calls must be explicit
9. **Substrate-Aware:** Routing must understand host, VM, Docker, and remote SSH capabilities
10. **Fallback-Ready:** Every routing decision must include fallback strategy

---

## Success Criteria

1. ✅ Web/news retrieval reduced by 60% (unnecessary calls eliminated)
2. ✅ Substrate confusion eliminated (clear host/VM/Docker/remote resolution)
3. ✅ Tool misuse prevented (100% of tool invocations validated)
4. ✅ GUI automation reduced by 50% (APIs preferred over GUI)
5. ✅ Execution fallback reasoning improved (clear recovery strategies)
6. ✅ Routing decisions deterministic and auditable (100% logged)
7. ✅ Routing latency <100ms (fast enough for real-time execution)
8. ✅ Zero fake capability responses (all limitations explained with alternatives)
9. ✅ MCP/tool misuse prevented (100% of invocations validated)
10. ✅ Local-first execution preserved (local queries never trigger external retrieval)

---

## Out of Scope

- AGI-style reasoning or unbounded learning
- Probabilistic tool selection or randomized routing
- Neural network-based routing (use decision trees/state machines instead)
- Changing KRIA's core architecture (work within existing bounds)
- Modifying tool schemas (work with existing tool definitions)
- Changing safety policy (work within existing safety layer)
