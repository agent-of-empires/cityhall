//! Workspace reverse proxy.
//!
//! Served on a dedicated second listener (not a path prefix of the main app):
//! the aoe dashboard uses root-absolute asset and WebSocket paths, so it gets
//! its own origin. In development that is just another loopback port; cookies
//! are host-scoped (ports ignored), so the regular CityHall session cookie
//! authenticates here with zero extra configuration, while browser origin
//! isolation still holds. In production the operator maps a subdomain or a
//! second external port to this listener.
//!
//! Every request is gated on the CityHall session (`workspaces.use`), request-
//! starts the caller's workspace, and is forwarded to it. The workspace itself
//! runs `aoe serve --auth=none --behind-proxy` and is only reachable through
//! loopback, so CityHall is the sole auth boundary.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::header::{
    HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONNECTION, CONTENT_LENGTH, COOKIE, HOST,
    SEC_WEBSOCKET_EXTENSIONS, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_PROTOCOL, SEC_WEBSOCKET_VERSION,
    TRANSFER_ENCODING, UPGRADE,
};
use axum::http::{request, Request, Response, StatusCode, Version};
use axum::response::IntoResponse;
use axum::Router;
use hyper_util::rt::TokioIo;
use tower_http::trace::TraceLayer;

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::state::AppState;
use crate::workspaces;

/// Address of the workspace proxy listener.
pub fn bind_addr() -> String {
    std::env::var("WORKSPACE_PROXY_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3001".to_string())
}

