//! Per-request correlation ID (MGR-003 AC2 "audit logging" — F1.6.3).
//!
//! Every request is assigned a server-generated correlation ID before it
//! reaches any route or other security middleware (origin/auth/rate-limit).
//! The ID is:
//!
//! - **Always server-generated**, never taken from an incoming `X-Request-Id`
//!   (or similar) header. A remote/untrusted caller controlling its own
//!   correlation ID would let it pollute audit logs with arbitrary caller-
//!   chosen values (log injection / cross-request confusion) — see the task
//!   scope note this module implements.
//! - Recorded on the `tracing` span that wraps the rest of the request, so
//!   every log line emitted while handling the request correlates (MGR-003
//!   AC2 "audit logging").
//! - Returned on every response via the `x-correlation-id` header — success
//!   or deny — so an operator/legitimate caller can report a specific
//!   request for investigation. A correlation ID is an opaque diagnostic
//!   token, not a protected label/identifier/count/topology (MGR-003 AC3
//!   only forbids revealing THOSE on deny) — every deny path in this crate
//!   (see `deny::deny_envelope`, F1.6.4) echoes it in the JSON deny body
//!   itself, not just the header.
//! - Inserted into the request's extensions as [`CorrelationId`] so deny
//!   helpers deeper in the stack (auth, rate-limit) can read the SAME id
//!   that is already bound to the wrapping tracing span.

use axum::{
    extract::Request,
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use tracing::Instrument;
use uuid::Uuid;

/// The correlation ID assigned to one request, threaded through extensions.
#[derive(Debug, Clone, Copy)]
pub struct CorrelationId(pub Uuid);

impl std::fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Read the correlation ID an earlier [`correlation_middleware`] pass already
/// attached to `request`'s extensions, if any. Deny helpers that only have
/// access to `&Request` (not a typed `Extension<CorrelationId>` handler
/// parameter) use this instead of generating a second, inconsistent ID.
pub fn correlation_id_of(request: &Request) -> Option<CorrelationId> {
    request.extensions().get::<CorrelationId>().copied()
}

/// Outermost-layered middleware (see `lib.rs::build_router`): generates one
/// server-side correlation ID per request, inserts it into the request's
/// extensions, wraps the rest of the request in a `tracing` span carrying it,
/// and stamps the response header on the way back out — including responses
/// produced by inner layers (body-size/timeout/concurrency limits, origin/
/// auth/rate-limit denials, and every route handler).
pub async fn correlation_middleware(mut request: Request, next: Next) -> Response {
    let id = CorrelationId(Uuid::new_v4());
    request.extensions_mut().insert(id);

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let span = tracing::info_span!(
        "http_request",
        correlation_id = %id,
        method = %method,
        path = %path,
    );

    async move {
        let mut response = next.run(request).await;
        if let Ok(value) = HeaderValue::from_str(&id.to_string()) {
            response.headers_mut().insert("x-correlation-id", value);
        }
        response
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn response_carries_a_correlation_id_header() {
        let app: Router = Router::new()
            .route("/ok", get(|| async { "hi" }))
            .layer(axum::middleware::from_fn(correlation_middleware));

        let res = app
            .oneshot(HttpRequest::get("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let header = res
            .headers()
            .get("x-correlation-id")
            .expect("correlation header present")
            .to_str()
            .unwrap();
        assert!(Uuid::parse_str(header).is_ok(), "header is a valid UUID");
    }

    #[tokio::test]
    async fn distinct_requests_get_distinct_correlation_ids() {
        let app: Router = Router::new()
            .route("/ok", get(|| async { "hi" }))
            .layer(axum::middleware::from_fn(correlation_middleware));

        let res1 = app
            .clone()
            .oneshot(HttpRequest::get("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let res2 = app
            .oneshot(HttpRequest::get("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let id1 = res1.headers().get("x-correlation-id").unwrap().to_str().unwrap();
        let id2 = res2.headers().get("x-correlation-id").unwrap().to_str().unwrap();
        assert_ne!(id1, id2);
    }
}
