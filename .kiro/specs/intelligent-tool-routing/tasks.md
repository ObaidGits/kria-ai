# Intelligent Tool Routing — Implementation Tasks

## Task Dependency Graph

```
┌─────────────────────────────────────────────────────────────┐
│ Task 1: Intent Classification Engine                        │
│ (Foundation for all routing decisions)                      │
└────────────────────┬────────────────────────────────────────┘
                     │
        ┌────────────┴────────────┐
        ▼                         ▼
┌──────────────────────┐  ┌──────────────────────┐
│ Task 2: Execution    │  │ Task 3: Capability   │
│ Context Resolver     │  │ Definitions          │
└────────┬─────────────┘  └──────────┬───────────┘
         │                           │
         └───────────────┬───────────┘
                         ▼
         ┌───────────────────────────────┐
         │ Task 4: Tool Router           │
         │ (Core routing logic)          │
         └───────────┬───────────────────┘
                     │
        ┌────────────┴────────────┐
        ▼                         ▼
┌──────────────────────┐  ┌──────────────────────┐
│ Task 5: Tool         │  │ Task 6: Verifier     │
│ Validator            │  │ Feedback Integration │
└────────┬─────────────┘  └──────────┬───────────┘
         │                           │
         └───────────────┬───────────┘
                         ▼
         ┌───────────────────────────────┐
         │ Task 7: Agent Loop Integration│
         │ (Wire into agent execution)   │
         └───────────────┬───────────────┘
                         ▼
         ┌───────────────────────────────┐
         │ Task 8: Testing & Validation  │
         │ (Unit, integration, property) │
         └───────────────────────────────┘
```

---

## Task 1: Intent Classification Engine

**Objective:** Implement semantic intent classification to guide routing decisions.

**Description:**

Create the `IntentClassifier` component that classifies user prompts into semantic categories (CONVERSATIONAL_META, CAPABILITY_QUESTION, LOCAL_QUERY, EXTERNAL_QUERY, EXECUTION_REQUEST, FRESHNESS_REQUIRED).

**Acceptance Criteria:**

1. ✅ `IntentClassifier` struct created with `classify(prompt, context) → IntentClassification` method
2. ✅ Classification logic implemented as decision tree (no randomness)
3. ✅ Pattern matching for conversational prompts (greetings, meta-interaction)
4. ✅ Temporal marker detection for freshness-required queries ("today", "latest", "current")
5. ✅ Local context detection ("my files", "my documents", "in my system")
6. ✅ Capability question detection ("can you", "do you support", "what can you do")
7. ✅ Classification latency <10ms
8. ✅ 100+ sample prompts tested with expected classifications
9. ✅ Classification reasoning logged for debugging
10. ✅ Unit tests with 95%+ coverage

**Implementation Location:**

- File: `crates/kria-core/src/routing/intent_classifier.rs`
- Module: `kria_core::routing::intent_classifier`

**Dependencies:**

- None (foundation component)

**Estimated Effort:** 2-3 days

**Success Metrics:**

- Classification accuracy >95% on test set
- Latency <10ms (p99)
- Zero false negatives on critical patterns (freshness, local context)

---

## Task 2: Execution Context Resolver

**Objective:** Resolve available substrates, capabilities, and resource constraints.

**Description:**

Create the `ExecutionContextResolver` component that detects available substrates (LOCAL_HOST, REMOTE_SSH, DOCKER_CONTAINER, VM_GUEST), resolves their capabilities (shell, network, filesystem, GPU, GUI), and checks resource constraints.

**Acceptance Criteria:**

1. ✅ `ExecutionContextResolver` struct created with `resolve(system_state) → ExecutionContext` method
2. ✅ Substrate detection: LOCAL_HOST (always available)
3. ✅ Substrate detection: REMOTE_SSH (check SSH config, connectivity)
4. ✅ Substrate detection: DOCKER_CONTAINER (check Docker daemon, running containers)
5. ✅ Substrate detection: VM_GUEST (check QEMU/Hyper-V, running VMs)
6. ✅ Capability resolution for each substrate (shell, network, filesystem, GPU, GUI)
7. ✅ Resource constraint checking (CPU, memory, disk available)
8. ✅ Substrate availability checking (online/offline status)
9. ✅ Deterministic substrate ordering (local-first)
10. ✅ Context resolution latency <20ms
11. ✅ Unit tests with various system configurations
12. ✅ Integration tests with real substrates (if available)

**Implementation Location:**

- File: `crates/kria-core/src/routing/execution_context.rs`
- Module: `kria_core::routing::execution_context`

