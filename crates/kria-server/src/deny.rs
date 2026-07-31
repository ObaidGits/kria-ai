//! Shared non-revealing deny envelope (MGR-003 AC3 — F1.6.4).
//!
//! F1.6.1–1.6.3 built several independent boundaries — bind/loopback
//! (`bind_security`), authentication (`auth`), origin (`origin`), rate
//! limiting (`rate_limit`), and capability gating (`memory_routes`) — each of
//! which constructed its own "non-revealing" deny JSON body ad hoc. Two real
//! inconsistencies existed before this module:
//!
//! - `auth::auth_denied()` and `memory_routes::capability_denied()` omitted
//!   `correlation_id` entirely, while `origin::origin_denied()` and
//!   `rate_limit::rate_limited()` included it — an operator could not always
//!   correlate a denied request to a specific log line depending on WHICH
//!   boundary denied it.
//! - the universal `RequestBodyLimitLayer`/`TimeoutLayer` (tower-http) built-
//!   in rejections returned their own default bodies (a plain-text
//!   `"length limit exceeded"` message, and an empty timeout body) instead
//!   of the JSON envelope convention every other deny path in this crate
//!   uses.
//!
//! This module fixes both by giving every deny path — custom middleware and
//! tower-http built-ins alike — ONE envelope constructor
//! ([`deny_envelope`]) and ONE middleware ([`normalize_builtin_denies`]) that
//! rewrites the two universal tower-http limit rejections into that same
//! shape.
//!
//! ## What "non-revealing" means here (resolving MGR-003 AC3 ambiguity)
//!
//! MGR-003 AC3 requires a deny response to reveal "no protected label,
//! identifier, count, topology, or reason detail". Design §8 states
//! `Unauthorized/Forbidden` (i.e. denials **within** the same boundary
//! category) "share status, body length class, timing budget, and empty
//! safe details remotely", and separately gives a canonical, deliberately
//! **category-distinguishing** HTTP status mapping (`401/403` auth, `413`
//! limit, `429` rate, `504`/`408` timeout, etc. — design §19 "Errors").
//! Read together: AC3's non-revealing requirement is scoped to WITHIN one
//! denial category (a caller cannot tell "malformed" from "expired" from
//! "replayed" bearer-token failures — all three are the identical `401`
//! `{"error":"unauthorized",...}` body) — it does NOT require collapsing
//! *different* boundary categories (auth vs. origin vs. rate-limit vs. body-
//! size) into one indistinguishable status/shape. The category itself
//! (which boundary rejected the request) is not a "protected label,
//! identifier, count, topology, or reason detail" about the caller's
//! memory/graph state — it is public API contract shape, already documented
//! per-category in design §19's HTTP mapping table. Making rate-limiting
//! return `403` identical to an auth failure would in fact be WRONG per that
//! canonical mapping. This module therefore normalizes the FIELD SET, field
//! order, and body-length class consistently ACROSS every category, while
//! preserving each category's own canonical status code.
//!
//! ## Length normalization
//!
//! Every envelope is a flat two-key JSON object: `error` (a short, fixed,
//! non-parameterized category string — never a caller-supplied value, an
//! identifier, or a count) and `correlation_id` (a `null` or UUID string).
//! Category strings across all denial paths (`unauthorized`,
//! `origin_not_allowed`, `rate_limited`, `unsupported_capability`,
//! `payload_too_large`, `request_timeout`) are all short, similar-length,
//! lowercase snake_case tokens, so the resulting bodies fall into one
//! narrow byte-length band (see the crate's shape-comparison test) without
//! fabricating meaningless padding — "sufficiently" normalized per this
//! task's own wording, not byte-identical.

use axum::{http::StatusCode, response::Response, Json};

use crate::correlation::CorrelationId;

/// Build the one non-revealing deny envelope shape used by every boundary in
/// this crate (loopback/auth/origin/replay/rate/limit — MGR-003 AC3).
///
/// `category` is a short, fixed, non-parameterized string identifying WHICH
/// boundary denied the request (`"unauthorized"`, `"origin_not_allowed"`,
/// `"rate_limited"`, `"unsupported_capability"`, `"payload_too_large"`,
/// `"request_timeout"`) — never caller-supplied content, an identifier, a
/// count, or a topology detail. `correlation_id` is an opaque per-request
/// diagnostic token (see `correlation` module docs for why echoing it is not
/// itself a protected disclosure) serialized as `null` when unavailable so
/// every envelope has the exact same field set regardless of category.
pub fn deny_envelope(
    status: StatusCode,
    category: &'static str,
    correlation_id: Option<CorrelationId>,
) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": category,
            "correlation_id": correlation_id.map(|c| c.to_string()),
        })),
    )
        .into_response()
}

