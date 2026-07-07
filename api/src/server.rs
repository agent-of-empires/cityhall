use std::path::PathBuf;

use axum::routing::{get, post};
use axum::Router;
use sea_orm::DatabaseConnection;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::handlers::{auth, users};

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
        .route("/users", get(users::list).post(users::create))
        .route(
            "/users/{id}",
            get(users::get).patch(users::update).delete(users::delete),
        )
        .with_state(db)
}

pub fn router(db: DatabaseConnection) -> Router {
    let dir = static_dir();
    // Serve the SPA: static assets first, fall back to index.html so client
    // routes (e.g. /login) resolve on hard refresh.
    let index = dir.join("index.html");
    let spa = ServeDir::new(&dir).not_found_service(ServeFile::new(index));

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
