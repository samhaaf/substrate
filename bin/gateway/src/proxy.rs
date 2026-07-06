//! REST proxy — forward requests to upstream node or GC services.
//!
//! Routes:
//! - `/api/inference/*path` → `{inference_url}/{path}`
//! - `/api/gc/*path`   → `{gc_url}/{path}`
//!
//! Method, headers, and body are forwarded verbatim. The upstream response
//! (status code, headers, body) is returned to the caller unchanged.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};

use crate::state::GatewayState;

/// Proxy `ANY /api/inference/*path` → inference, rewriting paths to include `/v1/` prefix.
///
/// Path mapping rules (applied in order):
/// 1. `health`  → `/health`    (liveness probe, no version prefix)
/// 2. `metrics` → `/metrics`   (Prometheus scrape, no version prefix)
/// 3. `v1/...`  → `/v1/...`    (already versioned — pass through)
/// 4. everything else → `/v1/{path}` (implicit v1 prefix)
pub async fn proxy_inference(
    State(state): State<Arc<GatewayState>>,
    Path(path): Path<String>,
    method: Method,
    headers: HeaderMap,
    req: Request,
) -> impl IntoResponse {
    let inference_base = state.config.inference_url.trim_end_matches('/');

    let upstream_path = if path == "health" || path == "metrics" {
        // Root-level probes — no version prefix.
        format!("/{path}")
    } else if path.starts_with("v1/") || path == "v1" {
        // Caller already included the version segment.
        format!("/{path}")
    } else {
        // Default: prepend /v1/ so dashboard calls like `/api/inference/completions`
        // reach `/v1/completions` on the inference service.
        format!("/v1/{path}")
    };

    let target = format!("{inference_base}{upstream_path}");
    forward(&state.http, &target, method, headers, req).await
}

/// Proxy `GET /api/gc/*path` → `{gc_url}/{path}`.
pub async fn proxy_gc(
    State(state): State<Arc<GatewayState>>,
    Path(path): Path<String>,
    method: Method,
    headers: HeaderMap,
    req: Request,
) -> impl IntoResponse {
    let target = format!("{}/{}", state.config.gc_url.trim_end_matches('/'), path);
    forward(&state.http, &target, method, headers, req).await
}

/// Forward an HTTP request to `target_url` and return the upstream response.
async fn forward(
    client: &reqwest::Client,
    target_url: &str,
    method: Method,
    headers: HeaderMap,
    req: Request,
) -> Response {
    // Collect request body.
    let body_bytes = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("failed to read request body: {e}"),
            )
                .into_response();
        }
    };

    // Build upstream request.
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);

    let mut upstream = client.request(reqwest_method, target_url);

    // Forward headers, skipping hop-by-hop headers.
    for (name, value) in &headers {
        let n = name.as_str().to_lowercase();
        if matches!(
            n.as_str(),
            "connection" | "keep-alive" | "transfer-encoding" | "te"
                | "trailer" | "upgrade" | "proxy-authorization" | "proxy-authenticate"
        ) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            upstream = upstream.header(name.as_str(), v);
        }
    }

    upstream = upstream.body(body_bytes);

    // Execute upstream request.
    match upstream.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            let mut response_headers = axum::http::HeaderMap::new();
            for (name, value) in resp.headers() {
                let n = name.as_str().to_lowercase();
                if matches!(
                    n.as_str(),
                    "connection" | "keep-alive" | "transfer-encoding" | "content-encoding"
                ) {
                    continue;
                }
                if let (Ok(hn), Ok(hv)) = (
                    axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
                    axum::http::HeaderValue::from_bytes(value.as_bytes()),
                ) {
                    response_headers.insert(hn, hv);
                }
            }

            let body_bytes = resp.bytes().await.unwrap_or_default();

            let mut response = Response::builder().status(status);
            *response.headers_mut().unwrap() = response_headers;

            response
                .body(Body::from(body_bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            tracing::warn!(url = target_url, error = %e, "upstream proxy request failed");
            (
                StatusCode::BAD_GATEWAY,
                format!("upstream error: {e}"),
            )
                .into_response()
        }
    }
}
