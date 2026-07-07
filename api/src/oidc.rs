//! OIDC (OpenID Connect) configuration resolution.
//!
//! Like SMTP, the effective config is resolved at call time: `OIDC_*`
//! environment variables win as a block (if `OIDC_ISSUER` is set, everything
//! comes from the env and the settings page is read-only); otherwise the
//! single-row `oidc_settings` table is used when it exists and is enabled.
//!
//! The generic OIDC flow (authorization code + PKCE) means any compliant
//! provider works from one config: Google, Microsoft/Entra, Okta, Auth0,
//! Keycloak, GitLab, Authentik, and so on.

use sea_orm::{DatabaseConnection, EntityTrait};

use crate::crypto;
use crate::entities::oidc_settings;
use crate::error::AppError;
use crate::mailer::Source;

/// The single-row settings table always uses this primary key.
pub const SETTINGS_ID: i32 = 1;

/// Path the IdP redirects back to; combined with the base URL to form the full
/// redirect URI registered with the provider.
pub const CALLBACK_PATH: &str = "/api/auth/oidc/callback";

fn default_scopes() -> Vec<String> {
    vec!["openid".into(), "email".into(), "profile".into()]
}

#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    /// Email domains allowed to auto-provision. Empty means any domain.
    pub allowed_domains: Vec<String>,
}

fn split_scopes(s: &str) -> Vec<String> {
    let v: Vec<String> = s.split_whitespace().map(|s| s.to_string()).collect();
    if v.is_empty() {
        default_scopes()
    } else {
        v
    }
}

fn split_domains(s: &str) -> Vec<String> {
    s.split(',')
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

/// Read OIDC config from the environment. `OIDC_ISSUER` (plus `OIDC_CLIENT_ID`)
/// being set is the signal that the whole configuration is env-managed.
pub fn from_env() -> Option<OidcConfig> {
    let issuer = std::env::var("OIDC_ISSUER")
        .ok()
        .filter(|s| !s.is_empty())?;
    let client_id = std::env::var("OIDC_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())?;
    Some(OidcConfig {
        issuer,
        client_id,
        client_secret: std::env::var("OIDC_CLIENT_SECRET")
            .ok()
            .filter(|s| !s.is_empty()),
        scopes: std::env::var("OIDC_SCOPES")
            .ok()
            .map(|s| split_scopes(&s))
            .unwrap_or_else(default_scopes),
        allowed_domains: std::env::var("OIDC_ALLOWED_DOMAINS")
            .ok()
            .map(|s| split_domains(&s))
            .unwrap_or_default(),
    })
}

/// True when the environment manages OIDC (settings page is then read-only).
pub fn env_managed() -> bool {
    from_env().is_some()
}

pub async fn load_row(db: &DatabaseConnection) -> Result<Option<oidc_settings::Model>, AppError> {
    Ok(oidc_settings::Entity::find_by_id(SETTINGS_ID)
        .one(db)
        .await?)
}

/// Resolve the effective configuration and its source. Returns `None` when OIDC
/// is neither configured via the environment nor enabled in the database.
pub async fn resolve(db: &DatabaseConnection) -> Result<Option<(OidcConfig, Source)>, AppError> {
    if let Some(cfg) = from_env() {
        return Ok(Some((cfg, Source::Env)));
    }
    let Some(row) = load_row(db).await? else {
        return Ok(None);
    };
    if !row.enabled {
        return Ok(None);
    }
    let client_secret = match &row.client_secret_encrypted {
        Some(enc) => Some(crypto::decrypt(enc)?),
        None => None,
    };
    let cfg = OidcConfig {
        issuer: row.issuer,
        client_id: row.client_id,
        client_secret,
        scopes: split_scopes(&row.scopes),
        allowed_domains: row
            .allowed_domains
            .as_deref()
            .map(split_domains)
            .unwrap_or_default(),
    };
    Ok(Some((cfg, Source::Database)))
}

/// Whether `email` may auto-provision under this config. An empty allow-list
/// permits any domain.
pub fn domain_allowed(cfg: &OidcConfig, email: &str) -> bool {
    if cfg.allowed_domains.is_empty() {
        return true;
    }
    match email.rsplit_once('@') {
        Some((_, domain)) => cfg.allowed_domains.contains(&domain.to_ascii_lowercase()),
        None => false,
    }
}

/// An HTTP client for OIDC discovery and token exchange. Redirects are disabled
/// per the openidconnect crate's guidance (prevents SSRF via redirect).
pub fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppError::Internal("failed to build HTTP client"))
}
