# KRIA Provider Orchestration

Last updated: 2026-05-27

## Purpose

Provider orchestration manages model backends under KRIA runtime authority. It lets KRIA switch between local llama.cpp/Ollama and remote providers while keeping tool execution, safety, HITL, and verifier authority outside the provider layer.

Providers generate model output. They do not execute tools directly, bypass policy, or decide workflow completion.

## Supported Provider Types

`ProviderType` supports:

- `llama_cpp`
- `ollama`
- `openai`
- `gemini`
- `anthropic`
- `openrouter`
- `openai_compatible`

Default endpoints:

| Provider | Default endpoint | API key required |
|---|---|---|
| `ollama` | `http://localhost:11434` | no |
| `llama_cpp` | `http://localhost:8080` | no |
| `openai` | `https://api.openai.com/v1` | yes |
| `gemini` | `https://generativelanguage.googleapis.com/v1beta` | yes |
| `anthropic` | `https://api.anthropic.com/v1` | yes |
| `openrouter` | `https://openrouter.ai/api/v1` | yes |
| `openai_compatible` | user supplied | depends on endpoint |

Default provider config:

- active provider: `llama_cpp`,
- fallback provider: none,
- `llama_cpp` enabled,
- `ollama` present but disabled,
- streaming preferred.

## Core Runtime Pieces

| Component | Location | Contract |
|---|---|---|
| Provider config | `llm/provider/config.rs` | Persistent provider definitions, endpoints, API keys, active model, streaming preference. |
| Provider registry | `llm/provider/registry.rs` | Backend lifecycle, active backend, provider switching, health snapshots. |
| Provider backends | `llm/provider/*` | Provider-specific request/stream bridging. |
| Unified stream | `llm/provider/streaming.rs` | Provider-independent stream chunks and cancellation. |
| Model router | `llm/model_router.rs` | Runtime routing mode and active provider/model binding. |
| Desktop provider commands | `crates/kria-desktop/src/commands/providers.rs` | Settings/UI command surface and live runtime apply. |
| Desktop runtime boot | `crates/kria-desktop/src/commands/runtime.rs` | Initial provider/router/orchestrator wiring. |

## Routing Modes

`ModelRouter` supports:

```text
local
colab
gemini
external
```

Routing behavior:

- `local`: use local llama.cpp backend.
- `colab`: prefer Colab/external backend, then local fallback.
- `gemini`: use configured Gemini backend.
- `external`: use selected external provider.

When the active provider is not `llama_cpp`, desktop startup skips local llama-server orchestration so GPU resources are not allocated unnecessarily.

## Config And Environment Precedence

Config sources:

1. `config/default.toml`
2. `~/.kria/config.toml`
3. environment variables

Provider environment overrides:

- `KRIA_ACTIVE_PROVIDER`
- `KRIA_ACTIVE_MODEL`
- `KRIA_PROVIDER_API_KEY`
- `KRIA_PROVIDER_<PROVIDER_ID>_API_KEY`
- `KRIA_OPENAI_API_KEY`
- `OPENAI_API_KEY`
- `KRIA_GEMINI_API_KEY`
- `GEMINI_API_KEY`
- `GOOGLE_API_KEY`
- `KRIA_ANTHROPIC_API_KEY`
- `ANTHROPIC_API_KEY`
- `KRIA_OPENROUTER_API_KEY`
- `OPENROUTER_API_KEY`
- `KRIA_OPENCODE_API_KEY`
- `KRIA_LLM_MODE`
- `KRIA_CLOUD_API_KEY`

The UI payload exposes active environment overrides through `config_source.env_wins` so provider surprises can be debugged.

## Legacy LLM Sync

KRIA still maintains legacy `[llm]` fields for runtime compatibility.

When a provider becomes active:

- `llama_cpp` maps to `llm.routing_mode = "local"`.
- `gemini` maps to `llm.routing_mode = "gemini"`.
- all other non-local providers map to `llm.routing_mode = "external"`.

The selected provider also syncs:

