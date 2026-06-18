# Phase 4.5 — Mobile Prompt-Control: Production Plan

> Method: iterative analysis → design → enhancement/correction passes → implement.
> Status legend: ⬜ todo · 🟦 in-progress · ✅ done

## 0. Scope decision (code vs infra) — STATUS

| Item | Nature | Status |
|------|--------|--------|
| 4.5.2 Server WS agent loop | code | ✅ verified (was already shipped) |
| 4.5.3 PWA shell | code (frontend) | ✅ manifest + service worker + install meta + mobile WS client (`buildWsUrl` tested). Remaining: voice-note input, file push, QR scanner UI |
| 4.5.4 Device pairing + token auth | code | ✅ `kria-core::mobile` (pair/token/verify/revoke, 5 tests) + server routes + `/ws` token gate. Remaining: QR scan UI |
| 4.5.4 Tailscale transport | infra | ✅ `bind_interface` config + wildcard-bind warning; mesh setup is operator infra |
| 4.5.5 ntfy push | code | ✅ `kria-core::notify::ntfy` (4 tests) + config + wired to approval/done events |
| 4.5.6 Session continuity | code | ✅ `MemoryStore` wired into `/ws` (load history + persist) + real `/api/sessions` |
| 4.5.1 Telegram hardening | code | ✅ allow-list + owner gate (existing) + per-message audit trail added. Remaining: inline-button HITL |

Security principle (from roadmap, kept verbatim intent): the PWA path stays behind
the WireGuard mesh; `kria-server` is never bound to `0.0.0.0` by default; every
remote device uses a per-device signed token with instant revocation; remote
write/destructive steps require HITL on the phone; every remote command is audited.

## 1. Current-state findings (audit)

- Vault / OAuth / `/ws` real loop = done (Phase 0).
- `kria-server/src/auth.rs` is a placeholder bearer check (TODO) → `/ws` is unauthenticated.
- Headless runtime builds tools with `None` store → server has no memory/session history.
- `/ws` chat sends only the latest user message (no history, no persistence).
- No notify module; no PWA manifest/service-worker.
- Reusable crypto already in tree: `hmac`, `sha2`, `base64`, `uuid`, `rusqlite`, `reqwest`.

## 2. Design

### 2.1 `kria-core::mobile` (device pairing + tokens)
- `DeviceRegistry` backed by SQLite (`<data_dir>/devices.db`).
- Signing key: random 32 bytes generated once, persisted in the **vault** under
  `mobile/device_signing_key` (reuses Phase 0.1, never on disk in plaintext).
- Pairing: `begin_pairing()` → short pairing code (TTL ~5 min) + QR payload string
  `kria-pair://<host>/<code>`. `complete_pairing(code, device_name)` → persists device,
  returns a signed device token.
- Token format: `v1.<device_id>.<exp_unix>.<b64url(HMAC_SHA256(key, "device_id.exp"))>`.
- `verify_token(token)` → signature + expiry + not-revoked → `DeviceId`.
- `revoke(device_id)`, `list_devices()`, `touch_last_seen()`.
- Tokens short-lived (config TTL), renewable via `renew(device_id)`.
- Unit tests: pair→token→verify roundtrip; expiry; revoke; tamper.

### 2.2 `kria-core::notify::ntfy`
- `NtfyConfig { enabled, server_url, topic, auth_token, default_priority }`.
- `NtfyClient::publish(NtfyMessage{ title, body, priority, tags, click })`.
- Pure `build_request_parts()` (URL, headers, body) → unit-testable without network.
- Safety: never include secret values; caller passes only summaries.

### 2.3 Config additions (`config.rs`)
- `MobileConfig { enabled, bind_interface, require_device_auth, token_ttl_secs, pairing_ttl_secs }`.
- `NtfyConfig { ... }`. Both added to `KriaConfig` with `Default`.

### 2.4 Server wiring (`kria-server`)
- `ServerState`: add `device_registry: Option<Arc<DeviceRegistry>>`,
  `notifier: Option<Arc<NtfyClient>>`, `session_store: Arc<ServerSessionStore>`.
- `mobile_routes.rs`: `POST /api/mobile/pair/begin`, `POST /api/mobile/pair/complete`,
  `GET /api/mobile/devices`, `POST /api/mobile/devices/{id}/revoke`.
- `/ws` guard: when `require_device_auth`, validate `?token=` (or `Authorization`)
  via `DeviceRegistry::verify_token` before upgrade; reject otherwise. Audit connect.
- ntfy push fired on `ApprovalRequired` (approval needed) and `Done` (task done) in `ws.rs`.
- `main.rs`: bind to `config.mobile.bind_interface` host when set; warn loudly on `0.0.0.0`.

### 2.5 Session continuity (`kria-server`)
- `ServerSessionStore` (rusqlite, `<data_dir>/server_sessions.db`): `turns(session_id, role, content, ts)`.
- `/ws` chat: load last N turns → prepend to messages; after `Done`, persist user+assistant.
- `GET /api/sessions` → distinct session ids + last activity.

### 2.6 PWA (`ui`)
- `ui/public/manifest.webmanifest` (standalone, theme color, icons).
- `ui/public/sw.js` (offline app-shell cache; network-first for API/WS).
- Register SW + link manifest in `index.html`; add iOS meta tags.
- `ui/src/lib/mobileClient.ts`: connect to `kria-server` `/ws` with device token,
  stream tokens/tool/approval frames, send approve/deny — reuses existing frame protocol.

## 3. Iteration / correction passes
- P1 core (`mobile`, `notify`, config) + unit tests → `cargo test -p kria-core`.
- P2 server routes + `/ws` guard + session store + ntfy wiring → `cargo build -p kria-server`.
- P3 PWA shell + mobile client → `npm run build` (ui).
- P4 Telegram audit-per-message → `cargo build`.
- Each pass: compile, test, fix before next.

## 4. Exit metric (roadmap)
From mobile data, send a prompt over the PWA path; KRIA executes on the laptop,
streams back; write steps require HITL on phone; losing one channel (Telegram) does
not break the PWA path. Device tokens revocable instantly.

## 5. Tracked enhancements (post-4.5, documented not built here)
- Telegram inline-button HITL (callback_query) instead of owner auto-approve.
- QR rendered server-side (currently payload string; UI renders QR).
- Tailscale/Headscale automated enrollment (currently manual mesh + bind config).
- Unified multi-device session fan-out.
