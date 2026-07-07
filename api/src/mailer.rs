//! SMTP configuration and sending.
//!
//! The effective configuration is resolved at call time: environment variables
//! win as a block (if `SMTP_HOST` is set, everything comes from the env and the
//! settings page is read-only); otherwise the single-row `smtp_settings` table
//! is used when it exists and is enabled.

use axum::http::{header, HeaderMap};
use lettre::message::{Mailbox, Message};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{AsyncTransport, Tokio1Executor};
use sea_orm::{DatabaseConnection, EntityTrait};

use crate::crypto;
use crate::entities::smtp_settings;
use crate::error::AppError;

/// The single-row settings table always uses this primary key.
pub const SETTINGS_ID: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encryption {
    /// No transport security (typically port 25; dev only).
    None,
    /// Upgrade a plaintext connection with STARTTLS (typically port 587).
    StartTls,
    /// Implicit TLS from the first byte (typically port 465).
    Tls,
}

impl Encryption {
    pub fn as_str(self) -> &'static str {
        match self {
            Encryption::None => "none",
            Encryption::StartTls => "starttls",
            Encryption::Tls => "tls",
        }
    }

    pub fn parse(s: &str) -> Result<Self, AppError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Encryption::None),
            "starttls" => Ok(Encryption::StartTls),
            "tls" | "ssl" => Ok(Encryption::Tls),
            _ => Err(AppError::BadRequest(
                "encryption must be one of: none, starttls, tls",
            )),
        }
    }

    /// Default port for this mode when none is given.
    pub fn default_port(self) -> u16 {
        match self {
            Encryption::None => 25,
            Encryption::StartTls => 587,
            Encryption::Tls => 465,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub encryption: Encryption,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
}

/// Where the effective config came from, surfaced to the settings UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Env,
    Database,
}

/// Read SMTP config from the environment. `SMTP_HOST` being set is the signal
/// that the whole configuration is env-managed.
pub fn from_env() -> Option<SmtpConfig> {
    let host = std::env::var("SMTP_HOST").ok().filter(|h| !h.is_empty())?;
    let encryption = std::env::var("SMTP_ENCRYPTION")
        .ok()
        .and_then(|s| Encryption::parse(&s).ok())
        .unwrap_or(Encryption::StartTls);
    let port = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| encryption.default_port());
    let username = std::env::var("SMTP_USERNAME")
        .ok()
        .filter(|s| !s.is_empty());
    let from_address = std::env::var("SMTP_FROM_ADDRESS")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| username.clone())
        .unwrap_or_default();
    Some(SmtpConfig {
        host,
        port,
        encryption,
        username,
        password: std::env::var("SMTP_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty()),
        from_address,
        from_name: std::env::var("SMTP_FROM_NAME")
            .ok()
            .filter(|s| !s.is_empty()),
    })
}

/// Fetch the settings row, if any.
pub async fn load_row(db: &DatabaseConnection) -> Result<Option<smtp_settings::Model>, AppError> {
    Ok(smtp_settings::Entity::find_by_id(SETTINGS_ID)
        .one(db)
        .await?)
}

/// Resolve the effective configuration and its source. Returns `None` when SMTP
/// is neither configured via the environment nor enabled in the database.
pub async fn resolve(db: &DatabaseConnection) -> Result<Option<(SmtpConfig, Source)>, AppError> {
    if let Some(cfg) = from_env() {
        return Ok(Some((cfg, Source::Env)));
    }
    let Some(row) = load_row(db).await? else {
        return Ok(None);
    };
    if !row.enabled {
        return Ok(None);
    }
    let password = match &row.password_encrypted {
        Some(enc) => Some(crypto::decrypt(enc)?),
        None => None,
    };
    let cfg = SmtpConfig {
        host: row.host,
        port: row.port as u16,
        encryption: Encryption::parse(&row.encryption)?,
        username: row.username,
        password,
        from_address: row.from_address,
        from_name: row.from_name,
    };
    Ok(Some((cfg, Source::Database)))
}

fn transport(cfg: &SmtpConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let mut builder = match cfg.encryption {
        Encryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host),
        Encryption::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host),
        Encryption::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            &cfg.host,
        )),
    }
    .map_err(|e| format!("TLS setup failed: {e}"))?
    .port(cfg.port);

    if let (Some(user), Some(pass)) = (&cfg.username, &cfg.password) {
        builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
    }
    Ok(builder.build())
}

fn from_mailbox(cfg: &SmtpConfig) -> Result<Mailbox, String> {
    if cfg.from_address.is_empty() {
        return Err("no from address configured".to_string());
    }
    let spec = match &cfg.from_name {
        Some(name) => format!("{name} <{}>", cfg.from_address),
        None => cfg.from_address.clone(),
    };
    spec.parse()
        .map_err(|e| format!("invalid from address: {e}"))
}

/// Send an email using the given config. The `Err` is a human-readable message
/// suitable for surfacing in the settings "test send" result; callers that must
/// not leak details should log it and return a generic error instead.
pub async fn send(cfg: &SmtpConfig, to: &str, subject: &str, body: String) -> Result<(), String> {
    let to: Mailbox = to
        .parse()
        .map_err(|e| format!("invalid recipient address: {e}"))?;
    let email = Message::builder()
        .from(from_mailbox(cfg)?)
        .to(to)
        .subject(subject)
        .body(body)
        .map_err(|e| format!("failed to build message: {e}"))?;

    transport(cfg)?
        .send(email)
        .await
        .map(|_| ())
        .map_err(|e| format!("send failed: {e}"))
}

/// The externally reachable base URL, used to build links in emails.
/// `CITYHALL_BASE_URL` wins; otherwise it is derived from the request (honoring
/// `X-Forwarded-Proto` behind a proxy).
pub fn base_url(headers: &HeaderMap) -> String {
    if let Ok(url) = std::env::var("CITYHALL_BASE_URL") {
        let url = url.trim_end_matches('/');
        if !url.is_empty() {
            return url.to_string();
        }
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:3000");
    format!("{scheme}://{host}")
}

/// Send a password-reset or account-setup email containing `link`.
pub async fn send_reset_link(
    cfg: &SmtpConfig,
    to: &str,
    link: &str,
    setup: bool,
) -> Result<(), String> {
    let (subject, intro) = if setup {
        (
            "Set up your CityHall account",
            "An account has been created for you on CityHall. Use the link below \
             to set your password:",
        )
    } else {
        (
            "Reset your CityHall password",
            "We received a request to reset your CityHall password. Use the link \
             below to choose a new one:",
        )
    };
    let body = format!("{intro}\n\n{link}\n\nIf you did not expect this email, you can ignore it.");
    send(cfg, to, subject, body).await
}