/// The origin browsers should use to reach the workspace proxy: the
/// `WORKSPACE_PROXY_PUBLIC_ORIGIN` override (production, behind a reverse
/// proxy), or the request's hostname with the proxy port (development).
pub fn public_origin(headers: &HeaderMap) -> String {
    if let Ok(origin) = std::env::var("WORKSPACE_PROXY_PUBLIC_ORIGIN") {
        return origin.trim_end_matches('/').to_string();
    }
    let host = headers
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1");
    let hostname = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    let bind = bind_addr();
    let port = bind.rsplit_once(':').map(|(_, p)| p).unwrap_or("3001");
    format!("http://{hostname}:{port}")
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .fallback(handler)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn handler(State(state): State<AppState>, req: Request<Body>) -> Response<Body> {
    match proxy(state, req).await {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

async fn proxy(state: AppState, req: Request<Body>) -> Result<Response<Body>, AppError> {
    let (mut parts, body) = req.into_parts();
    let caller = AuthUser::from_request_parts(&mut parts, &state).await?;
    caller.require("workspaces.use")?;
    let user_id = caller.user.id;

    // Request-driven start: any hit on the proxy resumes a stopped workspace.
    let addr = workspaces::ensure_started(&state, user_id).await?;
    state.activity.touch(user_id);

    let path_q = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = format!("http://{addr}{path_q}");

    if is_websocket_upgrade(&parts.headers) {
        websocket_tunnel(&state, &mut parts, &target, user_id).await
    } else {
        http_forward(&state, &parts, body, &target).await
    }
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// Hop-by-hop headers plus CityHall credentials and inbound forwarding
/// headers; none of these may reach the workspace.
const SKIP_REQUEST_HEADERS: &[HeaderName] = &[
    HOST,
    CONNECTION,
    UPGRADE,
    TRANSFER_ENCODING,
    CONTENT_LENGTH,
    COOKIE,
    AUTHORIZATION,
];

fn forward_headers(parts: &request::Parts) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        let lower = name.as_str();
        if SKIP_REQUEST_HEADERS.contains(name)
            || lower.starts_with("x-forwarded-")
            || lower == "keep-alive"
            || lower == "te"
            || lower == "trailer"
            || lower.starts_with("proxy-")
        {
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    if let Some(host) = parts.headers.get(HOST) {
        headers.insert("x-forwarded-host", host.clone());
    }
    headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    if let Some(ConnectInfo(peer)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() {
        if let Ok(v) = HeaderValue::from_str(&peer.ip().to_string()) {
            headers.insert("x-forwarded-for", v);
        }
    }
    headers
}

async fn http_forward(
    state: &AppState,
    parts: &request::Parts,
    body: Body,
    target: &str,
) -> Result<Response<Body>, AppError> {
    let upstream = state
        .proxy_client
        .request(parts.method.clone(), target)
        .headers(forward_headers(parts))
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(|e| AppError::WorkspaceUnavailable(format!("workspace request failed: {e}")))?;

    let mut builder = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers() {
        let lower = name.as_str();
        if lower == "connection" || lower == "keep-alive" || lower == "transfer-encoding" {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(|_| AppError::Internal("failed to build proxied response"))
}

/// Bridge a WebSocket: proxy the handshake to the workspace, and only after
/// its `101` return one downstream, then blindly pipe bytes both ways. The
/// tunnel holds an activity lease so the idle sweeper never cuts a live
/// terminal.
async fn websocket_tunnel(
    state: &AppState,
    parts: &mut request::Parts,
    target: &str,
    user_id: i32,
) -> Result<Response<Body>, AppError> {
    let on_upgrade = parts
        .extensions
        .remove::<hyper::upgrade::OnUpgrade>()
        .ok_or(AppError::BadRequest("connection is not upgradable"))?;

    let mut headers = forward_headers(parts);
    headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    for name in [
        SEC_WEBSOCKET_KEY,
        SEC_WEBSOCKET_VERSION,
        SEC_WEBSOCKET_PROTOCOL,
        SEC_WEBSOCKET_EXTENSIONS,
    ] {
        if let Some(v) = parts.headers.get(&name) {
            headers.insert(name, v.clone());
        }
    }

    let upstream = state
        .proxy_client
        .get(target)
        .version(Version::HTTP_11)
        .headers(headers)
        .send()
        .await
        .map_err(|e| AppError::WorkspaceUnavailable(format!("workspace handshake failed: {e}")))?;

    if upstream.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(AppError::WorkspaceUnavailable(format!(
            "workspace rejected websocket upgrade: {}",
            upstream.status()
        )));
    }

    // Echo the workspace's actual negotiation (accept key, selected
    // subprotocol, extensions), never the client's request.
    let mut builder = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
    for name in [
        UPGRADE,
        CONNECTION,
        HeaderName::from_static("sec-websocket-accept"),
        SEC_WEBSOCKET_PROTOCOL,
        SEC_WEBSOCKET_EXTENSIONS,
    ] {
        if let Some(v) = upstream.headers().get(&name) {
            builder = builder.header(name, v.clone());
        }
    }
    let response = builder
        .body(Body::empty())
        .map_err(|_| AppError::Internal("failed to build upgrade response"))?;

    let activity = state.activity.clone();
    activity.websocket_started(user_id);
    tokio::spawn(async move {
        let result = async {
            let upstream_io = upstream
                .upgrade()
                .await
                .map_err(|e| format!("upstream upgrade failed: {e}"))?;
            let downstream_io = on_upgrade
                .await
                .map_err(|e| format!("downstream upgrade failed: {e}"))?;
            let mut downstream = TokioIo::new(downstream_io);
            let mut upstream = upstream_io;
            tokio::io::copy_bidirectional(&mut downstream, &mut upstream)
                .await
                .map_err(|e| format!("tunnel closed with error: {e}"))?;
            Ok::<_, String>(())
        }
        .await;
        if let Err(e) = result {
            tracing::debug!(user_id, "websocket tunnel ended: {e}");
        }
        activity.websocket_ended(user_id);
    });

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_detection() {
        let mut headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
        headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
        assert!(is_websocket_upgrade(&headers));
        headers.insert(UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket_upgrade(&headers));
    }

    #[test]
    fn public_origin_derives_from_request_host() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("cityhall.local:3000"));
        // No env override in tests; derived from hostname + proxy port.
        assert_eq!(public_origin(&headers), "http://cityhall.local:3001");
    }

    #[test]
    fn forwarded_headers_strip_credentials() {
        let (mut parts, _) = Request::new(Body::empty()).into_parts();
        parts
            .headers
            .insert(COOKIE, HeaderValue::from_static("cityhall_session=x"));
        parts
            .headers
            .insert(AUTHORIZATION, HeaderValue::from_static("Bearer y"));
        parts
            .headers
            .insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        parts
            .headers
            .insert("accept", HeaderValue::from_static("text/html"));
        let out = forward_headers(&parts);
        assert!(out.get(COOKIE).is_none());
        assert!(out.get(AUTHORIZATION).is_none());
        // Inbound spoofable forwarding headers are dropped, not passed on.
        assert!(out.get("x-forwarded-for").is_none());
        assert_eq!(out.get("accept").unwrap(), "text/html");
    }
}
