# Universal Model Provider System

> **Status:** Implemented  
> **Last Updated:** 2026-05-15

---

## Overview

KRIA's Universal Model Provider system makes the assistant runtime **provider-independent**. Users can seamlessly switch between local models (Ollama, llama.cpp) and cloud APIs (OpenAI, Gemini, Anthropic, OpenRouter) without destabilizing the runtime, orchestration, memory, hardware scheduling, streaming, or voice systems.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Orchestration Layer                           │
│  (TurnGate → AgentLoop → ToolRegistry → MemoryManager)         │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │              Provider Registry (runtime)                   │  │
│  │                                                           │  │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌──────────────┐  │  │
│  │  │ Ollama  │ │llama.cpp│ │ OpenAI  │ │  Anthropic   │  │  │
│  │  │ Backend │ │ Backend │ │ Backend │ │   Backend    │  │  │
│  │  └────┬────┘ └────┬────┘ └────┬────┘ └──────┬───────┘  │  │
│  │       │            │           │              │           │  │
│  │  ┌────┴────┐ ┌────┴────┐ ┌────┴────┐ ┌──────┴───────┐  │  │
│  │  │ Gemini  │ │OpenRouter│ │ Custom  │ │   Future     │  │  │
│  │  │ Backend │ │ Backend │ │ Backend │ │  Providers   │  │  │
│  │  └─────────┘ └─────────┘ └─────────┘ └──────────────┘  │  │
│  └───────────────────────────────────────────────────────────┘  │
│                           │                                     │
│                    LlmBackend trait                              │
│              (chat, chat_stream, health_check)                   │
└─────────────────────────────────────────────────────────────────┘
                            │
              ┌─────────────┼─────────────┐
              │             │             │
    Hardware Orchestrator   │    Streaming Abstraction
    (GPU lease adapt)       │    (UnifiedStream)
                            │
                   Persistent Config
                   (~/.kria/config.toml)
```

---

## Supported Providers

| Provider | Type | Auth | Streaming | Tools | Vision |
|----------|------|------|-----------|-------|--------|
| llama.cpp | Local | None | ✅ | ✅ | ✅ |
| Ollama | Local | None | ✅ | ✅ | ✅ |
| OpenAI | Cloud | API Key | ✅ | ✅ | ✅ |
| Google Gemini | Cloud | API Key | ✅ | ✅ | ✅ |
| Anthropic | Cloud | API Key | ✅ | ✅ | ✅ |
| OpenRouter | Cloud | API Key | ✅ | ✅ | ✅ |
| OpenAI Compatible | Either | Optional | ✅ | ✅ | Varies |

---

## Configuration

### TOML Configuration

Providers are configured in `~/.kria/config.toml` under the `[providers]` section:

```toml
[providers]
active_provider = "llama_cpp"
prefer_streaming = true
# fallback_provider = "openai"  # Optional auto-fallback

[[providers.providers]]
id = "llama_cpp"
provider_type = "llama_cpp"
display_name = "llama.cpp (Local)"
enabled = true
active_model = "qwen2.5-vl-7b"
default_temperature = 0.7
default_max_tokens = 4096

[providers.providers.endpoint]
base_url = "http://127.0.0.1:8080/v1"
timeout_secs = 120

[[providers.providers]]
id = "openai"
provider_type = "openai"
display_name = "OpenAI"
enabled = true
active_model = "gpt-4o"

[providers.providers.endpoint]
base_url = "https://api.openai.com/v1"
api_key = "sk-..."
timeout_secs = 60
max_retries = 3
```

### Environment Variables

API keys can also be set via environment variables:
- `KRIA_OPENAI_API_KEY`
- `KRIA_GEMINI_API_KEY`
- `KRIA_ANTHROPIC_API_KEY`
- `KRIA_OPENROUTER_API_KEY`

---

## UI Settings

The Settings → Model tab now includes the **Provider Settings** panel:

1. **Active Runtime Banner** — Shows current provider, model, and execution location (Local/Cloud)
2. **Provider Cards** — Each configured provider with status, actions:
   - **Activate** — Switch to this provider
   - **Test** — Validate connection instantly
   - **Models** — Discover available models
   - **Remove** — Delete provider config
3. **Add Provider** — Form to configure new providers with type selection, endpoint, API key, and model

---

## Hardware Orchestrator Integration

When switching providers, the hardware orchestrator adapts:

| Scenario | Orchestrator Behavior |
|----------|----------------------|
| Switch to Local | GPU allocation active, VRAM management active, model loading orchestration active |
| Switch to Cloud | Release local VRAM pressure, reduce scheduling overhead, adapt queue priorities |
| Switch to Hybrid | Balanced resource strategy |

The `ProviderRegistry` notifies the orchestrator via a callback when the execution location changes.

---

## Connection Testing

Users can instantly validate provider connectivity:

```
User clicks "Test" → Provider-specific validation request →
  Ollama: GET /api/tags
  llama.cpp: GET /v1/models
  OpenAI: GET /models
  Gemini: GET /models?key=...
  Anthropic: POST /messages (minimal)
→ Result: success/unauthorized/unreachable/timeout/quota_exceeded
```

Results include latency, discovered models, and provider-specific diagnostics.

---

## Error System

All provider errors are normalized to `ProviderError` with:
- **Kind** — Classification (AuthFailure, RateLimited, Timeout, NetworkError, etc.)
- **Retryable** — Whether the error should be retried
- **User message** — Human-readable error for the UI
- **Provider code** — Provider-specific error code for debugging

---

## Streaming

The `UnifiedStream` abstraction normalizes streaming across providers:
- OpenAI/llama.cpp: SSE with `data: {...}` format
- Gemini: SSE with `data: {...}` format (different JSON structure)
- Anthropic: SSE with event types (`content_block_delta`, `message_stop`)
- Ollama: OpenAI-compatible SSE via `/v1/chat/completions`

All streams support cancellation via `CancellationToken`.

---

## Adding a New Provider

1. Create `crates/kria-core/src/llm/provider/<name>.rs`
2. Implement `LlmBackend` trait
3. Add to `ProviderType` enum in `config.rs`
4. Add backend creation in `registry.rs` → `create_backend()`
5. Add connection test in `connection_test.rs`
6. Add to `get_provider_types()` in desktop commands

No changes needed to orchestration, memory, tools, or streaming layers.

---

## API Endpoints

### REST (kria-server)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/providers` | List all providers with status |
| GET | `/api/providers/active` | Get active provider details |
| POST | `/api/providers/switch` | Switch active provider |
| POST | `/api/providers/switch-model` | Switch active model |
| POST | `/api/providers/{id}/test` | Test provider connection |
| GET | `/api/providers/{id}/models` | Discover available models |
| POST | `/api/providers/config` | Add/update provider |
| DELETE | `/api/providers/{id}` | Remove provider |
| POST | `/api/providers/test-config` | Test config without saving |

### Tauri IPC (kria-desktop)

| Command | Description |
|---------|-------------|
| `list_providers` | List all providers |
| `get_active_provider` | Get active provider |
| `switch_provider` | Switch provider |
| `switch_model` | Switch model |
| `test_provider_connection_cmd` | Test connection |
| `discover_provider_models` | Discover models |
| `upsert_provider` | Add/update provider |
| `remove_provider` | Remove provider |
| `get_provider_types` | List available types |
| `test_provider_config` | Test before save |
