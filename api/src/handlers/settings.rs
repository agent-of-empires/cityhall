use axum::extract::State;
use axum::Json;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::crypto;
use crate::entities::smtp_settings;
use crate::error::AppError;
use crate::mailer::{self, Encryption, SmtpConfig, Source, SETTINGS_ID};

/// SMTP settings as shown in the UI. Never includes the password itself; only
/// whether one is set.
#[derive(Serialize)]
pub struct SmtpSettingsResponse {
    /// True when configured via environment variables; the form is read-only.
    pub env_managed: bool,
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub encryption: String,
    pub username: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
    pub password_set: bool,
    /// Whether CITYHALL_SECRET_KEY is configured (required to store a password).
    pub secret_key_available: bool,
}

#[derive(Deserialize)]
pub struct UpdateSmtpRequest {
    pub host: String,
    pub port: u16,
    pub encryption: String,
    pub username: Option<String>,
    /// Omitted (or empty) keeps the stored password; a value replaces it.
    pub password: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct TestSmtpRequest {
    pub to: String,
}

#[derive(Serialize)]
pub struct TestSmtpResponse {
    pub ok: bool,
    pub error: Option<String>,
}

fn response_from_env(cfg: &SmtpConfig) -> SmtpSettingsResponse {
    SmtpSettingsResponse {
        env_managed: true,
        enabled: true,
        host: cfg.host.clone(),
        port: cfg.port,
        encryption: cfg.encryption.as_str().to_string(),
        username: cfg.username.clone(),
        from_address: cfg.from_address.clone(),
        from_name: cfg.from_name.clone(),
        password_set: cfg.password.is_some(),
        secret_key_available: crypto::key_available(),
    }
}

fn response_from_row(row: Option<smtp_settings::Model>) -> SmtpSettingsResponse {
    let secret_key_available = crypto::key_available();
    match row {
        Some(r) => SmtpSettingsResponse {
            env_managed: false,
            enabled: r.enabled,
            host: r.host,
            port: r.port as u16,
            encryption: r.encryption,
            username: r.username,
            from_address: r.from_address,
            from_name: r.from_name,
            password_set: r.password_encrypted.is_some(),
            secret_key_available,
        },
        // No row yet: sensible defaults for a fresh form.
        None => SmtpSettingsResponse {
            env_managed: false,
            enabled: false,
            host: String::new(),
            port: Encryption::StartTls.default_port(),
            encryption: Encryption::StartTls.as_str().to_string(),
            username: None,
            from_address: String::new(),
            from_name: None,
            password_set: false,
            secret_key_available,
        },
    }
}

pub async fn get(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
) -> Result<Json<SmtpSettingsResponse>, AppError> {
    caller.require("settings.read")?;
    if let Some(cfg) = mailer::from_env() {
        return Ok(Json(response_from_env(&cfg)));
    }
    Ok(Json(response_from_row(mailer::load_row(&db).await?)))
}

pub async fn update(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Json(body): Json<UpdateSmtpRequest>,
) -> Result<Json<SmtpSettingsResponse>, AppError> {
    caller.require("settings.write")?;
    if mailer::from_env().is_some() {
        return Err(AppError::Conflict(
            "SMTP is managed via environment variables",
        ));
    }

    if body.host.trim().is_empty() {
        return Err(AppError::BadRequest("host is required"));
    }
    if body.from_address.trim().is_empty() {
        return Err(AppError::BadRequest("from address is required"));
    }
    if body.port == 0 {
        return Err(AppError::BadRequest("port must be between 1 and 65535"));
    }
    let encryption = Encryption::parse(&body.encryption)?;
    let username = body.username.filter(|s| !s.is_empty());
    let from_name = body.from_name.filter(|s| !s.is_empty());

    let existing = mailer::load_row(&db).await?;

    // A provided password is encrypted and stored; otherwise keep the existing
    // ciphertext.
    let password_encrypted = match body.password.filter(|p| !p.is_empty()) {
        Some(pw) => Some(crypto::encrypt(&pw)?),
        None => existing.as_ref().and_then(|r| r.password_encrypted.clone()),
    };

    let now = chrono::Utc::now();
    let model = smtp_settings::ActiveModel {
        id: Set(SETTINGS_ID),
        host: Set(body.host.trim().to_string()),
        port: Set(body.port as i32),
        encryption: Set(encryption.as_str().to_string()),
        username: Set(username),
        password_encrypted: Set(password_encrypted),
        from_address: Set(body.from_address.trim().to_string()),
        from_name: Set(from_name),
        enabled: Set(body.enabled),
        updated_at: Set(now),
    };

    if existing.is_some() {
        model.update(&db).await?;
    } else {
        model.insert(&db).await?;
    }

    Ok(Json(response_from_row(mailer::load_row(&db).await?)))
}

pub async fn test(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Json(body): Json<TestSmtpRequest>,
) -> Result<Json<TestSmtpResponse>, AppError> {
    caller.require("settings.write")?;
    if body.to.trim().is_empty() {
        return Err(AppError::BadRequest("recipient address is required"));
    }
    let (cfg, source) = mailer::resolve(&db).await?.ok_or(AppError::BadRequest(
        "SMTP is not configured or not enabled",
    ))?;

    let where_from = match source {
        Source::Env => "environment variables",
        Source::Database => "the settings page",
    };
    let body_text = format!(
        "This is a test email from CityHall, sent to verify your SMTP \
         configuration (loaded from {where_from}).\n\nIf you received this, \
         email delivery is working."
    );

    match mailer::send(&cfg, body.to.trim(), "CityHall SMTP test", body_text).await {
        Ok(()) => Ok(Json(TestSmtpResponse {
            ok: true,
            error: None,
        })),
        Err(e) => {
            tracing::warn!("SMTP test send failed: {e}");
            Ok(Json(TestSmtpResponse {
                ok: false,
                error: Some(e),
            }))
        }
    }
}
