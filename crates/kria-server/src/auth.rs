//! Remote server bearer-token authentication (MGR-003 AC2/AC3/AC4 — F1.6.2).
//!
//! Replaces the F1.6.1-era placeholder (`validate_token` just checked
//! `!token.is_empty()`, and a caller with NO `Authorization` header at all was
//! silently allowed through) with a real, signed, expiring, replay-protected
//! bearer identity:
//!
//! - **Signed**: HMAC-SHA256 over a canonical payload, keyed by the server's
//!   configured `[server].jwt_secret` — the same secret `bind_security`
//!   (F1.6.1) already requires to be non-empty before a non-loopback bind is
//!   allowed. No `jsonwebtoken`/JWT-library dependency is added: the repo
//!   already pins `hmac`/`sha2`/`rand`/`base64` at the workspace level, and a
//!   full JWT-compliant format buys nothing here (no third-party JWT
//!   consumer exists) — see spec for the deliberate choice to build a small
//!   HMAC token instead of a JOSE/JWT stack.
//! - **Identity-bound**: carries `actor_id` and `device_id`, from which a
//!   real per-request [`CallerContext::authenticated_remote`] is constructed
//!   — never a single shared, server-wide caller.
//! - **Expiring**: carries `exp` (Unix seconds); rejected once passed.
//! - **Replay-protected**: carries a random `nonce`; a [`ReplayCache`]
//!   remembers nonces already seen and rejects reuse.
//!
//! ## Scope
//!
//! This module is the identity/session/expiry/replay boundary (F1.6.2) only.
//! It intentionally does NOT implement:
//! - origin allowlisting, transport (TLS) enforcement, or request/rate/
//!   concurrency/deadline limits (F1.6.3);
//! - a token issuance/enrollment UX (e.g. a pairing flow analogous to the
//!   mobile `DeviceRegistry`). [`issue_token`] exists so tokens CAN be minted
//!   (and is exercised by tests), but there is no HTTP endpoint or operator
//!   tooling wired up yet to hand a token to a legitimate remote client —
//!   that is a documented gap, not silently missing.
//! - richer per-token capability *scopes* beyond [`CallerOrigin`]. Every
//!   successfully verified token yields `CallerOrigin::AuthenticatedRemote`,
//!   which the existing [`is_command_capability_permitted`] lattice (F1.5.3)
//!   already gates correctly; scoping individual tokens to a subset of
//!   [`CommandKind`]s is future work.
//!
//! ## Replay cache lifetime (design note)
//!
//! The replay cache is a single in-memory, process-lifetime `HashMap`
//! ([`ReplayCache`]), not a persistent store. This is intentional and
//! sufficient for the current deployment reality (dev-context: single
//! process, single user, single laptop, pre-production): a persistent replay
//! store would need to survive process restarts to matter, and a restart
//! already invalidates every in-flight session in this architecture (no
//! multi-instance/HA server). Entries are purged once their token's own
//! expiry passes, so memory is bounded by the number of distinct valid
//! tokens seen within one TTL window.

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kria_core::memory::model::CallerContext;

use crate::correlation::correlation_id_of;
use crate::deny::deny_envelope;
use crate::ServerState;

type HmacSha256 = Hmac<Sha256>;

/// Token format version marker (first dot-separated field).
const TOKEN_VERSION: &str = "krav1";

/// Default TTL used by [`issue_token`] when the caller does not need a
/// different lifetime. The replay cache purge window tracks whatever `exp`
/// the token actually carries, not this constant.
pub const DEFAULT_TOKEN_TTL_SECS: i64 = 3600;

/// Why a bearer token failed verification. Never rendered to an HTTP
/// response body directly — every case maps to the SAME non-revealing deny
/// envelope (MGR-003 AC3); this exists for internal triage/logging only.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenError {
    Malformed,
    BadSignature,
    Expired,
    Replayed,
}

/// The authenticated identity claims carried by a verified token.
#[derive(Debug)]
struct TokenClaims {
    actor_id: String,
    device_id: String,
    nonce: String,
}

fn b64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>, ()> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn sign(secret: &[u8], payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    b64_encode(&mac.finalize().into_bytes())
}

/// Constant-time signature check (delegates to `hmac`'s `verify_slice`,
/// which uses a constant-time comparison internally — MGR-003 AC3 hardening:
/// a timing side-channel on signature comparison would itself leak whether a
/// token is "close" to valid).
///
/// ## Timing side-channel review (F1.6.4)
///
/// [`decode_and_verify_signature`] early-returns `Malformed` for a
/// wrong-shape token (wrong dot-count/version) BEFORE reaching this
/// function, so a malformed token is answered measurably faster than a
/// well-formed-but-wrong-signature one. This asymmetry is judged
/// acceptable and is NOT fixed by this task: the exploitable class of
/// timing side-channel here is a byte-level comparison oracle that would
/// let an attacker forge a valid signature one byte at a time by observing
/// timing differences on NEAR-MISS signatures — that is exactly what
/// `verify_slice`'s constant-time comparison already prevents, regardless
/// of how many nanoseconds earlier a garbage-shaped token was rejected.
/// Knowing "my token was malformed" vs. "my token had a shape-valid but
/// wrong signature" reveals nothing that helps forge a signature — both
/// outcomes are the identical `401 unauthorized` deny to the caller
/// (MGR-003 AC3 is about response CONTENT, and design §8's "share status,
/// body length class, timing budget" is read together with §19.8, not as a
/// requirement that literally every code path execute in identical wall-
/// clock time regardless of trivial early-exit validation). Making
/// malformed-shape rejection artificially as slow as a full HMAC
/// computation would only waste CPU without closing any real attack.
fn verify_sig(secret: &[u8], payload: &str, sig_b64: &str) -> bool {
    let Ok(sig) = b64_decode(sig_b64) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    mac.verify_slice(&sig).is_ok()
}