// `Response` already implements `IntoResponse` (identity), and `(StatusCode,
// Json<Value>)` needs `.into_response()` to become one — import brought in
// via the `IntoResponse` trait used above.
use axum::response::IntoResponse;

/// Rewrite the two UNIVERSAL tower-http built-in limit rejections
/// (`RequestBodyLimitLayer`'s `413`, `TimeoutLayer`'s configured timeout
/// status) into the shared [`deny_envelope`] shape instead of their raw
/// default bodies (a plain-text `"length limit exceeded"` message, and an
/// empty timeout body respectively) — MGR-003 AC3/F1.6.4: every deny path,
/// including these universal (loopback-and-remote) ones, uses the same JSON
/// envelope convention.
///
/// Layered so it runs AFTER `correlation::correlation_middleware` has
/// already inserted the request's [`CorrelationId`] extension but BEFORE
/// `RequestBodyLimitLayer`/`TimeoutLayer` (see `lib.rs::build_router`), so it
/// can both read the correlation id and observe/replace their responses on
/// the way back out.
///
/// Only these two specific statuses are rewritten — a `404`/`500`/`503`
/// application-level response from a route handler is untouched; those are
/// ordinary application errors, not MGR-003 threat-boundary denials, and
/// rewriting them would be out of this task's scope.
pub async fn normalize_builtin_denies(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let correlation_id = crate::correlation::correlation_id_of(&request);
    let response = next.run(request).await;

    match response.status() {
        StatusCode::PAYLOAD_TOO_LARGE => {
            deny_envelope(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", correlation_id)
        }
        StatusCode::REQUEST_TIMEOUT => {
            deny_envelope(StatusCode::REQUEST_TIMEOUT, "request_timeout", correlation_id)
        }
        _ => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The comparable-shape assertion: at least 3 different denial
    /// categories share the same JSON field set/schema and a comparable body
    /// length class — "sufficiently" normalized per this task, not
    /// byte-identical (see module docs on the chosen tolerance: same two
    /// keys, same value types, similar-length category strings).
    #[tokio::test]
    async fn distinct_denial_categories_share_the_same_envelope_schema_and_length_class() {
        let correlation_id = Some(CorrelationId(uuid::Uuid::new_v4()));

        let cases: Vec<(StatusCode, &'static str)> = vec![
            (StatusCode::UNAUTHORIZED, "unauthorized"),
            (StatusCode::FORBIDDEN, "origin_not_allowed"),
            (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            (StatusCode::FORBIDDEN, "unsupported_capability"),
            (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            (StatusCode::REQUEST_TIMEOUT, "request_timeout"),
        ];

        let mut lengths = Vec::new();
        for (status, category) in cases {
            let response = deny_envelope(status, category, correlation_id);
            assert_eq!(response.status(), status);

            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

            // Same schema: exactly the two keys, in the same value-type shape.
            let obj = value.as_object().unwrap();
            assert_eq!(obj.len(), 2, "category {category}: exactly two fields");
            assert!(obj.get("error").unwrap().is_string());
            assert!(obj.get("correlation_id").unwrap().is_string());

            lengths.push(body.len());
        }

        // Comparable body length class: every category-string token is
        // short/lowercase/snake_case, so the resulting bodies fall within a
        // narrow band. Tolerance chosen generously (not byte-identical) —
        // the shortest and longest category strings here differ by ~15
        // bytes, so a 40-byte band comfortably covers legitimate variation
        // while still catching a verbose/leaky outlier.
        let min = *lengths.iter().min().unwrap();
        let max = *lengths.iter().max().unwrap();
        assert!(
            max - min <= 40,
            "deny envelope lengths should be comparable across categories, got {lengths:?}"
        );
    }

    #[tokio::test]
    async fn correlation_id_is_null_when_unavailable_not_omitted() {
        let response = deny_envelope(StatusCode::UNAUTHORIZED, "unauthorized", None);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 2, "field is present as null, not omitted");
        assert!(obj.get("correlation_id").unwrap().is_null());
    }
}