- active model,
- local API URL for llama.cpp,
- cloud provider ID,
- cloud endpoint,
- cloud model ID,
- API key when present.

This keeps older routing code and newer provider settings aligned.

## Desktop Command Surface

Registered Tauri commands:

- `list_providers`
- `get_active_provider`
- `get_active_llm_runtime`
- `get_llm_runtime_apply_status`
- `set_active_llm_selection`
- `switch_provider`
- `switch_model`
- `test_provider_connection_cmd`
- `test_provider_config`
- `discover_provider_models`
- `upsert_provider`
- `remove_provider`
- `get_provider_types`

Behavior:

- `test_provider_config` validates a draft config before saving.
- `upsert_provider` preserves an existing API key when an update omits it.
- `remove_provider` refuses to remove the active provider.
- `set_active_llm_selection` applies provider and model atomically.
- Provider/model apply operations are serialized by an apply lock.

## Live Runtime Apply

Provider/model changes are applied without requiring a full app restart when possible.

External provider apply:

1. Publish `llm-runtime:apply` status as testing.
2. Test provider connection.
3. Bind `ModelRouter` to the selected provider.
4. Release local llama.cpp runtime if external provider is active.
5. Save config.
6. Publish final apply status.

Local llama.cpp apply:

1. Resolve selected GGUF file.
2. Build or derive local model metadata.
3. Derive model profile.
4. Tune orchestrator config for detected hardware.
5. Start/restart local orchestrator with the selected model.
6. Sync router status and save config.
7. Roll back to previous provider/model when startup fails.

Status is emitted through:

```text
llm-runtime:apply
```

## Provider Registry Behavior

`ProviderRegistry` provides a core provider management abstraction:

- instantiate configured/enabled backends,
- get active backend,
- switch active provider,
- switch model,
- add/update/remove providers,
- test provider connections,
- discover provider models,
- track health snapshots,
- notify hardware orchestrator when execution location changes.

Execution location:

- local providers: `Local`,
- remote providers: `Cloud`,
- mixed providers can be represented as `Hybrid`.

## Streaming Contract

`UnifiedStream` normalizes provider stream output into:

- text chunk,
- final flag,
- optional tool-call delta,
- finish reason,
- optional token usage.

It also supports cancellation through `CancellationToken`.

Provider-specific streaming/SSE behavior must be adapted below this abstraction. Higher layers should not depend on provider-native streaming formats.

## Failure Handling

| Failure | Behavior |
|---|---|
| Provider missing | Return explicit provider-not-found error. |
| Provider not configured | Reject switch/test with credential or endpoint error. |
| Active provider removal | Refuse removal until another provider is active. |
| Connection test failure | Do not bind external runtime. |
| Local model missing | Refuse local apply with model-path error. |
| Local orchestrator startup failure | Roll back to previous provider/model when possible. |
| Environment override active | Show in runtime payload so config/UI mismatch is explainable. |
| Stream cancellation | Stream returns cancelled and terminates. |

## Security And Safety

- API keys are config/environment data, not source-code constants.
- Providers are external trust domains unless local.
- Provider output may suggest tool use, but tool execution remains in KRIA's tool/policy path.
- Provider switching must not bypass HITL, verifier authority, rollback, or audit.
- Cloud provider mode should be treated as data egress and configured intentionally.

## Operational Checklist

Before enabling a provider:

1. Confirm endpoint and API key are configured.
2. Run the provider connection test.
3. Discover models if supported.
4. Select active model.
5. Confirm runtime payload shows the intended provider and model.
6. Check `config_source.env_wins` for unexpected environment overrides.
7. For local providers, confirm model file exists and orchestrator status is healthy.
8. For external providers, confirm local llama-server is released/skipped if not needed.

## Invariants

- Provider layer does not execute tools.
- Provider layer does not own policy decisions.
- Runtime selection is explicit and observable.
- Local/external switches update both provider config and legacy LLM routing state.
- Local model changes are applied through orchestrator startup, not by editing config alone.
- Failed provider applies must not leave the UI claiming a successful runtime switch.
