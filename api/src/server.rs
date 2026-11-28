use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::header::ORIGIN;
use axum::http::Method;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::Router;
use sea_orm::DatabaseConnection;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::handlers::{
    agent_credentials, auth, dashboard, oidc, roles, settings, setup, signup, users,
    workspace_config, workspaces,
};
use crate::mailer::base_url;
use crate::proxy;
use crate::state::AppState;

/// Directory holding the built frontend. Overridable via `STATIC_DIR` for
/// container images that place the bundle elsewhere.
fn static_dir() -> PathBuf {
    std::env::var("STATIC_DIR")
        .unwrap_or_else(|_| "web/dist".to_string())
        .into()
}

pub fn api_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/change-password", post(auth::change_password))
        .route("/me/onboarding-dismissed", post(auth::dismiss_onboarding))
        .route("/auth/forgot-password", post(auth::forgot_password))
        .route("/auth/reset-password", post(auth::reset_password))
        .route("/auth/providers", get(oidc::providers))
        .route("/auth/oidc/login", get(oidc::login))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/auth/register", post(signup::register))
        .route("/auth/verify-email", post(signup::verify_email))
        .route("/users", get(users::list).post(users::create))
        .route(
            "/users/{id}",
            get(users::get).patch(users::update).delete(users::delete),
        )
        .route("/dashboard", get(dashboard::overview))
        .route(
            "/me/dashboard-layout",
            get(dashboard::get_layout).put(dashboard::put_layout),
        )
        .route("/roles", get(roles::list).post(roles::create))
        .route("/roles/{id}", patch(roles::update).delete(roles::delete))
        .route("/permissions", get(roles::permissions))
        .route(
            "/workspaces",
            get(workspaces::list).patch(workspaces::bulk_set_version),
        )
        .route("/workspaces/me", get(workspaces::me))
        .route("/workspaces/me/restart", post(workspaces::restart_mine))
        .route("/workspaces/versions", get(workspaces::versions))
        .route(
            "/workspaces/{user_id}",
            patch(workspaces::set_version).delete(workspaces::destroy),
        )
        .route("/workspaces/{user_id}/start", post(workspaces::start))
        .route(
            "/workspaces/{user_id}/access-url",
            post(workspaces::access_url),
        )
        .route("/workspaces/{user_id}/stop", post(workspaces::stop))
        .route("/settings/smtp", get(settings::get).put(settings::update))
        .route("/settings/smtp/test", post(settings::test))
        .route(
            "/settings/oidc",
            get(oidc::get_settings).put(oidc::update_settings),
        )
        .route(
            "/settings/signup",
            get(signup::get_settings).put(signup::update_settings),
        )
        .route(
            "/settings/workspaces",
            get(workspaces::get_settings).put(workspaces::update_settings),
        )
        .route(
            "/settings/workspace-config",
            get(workspace_config::get_config).put(workspace_config::update_config),
        )
        .route("/settings/setup", get(setup::get).put(setup::update))
        .route(
            "/me/git-credential",
            get(workspace_config::get_git_credential)
                .put(workspace_config::update_git_credential)
                .delete(workspace_config::delete_git_credential),
        )
        .route(
            "/me/git-ssh-key",
            get(workspace_config::get_git_ssh_key)
                .put(workspace_config::update_git_ssh_key)
                .delete(workspace_config::delete_git_ssh_key),
        )
        .route("/me/agent-credentials", get(agent_credentials::get_mine))
        .route(
            "/me/agent-credentials/{env_var}",
            put(agent_credentials::put_mine).delete(agent_credentials::delete_mine),
        )
        .route(
            "/users/{user_id}/agent-credentials",
            get(agent_credentials::get_for_user),
        )
        .route(
            "/users/{user_id}/agent-credentials/{env_var}",
            put(agent_credentials::put_for_user).delete(agent_credentials::delete_for_user),
        )
        // Fetched by a workspace container, authenticated by its own bearer
        // token rather than a session cookie.
        .route("/workspace-bundle", get(workspace_config::serve_bundle))
        // Unknown /api/* paths return a JSON 404 instead of falling through to
        // the SPA index (which the outer fallback_service would otherwise serve).
        .fallback(|| async { AppError::NotFound("not found") })
        .layer(middleware::from_fn(same_origin_only))
        .with_state(state)
}

/// Whether `origin` (an `Origin` header value) is the origin of `base`.
/// `Url::origin` drops the path and default ports, as browsers do, and an
/// opaque origin (`null`) never parses to a match.
fn is_base_origin(origin: &str, base: &str) -> bool {
    let parse = |s: &str| url::Url::parse(s).ok().map(|u| u.origin());
    let origin = parse(origin);
    origin.is_some() && origin == parse(base)
}