/// Issue a signed bearer token for `actor_id`/`device_id`, valid for
/// `ttl_secs` seconds from now. `secret` is the server's `[server].jwt_secret`
/// bytes.
///
/// Format: `krav1.<actor_b64url>.<device_b64url>.<exp>.<nonce_b64url>.<sig_b64url>`
/// — every field is base64url(no-pad) or decimal, so no field's content can
/// introduce an ambiguous extra `.` when splitting the token back apart.
/// `sig` signs the five fields preceding it, joined by `.`.
pub fn issue_token(secret: &[u8], actor_id: &str, device_id: &str, ttl_secs: i64) -> String {
    let exp = now() + ttl_secs.max(1);
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce_b64 = b64_encode(&nonce_bytes);
    let actor_b64 = b64_encode(actor_id.as_bytes());
    let device_b64 = b64_encode(device_id.as_bytes());
    let payload = format!("{TOKEN_VERSION}.{actor_b64}.{device_b64}.{exp}.{nonce_b64}");
    let sig = sign(secret, &payload);
    format!("{payload}.{sig}")
}

/// Parse + verify a token's format and signature only (no expiry/replay
/// check yet — callers that need those call [`verify_token`] instead, which
/// wraps this).
fn decode_and_verify_signature(secret: &[u8], token: &str) -> Result<(TokenClaims, i64), TokenError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 6 || parts[0] != TOKEN_VERSION {
        return Err(TokenError::Malformed);
    }
    let payload = parts[0..5].join(".");
    if !verify_sig(secret, &payload, parts[5]) {
        return Err(TokenError::BadSignature);
    }
    let actor_id = b64_decode(parts[1])
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or(TokenError::Malformed)?;
    let device_id = b64_decode(parts[2])
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or(TokenError::Malformed)?;
    let exp: i64 = parts[3].parse().map_err(|_| TokenError::Malformed)?;
    let nonce = parts[4].to_string();
    if actor_id.trim().is_empty() || device_id.trim().is_empty() {
        return Err(TokenError::Malformed);
    }
    Ok((
        TokenClaims {
            actor_id,
            device_id,
            nonce,
        },
        exp,
    ))
}

/// Full verification: signature, expiry, and replay — in that order (a
/// tampered token is rejected before we even look at its claimed expiry or
/// consume a nonce slot).
fn verify_token(secret: &[u8], token: &str, replay: &ReplayCache) -> Result<TokenClaims, TokenError> {
    let (claims, exp) = decode_and_verify_signature(secret, token)?;
    if now() >= exp {
        return Err(TokenError::Expired);
    }
    if !replay.check_and_record(&claims.nonce, exp) {
        return Err(TokenError::Replayed);
    }
    Ok(claims)
}

/// In-memory nonce replay cache (see module docs for why this is not a
/// persistent store). Bounded by purging entries once their own token's
/// expiry has passed.
struct ReplayCache {
    seen: Mutex<HashMap<String, i64>>,
}

