use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    Forbidden(&'static str),
    NotFound(&'static str),
    Conflict(&'static str),
    BadRequest(&'static str),
    Internal(&'static str),
    /// A workspace could not be reached or materialized; the message carries
    /// operator guidance (image missing, runtime down...).
    WorkspaceUnavailable(String),
    /// The workspace's artifact is being fetched/built in the background;
    /// clients should retry shortly.
    WorkspaceProvisioning(String),
    Db(sea_orm::DbErr),
}

impl From<sea_orm::DbErr> for AppError {
    fn from(e: sea_orm::DbErr) -> Self {
        AppError::Db(e)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Unauthorized => write!(f, "unauthorized"),
            AppError::Forbidden(m) | AppError::NotFound(m) | AppError::Conflict(m) => {
                write!(f, "{m}")
            }
            AppError::BadRequest(m) | AppError::Internal(m) => write!(f, "{m}"),
            AppError::WorkspaceUnavailable(m) | AppError::WorkspaceProvisioning(m) => {
                write!(f, "{m}")
            }
            AppError::Db(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m.to_string()),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m.to_string()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m.to_string()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.to_string()),
            AppError::Internal(m) => {
                tracing::error!("internal error: {m}");
                (StatusCode::INTERNAL_SERVER_ERROR, m.to_string())
            }
            AppError::WorkspaceUnavailable(m) => {
                tracing::warn!("workspace unavailable: {m}");
                (StatusCode::SERVICE_UNAVAILABLE, m)
            }
            AppError::WorkspaceProvisioning(m) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    // Providing a retry hint: provisioning resolves on its own.
                    [(axum::http::header::RETRY_AFTER, "5")],
                    Json(json!({ "error": m, "provisioning": true })),
                )
                    .into_response();
            }
            AppError::Db(e) => {
                tracing::error!("database error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