**Dependencies:**

- Task 1 (Intent Classification) — optional, for context enrichment

**Estimated Effort:** 3-4 days

**Success Metrics:**

- Substrate detection accuracy >95%
- Capability resolution accuracy >95%
- Context resolution latency <20ms (p99)
- Deterministic substrate ordering (same system state → same order)

---

## Task 3: Capability Definitions

**Objective:** Define tool and substrate capabilities as structured data.

**Description:**

Create capability definitions for all tools and substrates. Define what capabilities each tool requires and what capabilities each substrate provides.

**Acceptance Criteria:**

1. ✅ `ToolCapabilities` struct created with required capabilities list
2. ✅ `SubstrateCapabilities` struct created with available capabilities list
3. ✅ Capability matching logic: `tool_capabilities ⊆ substrate_capabilities`
4. ✅ All 60+ KRIA tools have capability definitions
5. ✅ All 4 substrates (host, remote, docker, vm) have capability definitions
6. ✅ Capability definitions are versioned (for future extensibility)
7. ✅ Capability mismatch detection with clear error messages
8. ✅ Unit tests for capability matching logic
9. ✅ Documentation of all capabilities and their meanings

**Implementation Location:**

- File: `crates/kria-core/src/routing/capabilities.rs`
- Module: `kria_core::routing::capabilities`

**Dependencies:**

- Task 2 (Execution Context Resolver) — for substrate capabilities

**Estimated Effort:** 2-3 days

**Success Metrics:**

- All 60+ tools have capability definitions
- All 4 substrates have capability definitions
- Capability matching logic 100% correct
- Zero capability mismatches in production

---

## Task 4: Capability-Aware Tool Router

**Objective:** Implement core routing logic that selects tool + substrate + fallback strategy.

**Description:**

Create the `ToolRouter` component that implements the routing decision logic. This is the core component that makes routing decisions based on intent classification, execution context, and capability matching.

**Acceptance Criteria:**

1. ✅ `ToolRouter` struct created with `route(intent, context) → RoutingDecision` method
2. ✅ Routing decision structure includes: primary_tool, primary_substrate, fallback_tool, fallback_substrate, reasoning
3. ✅ Intent-based routing logic implemented (CONVERSATIONAL_META → deny, FRESHNESS_REQUIRED → allow, etc.)
4. ✅ Local-first policy enforcement (local queries never trigger external retrieval)
5. ✅ Capability matching: tool_capabilities ⊆ substrate_capabilities
6. ✅ Fallback strategy selection (PRIMARY_TOOL | FALLBACK_TOOL | FALLBACK_SUBSTRATE | LOCAL_KNOWLEDGE)
7. ✅ Routing decision logging with reasoning
8. ✅ Routing latency <30ms
9. ✅ Deterministic routing (same input → same output)
10. ✅ Unit tests with 100+ routing scenarios
11. ✅ Integration tests with real tools and substrates
12. ✅ Property-based tests for determinism and local-first preservation

**Implementation Location:**

- File: `crates/kria-core/src/routing/tool_router.rs`
- Module: `kria_core::routing::tool_router`

**Dependencies:**

- Task 1 (Intent Classification Engine)
- Task 2 (Execution Context Resolver)
- Task 3 (Capability Definitions)

**Estimated Effort:** 4-5 days

**Success Metrics:**

- Routing latency <30ms (p99)
- Deterministic routing (100% reproducible)
- Local-first policy preserved (0% unnecessary external retrievals)
- Fallback strategy available for 100% of routing decisions
- Capability matching 100% correct

---

## Task 5: Tool Invocation Validator

**Objective:** Validate tool invocations before execution to prevent misuse.

**Description:**

Create the `ToolInvocationValidator` component that validates tool invocations against schema, capabilities, and availability before execution.

**Acceptance Criteria:**

1. ✅ `ToolInvocationValidator` struct created with `validate(routing_decision, parameters) → ValidationResult` method
2. ✅ Tool existence check (tool exists in registry)
3. ✅ Capability matching check (tool_capabilities ⊆ substrate_capabilities)
4. ✅ Schema validation (parameters match tool schema)
5. ✅ Availability check (tool is available on substrate)
6. ✅ MCP validation (MCP server running, tool registered, payload compatible)
7. ✅ Safety policy check (tool not blacklisted, HITL approval if needed)
8. ✅ Validation error messages are clear and actionable
9. ✅ Validation latency <20ms
10. ✅ Unit tests for all validation checks
11. ✅ Integration tests with real tools and MCP servers

**Implementation Location:**