/// CSRF guard. The session cookie is shared with the workspace proxy host once
/// scoped to the parent domain, so a page served there could otherwise fire
/// cookie-bearing POSTs (some need no preflight). Unsafe methods must come
/// from the CityHall origin: a foreign `Origin` is refused, and without
/// `Origin` a `Sec-Fetch-Site` other than `same-origin`/`none` is refused.
/// Requests carrying neither header (curl, API clients) pass.
async fn same_origin_only(req: Request, next: Next) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        let h = req.headers();
        let allowed = match h.get(ORIGIN) {
            Some(origin) => is_base_origin(origin.to_str().unwrap_or(""), &base_url(h)),
            None => h
                .get("sec-fetch-site")
                .is_none_or(|v| matches!(v.as_bytes(), b"same-origin" | b"none")),
        };
        if !allowed {
            return AppError::Forbidden("cross-origin request refused").into_response();
        }
    }
    next.run(req).await
}

pub fn router(state: AppState) -> Router {
    let dir = static_dir();
    // Serve the SPA: static assets first, fall back to index.html so client
    // routes (e.g. /login) resolve on hard refresh.
    let index = dir.join("index.html");
    // `.fallback` (not `.not_found_service`, which forces a 404 status) so
    // client routes like /login resolve with 200 on hard refresh.
    let spa = ServeDir::new(&dir).fallback(ServeFile::new(index));

    Router::new()
        .nest("/api", api_router(state))
        .fallback_service(spa)
        .layer(TraceLayer::new_for_http())
}

pub fn build_state(db: DatabaseConnection) -> Result<AppState, Box<dyn std::error::Error>> {
    // HTTP/1.1 only and no redirect following: WebSocket upgrades need 1.1,
    // and a proxy passes redirects through rather than chasing them.
    let proxy_client = reqwest::Client::builder()
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let (orchestrator, provisioning) = crate::orchestrator::from_env()?;
    Ok(AppState {
        db,
        orchestrator,
        activity: Arc::default(),
        locks: Arc::default(),
        endpoints: Arc::default(),
        provisioning,
        versions: Arc::default(),
        metrics: Arc::default(),
        proxy_client,
    })
}

pub async fn serve(db: DatabaseConnection) -> Result<(), Box<dyn std::error::Error>> {
    let state = build_state(db)?;
    crate::workspaces::seed_default_version(&state.db).await?;
    tokio::spawn(crate::workspaces::idle_sweeper(state.clone()));
    tokio::spawn(crate::metrics::sampler(state.clone()));

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("cityhall listening on http://{addr}");

    let proxy_addr = proxy::bind_addr();
    let proxy_listener = tokio::net::TcpListener::bind(&proxy_addr).await?;
    tracing::info!("workspace proxy listening on http://{proxy_addr}");

    let main_srv = axum::serve(listener, router(state.clone()));
    // ConnectInfo gives the proxy the peer IP for X-Forwarded-For.
    let proxy_srv = axum::serve(
        proxy_listener,
        proxy::router(state).into_make_service_with_connect_info::<SocketAddr>(),
    );
    tokio::try_join!(main_srv, proxy_srv)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use sea_orm::Database;
    use tower::ServiceExt;

    async fn status(method: &str, path: &str, headers: &[(&str, &str)]) -> StatusCode {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "cityhall.test");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let app = api_router(build_state(db).unwrap());
        app.oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    const LOGOUT: &str = "/auth/logout";

    #[tokio::test]
    async fn foreign_origin_post_is_refused() {
        let got = status("POST", LOGOUT, &[("origin", "https://evil.test")]).await;
        assert_eq!(got, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn same_site_sibling_origin_post_is_refused() {
        let got = status("POST", LOGOUT, &[("origin", "http://ws.cityhall.test")]).await;
        assert_eq!(got, StatusCode::FORBIDDEN);
        // A state-changing route other than logout.
        let got = status(
            "POST",
            "/workspaces/1/stop",
            &[("origin", "http://ws.cityhall.test")],
        )
        .await;
        assert_eq!(got, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn null_origin_is_refused() {
        let got = status("POST", LOGOUT, &[("origin", "null")]).await;
        assert_eq!(got, StatusCode::FORBIDDEN);
    }

    #[test]
    fn base_url_origin_ignores_default_ports_and_path() {
        let base = "http://cityhall.test:80/app";
        assert!(is_base_origin("http://cityhall.test", base));
        assert!(is_base_origin("http://CITYHALL.test", base));
        assert!(is_base_origin(
            "https://cityhall.test",
            "https://cityhall.test:443"
        ));
        assert!(!is_base_origin("http://cityhall.test:8080", base));
        assert!(!is_base_origin("https://cityhall.test", base));
        assert!(!is_base_origin("", "cityhall.test"));
        assert!(!is_base_origin("null", base));
    }

    #[tokio::test]
    async fn post_without_origin_or_fetch_metadata_passes() {
        assert_eq!(status("POST", LOGOUT, &[]).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn fetch_metadata_decides_when_origin_is_absent() {
        for (site, want) in [
            ("same-site", StatusCode::FORBIDDEN),
            ("cross-site", StatusCode::FORBIDDEN),
            ("same-origin", StatusCode::OK),
            ("none", StatusCode::OK),
        ] {
            assert_eq!(
                status("POST", LOGOUT, &[("sec-fetch-site", site)]).await,
                want
            );
        }
    }

    #[tokio::test]
    async fn get_is_unaffected() {
        let got = status("GET", "/health", &[("origin", "https://evil.test")]).await;
        assert_eq!(got, StatusCode::OK);
    }
}
