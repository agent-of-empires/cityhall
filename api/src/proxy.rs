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

    if let Some(resp) = admin_access_interception(&parts, &caller)? {
        return Ok(resp);
    }

    // Admin access: a valid impersonation cookie routes this browser to the
    // target user's workspace. Everything downstream (start, activity,
    // websocket leases) is keyed to the target so their idle accounting
    // keeps working; the admin's own workspace is untouched.
    let jar = axum_extra::extract::cookie::CookieJar::from_headers(&parts.headers);
    let user_id = match jar.get(ADMIN_COOKIE) {
        Some(cookie) => {
            let authorized = decode_token(cookie.value(), SESSION_PURPOSE)
                .filter(|t| t.a == caller.user.id)
                .filter(|_| caller.require("workspaces.impersonate").is_ok());
            match authorized {
                Some(token) => token.t,
                // Expired, tampered, or revoked: refuse rather than silently
                // switching this browser back to the admin's own workspace
                // (the UI would keep looking like the target's).
                None => return Ok(admin_access_denied()),
            }
        }
        None => {
            caller.require("workspaces.use")?;
            caller.user.id
        }
    };

    // Request-driven start: any hit on the proxy resumes a stopped workspace.
    let addr = workspaces::ensure_started(&state, user_id).await?;
    state.activity.touch(user_id);

    let path_q = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = format!("http://{addr}{path_q}");

    let result = if is_websocket_upgrade(&parts.headers) {
        websocket_tunnel(&state, &mut parts, &target, user_id).await
    } else {
        http_forward(&state, &parts, body, &target).await
    };
    if result.is_err() {
        // The workspace may have died out-of-band (crashed container, killed
        // process): drop the cached address so the next request reconciles
        // with the backend instead of dialing a dead endpoint forever.
        state.endpoints.invalidate(user_id);
    }
    result
}

/// Query parameter carrying a freshly minted admin access token.
pub const ACCESS_PARAM: &str = "cityhall_ws_access";
/// Query parameter ending admin access (clears the cookie).
const EXIT_PARAM: &str = "cityhall_ws_exit";
/// Cookie scoping this browser's proxy requests to another user's workspace.
const ADMIN_COOKIE: &str = "cityhall_ws_admin";
const EXCHANGE_PURPOSE: &str = "ws-exchange";
const SESSION_PURPOSE: &str = "ws-session";
/// How long a minted access URL stays exchangeable.
const EXCHANGE_TTL_SECS: i64 = 120;
/// How long an admin browses the target's workspace before re-opening.
const SESSION_TTL_SECS: i64 = 30 * 60;

/// Encrypted admin-access token payload (URL and cookie).
#[derive(serde::Serialize, serde::Deserialize)]
struct AccessToken {
    v: u8,
    /// Purpose: exchange (URL) vs session (cookie), so one cannot stand in
    /// for the other.
    p: String,
    /// The admin's user id; only their session can use the token.
    a: i32,
    /// The target user whose workspace is opened.
    t: i32,
    /// Unix expiry.
    exp: i64,
}

/// Mint the URL token `POST /api/workspaces/{id}/access-url` returns.
pub fn mint_exchange_token(admin_id: i32, target_id: i32) -> Result<String, AppError> {
    mint_token(admin_id, target_id, EXCHANGE_PURPOSE, EXCHANGE_TTL_SECS)
}

fn mint_token(admin_id: i32, target_id: i32, purpose: &str, ttl: i64) -> Result<String, AppError> {
    let payload = serde_json::to_string(&AccessToken {
        v: 1,
        p: purpose.to_string(),
        a: admin_id,
        t: target_id,
        exp: chrono::Utc::now().timestamp() + ttl,
    })
    .map_err(|_| AppError::Internal("failed to encode access token"))?;
    crate::crypto::encrypt(&payload)
}

/// Decrypt and validate a token; `None` for anything tampered, expired, or
/// of the wrong purpose (callers respond generically, revealing nothing).
fn decode_token(raw: &str, purpose: &str) -> Option<AccessToken> {
    validate_payload(&crate::crypto::decrypt(raw).ok()?, purpose)
}

fn validate_payload(json: &str, purpose: &str) -> Option<AccessToken> {
    let token: AccessToken = serde_json::from_str(json).ok()?;
    (token.v == 1 && token.p == purpose && token.exp > chrono::Utc::now().timestamp())
        .then_some(token)
}

