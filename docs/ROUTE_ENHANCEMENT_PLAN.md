# RFC: Unified Intelligent Routing System Enhancement

**Status:** Proposed  
**Date:** 2026-05-08  
**Owner:** Systems Architecture  
**Scope:** `crates/kria-core/src/routing/`, `crates/kria-core/src/agent/`, `ui/src/`  
**Supersedes:** Current 3-layer cascade (regex → FastEmbed → ONNX hints)  
**Target:** Voice-first AI assistant with <500ms routing latency, 92%+ accuracy, Hinglish native support

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Current State Analysis](#2-current-state-analysis)
3. [Target Architecture](#3-target-architecture)
4. [Phase 1: Context-Aware Routing](#phase-1-context-aware-routing)
5. [Phase 2: Fine-Tuned Intent Classifier](#phase-2-fine-tuned-intent-classifier)
6. [Phase 3: Tool-Level Semantic Index](#phase-3-tool-level-semantic-index)
7. [Phase 4: Speculative Pre-Warming](#phase-4-speculative-pre-warming)
8. [Phase 5: Online Learning Feedback Loop](#phase-5-online-learning-feedback-loop)
9. [Frontend Changes](#9-frontend-changes)
10. [Server API Changes](#10-server-api-changes)
11. [Test Strategy](#11-test-strategy)
12. [Rollout & Rollback Plan](#12-rollout--rollback-plan)
13. [Risk Matrix](#13-risk-matrix)
14. [Acceptance Criteria](#14-acceptance-criteria)

---

## 1. Executive Summary

KRIA's current routing pipeline is a 3-layer cascade that makes routing decisions in isolation per turn:

```
User text → Regex (60+ patterns) → FastEmbed (12 domain centroids) → ONNX (18 examples) → LLM tool selection
```

**Problems:**
- Regex patterns are brittle (fails on Hinglish, paraphrasing, typos)
- Each turn routed independently (no conversation context)
- ONNX classifier has only 18 corpus examples (near-useless)
- LLM is always invoked for tool selection even when obvious (adds 500ms+)
- No learning from routing mistakes
- Voice latency budget (1–2s) is consumed by routing overhead

**Solution:** A unified 5-phase enhancement that transforms routing from a static cascade into an intelligent, context-aware, self-improving system.

**Target Metrics:**

| Metric | Current | Target | Method |
|--------|---------|--------|--------|
| Routing accuracy | ~70% | 92–95% | Phases 1–3 |
| Voice latency (simple) | 800–1200ms | 200–400ms | Phase 3 + 4 |
| Voice latency (complex) | 2000–3000ms | 1200–1800ms | Phase 2 |
| Hinglish support | Partial | Native | Phase 2 |
| Multi-turn context | None | Full | Phase 1 |
| LLM calls per turn | 100% | ~30% | Phase 3 |
| Personalization | None | Adaptive | Phase 5 |

---

## 2. Current State Analysis

### 2.1 Pipeline Architecture (Today)

```
┌─────────────────────────────────────────────────────────────────┐
│  Stage A: Deterministic Guards (router.rs)                      │
│  ├─ 60+ regex patterns                                          │
│  ├─ Confidence: 0.3–0.85                                        │
│  └─ Output: Intent { Conversation | DirectTool | ComplexTask }  │
│                                                                  │
│  Stage B: Semantic Routing (routing/*.rs)                        │
│  ├─ FastEmbed multilingual-e5-small                              │
│  ├─ 12 domain centroids (5-6 anchor sentences each)             │
│  ├─ Verb modality detection (7 types)                            │
│  ├─ Multi-command segmentation                                   │
│  ├─ OOD detection (z-score + entropy)                            │
│  └─ Output: RouteDecision { Conversation | SingleDomain |        │
│               MultiDomain | Ambiguous }                          │
│                                                                  │
│  Stage C: Optional ONNX (onnx_classifier.rs)                     │
│  ├─ 18 corpus examples (6 per operation)                         │
│  ├─ Non-authoritative hints only                                 │
│  └─ Output: OnnxHint { operation, compute, confidence }          │
│                                                                  │
│  TurnGate Validator (turn_gate.rs)                               │
│  ├─ Merge all signals → IntentEnvelope                           │
│  ├─ Compile ResourcePlan                                         │
│  └─ Output: TurnGatePlan { intent, resources, tool_hints }      │
│                                                                  │
│  AgentLoop Executor (loop_engine/mod.rs)                         │
│  ├─ Filter tools by domain                                       │
│  ├─ Score tool relevance (token matching)                        │
│  ├─ Select top 8 tool schemas                                    │
│  └─ ALWAYS invokes LLM for tool selection                        │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 Quantitative Indicators

| Indicator | Value | Location |
|-----------|-------|----------|
| Regex patterns | 60+ | `crates/kria-core/src/agent/router.rs` |
| Domain centroids | 12 | `crates/kria-core/src/routing/domain.rs` |
| ONNX corpus examples | 18 | `ai-context/onnx_l0_corpus.jsonl` |
| Anchor sentences per domain | 5–6 | `crates/kria-core/src/routing/domain.rs` |
| Verb modality types | 7 | `crates/kria-core/src/routing/verbs.rs` |
| Segment split patterns | 1 regex | `crates/kria-core/src/routing/segment.rs` |
| OOD calibration prompts | ~500 | `~/.kria/cache/router/ood_calibration.v1.bin` |
| Max routed tool schemas | 8 | `crates/kria-core/src/agent/loop_engine/mod.rs` |
| Routing inline tests | 5 | `routing/segment.rs` (3), `routing/ood.rs` (2) |
| Integration routing tests | 3 | `tests/voice_live_tests.rs` |
| Routing config thresholds | 5 | `crates/kria-core/src/config.rs` |

### 2.3 Key Source Files

| File | Purpose | Lines |
|------|---------|-------|
| `routing/mod.rs` | Router entry point, embed + route | ~300 |
| `routing/decide.rs` | Decision algorithm | ~120 |
| `routing/ood.rs` | OOD detection | ~130 |
| `routing/embed.rs` | FastEmbed wrapper | ~80 |
| `routing/domain.rs` | Domain enum + anchor sentences | ~200 |
| `routing/verbs.rs` | Verb/modality classifier | ~120 |
| `routing/segment.rs` | Multi-command segmenter | ~80 |
| `routing/cache.rs` | Centroid + OOD cache | ~500 |
| `routing/trace.rs` | Observability event | ~50 |
| `agent/router.rs` | Legacy regex router | ~400 |
| `agent/turn_gate.rs` | IntentEnvelope + ResourcePlan | ~300 |
| `agent/onnx_classifier.rs` | ONNX L0 classifier | ~300 |
| `agent/planner.rs` | Multi-step plan parser | ~80 |
| `agent/loop_engine/mod.rs` | Agent loop + tool selection | ~500 |

---

## 3. Target Architecture

### 3.1 Unified Router (After All Phases)

```
┌─────────────────────────────────────────────────────────────────────┐
│                     UNIFIED INTELLIGENT ROUTER                      │
│                                                                     │
│  User text + Voice partial ─────────────────────────────────────┐   │
│                                                                 │   │
│  ┌──────────────────────────────────────────────────────────┐   │   │
│  │ Layer 0: Context Enrichment (Phase 1)                    │   │   │
│  │ ├─ RoutingContext from last 3 turns                      │   │   │
│  │ ├─ Correction detection ("no I meant X")                 │   │   │
│  │ ├─ Topic continuation / domain carry                     │   │   │
│  │ └─ Output: enriched_text + context_signals               │   │   │
│  └──────────────────────┬───────────────────────────────────┘   │   │
│                         ↓                                        │   │
│  ┌──────────────────────────────────────────────────────────┐   │   │
│  │ Layer 1: Intent Classification (Phase 2)                 │   │   │
│  │ ├─ Fine-tuned Qwen2.5-0.5B (ONNX, CPU worker)           │   │   │
│  │ ├─ Input: enriched_text + context_signals                │   │   │
│  │ ├─ Output: Operation + ComputeClass + HazardHint         │   │   │
│  │ └─ Latency: ~15-25ms (replaces regex + old ONNX)         │   │   │
│  └──────────────────────┬───────────────────────────────────┘   │   │
│                         ↓                                        │   │
│  ┌──────────────────────────────────────────────────────────┐   │   │
│  │ Layer 2: Tool Semantic Match (Phase 3)                   │   │   │
│  │ ├─ Embedding index over ~100 tool descriptions           │   │   │
│  │ ├─ Cosine similarity: query → tool descriptions          │   │   │
│  │ ├─ Confidence ≥ 0.85 → DIRECT EXECUTION (skip LLM)      │   │   │
│  │ └─ Confidence < 0.85 → pass to LLM with narrowed tools  │   │   │
│  └──────────────────────┬───────────────────────────────────┘   │   │
│                         ↓                                        │   │
│  ┌──────────────────────────────────────────────────────────┐   │   │
│  │ Layer 3: Speculative Pre-Warming (Phase 4)               │   │   │
│  │ ├─ Runs on partial transcripts (voice only)              │   │   │
│  │ ├─ Pre-acquires GPU lease for predicted path             │   │   │
│  │ └─ Cancels on wrong prediction (TurnAdmission handles)   │   │   │
│  └──────────────────────┬───────────────────────────────────┘   │   │
│                         ↓                                        │   │
│  ┌──────────────────────────────────────────────────────────┐   │   │
│  │ Layer 4: Feedback Loop (Phase 5)                         │   │   │
│  │ ├─ Collects routing outcomes                             │   │   │
│  │ ├─ Nightly centroid weight adjustment                    │   │   │
│  │ └─ Continuous accuracy improvement                       │   │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ TurnGate (existing, enhanced)                            │   │
│  │ ├─ IntentEnvelope from all layers                        │   │
│  │ ├─ ResourcePlan compilation                              │   │
│  │ └─ Safety invariants enforcement                         │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.2 New File Layout

```
crates/kria-core/src/routing/
├── mod.rs              ← ENHANCED: unified router entry point
├── decide.rs           ← ENHANCED: context-aware decision algorithm
├── ood.rs              ← UNCHANGED
├── embed.rs            ← UNCHANGED
├── domain.rs           ← ENHANCED: add new anchor sentences for Hinglish
├── verbs.rs            ← ENHANGED: add Hinglish verb patterns
├── segment.rs          ← UNCHANGED
├── cache.rs            ← ENHANCED: add tool embedding cache
├── trace.rs            ← ENHANCED: add new trace fields
├── context.rs          ← NEW: Phase 1 - RoutingContext management
├── intent_classifier.rs← NEW: Phase 2 - Fine-tuned LM classifier
├── tool_index.rs       ← NEW: Phase 3 - Tool-level semantic index
├── speculative.rs      ← NEW: Phase 4 - Speculative pre-warming
└── feedback.rs         ← NEW: Phase 5 - Online learning feedback

crates/kria-core/src/agent/
├── router.rs           ← DEPRECATED: regex router (kept for fallback)
├── onnx_classifier.rs  ← DEPRECATED: replaced by intent_classifier.rs
├── turn_gate.rs        ← ENHANCED: integrate new routing layers
├── planner.rs          ← ENHANCED: use tool_index for plan generation
└── loop_engine/
    └── mod.rs          ← ENHANCED: direct-execution fast path

models/
├── classifier/
│   ├── model.onnx      ← EXISTING (to be replaced)
│   ├── tokenizer.json  ← EXISTING (to be replaced)
│   └── intent_v2.onnx  ← NEW: Phase 2 fine-tuned model
└── tool_embeddings/
    └── tool_index.bin   ← NEW: Phase 3 pre-computed embeddings

ui/src/
├── components/
│   ├── RoutingDebug.tsx ← NEW: Phase 1 - routing decision visualization
│   └── RoutingFeedback.tsx ← NEW: Phase 5 - user correction UI
└── stores/
    └── routingStore.ts  ← NEW: Phase 1 - routing state management
```

---

## Phase 1: Context-Aware Routing

**Duration:** 1 week  
**Goal:** Routing decisions incorporate conversation history, correction detection, and topic continuity  
**Risk:** Low (pure string manipulation, no new models)  
**Voice latency impact:** +0ms

### 1.1 Backend: `routing/context.rs` (NEW)

```rust
/// Conversation context carried between routing decisions.
#[derive(Debug, Clone, Default)]
pub struct RoutingContext {
    /// Domain of the previous turn (if successfully routed).
    pub last_domain: Option<Domain>,
    /// Tool name of the previous turn (if directly matched).
    pub last_tool: Option<String>,
    /// Modality of the previous turn.
    pub last_modality: IntentModality,
    /// How many consecutive turns in the same domain.
    pub turn_count_in_topic: usize,
    /// Whether the user explicitly corrected the previous routing.
    pub correction_pending: bool,
    /// The embedding of the previous turn (for similarity carry).
    pub last_embedding: Option<Vec<f32>>,
    /// Timestamp of the last turn (for staleness detection).
    pub last_turn_at: Option<std::time::Instant>,
}

impl RoutingContext {
    /// Update context after a successful routing decision.
    pub fn record_turn(&mut self, domain: Domain, tool: Option<String>, modality: IntentModality, embedding: Vec<f32>) { ... }

    /// Reset context on topic change or long silence (>60s).
    pub fn reset(&mut self) { ... }

    /// Check if context is stale (>60s since last turn).
    pub fn is_stale(&self) -> bool { ... }
}
```

#### 1.1.1 Correction Detection

Detect explicit correction phrases in user input:

```rust
/// Correction patterns (multilingual).
static CORRECTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(no|nahi|nahin|wrong|galat|actually|I meant|mera matlab|the other|wo nahi|ye nahi)\b").unwrap()
});

/// Detect if user is correcting the previous routing.
pub fn detect_correction(text: &str, ctx: &RoutingContext) -> CorrectionSignal {
    if !CORRECTION_RE.is_match(text) {
        return CorrectionSignal::None;
    }
    // Extract the corrected target from the text
    CorrectionSignal::Explicit { text: text.to_string() }
}
```

#### 1.1.2 Context Enrichment

Enrich short/ambiguous inputs with context from previous turns:

```rust
/// Enrich user text with routing context for better embedding quality.
pub fn enrich_with_context(text: &str, ctx: &RoutingContext) -> EnrichedInput {
    // Don't enrich if context is stale or text is already specific
    if ctx.is_stale() || text.len() > 80 {
        return EnrichedInput::original(text);
    }

    // Correction: prepend "correcting previous" signal
    if ctx.correction_pending {
        return EnrichedInput::enriched(
            format!("(correction) {}", text),
            EnrichmentReason::Correction,
        );
    }

    // Short ambiguous input with strong context: carry domain
    if text.len() < 30 && ctx.last_domain.is_some() && ctx.turn_count_in_topic >= 1 {
        let domain_hint = ctx.last_domain.unwrap().anchor_sentences()[0];
        return EnrichedInput::enriched(
            format!("{} [context: {}]", text, domain_hint),
            EnrichmentReason::TopicContinuation,
        );
    }

    EnrichedInput::original(text)
}
```

### 1.2 Backend: Modify `routing/mod.rs`

```rust
// In Router::route(), add context parameter:
pub async fn route(&self, text: &str, ctx: &RoutingContext) -> (RouteDecision, ModalityResult, RouterTrace) {
    // ... existing modality classification ...

    // NEW: Enrich with context
    let enriched = context::enrich_with_context(text, ctx);
    let route_text = enriched.effective_text();

    // ... existing embedding + domain scoring using route_text instead of text ...
    // ... existing decision algorithm ...

    // NEW: Record turn in context after routing
    // (caller is responsible for updating RoutingContext)
}
```

### 1.3 Backend: Modify `routing/decide.rs`

```rust
// Extend DecideInput with context signals:
pub struct DecideInput<'a> {
    // ... existing fields ...
    /// Context from previous turns.
    pub context: &'a RoutingContext,
}

// In decide(), add context-aware logic BEFORE OOD check:
pub fn decide(input: &DecideInput<'_>) -> RouteDecision {
    // NEW: If correction detected and previous domain was strong,
    // boost that domain's similarity
    if input.context.correction_pending {
        if let Some(last_domain) = input.context.last_domain {
            // Check if any segment mentions the correction target
            // Re-route with boosted previous-domain similarity
        }
    }

    // NEW: Topic continuation — if short input + strong context
    if input.context.turn_count_in_topic >= 2 && input.segments.len() <= 1 {
        if let Some((top_domain, top_sim)) = input.domain_sims.first() {
            if let Some(last_domain) = input.context.last_domain {
                if *top_domain == last_domain && *top_sim > 0.3 {
                    return RouteDecision::SingleDomain(*top_domain);
                }
            }
        }
    }

    // ... existing OOD check + decision logic ...
}
```

### 1.4 Backend: Modify `agent/turn_gate.rs`

```rust
// Add RoutingContext to TurnGate:
pub struct TurnGate {
    // ... existing fields ...
    context: RoutingContext,
}

impl TurnGate {
    pub fn plan_turn(&mut self, user_text: &str, has_images: bool) -> TurnGatePlan {
        // ... existing logic, but pass self.context to routing ...
        let (decision, modality, trace) = self.router.route(user_text, &self.context).await;

        // Update context after routing
        if let RouteDecision::SingleDomain(d) = &decision {
            self.context.record_turn(*d, None, modality.primary, embedding);
        }

        // ... rest of existing logic ...
    }
}
```

### 1.5 Backend: Modify `agent/loop_engine/mod.rs`

```rust
// Pass RoutingContext through the agent loop:
// - Extract from recent conversation turns stored in MemoryStore
// - Rebuild RoutingContext at the start of each turn
// - Wire through to TurnGate
```

### 1.6 Phase 1 Acceptance Criteria

| # | Criterion | Test |
|---|-----------|------|
| 1.1 | Short ambiguous input with prior context routes correctly | Unit test |
| 1.2 | Correction phrases ("no I meant X") re-route to correct domain | Unit test |
| 1.3 | Topic continuation carries domain across 3+ turns | Unit test |
| 1.4 | Stale context (>60s) falls back to standard routing | Unit test |
| 1.5 | Hinglish continuation ("uska status bhi dikhao") works | Integration test |
| 1.6 | Existing routing tests still pass (no regression) | Full test suite |
| 1.7 | RoutingContext serialization for debug/trace | Unit test |
| 1.8 | No latency increase (<1ms overhead) | Benchmark |

### 1.7 Phase 1 Test Matrix

```rust
// crates/kria-core/tests/phase6_routing_context_tests.rs

#[test]
fn ctx01_topic_continuation_carries_domain() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(Domain::SystemInfo, Some("check_system_health".into()), IntentModality::Read, vec![0.1; 384]);

    let enriched = enrich_with_context("also check disk", &ctx);
    assert!(enriched.text.contains("system")); // context injected
}

#[test]
fn ctx02_correction_detection() {
    let ctx = RoutingContext {
        last_domain: Some(Domain::SystemInfo),
        correction_pending: false,
        ..Default::default()
    };
    let signal = detect_correction("no I meant the network", &ctx);
    assert!(matches!(signal, CorrectionSignal::Explicit { .. }));
}

#[test]
fn ctx03_stale_context_not_used() {
    let mut ctx = RoutingContext::default();
    ctx.last_turn_at = Some(Instant::now() - Duration::from_secs(120));
    ctx.record_turn(Domain::SystemInfo, None, IntentModality::Read, vec![0.1; 384]);

    let enriched = enrich_with_context("check disk", &ctx);
    assert!(!enriched.text.contains("context")); // stale, no enrichment
}

#[test]
fn ctx04_long_input_not_enriched() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(Domain::SystemInfo, None, IntentModality::Read, vec![0.1; 384]);

    let long_text = "a".repeat(100);
    let enriched = enrich_with_context(&long_text, &ctx);
    assert_eq!(enriched.text, long_text); // too long, no enrichment
}

#[test]
fn ctx05_multi_turn_hinglish_continuation() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(Domain::SystemInfo, None, IntentModality::Read, vec![0.1; 384]);
    ctx.turn_count_in_topic = 2;

    let enriched = enrich_with_context("uska status bhi dikhao", &ctx);
    assert!(enriched.text.contains("system")); // Hinglish carry
}

#[test]
fn ctx06_context_resets_on_topic_change() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(Domain::SystemInfo, None, IntentModality::Read, vec![0.1; 384]);
    ctx.turn_count_in_topic = 3;

    // Simulate routing to different domain
    ctx.record_turn(Domain::Comms, None, IntentModality::Send, vec![0.2; 384]);
    assert_eq!(ctx.turn_count_in_topic, 1); // reset to 1 for new topic
}

#[test]
fn ctx07_correction_boosts_previous_domain() {
    let mut ctx = RoutingContext::default();
    ctx.record_turn(Domain::SystemInfo, None, IntentModality::Read, vec![0.1; 384]);
    ctx.correction_pending = true;

    let input = DecideInput {
        domain_sims: &[(Domain::Knowledge, 0.5), (Domain::SystemInfo, 0.4)],
        context: &ctx,
        // ... other fields
    };
    let decision = decide(&input);
    // Should route to SystemInfo (correction boost), not Knowledge
    assert!(matches!(decision, RouteDecision::SingleDomain(Domain::SystemInfo)));
}

#[test]
fn ctx08_no_context_preserves_standard_routing() {
    let ctx = RoutingContext::default(); // empty context
    let enriched = enrich_with_context("open Chrome", &ctx);
    assert_eq!(enriched.text, "open Chrome"); // no change
}

#[test]
fn ctx09_serialization_roundtrip() {
    let ctx = RoutingContext {
        last_domain: Some(Domain::FileOps),
        last_tool: Some("read_file".into()),
        turn_count_in_topic: 3,
        ..Default::default()
    };
    let json = serde_json::to_string(&ctx).unwrap();
    let restored: RoutingContext = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.last_domain, Some(Domain::FileOps));
    assert_eq!(restored.turn_count_in_topic, 3);
}

#[test]
fn ctx10_latency_under_budget() {
    let ctx = RoutingContext::default();
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = enrich_with_context("check system status", &ctx);
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(10)); // 1000 enrichments < 10ms
}
```

### 1.8 Phase 1 Checkpoints

- [ ] `routing/context.rs` compiles and all unit tests pass
- [ ] `routing/mod.rs` accepts `RoutingContext` parameter
- [ ] `routing/decide.rs` uses context signals in decision algorithm
- [ ] `agent/turn_gate.rs` maintains `RoutingContext` across turns
- [ ] `agent/loop_engine/mod.rs` rebuilds context from `MemoryStore`
- [ ] All existing tests pass (zero regression)
- [ ] Phase 6 test file passes all 10 tests
- [ ] Benchmark: routing latency <1ms overhead

---

## Phase 2: Fine-Tuned Intent Classifier

**Duration:** 2 weeks  
**Goal:** Replace regex router + old ONNX classifier with fine-tuned small LM  
**Risk:** Medium (model training + integration)  
**Voice latency impact:** ~15–25ms (same slot as current ONNX)

### 2.1 Training Data Pipeline

#### 2.1.1 Data Sources

| Source | Location | Examples |
|--------|----------|----------|
| Regex patterns | `agent/router.rs` | 60+ pattern→tool mappings |
| ONNX corpus | `ai-context/onnx_l0_corpus.jsonl` | 18 examples |
| Test prompts | `TestPrompts.txt` | ~100 prompts |
| VM test prompts | `VMTestPrompts.txt` | ~50 prompts |
| Voice live tests | `tests/voice_live_tests.rs` | ~20 phrases |
| Graph tool nodes | `graphify-out/graph.json` | ~100 tool descriptions |
| Augmented Hinglish | Script-generated | ~500 paraphrases |

**Total estimated training examples: ~2000+**

#### 2.1.2 Training Script

```python
# scripts/train_intent_classifier.py

"""
Fine-tune Qwen2.5-0.5B for KRIA intent classification.
Uses LoRA for parameter-efficient fine-tuning.
Exports to ONNX for CPU inference.
"""

import json
from unsloth import FastLanguageModel

# 1. Load base model
model, tokenizer = FastLanguageModel.from_pretrained(
    model_name="unsloth/Qwen2.5-0.5B-Instruct",
    max_seq_length=128,  # Short inputs only
    load_in_4bit=True,
)

# 2. Apply LoRA
model = FastLanguageModel.get_peft_model(
    model,
    r=16,
    target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
    lora_alpha=16,
    lora_dropout=0,
    bias="none",
)

# 3. Load training data
# Format: {"text": "set volume to 50", "operation": "ConfigureSystem", "compute": "ReflexRust", "hazard": "Green"}
train_data = load_training_data()  # Aggregates all sources above

# 4. Fine-tune
trainer = ...  # Standard SFTTrainer setup
trainer.train()

# 5. Export to ONNX
model.save_pretrained_gguf("models/classifier/intent_v2", tokenizer, quantization_method="q4_k_m")
# Also export ONNX for ort runtime:
export_to_onnx(model, tokenizer, "models/classifier/intent_v2.onnx")
```

#### 2.1.3 Label Taxonomy

```rust
/// Classification labels (matching existing IntentEnvelope types).
pub enum IntentLabel {
    // Operations (from turn_gate.rs)
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
}
```

### 2.2 Backend: `routing/intent_classifier.rs` (NEW)

```rust
//! Fine-tuned intent classifier replacing regex router + ONNX L0.
//!
//! Runs in a dedicated CPU worker thread (same architecture as onnx_classifier.rs).
//! Uses ONNX Runtime via `ort` crate (already a dependency).

use ort::{Session, Value};
use tokenizers::Tokenizer;

pub struct IntentClassifier {
    session: Session,
    tokenizer: Tokenizer,
    label_map: Vec<IntentLabel>,
    context: RoutingContext,
}

pub struct IntentClassification {
    pub operation: Operation,
    pub compute: ComputeClass,
    pub hazard: HazardHint,
    pub confidence: f32,
    pub source: IntentSource,
}

impl IntentClassifier {
    pub fn new(model_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let session = Session::builder()?.commit_from_file(model_path)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)?;
        Ok(Self {
            session,
            tokenizer,
            label_map: Self::build_label_map(),
            context: RoutingContext::default(),
        })
    }

    pub fn classify(&mut self, text: &str, ctx: &RoutingContext) -> Option<IntentClassification> {
        // 1. Tokenize input
        let encoding = self.tokenizer.encode(text, false).ok()?;
        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        // 2. Create ONNX tensors
        let input_tensor = Value::from_array(([1, input_ids.len()], input_ids))?;
        let mask_tensor = Value::from_array(([1, attention_mask.len()], attention_mask))?;

        // 3. Run inference
        let outputs = self.session.run(inputs![input_tensor, mask_tensor]?)?;
        let logits = outputs[0].try_extract_tensor::<f32>()?;

        // 4. Softmax + argmax
        let (best_idx, confidence) = softmax_argmax(logits);
        let label = &self.label_map[best_idx];

        // 5. Map to IntentEnvelope fields
        Some(IntentClassification {
            operation: label.to_operation(),
            compute: label.to_compute_class(),
            hazard: label.to_hazard_hint(),
            confidence,
            source: IntentSource::IntentClassifier,
        })
    }
}
```

### 2.3 Backend: Modify `routing/mod.rs`

```rust
// Replace regex router + ONNX classifier with unified IntentClassifier:
pub struct Router {
    // ... existing fields ...
    intent_classifier: IntentClassifier,  // NEW: replaces router.rs + onnx_classifier.rs
}

impl Router {
    pub async fn route(&self, text: &str, ctx: &RoutingContext) -> (RouteDecision, ModalityResult, RouterTrace) {
        // Stage 1: Context enrichment (Phase 1)
        let enriched = context::enrich_with_context(text, ctx);

        // Stage 2: Verb/modality (unchanged)
        let modality = verbs::classify_modality(enriched.effective_text());

        // Stage 3: Intent classification (Phase 2 - replaces regex + old ONNX)
        let intent_hint = self.intent_classifier.classify(enriched.effective_text(), ctx);

        // Stage 4: Semantic domain routing (unchanged FastEmbed)
        let domain_decision = self.semantic_route(enriched.effective_text()).await;

        // Stage 5: Merge intent_hint + domain_decision
        let final_decision = self.merge_signals(domain_decision, intent_hint, ctx);

        (final_decision, modality, trace)
    }
}
```

### 2.4 Backend: Deprecation Strategy

```rust
// agent/router.rs - Mark as deprecated, keep for fallback
#[deprecated(since = "0.7.0", note = "Use IntentClassifier instead")]
pub struct IntentRouter { ... }

// agent/onnx_classifier.rs - Mark as deprecated
#[deprecated(since = "0.7.0", note = "Use IntentClassifier instead")]
pub struct OnnxClassifier { ... }

// Feature flag for gradual rollout:
// KRIA_ROUTING_V2=1 → use IntentClassifier
// KRIA_ROUTING_V2=0 → use legacy regex + ONNX (default initially)
```

### 2.5 Phase 2 Acceptance Criteria

| # | Criterion | Test |
|---|-----------|------|
| 2.1 | Training data ≥2000 examples, balanced across 16 labels | Data validation script |
| 2.2 | Model accuracy on held-out test set ≥90% | Training evaluation |
| 2.3 | ONNX export inference latency ≤25ms on CPU | Benchmark |
| 2.4 | Hinglish inputs classified correctly ("volume badhao" → ConfigureSystem) | Unit test |
| 2.5 | Paraphrases classified correctly ("how much RAM" → SystemInfo) | Unit test |
| 2.6 | Typos handled ("chekc systm stats" → SystemInfo) | Unit test |
| 2.7 | Feature flag KRIA_ROUTING_V2 toggles between old and new | Integration test |
| 2.8 | All existing tests pass with new classifier | Full test suite |
| 2.9 | Graceful degradation: model file missing → fallback to FastEmbed only | Unit test |

### 2.6 Phase 2 Test Matrix

```rust
// crates/kria-core/tests/phase6_intent_classifier_tests.rs

#[test]
fn ic01_hinglish_system_command() {
    let mut classifier = load_test_classifier();
    let result = classifier.classify("system ki info dikhao", &RoutingContext::default());
    assert_eq!(result.unwrap().operation, Operation::Read);
}

#[test]
fn ic02_hinglish_email_command() {
    let mut classifier = load_test_classifier();
    let result = classifier.classify("email bhejo boss ko", &RoutingContext::default());
    assert_eq!(result.unwrap().operation, Operation::Send);
}

#[test]
fn ic03_paraphrase_memory() {
    let mut classifier = load_test_classifier();
    let result = classifier.classify("what do you remember about my schedule", &RoutingContext::default());
    assert_eq!(result.unwrap().operation, Operation::RetrieveMemory);
}

#[test]
fn ic04_typo_handling() {
    let mut classifier = load_test_classifier();
    let result = classifier.classify("chekc systm stats", &RoutingContext::default());
    assert_eq!(result.unwrap().operation, Operation::Read);
}

#[test]
fn ic05_generate_image() {
    let mut classifier = load_test_classifier();
    let result = classifier.classify("generate image of a sunset over mountains", &RoutingContext::default());
    assert_eq!(result.unwrap().operation, Operation::GenerateImage);
    assert_eq!(result.unwrap().compute, ComputeClass::ImageGpu);
}

#[test]
fn ic06_cancel_command() {
    let mut classifier = load_test_classifier();
    let result = classifier.classify("stop now", &RoutingContext::default());
    assert_eq!(result.unwrap().operation, Operation::Cancel);
}

#[test]
fn ic07_context_enrichment_improves_accuracy() {
    let mut classifier = load_test_classifier();
    let ctx = RoutingContext {
        last_domain: Some(Domain::SystemInfo),
        turn_count_in_topic: 2,
        ..Default::default()
    };
    let result = classifier.classify("also the network", &ctx);
    assert_eq!(result.unwrap().operation, Operation::Read); // context carries
}

#[test]
fn ic08_graceful_fallback_no_model() {
    // Test with missing model file
    let result = IntentClassifier::new(Path::new("/nonexistent/model.onnx"), Path::new("/nonexistent/tokenizer.json"));
    assert!(result.is_err());
    // System should fall back to FastEmbed-only routing
}

#[test]
fn ic09_latency_benchmark() {
    let mut classifier = load_test_classifier();
    let ctx = RoutingContext::default();
    let start = Instant::now();
    for _ in 0..100 {
        let _ = classifier.classify("check system health", &ctx);
    }
    let avg = start.elapsed() / 100;
    assert!(avg < Duration::from_millis(25)); // <25ms per classification
}

#[test]
fn ic10_feature_flag_toggles() {
    // With KRIA_ROUTING_V2=1
    std::env::set_var("KRIA_ROUTING_V2", "1");
    let router = Router::new(config).unwrap();
    assert!(router.uses_intent_classifier());

    // With KRIA_ROUTING_V2=0
    std::env::set_var("KRIA_ROUTING_V2", "0");
    let router = Router::new(config).unwrap();
    assert!(!router.uses_intent_classifier());
}

#[test]
fn ic11_all_operations_covered() {
    let mut classifier = load_test_classifier();
    let test_cases = vec![
        ("hello", Operation::Converse),
        ("open Chrome", Operation::Execute),
        ("delete file test.txt", Operation::Delete),
        ("search web for rust async", Operation::Search),
        ("set alarm for 7am", Operation::Schedule),
        ("take screenshot", Operation::AnalyzeImage),
    ];
    for (input, expected) in test_cases {
        let result = classifier.classify(input, &RoutingContext::default());
        assert_eq!(result.unwrap().operation, expected, "Failed for: {}", input);
    }
}
```

### 2.7 Phase 2 Checkpoints

- [ ] Training data collected and validated (≥2000 examples)
- [ ] Model fine-tuned with LoRA, accuracy ≥90% on test set
- [ ] ONNX export verified, inference ≤25ms on CPU
- [ ] `routing/intent_classifier.rs` compiles and passes unit tests
- [ ] Feature flag `KRIA_ROUTING_V2` works in both directions
- [ ] Graceful degradation: missing model → FastEmbed fallback
- [ ] All existing tests pass with feature flag OFF
- [ ] Phase 6 test file passes all 11 tests
- [ ] Deprecation warnings added to `router.rs` and `onnx_classifier.rs`

---

## Phase 3: Tool-Level Semantic Index

**Duration:** 1 week  
**Goal:** Skip LLM for obvious tool matches (set_volume, check_health, etc.)  
**Risk:** Low (additive layer, existing fallback preserved)  
**Voice latency impact:** -500ms for matched tools

### 3.1 Backend: `routing/tool_index.rs` (NEW)

```rust
//! Tool-level semantic matching index.
//!
//! Pre-computes embeddings for all registered tool descriptions.
//! On query, finds the best matching tool via cosine similarity.
//! If confidence ≥ threshold → direct execution (skip LLM).

use super::embed;

pub struct ToolEmbeddingIndex {
    /// (tool_name, description_embedding, per-tool threshold)
    entries: Vec<ToolEntry>,
    /// Global fallback threshold.
    global_threshold: f32,
}

struct ToolEntry {
    name: String,
    description: String,
    embedding: Vec<f32>,
    threshold: f32,
    /// Minimum hardware tier required.
    min_tier: HardwareTier,
}

pub struct ToolMatch {
    pub name: String,
    pub confidence: f32,
    pub direct_execution: bool,  // true if confidence ≥ threshold
}

impl ToolEmbeddingIndex {
    /// Build index from ToolRegistry.
    pub fn from_registry(registry: &ToolRegistry) -> Result<Self> {
        let mut entries = Vec::new();
        for tool in registry.all_tools() {
            // Generate rich description for embedding
            let desc = format!(
                "{}: {} Parameters: {}",
                tool.name,
                tool.description,
                tool.parameters_summary()
            );
            let embedding = embed::embed_batch(&[&desc])?.pop().unwrap();
            entries.push(ToolEntry {
                name: tool.name.clone(),
                description: desc,
                embedding,
                threshold: 0.85,  // Default, can be overridden per tool
                min_tier: tool.min_tier,
            });
        }
        Ok(Self { entries, global_threshold: 0.85 })
    }

    /// Match query embedding against tool index.
    pub fn match_tool(&self, query_embedding: &[f32], current_tier: HardwareTier) -> Option<ToolMatch> {
        let mut best: Option<ToolMatch> = None;
        for entry in &self.entries {
            // Skip tools above current hardware tier
            if entry.min_tier > current_tier {
                continue;
            }
            let sim = embed::cosine_sim(query_embedding, &entry.embedding);
            let threshold = entry.threshold;
            if sim >= threshold {
                if best.as_ref().map_or(true, |b| sim > b.confidence) {
                    best = Some(ToolMatch {
                        name: entry.name.clone(),
                        confidence: sim,
                        direct_execution: true,
                    });
                }
            }
        }
        best
    }

    /// Rebuild index (called on tool registration/deregistration).
    pub fn rebuild(&mut self, registry: &ToolRegistry) -> Result<()> {
        *self = Self::from_registry(registry)?;
        Ok(())
    }
}
```

### 3.2 Backend: Modify `agent/loop_engine/mod.rs`

```rust
// Add direct-execution fast path:
async fn execute_turn(&self, ...) {
    // ... existing routing ...

    // NEW: Check tool index for direct match
    if let Some(tool_match) = self.tool_index.match_tool(&query_embedding, current_tier) {
        if tool_match.direct_execution {
            // Skip LLM entirely — execute tool directly
            log::info!("Direct tool match: {} (confidence: {:.2})", tool_match.name, tool_match.confidence);
            let result = self.execute_tool_directly(&tool_match.name, &params).await;
            return TurnResult::direct(result);
        }
    }

    // Existing: invoke LLM with narrowed tool set
    // ...
}
```

### 3.3 Backend: Modify `routing/cache.rs`

```rust
// Add tool embedding cache:
pub struct RouterCache {
    // ... existing fields ...
    tool_index: RwLock<ToolEmbeddingIndex>,
}

impl RouterCache {
    pub async fn rebuild_tool_index(&self, registry: &ToolRegistry) {
        let mut index = self.tool_index.write().await;
        if let Err(e) = index.rebuild(registry) {
            warn!("Failed to rebuild tool index: {e}");
        }
    }
}
```

### 3.4 MCP Server Integration

When MCP servers are reconciled, rebuild the tool index:

```rust
// In McpServerManager::reconcile():
pub fn reconcile(&self, ...) {
    // ... existing reconciliation ...
    self.router_cache.rebuild_tool_index(&self.tool_registry).await;
}
```

### 3.5 Phase 3 Acceptance Criteria

| # | Criterion | Test |
|---|-----------|------|
| 3.1 | Tool index builds from ToolRegistry without errors | Unit test |
| 3.2 | "set volume to 50" matches `set_volume` with confidence ≥0.85 | Unit test |
| 3.3 | "check system health" matches `check_system_health` | Unit test |
| 3.4 | Ambiguous queries ("do something") do NOT match above threshold | Unit test |
| 3.5 | Hardware tier filtering works (lite tier skips GPU tools) | Unit test |
| 3.6 | Index rebuilds on MCP tool registration | Integration test |
| 3.7 | Direct execution path skips LLM (verify via trace) | Integration test |
| 3.8 | Fallback to LLM when no tool matches ≥0.85 | Integration test |
| 3.9 | Index build latency <500ms for 100 tools | Benchmark |

### 3.6 Phase 3 Test Matrix

```rust
// crates/kria-core/tests/phase6_tool_index_tests.rs

#[test]
fn ti01_index_builds_from_registry() {
    let registry = build_test_registry();
    let index = ToolEmbeddingIndex::from_registry(&registry).unwrap();
    assert!(index.entries.len() >= 50); // all tools indexed
}

#[test]
fn ti02_volume_command_matches_set_volume() {
    let index = build_test_index();
    let query_emb = embed::embed_batch(&["set volume to 50"]).unwrap().pop().unwrap();
    let result = index.match_tool(&query_emb, HardwareTier::Standard);
    assert_eq!(result.unwrap().name, "set_volume");
    assert!(result.unwrap().direct_execution);
}

#[test]
fn ti03_system_health_match() {
    let index = build_test_index();
    let query_emb = embed::embed_batch(&["check system health"]).unwrap().pop().unwrap();
    let result = index.match_tool(&query_emb, HardwareTier::Lite);
    assert_eq!(result.unwrap().name, "check_system_health");
}

#[test]
fn ti04_ambiguous_no_match() {
    let index = build_test_index();
    let query_emb = embed::embed_batch(&["do something for me"]).unwrap().pop().unwrap();
    let result = index.match_tool(&query_emb, HardwareTier::Standard);
    assert!(result.is_none() || !result.unwrap().direct_execution);
}

#[test]
fn ti05_tier_filtering() {
    let index = build_test_index();
    let query_emb = embed::embed_batch(&["generate image of a cat"]).unwrap().pop().unwrap();
    // Lite tier should not match GPU-intensive tools
    let result = index.match_tool(&query_emb, HardwareTier::Lite);
    assert!(result.is_none());
}

#[test]
fn ti06_index_rebuild_on_new_tool() {
    let mut index = build_test_index();
    let count_before = index.entries.len();
    let mut registry = build_test_registry();
    registry.register(new_test_tool());
    index.rebuild(&registry).unwrap();
    assert_eq!(index.entries.len(), count_before + 1);
}

#[test]
fn ti07_direct_execution_skips_llm() {
    // Integration: verify that direct match produces no LLM call
    let trace = run_routing_with_trace("set brightness to 80");
    assert!(!trace.llm_invoked);
    assert_eq!(trace.tool_executed, Some("set_brightness".to_string()));
}

#[test]
fn ti08_fallback_to_llm() {
    let trace = run_routing_with_trace("summarize the document I sent yesterday");
    assert!(trace.llm_invoked); // too complex for direct match
}

#[test]
fn ti09_build_latency() {
    let registry = build_large_registry(100); // 100 tools
    let start = Instant::now();
    let _index = ToolEmbeddingIndex::from_registry(&registry).unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
}
```

### 3.7 Phase 3 Checkpoints

- [ ] `routing/tool_index.rs` compiles and all unit tests pass
- [ ] Tool index builds from ToolRegistry (≥50 tools)
- [ ] Direct execution path works for high-confidence matches
- [ ] Hardware tier filtering prevents inappropriate direct matches
- [ ] MCP reconciliation triggers index rebuild
- [ ] All existing tests pass (no regression)
- [ ] Phase 6 test file passes all 9 tests
- [ ] Benchmark: index build <500ms, match <1ms

---

## Phase 4: Speculative Pre-Warming

**Duration:** 1 week  
**Goal:** Reduce perceived voice latency by pre-warming resources on partial transcripts  
**Risk:** Medium (resource contention, cancellation race conditions)  
**Voice latency impact:** -200–400ms perceived

### 4.1 Backend: `routing/speculative.rs` (NEW)

```rust
//! Speculative routing on partial voice transcripts.
//!
//! When a partial transcript arrives, predict the most likely routing path
//! and pre-acquire resources (GPU lease, tool serialization).
//! If prediction is wrong, cancel and re-route.

use crate::agent::turn_gate::{ComputeClass, ResourcePlan};

pub struct SpeculativeRouter {
    /// Fast classifier for partial transcripts (lighter than full IntentClassifier).
    fast_classifier: FastClassifier,
    /// Active speculative state.
    active_speculation: Option<SpeculativeState>,
}

struct SpeculativeState {
    /// Predicted resource plan.
    predicted_plan: ResourcePlan,
    /// Pre-acquired GPU lease (if any).
    gpu_lease: Option<GpuLeaseGuard>,
    /// Timestamp of speculation start.
    started_at: Instant,
    /// Confidence of the prediction.
    confidence: f32,
}

impl SpeculativeRouter {
    /// Process a partial transcript and speculate.
    pub fn on_partial(&mut self, partial: &str, confidence: f32) -> SpeculativeAction {
        // Only speculate on high-confidence partials with enough tokens
        if confidence < 0.6 || partial.split_whitespace().count() < 2 {
            return SpeculativeAction::Wait;
        }

        let prediction = self.fast_classifier.predict(partial);
        if prediction.confidence < 0.7 {
            return SpeculativeAction::Wait;
        }

        // Start speculation
        let lease = self.try_acquire_lease(&prediction);
        self.active_speculation = Some(SpeculativeState {
            predicted_plan: prediction.resource_plan,
            gpu_lease: lease,
            started_at: Instant::now(),
            confidence: prediction.confidence,
        });

        SpeculativeAction::Speculating
    }

    /// Confirm or reject speculation when final transcript arrives.
    pub fn on_final(&mut self, final_text: &str) -> SpeculativeResult {
        let state = self.active_speculation.take();
        match state {
            Some(spec) => {
                let actual = self.fast_classifier.predict(final_text);
                if actual.resource_plan.matches(&spec.predicted_plan) {
                    SpeculativeResult::Hit { prewarmed: spec }
                } else {
                    // Wrong prediction — cancel pre-warmed resources
                    spec.cancel();
                    SpeculativeResult::Miss
                }
            }
            None => SpeculativeResult::NoSpeculation,
        }
    }
}
```

### 4.2 Backend: Modify `voice/pipeline.rs`

```rust
// Hook into voice pipeline for partial transcripts:
fn process_partial(&mut self, partial: &PartialTranscript) {
    // ... existing partial handling ...

    // NEW: Speculative routing
    self.speculative_router.on_partial(&partial.text, partial.confidence);
}

fn process_final(&mut self, final_text: &str) {
    // ... existing final handling ...

    // NEW: Check speculation result
    match self.speculative_router.on_final(final_text) {
        SpeculativeResult::Hit { prewarmed } => {
            // Use pre-warmed resources — skip GPU lease acquisition
            self.execute_with_prewarm(prewarmed, final_text).await;
        }
        SpeculativeResult::Miss | SpeculativeResult::NoSpeculation => {
            // Standard path — acquire resources from scratch
            self.execute_standard(final_text).await;
        }
    }
}
```

### 4.3 Phase 4 Acceptance Criteria

| # | Criterion | Test |
|---|-----------|------|
| 4.1 | Partial transcript with ≥2 tokens triggers speculation | Unit test |
| 4.2 | Low-confidence partials (<0.6) do not trigger speculation | Unit test |
| 4.3 | Correct prediction → Hit (pre-warmed resources used) | Unit test |
| 4.4 | Wrong prediction → Miss (resources cancelled) | Unit test |
| 4.5 | GPU lease acquired speculatively is released on miss | Unit test |
| 4.6 | No resource leak on rapid speculation/cancellation cycles | Stress test |
| 4.7 | Voice end-to-end latency reduced by ≥200ms | Benchmark |
| 4.8 | Existing voice tests still pass | Full test suite |

### 4.4 Phase 4 Test Matrix

```rust
// crates/kria-core/tests/phase6_speculative_tests.rs

#[test]
fn sp01_partial_triggers_speculation() {
    let mut router = SpeculativeRouter::new();
    let action = router.on_partial("set volume", 0.8);
    assert!(matches!(action, SpeculativeAction::Speculating));
}

#[test]
fn sp02_low_confidence_waits() {
    let mut router = SpeculativeRouter::new();
    let action = router.on_partial("set", 0.3);
    assert!(matches!(action, SpeculativeAction::Wait));
}

#[test]
fn sp03_short_partial_waits() {
    let mut router = SpeculativeRouter::new();
    let action = router.on_partial("set", 0.9); // only 1 word
    assert!(matches!(action, SpeculativeAction::Wait));
}

#[test]
fn sp04_correct_prediction_hit() {
    let mut router = SpeculativeRouter::new();
    router.on_partial("set volume to", 0.8);
    let result = router.on_final("set volume to 50");
    assert!(matches!(result, SpeculativeResult::Hit { .. }));
}

#[test]
fn sp05_wrong_prediction_miss() {
    let mut router = SpeculativeRouter::new();
    router.on_partial("set volume", 0.8);
    let result = router.on_final("send email to boss"); // completely different
    assert!(matches!(result, SpeculativeResult::Miss));
}

#[test]
fn sp06_no_speculation_still_works() {
    let mut router = SpeculativeRouter::new();
    let result = router.on_final("check system health");
    assert!(matches!(result, SpeculativeResult::NoSpeculation));
}

#[test]
fn sp07_gpu_lease_released_on_miss() {
    let mut router = SpeculativeRouter::new();
    router.on_partial("generate image", 0.9);
    router.on_final("check system health"); // miss
    assert!(!router.has_active_lease()); // lease released
}

#[test]
fn sp08_rapid_speculation_no_leak() {
    let mut router = SpeculativeRouter::new();
    for i in 0..100 {
        router.on_partial(&format!("partial {}", i), 0.8);
        router.on_final("something else");
    }
    assert!(!router.has_active_lease()); // no leak
}

#[test]
fn sp09_latency_improvement() {
    // Benchmark: measure end-to-end with and without speculation
    let with_spec = measure_voice_latency(true);
    let without_spec = measure_voice_latency(false);
    assert!(without_spec - with_spec >= Duration::from_millis(200));
}
```

### 4.5 Phase 4 Checkpoints

- [ ] `routing/speculative.rs` compiles and all unit tests pass
- [ ] Partial transcript hook works in voice pipeline
- [ ] GPU lease pre-acquisition + release works correctly
- [ ] No resource leaks under rapid speculation cycles
- [ ] All existing voice tests pass
- [ ] Phase 6 test file passes all 9 tests
- [ ] Benchmark: ≥200ms latency improvement

---

## Phase 5: Online Learning Feedback Loop

**Duration:** 2 weeks (initial) + ongoing  
**Goal:** Router accuracy improves continuously from user behavior  
**Risk:** Low (additive, doesn't change routing logic directly)  
**Voice latency impact:** 0ms (learning is async)

### 5.1 Backend: `routing/feedback.rs` (NEW)

```rust
//! Online learning feedback collection and centroid adjustment.
//!
//! Collects routing outcomes from user behavior signals.
//! Periodically adjusts domain centroids and tool thresholds.

use std::collections::HashMap;
use crate::routing::domain::Domain;

pub struct FeedbackCollector {
    /// Pending feedback entries (written per-turn).
    buffer: Vec<RoutingFeedback>,
    /// Persisted feedback history.
    history: Vec<RoutingFeedback>,
    /// Maximum buffer size before flush.
    max_buffer: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingFeedback {
    pub input_text_hash: u64,
    pub domain_selected: Domain,
    pub tool_selected: Option<String>,
    pub intent_source: IntentSource,
    pub confidence: f32,
    pub outcome: RoutingOutcome,
    pub timestamp: i64,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingOutcome {
    /// Tool executed successfully, user moved on.
    Success,
    /// User rephrased the same request (routing was wrong).
    Rephrased,
    /// User explicitly corrected the routing ("no, I meant X").
    Corrected { correct_domain: Domain, correct_tool: Option<String> },
    /// User barged in during response (wrong tool or wrong style).
    BargedIn,
    /// HITL denied the action.
    HitlDenied,
    /// Tool execution failed.
    ToolError { error: String },
    /// Unknown outcome (no signal detected).
    Unknown,
}

impl FeedbackCollector {
    pub fn new() -> Self { ... }

    /// Record a routing feedback entry.
    pub fn record(&mut self, feedback: RoutingFeedback) {
        self.buffer.push(feedback);
        if self.buffer.len() >= self.max_buffer {
            self.flush_to_disk();
        }
    }

    /// Flush buffer to persistent storage.
    fn flush_to_disk(&self) {
        // Append to ~/.kria/feedback/routing_feedback.jsonl
    }

    /// Load all historical feedback.
    pub fn load_history(&self) -> Vec<RoutingFeedback> {
        // Read from ~/.kria/feedback/routing_feedback.jsonl
    }
}
```

### 5.2 Backend: Nightly Centroid Adjustment

```rust
/// Adjust domain centroids based on feedback.
/// Run as a background task (nightly or on-demand).
pub fn adjust_centroids(
    feedback: &[RoutingFeedback],
    centroids: &mut HashMap<Domain, Vec<f32>>,
    learning_rate: f32,
) -> CentroidAdjustmentReport {
    let mut report = CentroidAdjustmentReport::default();

    for entry in feedback {
        match &entry.outcome {
            RoutingOutcome::Success => {
                // Pull centroid slightly toward this successful embedding
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, learning_rate);
                    report.success_nudges += 1;
                }
            }
            RoutingOutcome::Corrected { correct_domain, .. } => {
                // Push away from wrong domain
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, -learning_rate * 2.0);
                    report.correction_pushes += 1;
                }
                // Pull toward correct domain
                if let Some(centroid) = centroids.get_mut(correct_domain) {
                    nudge_centroid(centroid, &entry.embedding, learning_rate * 2.0);
                    report.correction_pulls += 1;
                }
            }
            RoutingOutcome::Rephrased => {
                // Weak negative signal
                if let Some(centroid) = centroids.get_mut(&entry.domain_selected) {
                    nudge_centroid(centroid, &entry.embedding, -learning_rate * 0.5);
                    report.rephrase_pushes += 1;
                }
            }
            _ => {} // BargedIn, HitlDenied, ToolError, Unknown — no centroid change
        }
    }

    report
}

fn nudge_centroid(centroid: &mut Vec<f32>, embedding: &[f32], rate: f32) {
    for (c, e) in centroid.iter_mut().zip(embedding.iter()) {
        *c += rate * (*e - *c);
    }
    // Re-normalize
    let norm: f32 = centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        centroid.iter_mut().for_each(|x| *x /= norm);
    }
}
```

### 5.3 Backend: Outcome Signal Detection

```rust
/// Detect routing outcomes from user behavior.
pub fn detect_outcome(
    current_turn: &Turn,
    next_turn_text: Option<&str>,
    hitl_decision: Option<&HitlDecision>,
) -> RoutingOutcome {
    // Check HITL decision
    if let Some(HitlDecision::Denied) = hitl_decision {
        return RoutingOutcome::HitlDenied;
    }

    // Check if user rephrased (similar meaning, different words)
    if let Some(next_text) = next_turn_text {
        if is_rephrase(&current_turn.user_text, next_text) {
            return RoutingOutcome::Rephrased;
        }
        // Check for explicit correction
        if CORRECTION_RE.is_match(next_text) {
            return RoutingOutcome::Corrected {
                correct_domain: Domain::Unknown, // resolved by context
                correct_tool: None,
            };
        }
    }

    // Check tool execution result
    if current_turn.tool_error.is_some() {
        return RoutingOutcome::ToolError {
            error: current_turn.tool_error.clone().unwrap(),
        };
    }

    // Default: success
    RoutingOutcome::Success
}
```

### 5.4 Phase 5 Acceptance Criteria

| # | Criterion | Test |
|---|-----------|------|
| 5.1 | Feedback entries are recorded correctly | Unit test |
| 5.2 | Buffer flushes to disk at capacity | Unit test |
| 5.3 | Success nudges centroid toward successful embedding | Unit test |
| 5.4 | Correction pushes away from wrong domain | Unit test |
| 5.5 | Correction pulls toward correct domain | Unit test |
| 5.6 | Centroid normalization preserved after adjustment | Unit test |
| 5.7 | Rephrase detection works ("same thing again" pattern) | Unit test |
| 5.8 | Feedback file is append-only, no data loss | Integration test |
| 5.9 | A/B test: adjusted centroids outperform original | Statistical test |

### 5.5 Phase 5 Test Matrix

```rust
// crates/kria-core/tests/phase6_feedback_tests.rs

#[test]
fn fb01_record_feedback() {
    let mut collector = FeedbackCollector::new();
    collector.record(RoutingFeedback {
        domain_selected: Domain::SystemInfo,
        outcome: RoutingOutcome::Success,
        ..default_feedback()
    });
    assert_eq!(collector.buffer.len(), 1);
}

#[test]
fn fb02_flush_at_capacity() {
    let mut collector = FeedbackCollector { max_buffer: 5, ..FeedbackCollector::new() };
    for _ in 0..6 {
        collector.record(default_feedback());
    }
    assert_eq!(collector.buffer.len(), 1); // 5 flushed, 1 remaining
}

#[test]
fn fb03_success_nudge() {
    let mut centroids = test_centroids();
    let original = centroids[&Domain::SystemInfo].clone();
    let feedback = vec![RoutingFeedback {
        domain_selected: Domain::SystemInfo,
        outcome: RoutingOutcome::Success,
        embedding: vec![0.5; 384],
        ..default_feedback()
    }];
    adjust_centroids(&feedback, &mut centroids, 0.01);
    assert_ne!(centroids[&Domain::SystemInfo], original); // centroid moved
}

#[test]
fn fb04_correction_push_pull() {
    let mut centroids = test_centroids();
    let feedback = vec![RoutingFeedback {
        domain_selected: Domain::Knowledge,  // wrong domain
        outcome: RoutingOutcome::Corrected {
            correct_domain: Domain::SystemInfo,
            correct_tool: Some("check_system_health".into()),
        },
        embedding: vec![0.3; 384],
        ..default_feedback()
    }];
    adjust_centroids(&feedback, &mut centroids, 0.01);
    // Knowledge pushed away, SystemInfo pulled toward
    // (verify via cosine similarity comparison)
}

#[test]
fn fb05_normalization_preserved() {
    let mut centroids = test_centroids();
    for _ in 0..100 {
        adjust_centroids(&[default_feedback()], &mut centroids, 0.01);
    }
    for (_, centroid) in &centroids {
        let norm: f32 = centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001); // still L2-normalized
    }
}

#[test]
fn fb06_rephrase_detection() {
    assert!(is_rephrase("check system status", "show me system status"));
    assert!(!is_rephrase("check system status", "send email to boss"));
}

#[test]
fn fb07_correction_detection() {
    let outcome = detect_outcome(
        &turn_with_domain(Domain::Knowledge),
        Some("no I meant the system info"),
        None,
    );
    assert!(matches!(outcome, RoutingOutcome::Corrected { .. }));
}

#[test]
fn fb08_hitl_denial() {
    let outcome = detect_outcome(
        &default_turn(),
        None,
        Some(&HitlDecision::Denied),
    );
    assert!(matches!(outcome, RoutingOutcome::HitlDenied));
}

#[test]
fn fb09_ab_test_adjusted_beats_original() {
    let original_centroids = test_centroids();
    let mut adjusted_centroids = test_centroids();
    let feedback = generate_synthetic_feedback(1000);
    adjust_centroids(&feedback, &mut adjusted_centroids, 0.01);

    let test_prompts = load_test_prompts();
    let original_accuracy = evaluate_accuracy(&original_centroids, &test_prompts);
    let adjusted_accuracy = evaluate_accuracy(&adjusted_centroids, &test_prompts);
    assert!(adjusted_accuracy >= original_accuracy); // adjusted should be >=
}
```

### 5.6 Phase 5 Checkpoints

- [ ] `routing/feedback.rs` compiles and all unit tests pass
- [ ] Feedback buffer flushes to disk correctly
- [ ] Centroid adjustment mathematically correct
- [ ] Normalization preserved after adjustments
- [ ] Outcome signal detection works for all signal types
- [ ] A/B test shows adjusted centroids ≥ original accuracy
- [ ] All existing tests pass (no regression)
- [ ] Phase 6 test file passes all 9 tests

---

## 9. Frontend Changes

### 9.1 `ui/src/components/RoutingDebug.tsx` (NEW — Phase 1)

Debug panel showing real-time routing decisions:

```tsx
interface RoutingDebugProps {
  trace: RouterTrace;
}

export function RoutingDebug({ trace }: RoutingDebugProps) {
  return (
    <div className="routing-debug-panel">
      <h3>Routing Decision</h3>
      <div className="trace-grid">
        <span>Decision:</span> <code>{trace.decision}</code>
        <span>Domains:</span> <code>{trace.top_domains.map(([d, s]) => `${d}(${s.toFixed(2)})`).join(', ')}</code>
        <span>Modality:</span> <code>{trace.primary_modality}</code>
        <span>Destructive:</span> <code>{trace.destructive ? '⚠️ YES' : 'NO'}</code>
        <span>Segments:</span> <code>{trace.segments.length}</code>
        <span>Tools:</span> <code>{trace.selected_tools.join(', ')}</code>
        <span>Latency:</span> <code>{trace.latency_ms}ms</code>
        <span>Source:</span> <code>{trace.intent_source}</code>
      </div>
      {trace.context_enrichment && (
        <div className="context-info">
          <span>Context:</span> <code>{trace.context_enrichment.reason}</code>
        </div>
      )}
    </div>
  );
}
```

### 9.2 `ui/src/stores/routingStore.ts` (NEW — Phase 1)

```typescript
import { create } from 'zustand';

interface RoutingState {
  lastTrace: RouterTrace | null;
  routingHistory: RouterTrace[];
  feedbackPending: RoutingFeedback | null;

  setTrace: (trace: RouterTrace) => void;
  submitFeedback: (feedback: RoutingFeedback) => void;
  clearHistory: () => void;
}

export const useRoutingStore = create<RoutingState>((set) => ({
  lastTrace: null,
  routingHistory: [],
  feedbackPending: null,

  setTrace: (trace) => set((state) => ({
    lastTrace: trace,
    routingHistory: [...state.routingHistory.slice(-49), trace],
  })),

  submitFeedback: (feedback) => {
    // Send to backend API
    fetch('/api/routing/feedback', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(feedback),
    });
    set({ feedbackPending: null });
  },

  clearHistory: () => set({ routingHistory: [] }),
}));
```

### 9.3 `ui/src/components/RoutingFeedback.tsx` (NEW — Phase 5)

User-facing correction UI:

```tsx
export function RoutingFeedback({ trace }: { trace: RouterTrace }) {
  const submitFeedback = useRoutingStore((s) => s.submitFeedback);

  return (
    <div className="routing-feedback">
      <p>Routed to: <strong>{trace.decision}</strong></p>
      <div className="feedback-buttons">
        <button onClick={() => submitFeedback({
          outcome: 'Success',
          domain_selected: trace.top_domains[0][0],
        })}>✅ Correct</button>

        <button onClick={() => submitFeedback({
          outcome: 'Rephrased',
          domain_selected: trace.top_domains[0][0],
        })}>🔄 Wrong, let me rephrase</button>
      </div>
    </div>
  );
}
```

### 9.4 Frontend Phase Checklist

- [ ] Phase 1: `RoutingDebug.tsx` component created
- [ ] Phase 1: `routingStore.ts` created
- [ ] Phase 1: Debug panel visible in dev mode
- [ ] Phase 5: `RoutingFeedback.tsx` component created
- [ ] Phase 5: Feedback submission API endpoint works
- [ ] All frontend tests pass

---

## 10. Server API Changes

### 10.1 New Endpoints

```rust
// crates/kria-server/src/routes.rs — Add routing feedback endpoint:

/// POST /api/routing/feedback
/// Submit routing feedback for online learning.
async fn routing_feedback(
    State(state): State<Arc<ServerState>>,
    Json(feedback): Json<RoutingFeedback>,
) -> Json<serde_json::Value> {
    state.routing_feedback.record(feedback);
    Json(serde_json::json!({ "status": "recorded" }))
}

/// GET /api/routing/trace/:session_id
/// Get routing traces for a session (debug).
async fn routing_trace(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
) -> Json<Vec<RouterTrace>> {
    let traces = state.routing_traces.get(&session_id).unwrap_or_default();
    Json(traces)
}

// Register routes in api_routes():
pub fn api_routes() -> Router<Arc<ServerState>> {
    Router::new()
        // ... existing routes ...
        .route("/api/routing/feedback", post(routing_feedback))
        .route("/api/routing/trace/{session_id}", get(routing_trace))
}
```

### 10.2 Server Phase Checklist

- [ ] `POST /api/routing/feedback` endpoint works
- [ ] `GET /api/routing/trace/:session_id` endpoint works
- [ ] Integration tests for new endpoints
- [ ] CORS configuration for frontend access

---

## 11. Test Strategy

### 11.1 Test File Layout

```
crates/kria-core/tests/
├── phase6_routing_context_tests.rs      ← Phase 1 (10 tests)
├── phase6_intent_classifier_tests.rs    ← Phase 2 (11 tests)
├── phase6_tool_index_tests.rs           ← Phase 3 (9 tests)
├── phase6_speculative_tests.rs          ← Phase 4 (9 tests)
├── phase6_feedback_tests.rs             ← Phase 5 (9 tests)
└── phase6_routing_integration_tests.rs  ← Cross-phase (10 tests)

crates/kria-server/tests/
└── routing_api_tests.rs                 ← API tests (5 tests)

ui/src/test/
└── routingComponents.test.tsx           ← Frontend tests (6 tests)
```

### 11.2 Cross-Phase Integration Tests

```rust
// crates/kria-core/tests/phase6_routing_integration_tests.rs

#[test]
fn ri01_full_pipeline_context_to_direct_execution() {
    // Phase 1 + 3: Context carries domain → tool index matches directly
    let ctx = RoutingContext {
        last_domain: Some(Domain::SystemInfo),
        turn_count_in_topic: 2,
        ..Default::default()
    };
    let result = route_with_full_pipeline("also the disk", &ctx);
    assert!(!result.llm_invoked); // direct execution via tool index
    assert_eq!(result.tool_executed, Some("get_disk_space".to_string()));
}

#[test]
fn ri02_hinglish_with_context_and_tool_match() {
    // Phase 1 + 2 + 3: Hinglish + context + direct execution
    let ctx = RoutingContext {
        last_domain: Some(Domain::Power),
        turn_count_in_topic: 1,
        ..Default::default()
    };
    let result = route_with_full_pipeline("aur brightness badha do", &ctx);
    assert_eq!(result.tool_executed, Some("set_brightness".to_string()));
}

#[test]
fn ri03_correction_re_routes_correctly() {
    // Phase 1 + 2: Correction detected → re-routes with context
    let ctx = RoutingContext {
        last_domain: Some(Domain::FileOps),
        correction_pending: true,
        ..Default::default()
    };
    let result = route_with_full_pipeline("no I meant the other file", &ctx);
    assert_eq!(result.domain, Domain::FileOps); // correction preserved
}

#[test]
fn ri04_speculation_with_context() {
    // Phase 1 + 4: Context + speculation on partial transcript
    let ctx = RoutingContext {
        last_domain: Some(Domain::Comms),
        turn_count_in_topic: 1,
        ..Default::default()
    };
    let partial_result = speculate_with_context("send email", 0.8, &ctx);
    assert!(matches!(partial_result, SpeculativeAction::Speculating));
}

#[test]
fn ri05_feedback_from_full_pipeline() {
    // Phase 1 + 2 + 5: Routing → feedback recording
    let result = route_with_full_pipeline("check system health", &RoutingContext::default());
    let feedback = detect_outcome_from_result(&result, RoutingOutcome::Success);
    assert_eq!(feedback.domain_selected, Domain::SystemInfo);
}

#[test]
fn ri06_fallback_chain_preserved() {
    // All phases: graceful degradation at each layer
    // Test with: no model file, no tool index, stale context
    let result = route_with_degraded_pipeline("open Chrome");
    assert!(result.llm_invoked); // falls back to LLM
    assert_eq!(result.domain, Domain::AppLifecycle);
}

#[test]
fn ri07_latency_full_pipeline() {
    // End-to-end latency with all phases active
    let start = Instant::now();
    let _ = route_with_full_pipeline("set volume to 50", &RoutingContext::default());
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(50)); // <50ms for direct execution
}

#[test]
fn ri08_voice_hinglish_end_to_end() {
    // Full voice flow simulation: partial → final → execution
    let mut pipeline = VoiceRoutingPipeline::new();
    pipeline.on_partial("volume", 0.7);
    pipeline.on_partial("volume badhao", 0.9);
    let result = pipeline.on_final("volume badhao");
    assert_eq!(result.tool_executed, Some("set_volume".to_string()));
    assert!(result.latency < Duration::from_millis(100));
}

#[test]
fn ri09_multi_turn_conversation_accuracy() {
    // 5-turn conversation test
    let turns = vec![
        ("check system status", Domain::SystemInfo),
        ("also the disk", Domain::SystemInfo),      // context carry
        ("no I meant network", Domain::SystemInfo), // correction
        ("what about memory", Domain::SystemInfo),  // continuation
        ("now send email to boss", Domain::Comms),  // topic switch
    ];
    let mut ctx = RoutingContext::default();
    for (text, expected_domain) in turns {
        let result = route_with_full_pipeline(text, &ctx);
        assert_eq!(result.domain, expected_domain, "Failed for: {}", text);
        ctx = result.updated_context;
    }
}

#[test]
fn ri10_no_regression_vs_baseline() {
    // Run same 100 test prompts through old and new pipeline
    let test_prompts = load_test_prompts();
    let old_results = test_prompts.iter().map(|p| route_with_legacy_pipeline(p)).collect::<Vec<_>>();
    let new_results = test_prompts.iter().map(|p| route_with_full_pipeline(p, &RoutingContext::default())).collect::<Vec<_>>();

    let old_accuracy = evaluate_accuracy(&old_results);
    let new_accuracy = evaluate_accuracy(&new_results);
    assert!(new_accuracy >= old_accuracy); // must be equal or better
}
```

### 11.3 Test Execution Commands

```bash
# Run all Phase 6 routing tests
cargo test -p kria-core --test phase6_routing_context_tests
cargo test -p kria-core --test phase6_intent_classifier_tests
cargo test -p kria-core --test phase6_tool_index_tests
cargo test -p kria-core --test phase6_speculative_tests
cargo test -p kria-core --test phase6_feedback_tests
cargo test -p kria-core --test phase6_routing_integration_tests

# Run server routing API tests
cargo test -p kria-server --test routing_api_tests

# Run all routing-related tests (including existing)
cargo test -p kria-core routing
cargo test -p kria-core router
cargo test -p kria-core intent
cargo test -p kria-core turn_gate

# Run frontend tests
cd ui && npm test -- --testPathPattern=routing

# Run benchmarks
cargo test -p kria-core --test phase6_routing_integration_tests -- --nocapture ri07
```

---

## 12. Rollout & Rollback Plan

### 12.1 Feature Flags

```toml
# kria_config.toml

[routing]
# Phase 1: Context-aware routing
context_enabled = true          # Default: true (safe, no model dependency)

# Phase 2: Intent classifier
intent_classifier = false       # Default: false (requires model file)
intent_classifier_path = "models/classifier/intent_v2.onnx"

# Phase 3: Tool semantic index
tool_index_enabled = true       # Default: true (safe, additive layer)
tool_index_threshold = 0.85     # Confidence threshold for direct execution

# Phase 4: Speculative pre-warming
speculative_enabled = false     # Default: false (voice-only, needs testing)

# Phase 5: Online learning
feedback_enabled = true         # Default: true (safe, async)
feedback_learning_rate = 0.01   # Centroid adjustment rate
```

### 12.2 Rollout Phases

| Phase | Rollout | Rollback |
|-------|---------|----------|
| 1 | `context_enabled = true` (default on) | Set to `false` |
| 2 | `intent_classifier = true` (opt-in) | Set to `false` → falls back to regex + FastEmbed |
| 3 | `tool_index_enabled = true` (default on) | Set to `false` → always use LLM |
| 4 | `speculative_enabled = true` (opt-in, voice-only) | Set to `false` |
| 5 | `feedback_enabled = true` (default on) | Set to `false` → stop learning |

### 12.3 Rollback Checklist

- [ ] Feature flag immediately disables the phase
- [ ] No persistent state corruption on rollback
- [ ] All tests pass with phase disabled
- [ ] Routing degrades gracefully (not crashes)

---

## 13. Risk Matrix

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Model file missing/corrupt | High | Low | Graceful fallback to FastEmbed-only |
| Tool index stale after MCP update | Medium | Medium | Auto-rebuild on MCP reconcile |
| Speculation causes GPU contention | Medium | Low | Bounded speculation, cancel on miss |
| Feedback loop introduces drift | Medium | Low | A/B test before promoting weights |
| Context enrichment changes OOD behavior | Low | Medium | OOD check runs BEFORE context enrichment |
| Regression in existing routing | High | Low | Full test suite runs on every phase |

---

## 14. Acceptance Criteria

### 14.1 Global (All Phases)

| # | Criterion |
|---|-----------|
| G1 | All existing tests pass (zero regression) |
| G2 | Routing latency ≤50ms for direct execution |
| G3 | Routing latency ≤100ms for LLM-assisted routing |
| G4 | No resource leaks (GPU leases, file handles) |
| G5 | Feature flags work in both directions |
| G6 | Graceful degradation at every layer |
| G7 | `RouterTrace` captures all routing decisions |
| G8 | Hinglish inputs routed correctly ≥90% |
| G9 | Multi-turn context works across ≥3 turns |
| G10 | LLM invocation rate drops to ≤40% of turns |

### 14.2 Per-Phase Summary

| Phase | Tests | Duration | Key Metric |
|-------|-------|----------|------------|
| 1: Context | 10 | 1 week | Multi-turn accuracy ≥85% |
| 2: Intent Classifier | 11 | 2 weeks | Single-turn accuracy ≥90% |
| 3: Tool Index | 9 | 1 week | Direct execution rate ≥60% |
| 4: Speculative | 9 | 1 week | Voice latency -200ms |
| 5: Feedback | 9 | 2 weeks | Accuracy improves ≥2%/week |
| Integration | 10 | 1 week | End-to-end accuracy ≥92% |
| **Total** | **58** | **~8 weeks** | **92%+ routing accuracy** |

---

## Appendix A: Configuration Reference

```toml
# Full routing configuration (kria_config.toml)

[routing]
context_enabled = true
context_stale_secs = 60
context_min_enrich_length = 30

intent_classifier = false
intent_classifier_path = "models/classifier/intent_v2.onnx"
intent_classifier_tokenizer_path = "models/classifier/tokenizer.json"
intent_classifier_timeout_ms = 25

tool_index_enabled = true
tool_index_threshold = 0.85
tool_index_rebuild_on_mcp_change = true

speculative_enabled = false
speculative_min_confidence = 0.7
speculative_min_tokens = 2

feedback_enabled = true
feedback_learning_rate = 0.01
feedback_max_buffer = 1000
feedback_path = "~/.kria/feedback/routing_feedback.jsonl"
feedback_adjustment_interval_hours = 24
```

## Appendix B: Dependency Changes

```toml
# crates/kria-core/Cargo.toml — additions

[dependencies]
# Phase 2: ONNX runtime (already present, no change)
ort = { version = "2.0", features = ["load-dynamic"] }
tokenizers = "0.19"

# Phase 5: Feedback persistence
tokio-cron-scheduler = "0.10"  # For nightly centroid adjustment
```

## Appendix C: Migration from Legacy Router

```rust
// Deprecation timeline:
// v0.7.0: Mark router.rs and onnx_classifier.rs as deprecated
// v0.8.0: Default to IntentClassifier (KRIA_ROUTING_V2=1)
// v0.9.0: Remove legacy router.rs and onnx_classifier.rs

// During transition, both paths are available:
#[cfg(feature = "routing_v2")]
pub use intent_classifier::IntentClassifier as ActiveClassifier;

#[cfg(not(feature = "routing_v2"))]
pub use router::IntentRouter as ActiveClassifier;
```
