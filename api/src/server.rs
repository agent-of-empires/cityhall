use std::path::PathBuf;

use axum::routing::{get, patch, post};
use axum::Router;
use sea_orm::DatabaseConnection;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::handlers::{auth, oidc, roles, settings, users};

/// Directory holding the built frontend. Overridable via `STATIC_DIR` for
/// container images that place the bundle elsewhere.
fn static_dir() -> PathBuf {
    std::env::var("STATIC_DIR")
        .unwrap_or_else(|_| "web/dist".to_string())
        .into()
}

pub fn api_router(db: DatabaseConnection) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/change-password", post(auth::change_password))
        .route("/auth/forgot-password", post(auth::forgot_password))
        .route("/auth/reset-password", post(auth::reset_password))
        .route("/auth/providers", get(oidc::providers))
        .route("/auth/oidc/login", get(oidc::login))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/users", get(users::list).post(users::create))
        .route(
            "/users/{id}",
            get(users::get).patch(users::update).delete(users::delete),
        )
        .route("/roles", get(roles::list).post(roles::create))
        .route("/roles/{id}", patch(roles::update).delete(roles::delete))
        .route("/permissions", get(roles::permissions))
        .route("/settings/smtp", get(settings::get).put(settings::update))
        .route("/settings/smtp/test", post(settings::test))
        .route(
            "/settings/oidc",
            get(oidc::get_settings).put(oidc::update_settings),
        )
        // Unknown /api/* paths return a JSON 404 instead of falling through to
        // the SPA index (which the outer fallback_service would otherwise serve).
        .fallback(|| async { AppError::NotFound("not found") })
        .with_state(db)
}

pub fn router(db: DatabaseConnection) -> Router {
    let dir = static_dir();
    // Serve the SPA: static assets first, fall back to index.html so client
    // routes (e.g. /login) resolve on hard refresh.
    let index = dir.join("index.html");
    // `.fallback` (not `.not_found_service`, which forces a 404 status) so
    // client routes like /login resolve with 200 on hard refresh.
    let spa = ServeDir::new(&dir).fallback(ServeFile::new(index));

    Router::new()
        .nest("/api", api_router(db))
        .fallback_service(spa)
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(db: DatabaseConnection) -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("cityhall listening on http://{addr}");
    axum::serve(listener, router(db)).await?;
    Ok(())
}