- File: `crates/kria-core/src/routing/tool_validator.rs`
- Module: `kria_core::routing::tool_validator`

**Dependencies:**

- Task 3 (Capability Definitions)
- Task 4 (Tool Router)

**Estimated Effort:** 2-3 days

**Success Metrics:**

- Validation latency <20ms (p99)
- 100% of invalid invocations caught
- 0% false positives (valid invocations rejected)
- Clear error messages for all validation failures

---

## Task 6: Verifier Feedback Integration

**Objective:** Integrate verifier feedback into routing decisions to improve accuracy over time.

**Description:**

Create the `VerifierFeedbackManager` component that stores verifier feedback, updates routing feedback, and integrates feedback into routing decisions with bounded learning.

**Acceptance Criteria:**

1. ✅ `VerifierFeedback` struct created with: tool_name, substrate, success/failure, reason, timestamp
2. ✅ `RoutingFeedback` struct created with: success_rate, failure_count, last_failure
3. ✅ Feedback storage in SQLite (persistent across sessions)
4. ✅ Feedback-aware routing logic (prefer high-success-rate combinations)
5. ✅ Bounded learning: max 100 feedback entries per tool/substrate, 7-day expiry
6. ✅ Feedback update logic: success_rate calculation, failure tracking
7. ✅ Feedback query logic: retrieve feedback for tool/substrate combination
8. ✅ Feedback reset/recalibration logic (for stale feedback)
9. ✅ Unit tests for feedback storage and retrieval
10. ✅ Integration tests with verifier feedback loop
11. ✅ Property-based tests for bounded learning

**Implementation Location:**

- File: `crates/kria-core/src/routing/verifier_feedback.rs`
- Module: `kria_core::routing::verifier_feedback`

**Dependencies:**

- Task 4 (Tool Router)
- Task 5 (Tool Validator)

**Estimated Effort:** 3-4 days

**Success Metrics:**

- Feedback storage/retrieval latency <10ms
- Bounded learning prevents unbounded adaptation
- Feedback improves routing accuracy by 10-20%
- Feedback expiry prevents stale data

---

## Task 7: Agent Loop Integration

**Objective:** Integrate intelligent tool routing into KRIA's agent loop.

**Description:**

Wire the intelligent tool routing components into the agent loop so that all tool invocations go through the routing pipeline.

**Acceptance Criteria:**

1. ✅ Agent loop modified to use `ToolRouter` for all tool selection
2. ✅ Agent loop uses `ToolInvocationValidator` before tool execution
3. ✅ Agent loop integrates `VerifierFeedbackManager` after tool execution
4. ✅ Routing decisions logged in agent execution trace
5. ✅ Fallback execution implemented (primary failure → fallback attempt)
6. ✅ Routing latency doesn't exceed agent loop budget (<100ms total)
7. ✅ Backward compatibility maintained (existing agent behavior preserved)
8. ✅ Integration tests with real agent loop
9. ✅ End-to-end tests with multiple tool invocations
10. ✅ Performance benchmarks (routing latency, agent loop throughput)

**Implementation Location:**

- File: `crates/kria-core/src/agent/loop_engine/routing_integration.rs`
- Module: `kria_core::agent::loop_engine::routing_integration`

**Dependencies:**

- Task 1-6 (All routing components)

**Estimated Effort:** 3-4 days

**Success Metrics:**

- Agent loop routing latency <100ms (p99)
- 100% of tool invocations go through routing pipeline
- Fallback execution works correctly
- Backward compatibility maintained

---

## Task 8: Testing & Validation

**Objective:** Comprehensive testing and validation of intelligent tool routing.

**Description:**

Create comprehensive test suites covering unit tests, integration tests, property-based tests, and performance benchmarks.

**Acceptance Criteria:**

1. ✅ Unit tests for all routing components (95%+ coverage)
2. ✅ Integration tests for end-to-end routing (intent → execution)
3. ✅ Property-based tests for determinism, local-first, capability safety
4. ✅ Performance benchmarks (routing latency, substrate resolution, capability matching)
5. ✅ Fallback execution tests (primary failure → fallback success)
6. ✅ Verifier feedback loop tests (execution → feedback → improved routing)
7. ✅ Substrate switching tests (local → remote → docker)
8. ✅ MCP tool validation tests
9. ✅ Edge case tests (all substrates unavailable, tool not available, etc.)
10. ✅ Regression tests (ensure existing behavior preserved)
11. ✅ Documentation of test coverage and results
12. ✅ CI/CD integration (tests run on every commit)

**Implementation Location:**

