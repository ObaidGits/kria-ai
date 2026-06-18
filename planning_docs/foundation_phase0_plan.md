# Phase 0 Foundation — Implementation Plan (0.1 Vault, 0.2 OAuth, 0.4 Server WS)

Status: implementation in progress. This is the authoritative engineering plan for the
three foundation milestones. All code lands in the existing workspace, matching current
conventions (per-module `thiserror` enums, `tracing`, workspace-pinned deps).

---

## 0.1 Secrets Vault — `crates/kria-core/src/auth/vault.rs`

**Goal:** One encrypted, on-device store for every credential (OAuth tokens, API keys),
so nothing lives in plaintext `.env` files.

### Design
- **Cipher:** AES-256-GCM (`aes-gcm`) for authenticated encryption of the whole vault blob.
- **Key derivation:** Argon2id (`argon2`) from a passphrase when `KRIA_VAULT_PASSPHRASE` is set.
- **Master key resolution order:**
  1. `KRIA_VAULT_PASSPHRASE` env → Argon2id(passphrase, salt) → 32-byte key. *(recommended)*
  2. Fallback: random 32-byte key persisted to `~/.kria/vault.key` (mode `0600`) with a logged
     security warning recommending the passphrase (or, later, the OS keyring).
- **OS keyring** (`keyring` crate) is deliberately gated behind a future `vault-keyring`
  feature, not the default — its Linux backend pulls in Secret Service/D-Bus and complicates
  headless/server builds. The encrypted-file backend is fully production-grade on its own.
- **File format** (`~/.kria/vault.enc`): `magic "KRV1"(4) | salt(16) | nonce(12) | ciphertext`.
  Ciphertext = AES-256-GCM of `serde_json(HashMap<String, SecretEntry>)`.
- **Atomic writes:** write to `vault.enc.tmp`, fsync, rename; file mode `0600`.
- **Memory hygiene:** master key held in `zeroize::Zeroizing<[u8;32]>`.

### API
`SecretsVault::open_default()`, `open(path, master)`, `get/get_entry/set/set_entry/delete/list`,
`persist`. `SecretEntry { value, updated_at, metadata }`.

### Tests
Round-trip set/get/delete, persistence reload with same passphrase, wrong-passphrase fails to
decrypt, atomic overwrite. (tempdir + fixed passphrase.)

---

## 0.2 OAuth Engine — `crates/kria-core/src/auth/oauth.rs`

**Goal:** Authorization-Code + PKCE login for Google, GitHub, Microsoft; tokens persisted in
the vault; transparent refresh.

### Design decision (justified deviation)
Implemented directly on **`reqwest`** (already a workspace dep) with manual PKCE rather than
pulling in the `oauth2` crate. Rationale: avoids a fragile dependency on the `oauth2` v5
typestate API, keeps full control over the 3 provider quirks (Google `access_type=offline`,
GitHub `Accept: application/json`, Microsoft `common` tenant), and guarantees a compiling,
testable result with zero new third-party surface. PKCE/exchange/refresh are ~standard POSTs.

### Flow
1. `begin_authorization(provider)` → `AuthSession { url, state, pkce_verifier }`.
2. User approves in browser; `capture_authorization_code(port, state)` (tokio loopback
   listener on `127.0.0.1`) catches `?code=&state=`, validates state, returns the code.
3. `complete_authorization(provider, code, verifier)` → POST token endpoint → `StoredToken`
   persisted in vault under key `oauth/{provider}`.
4. `get_access_token(provider)` → returns access token, auto-refreshing when expired via the
   stored `refresh_token`.

### Types
`OAuthProvider`, `ProviderConfig`, `StoredToken { access_token, refresh_token, expires_at,
scopes, token_type }`, `AuthSession`. Credentials read from env
(`KRIA_{GOOGLE,GITHUB,MICROSOFT}_CLIENT_ID/SECRET`, `KRIA_OAUTH_REDIRECT`).

### Tests
PKCE challenge correctness (S256), `StoredToken` expiry logic, vault persistence of a token.

---

## 0.4 Server WS Agent Loop — `crates/kria-server/src/ws.rs`

**Goal:** Replace the welcome-echo stub with the real agent loop streamed over WebSocket —
the same loop desktop/Telegram use. Unblocks the mobile PWA (Phase 4.5).

### Design
- Add `agent_loop: Option<Arc<kria_core::agent::AgentLoop>>` to `ServerState`.
- WS handler splits the socket; the sink is shared (`Arc<Mutex<SplitSink>>`) so the
  event-drain task and control handlers can both write.
- `chat` frame → build `Vec<ChatMessage>` (user turn) → spawn `agent.run_with_profile(...)`
  with an `mpsc::UnboundedSender<StreamEvent>` → a drain task maps each `StreamEvent` variant
  to a JSON frame (`token`, `tool_start`, `tool_end`, `task_step`, `approval_required`,
  `plan`, `error`, `done`, …) and sends to the client.
- `approve`/`deny` frame (with `request_id`) → `agent_loop.hitl_gateway().respond(id, …)`.
- `cancel` frame → `turn_admission.cancel_session(session_id)`.
- Concurrency: the receiver loop keeps reading control frames *while* the drain task streams,
  so HITL approvals work mid-turn.
- When `agent_loop` is `None`, the handler returns a clear `error` frame.

### Follow-up (called out, not hidden)
Constructing the full `AgentLoop` server-side mirrors the ~1000-line desktop
`runtime.rs` builder. To do this safely without breaking the desktop build, that builder
should be **extracted into a shared headless constructor** in `kria-core` (reused by desktop,
server, Telegram). This plan wires the WS *consumption* side fully now and sets
`agent_loop = None` in `main.rs` until the shared builder lands as its own focused task.

### Security (applies to all three)
Vault file `0600`; never log secret values (reference by key name); WS remains intended for
the private-mesh path (Phase 4.5) — do not expose publicly without device pairing + HITL.