/// Handle the admin-access control queries (`cityhall_ws_access` exchange,
/// `cityhall_ws_exit`) if present. Returns the response to short-circuit
/// with, or `None` for a normal proxied request.
fn admin_access_interception(
    parts: &request::Parts,
    caller: &AuthUser,
) -> Result<Option<Response<Body>>, AppError> {
    let query = parts.uri.query().unwrap_or("");
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();

    if pairs.iter().any(|(k, _)| k == EXIT_PARAM) {
        tracing::info!(
            event = "workspace_admin_access_ended",
            admin_user_id = caller.user.id,
            "admin workspace access ended"
        );
        return Ok(Some(redirect_without(
            parts,
            &pairs,
            EXIT_PARAM,
            clear_cookie(),
        )?));
    }

    let Some((_, raw)) = pairs.iter().find(|(k, _)| k == ACCESS_PARAM) else {
        return Ok(None);
    };
    let token = decode_token(raw, EXCHANGE_PURPOSE)
        .filter(|t| t.a == caller.user.id)
        .ok_or(AppError::Forbidden("invalid or expired access link"))?;
    caller.require("workspaces.impersonate")?;

    let peer_ip = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| peer.ip().to_string());
    tracing::info!(
        event = "workspace_admin_access_started",
        admin_user_id = caller.user.id,
        admin_username = %caller.user.username,
        target_user_id = token.t,
        peer_ip = peer_ip.as_deref().unwrap_or("unknown"),
        "admin entered another user's workspace"
    );

    let session = mint_token(token.a, token.t, SESSION_PURPOSE, SESSION_TTL_SECS)?;
    let secure = parts
        .headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"));
    let cookie = format!(
        "{ADMIN_COOKIE}={session}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_TTL_SECS}{}",
        if secure { "; Secure" } else { "" }
    );
    Ok(Some(redirect_without(parts, &pairs, ACCESS_PARAM, cookie)?))
}

/// A 303 to the same URI minus `param` (other query params preserved),
/// carrying `set_cookie`. Marked uncacheable and referrer-free so the
/// token-bearing URL leaks nowhere.
fn redirect_without(
    parts: &request::Parts,
    pairs: &[(String, String)],
    param: &str,
    set_cookie: String,
) -> Result<Response<Body>, AppError> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs.iter().filter(|(k, _)| k != param) {
        serializer.append_pair(k, v);
    }
    let query = serializer.finish();
    let location = if query.is_empty() {
        parts.uri.path().to_string()
    } else {
        format!("{}?{query}", parts.uri.path())
    };
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header("location", location)
        .header("set-cookie", set_cookie)
        .header("cache-control", "no-store")
        .header("referrer-policy", "no-referrer")
        .body(Body::empty())
        .map_err(|_| AppError::Internal("failed to build redirect"))
}

fn clear_cookie() -> String {
    format!("{ADMIN_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

/// 403 clearing the stale impersonation cookie; the next request is a normal
/// self-routed one.
fn admin_access_denied() -> Response<Body> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("set-cookie", clear_cookie())
        .header("cache-control", "no-store")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"error":"workspace admin access expired or revoked; reload to continue"}"#,
        ))
        .unwrap_or_else(|_| StatusCode::FORBIDDEN.into_response())
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
    fn access_payload_validation() {
        let fresh = chrono::Utc::now().timestamp() + 60;
        let ok = format!(r#"{{"v":1,"p":"ws-exchange","a":1,"t":2,"exp":{fresh}}}"#);
        let token = validate_payload(&ok, EXCHANGE_PURPOSE).unwrap();
        assert_eq!((token.a, token.t), (1, 2));

        // Purpose confusion: a URL token is not a session cookie.
        assert!(validate_payload(&ok, SESSION_PURPOSE).is_none());
        // Expired.
        let stale = r#"{"v":1,"p":"ws-exchange","a":1,"t":2,"exp":1}"#;
        assert!(validate_payload(stale, EXCHANGE_PURPOSE).is_none());
        // Unknown version or garbage.
        let vnext = format!(r#"{{"v":2,"p":"ws-exchange","a":1,"t":2,"exp":{fresh}}}"#);
        assert!(validate_payload(&vnext, EXCHANGE_PURPOSE).is_none());
        assert!(validate_payload("not json", EXCHANGE_PURPOSE).is_none());
    }

    #[test]
    fn redirect_strips_only_the_access_param() {
        let req = Request::builder()
            .uri("/sessions/abc?cityhall_ws_access=tok&tab=2")
            .body(Body::empty())
            .unwrap();
        let (parts, _) = req.into_parts();
        let pairs: Vec<(String, String)> =
            url::form_urlencoded::parse(parts.uri.query().unwrap().as_bytes())
                .into_owned()
                .collect();
        let resp = redirect_without(&parts, &pairs, ACCESS_PARAM, clear_cookie()).unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get("location").unwrap(),
            "/sessions/abc?tab=2"
        );
        assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
        // The stale cookie is cleared on exit.
        let cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
        assert!(cookie.starts_with("cityhall_ws_admin=;"));
        assert!(cookie.contains("Max-Age=0"));
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