- Directory: `crates/kria-core/tests/routing/`
- Modules: `tests::routing::*`

**Dependencies:**

- Task 1-7 (All routing components)

**Estimated Effort:** 4-5 days

**Success Metrics:**

- Unit test coverage >95%
- Integration test coverage >90%
- All property-based tests pass
- Performance benchmarks meet targets (<100ms routing latency)
- Zero regressions in existing behavior

---

## Task 9: Documentation & Observability

**Objective:** Document intelligent tool routing and add observability for production monitoring.

**Description:**

Create comprehensive documentation and add observability (metrics, logging, tracing) for production monitoring.

**Acceptance Criteria:**

1. ✅ Architecture documentation (high-level overview, component interactions)
2. ✅ API documentation (routing decision structure, validator interface, etc.)
3. ✅ Configuration documentation (routing policies, fallback strategies, etc.)
4. ✅ Troubleshooting guide (common issues, debugging tips)
5. ✅ Metrics implementation (routing latency, success rates, fallback frequency)
6. ✅ Logging implementation (routing decisions, validation failures, feedback updates)
7. ✅ Tracing implementation (end-to-end routing traces)
8. ✅ Dashboard/monitoring setup (visualize routing metrics)
9. ✅ Runbook for common operational tasks
10. ✅ Examples of routing decisions and fallback scenarios

**Implementation Location:**

- Directory: `docs/routing/`
- Files: `ARCHITECTURE.md`, `API.md`, `CONFIGURATION.md`, `TROUBLESHOOTING.md`

**Dependencies:**

- Task 1-8 (All routing components)

**Estimated Effort:** 2-3 days

**Success Metrics:**

- Documentation covers all components and use cases
- Metrics and logging provide visibility into routing behavior
- Troubleshooting guide helps resolve common issues

---

## Task 10: Production Rollout & Monitoring

**Objective:** Roll out intelligent tool routing to production with monitoring and gradual adoption.

**Description:**

Plan and execute production rollout with monitoring, gradual adoption, and rollback capability.

**Acceptance Criteria:**

1. ✅ Rollout plan created (phased adoption, monitoring, rollback strategy)
2. ✅ Feature flag implemented (enable/disable routing per user/session)
3. ✅ Monitoring dashboard created (routing metrics, success rates, fallback frequency)
4. ✅ Alert rules configured (routing latency, validation failures, feedback anomalies)
5. ✅ Rollback procedure documented (revert to previous routing logic)
6. ✅ Gradual adoption plan (10% → 50% → 100% of users)
7. ✅ A/B testing setup (compare old vs. new routing)
8. ✅ User feedback collection (gather feedback on routing improvements)
9. ✅ Performance validation (ensure no regression in agent loop throughput)
10. ✅ Post-rollout analysis (measure impact on web retrieval, substrate selection, etc.)

**Implementation Location:**

- Directory: `crates/kria-desktop/src/commands/`
- Files: Feature flag configuration, monitoring setup

**Dependencies:**

- Task 1-9 (All routing components)

**Estimated Effort:** 2-3 days

**Success Metrics:**

- Rollout completed with 0% downtime
- Monitoring shows expected improvements (60% reduction in unnecessary retrieval)
- No regressions in agent loop performance
- User feedback positive

---

## Summary

| Task | Effort | Dependencies | Status |
|------|--------|--------------|--------|
| 1. Intent Classification | 2-3d | None | Ready |
| 2. Execution Context Resolver | 3-4d | Task 1 | Ready |
| 3. Capability Definitions | 2-3d | Task 2 | Ready |
| 4. Tool Router | 4-5d | Tasks 1-3 | Ready |
| 5. Tool Validator | 2-3d | Tasks 3-4 | Ready |
| 6. Verifier Feedback | 3-4d | Tasks 4-5 | Ready |
| 7. Agent Loop Integration | 3-4d | Tasks 1-6 | Ready |
| 8. Testing & Validation | 4-5d | Tasks 1-7 | Ready |
| 9. Documentation | 2-3d | Tasks 1-8 | Ready |
| 10. Production Rollout | 2-3d | Tasks 1-9 | Ready |
| **Total** | **28-38 days** | — | — |

---

## Execution Notes

- **Parallel Execution:** Tasks 2 and 3 can run in parallel with Task 1
- **Iterative Development:** Each task should include unit tests; integration happens in Task 7
- **Code Review:** Each task should be reviewed before proceeding to dependent tasks
- **Documentation:** Update docs incrementally as each task completes
- **Monitoring:** Add observability from Task 1 onward (not just Task 9)
