# KRIA Tool System

> **Last Updated:** 2026-05-11
> **Status:** Production

---

## Executive Summary

KRIA's tool system provides 60+ capabilities through three execution layers: **Native Rust tools**, **MCP server tools**, and **OpenClaw sandboxed skills**. All tools flow through a unified `ToolRegistry` with consistent policy enforcement, audit logging, and human-in-the-loop approval.

---

## Tool Taxonomy

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         ToolRegistry                                    │
│                                                                         │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────────────────┐ │
│  │ Native Rust  │  │ MCP Servers  │  │  OpenClaw Skill Substrate     │ │
│  │ Tools (60+)  │  │ (existing)   │  │  (sandboxed, isolated)         │ │
│  │              │  │              │  │                               │ │
│  │ file_ops     │  │ fs           │  │  oc_web_search                │ │
│  │ shell        │  │ gworkspace   │  │  oc_code_sandbox              │ │
│  │ browser      │  │ colab-mcp    │  │  oc_<community_skill>         │ │
│  │ packages     │  │ ...          │  │                               │ │
│  │ system_info  │  │              │  │                               │ │
│  │ ...          │  │              │  │                               │ │
│  └──────────────┘  └──────────────┘  └───────────────────────────────┘ │
│                                                                         │
│  Priority: Native > MCP > OpenClaw                                      │
└─────────────────────────────────────────────────────────────────────────┘
```

### Priority Order

1. **Native Rust tools** — Highest trust, lowest latency, full policy integration
2. **MCP server tools** — Semi-trusted, external processes, capability-scoped
3. **OpenClaw skills** — Untrusted, sandboxed in Docker containers

---

## Tool Definition Schema

```rust
pub struct ToolDef {
    pub name: String,           // snake_case, verb-first
    pub description: String,    // LLM-visible description (≤140 chars)
    pub category: String,       // Grouping for UI and bulk operations
    pub parameters: Vec<ParamDef>,
    pub default_tier: RiskLevel, // Green | Yellow | Red | Black
    pub min_tier: &'static str, // lite | standard | performance | high
}

pub struct ParamDef {
    pub name: String,
    pub param_type: String,     // string | number | boolean | array | object
    pub description: String,
    pub required: bool,
    pub default: Option<serde_json::Value>,
}
```

---

## Tool Handler Interface

```rust
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn execute(&self, params: serde_json::Value) -> ToolResult;
}

pub struct ToolResult {
    pub success: bool,
    pub data: serde_json::Value,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(data: serde_json::Value) -> Self { ... }
    pub fn err(message: impl Into<String>) -> Self { ... }
    pub fn ok_text(message: &str) -> Self { ... }
}
```

---

## Safety Tiers

| Tier | Set | Behavior | Examples |
|------|-----|----------|----------|
| **Green** | `GREEN_ACTIONS` | Auto-execute, no approval | `get_cpu_usage`, `list_files`, `web_search` |
| **Yellow** | `YELLOW_ACTIONS` | Execute + notify user (post-hoc) | `set_volume`, `write_file`, `send_email` |
| **Red** | `RED_ACTIONS` | Block, require PIN approval | `delete_file`, `install_package`, `shutdown` |
| **Black** | `BLACK_ACTIONS` | Always denied | `rm -rf /`, writing to `/etc`, reading `~/.ssh/id_rsa` |

### Policy Evaluation

```rust
impl PolicyEngine {
    pub fn evaluate(&self, tool: &str, params: &Value) -> RiskLevel {
        // 1. Check BLACK_ACTIONS (always deny)
        // 2. Check parameter-dependent rules (path-based, scope-based)
        // 3. Check static tier sets (GREEN/YELLOW/RED)
        // 4. Default to Yellow (conservative)
    }
}
```

---

## Semantic Tool Injection

KRIA uses semantic similarity to inject relevant tools into the LLM context:

```rust
// In loop_engine/mod.rs
let matches = tool_index
    .top_k_by_text(&round_focus_text, 3, &self.hardware_tier)
    .await;

// Filter by minimum similarity threshold
let injections: Vec<SemanticInjection> = matches
    .into_iter()
    .filter(|m| m.confidence >= 0.35)  // Prevent irrelevant tools
    .map(|m| SemanticInjection { ... })
    .collect();
```

This ensures the LLM only sees tools relevant to the current conversation focus.

---

## Adding a New Tool

### Step 1: Implement Handler

Create in `crates/kria-core/src/tools/<category>.rs`:

```rust
struct GetThing;

