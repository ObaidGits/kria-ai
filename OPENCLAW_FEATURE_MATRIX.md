# OpenClaw / CPP Feature Matrix

Generated from actual implementation + validation evidence (Capability Provider Platform).
Status: Implemented (real-validated) · Implemented (build/unit) · Partial · Pending.
Evidence lives in `.kiro/specs/capability-provider-platform/PROGRESS.md`.

| Feature | Status | Evidence |
|---|---|---|
| Provider-neutral boundary (trait + descriptor v1.1 + protocol) | Implemented (real-validated) | 75 lib tests; boundary-integrity grep clean |
| OpenClaw as a provider (ACL) | Implemented (real-validated) | Real Docker `oc_calculator 3+3→6`, 0 leaks |
| MCP provider (any stdio MCP server) | Implemented (real-validated) | Real node stub federated + executed |
| Multi-provider federation | Implemented (real-validated) | OpenClaw+MCP: 2 providers, discovery routes by goal |
| Federated discovery (dense+lexical fusion) | Implemented (real-validated) | Real skills.db → discover ranks correct skill |
| Capability descriptor v1.1 (effects/guidance/expectations) | Implemented (real-validated) | Round-trip + forward-compat + conservative defaults |
| Provider negotiation + optional facets | Implemented (real-validated) | Feature intersection + unknown-feature preservation |
| Permission engine (descriptor-effects, 7 tiers) | Implemented (real-validated) | Property 7; real Docker effects→gate |
| Durable scoped grants (SQLite) | Implemented (real-validated) | Scope/reuse/revoke/expiry + across-reopen |
| Live approval flow (authorize→approve→execute→revoke→re-prompt) | Implemented (real-validated) | `capability_approval_flow_docker.rs`: real Docker, GrantStore-reopen durability, 0 leaks |
| Gated desktop execution (`cpp_execute`) | Implemented (real-validated) | Permission gate → platform execute; reference default-on flow |
| Execution seam de-enum (`provider_id`) | Implemented (real-validated) | Workspace builds; provider-addressed exec via ExecutionEngine |
| Capability acquisition (marketplace install) | Implemented (real-validated) | Installed `oc_code_sandbox` from live repo → describe → remove |
| Capability removal (lifecycle) | Implemented (real-validated) | `remove` uninstalls; not-enabled after |
| Descriptor refresh (post-acquire) | Implemented (real-validated) | `acquire` returns refreshed descriptor from registry |
| Recommendation (across providers) | Implemented (build/unit) | Recommender operates on `provider_id`/federated index |
| Learning loop (outcome → ranking) | Implemented (real-validated) | Success signal shifts ranking (unit) |
| Observability event stream + timeline | Implemented (real-validated) | 4 real execute events, provider/capability tagged |
| Circuit-breaker recovery | Implemented (real-validated) | Open-after-3-failures + reset (unit) |
| Provider SDK + conformance harness | Implemented (real-validated) | Both live providers + brand-new provider pass |
| Diverse capability execution battery | Implemented (real-validated) | 9/9 skills across 2 providers on Docker, 0 leaks |
| Desktop `cpp_*` commands (status/providers/discover/catalog/recommend/descriptor/grants/authorize/approve/execute/timeline) | Implemented (build) | `cargo build -p kria-desktop` clean |
| Desktop Capabilities UI (Providers/Browser+Run/Marketplace/Approval Center/Timeline/Descriptor Viewer) | Implemented (build) | `npm run build` clean; tabbed area |
| Observability timeline / runtime monitor / recovery UI | Implemented (build) | `cpp_timeline` event ring → Timeline tab |
| Descriptor Viewer (v1.1 guidance/expectations) | Implemented (build) | `cpp_descriptor` → modal |
| Docker cleanup / leak-freedom | Implemented (real-validated) | 0 leaked containers after every Docker run |
| Marketplace catalog federation (installable recommendations) | Implemented (real-validated) | `catalog()`+`recommend`; `platform_recommends_installable_catalog_entries` |
| Real-LLM A9 generation pipeline (task 11.2) | Implemented (real-validated) | Cloud LLM generated+installed 3 skills (q~0.99); generated `oc_word_count` executed in real container → `{"wordCount":5}`, 0 leaks |
| Production DoD gate (aggregation + report) | Implemented (real-validated) | `scripts/cpp_production_gate.sh` → GO 3/3, 0 leaks |
| Live tauri-driver desktop drive | Harnessed (READY) | `scripts/cpp_tauri_driver_drive.mjs`; tauri-driver + WebKitWebDriver present |
| Multi-hour soak | Harnessed (READY) | `scripts/cpp_soak.sh` (wall-clock; deferred by directive) |
| CPP default-on migration | Pending | gated on soak green |
| Legacy removal (flag-off path, old CIL/grant engine) | Pending | gated on default-on + soak |