impl ReplayCache {
    fn new() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` and records `nonce` if it has not been seen before;
    /// returns `false` (a replay) if it has. Opportunistically purges
    /// expired entries on every call.
    fn check_and_record(&self, nonce: &str, exp: i64) -> bool {
        let mut seen = self.seen.lock().unwrap();
        let now_ts = now();
        seen.retain(|_, recorded_exp| *recorded_exp > now_ts);
        if seen.contains_key(nonce) {
            false
        } else {
            seen.insert(nonce.to_string(), exp);
            true
        }
    }
}

/// The single process-lifetime replay cache. A module-level singleton (not a
/// `ServerState` field) is deliberate: this server is one process per
/// deployment (dev-context: single-laptop, pre-production), so a global is
/// equivalent to a per-instance field here without threading a new field
/// through every `ServerState` construction site (main + test harnesses).
fn replay_cache() -> &'static ReplayCache {
    static CACHE: OnceLock<ReplayCache> = OnceLock::new();
    CACHE.get_or_init(ReplayCache::new)
}

/// The single non-revealing deny envelope for every authentication failure
/// (missing header, malformed header, bad signature, expired, or replayed —
/// MGR-003 AC3). No field discloses WHICH of those occurred. Delegates to
/// the crate-wide [`deny_envelope`] (F1.6.4) so this shares its exact field
/// set/order/length class with every other boundary's deny path, and
/// includes the request's correlation ID exactly like `origin`/`rate_limit`
/// already do.
fn auth_denied(request: &Request) -> Response {
    deny_envelope(StatusCode::UNAUTHORIZED, "unauthorized", correlation_id_of(request))
}

/// Real bearer-token authentication middleware for remote server mode
/// (MGR-003 AC2/AC4 — F1.6.2). Only layered onto the router when
/// `[server].remote_enabled = true` (see `build_router`); the default
/// loopback path never runs this middleware at all (see module/`lib.rs`
/// docs for why that is the correct reading of MGR-003 AC1/AC2).
///
/// On success, inserts a per-request [`CallerContext::authenticated_remote`]
/// (built from the token's verified `actor_id`/`device_id`) into the
/// request's extensions, so downstream handlers see the REAL authenticated
/// caller instead of the single static `ServerState::caller` — see
/// `memory_routes::effective_caller`.
pub async fn auth_middleware(State(state): State<Arc<ServerState>>, mut request: Request, next: Next) -> Response {
    let secret = state.config.server.jwt_secret.as_bytes();

    let token = match request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => return auth_denied(&request),
    };

    let claims = match verify_token(secret, &token, replay_cache()) {
        Ok(c) => c,
        Err(_) => return auth_denied(&request),
    };

    // The partition itself is not per-token scope in this build (see module
    // docs: richer per-token scopes are deferred future work) — every
    // authenticated remote caller operates within the same configured
    // partition the server's static caller already used, but now with the
    // REAL verified actor_id/device_id rather than the fixed "local-server"
    // placeholder identity.
    let partition = state.caller.partition().clone();
    let caller = match CallerContext::authenticated_remote(claims.actor_id, claims.device_id, partition) {
        Ok(c) => c,
        Err(_) => return auth_denied(&request),
    };

    request.extensions_mut().insert(caller);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-signing-secret-0123456789";

    #[test]
    fn issue_then_verify_roundtrip_yields_claims() {
        let replay = ReplayCache::new();
        let token = issue_token(SECRET, "actor-1", "device-1", 60);
        let claims = verify_token(SECRET, &token, &replay).expect("valid token verifies");
        assert_eq!(claims.actor_id, "actor-1");
        assert_eq!(claims.device_id, "device-1");
    }

    #[test]
    fn expired_token_is_rejected() {
        let replay = ReplayCache::new();
        // ttl_secs.max(1) means we cannot issue an already-expired token via
        // the public API; sign one directly with a past `exp` instead.
        let exp = now() - 5;
        let actor_b64 = b64_encode(b"actor-1");
        let device_b64 = b64_encode(b"device-1");
        let nonce_b64 = b64_encode(b"0123456789abcdef");
        let payload = format!("{TOKEN_VERSION}.{actor_b64}.{device_b64}.{exp}.{nonce_b64}");
        let sig = sign(SECRET, &payload);
        let token = format!("{payload}.{sig}");

        let err = verify_token(SECRET, &token, &replay).unwrap_err();
        assert_eq!(err, TokenError::Expired);
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let replay = ReplayCache::new();
        let token = issue_token(SECRET, "actor-1", "device-1", 60);
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[5] = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let tampered = parts.join(".");

        let err = verify_token(SECRET, &tampered, &replay).unwrap_err();
        assert_eq!(err, TokenError::BadSignature);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let replay = ReplayCache::new();
        let token = issue_token(SECRET, "actor-1", "device-1", 60);
        let err = verify_token(b"a-completely-different-secret", &token, &replay).unwrap_err();
        assert_eq!(err, TokenError::BadSignature);
    }

    #[test]
    fn replayed_nonce_is_rejected_on_second_use() {
        let replay = ReplayCache::new();
        let token = issue_token(SECRET, "actor-1", "device-1", 60);
        assert!(verify_token(SECRET, &token, &replay).is_ok());
        let err = verify_token(SECRET, &token, &replay).unwrap_err();
        assert_eq!(err, TokenError::Replayed);
    }

    #[test]
    fn malformed_token_shapes_are_rejected() {
        let replay = ReplayCache::new();
        for bad in [
            "",
            "not-a-token",
            "krav1.only.three.parts",
            "wrongversion.a.b.123.c.d",
        ] {
            let err = verify_token(SECRET, bad, &replay).unwrap_err();
            assert_eq!(err, TokenError::Malformed, "input: {bad:?}");
        }
    }

    #[test]
    fn distinct_tokens_for_same_identity_have_distinct_nonces_and_both_verify() {
        let replay = ReplayCache::new();
        let t1 = issue_token(SECRET, "actor-1", "device-1", 60);
        let t2 = issue_token(SECRET, "actor-1", "device-1", 60);
        assert_ne!(t1, t2, "nonce must differ between issuances");
        assert!(verify_token(SECRET, &t1, &replay).is_ok());
        assert!(verify_token(SECRET, &t2, &replay).is_ok());
    }
}