#[async_trait]
impl ToolHandler for GetThing {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        // 1. Validate params
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return ToolResult::err("`id` is required (string)"),
        };

        // 2. Do the work (async only)
        let value = fetch_thing(&id).await?;

        // 3. Return structured JSON
        ToolResult::ok(serde_json::json!({
            "id": id,
            "value": value,
        }))
    }
}
```

**Hard rules:**
- Never `panic!` or `unwrap()` on user input
- Never block the async runtime (use `tokio::fs`, `tokio::process`)
- Return `ToolResult::err()` for all failures

### Step 2: Register Tool

```rust
pub fn register(reg: &ToolRegistry) {
    reg.register(
        ToolDef {
            name: "get_thing".into(),
            description: "Fetch a Thing by id. Returns id and value.".into(),
            category: "things".into(),
            parameters: vec![
                ParamDef {
                    name: "id".into(),
                    param_type: "string".into(),
                    description: "Identifier of the Thing".into(),
                    required: true,
                    default: None,
                },
            ],
            default_tier: RiskLevel::Green,
            min_tier: "lite",
        },
        Arc::new(GetThing),
    );
}
```

### Step 3: Add to Policy

In `crates/kria-core/src/safety/policy.rs`:

```rust
static GREEN_ACTIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut s = HashSet::new();
    s.extend([
        // ... existing tools
        "get_thing",  // Add here
    ]);
    s
});
```

### Step 4: Add Router Rule (Recommended)

In `crates/kria-core/src/agent/router.rs`:

```rust
static DIRECT_TOOL_RE: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    vec![
        // ... existing rules
        (Regex::new(r"(?i)\b(get|fetch|show)\s+thing\s+\w+").unwrap(), "get_thing"),
    ]
});
```

### Step 5: Add Regression Test

In `crates/kria-core/tests/test_chat_regression.rs`:

```rust
#[test]
fn reg_get_thing_routes_correctly() {
    use kria_core::agent::router::{Intent, IntentRouter};
    for prompt in ["get thing 42", "show me thing abc", "fetch thing xyz"] {
        match IntentRouter::classify(prompt).intent {
            Intent::DirectTool(t) => assert_eq!(t, "get_thing"),
            other => panic!("'{prompt}' must route to get_thing, got {other:?}"),
        }
    }
}
```

---

## Output Shape Guidelines

| Tool Kind | Shape | Why |
|-----------|-------|-----|
| Single fact | `{ "key": "value" }` flat object | Direct access |
| List | `{ "total": N, "items": [...] }` | Summarizer detects array |
| Aggregate | `{ "cpu": {...}, "memory": {...} }` | Multi-section dashboard |
| Failure | `ToolResult::err("user-friendly reason")` | No stack traces |
| Multi-step | `{ "step": 2, "of": 5, "status": "..." }` | Progress tracking |

**Anti-patterns (data lost):**
- `ToolResult::ok(Value::Null)`
- `ToolResult::ok(json!({}))`
- `ToolResult::ok(json!({ "ok": true }))`

---

## Hardware Tiers

| Tier | RAM | GPU | Examples |
|------|-----|-----|----------|
| `lite` | ≥4GB | none | system_info, file_ops, HTTP GET |
| `standard` | ≥8GB | optional | git, packages, documents |
| `performance` | ≥16GB | small dGPU | OCR, small LLM |
| `high` | ≥16GB | ≥6GB VRAM | Vision, embeddings, audio |

---

## Tool Categories

| Category | Description | Examples |
|----------|-------------|----------|
| `system_info` | Hardware and OS status | `get_cpu_usage`, `get_memory_info` |
| `file_ops` | File system operations | `read_file`, `write_file`, `search_files` |
| `shell` | Command execution | `run_command` |
| `internet` | Web operations | `web_search`, `web_fetch` |
| `packages` | Package management | `install_package`, `list_packages` |
| `knowledge` | Memory and facts | `store_fact`, `search_facts` |
| `communication` | Messaging | `send_email`, `send_notification` |
| `desktop` | Desktop control | `set_volume`, `set_brightness` |
| `vision` | Image processing | `analyze_image`, `ocr_image` |
| `rag` | Document RAG | `index_document`, `query_documents` |

---

## MCP Server Tools

MCP (Model Context Protocol) servers provide external tool capabilities:

| Server | Tools | Trust Level |
|--------|-------|-------------|
| `fs` | File system operations | Semi-trusted |
| `gworkspace` | Google Workspace API | Semi-trusted |
| `colab-mcp` | Google Colab execution | Semi-trusted |

MCP tools are registered with `gw_` or `mcp_` prefix to distinguish from native tools.

---

## OpenClaw Skill Tools

OpenClaw skills are community-contributed tools running in sandboxed Docker containers:

- **Prefix:** `oc_` (e.g., `oc_web_search`, `oc_code_sandbox`)
- **Trust Level:** Untrusted (sandboxed)
- **Execution:** Isolated container per invocation
- **Network:** Controlled via Tinyproxy egress proxy

See **OPENCLAW.md** for details.

---

## Related Documentation

- **OPENCLAW.md** — OpenClaw skill integration
- **SAFETY.md** — Policy engine and HITL
- **NewToolGuidelines.md** — Detailed tool development guide (kept for reference)
