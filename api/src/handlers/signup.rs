//! Self-signup: public registration with email verification, plus the
//! admin-facing signup settings. Registration is gated by `signup_enabled`
//! (off by default) and an optional email-domain allow-list.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::auth::{consume_reset_token, create_reset_token, AuthUser, EMAIL_VERIFY_TTL_HOURS};
use crate::entities::{auth_settings, user};
use crate::error::AppError;
use crate::{mailer, rbac, service};

const SETTINGS_ID: i32 = 1;

fn parse_domains(s: &Option<String>) -> Vec<String> {
    s.as_deref()
        .unwrap_or("")
        .split(',')
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

fn domain_allowed(domains: &[String], email: &str) -> bool {
    if domains.is_empty() {
        return true;
    }
    match email.rsplit_once('@') {
        Some((_, domain)) => domains.contains(&domain.to_ascii_lowercase()),
        None => false,
    }
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

/// `POST /api/auth/register` — public. Creates an unverified account and emails
/// a verification link. Rejected unless signup is enabled and SMTP configured.
pub async fn register(
    State(db): State<DatabaseConnection>,
    headers: HeaderMap,
    Json(body): Json<RegisterRequest>,
) -> Result<StatusCode, AppError> {
    let settings = service::find_auth_settings(&db).await?;
    let enabled = settings.as_ref().map(|s| s.signup_enabled).unwrap_or(false);
    if !enabled {
        return Err(AppError::BadRequest("self-signup is disabled"));
    }

    let username = body.username.trim();
    let email = body.email.trim();
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required"));
    }
    if email.is_empty() {
        return Err(AppError::BadRequest("email is required"));
    }
    if body.password.len() < 8 {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters",
        ));
    }

    let domains = parse_domains(
        &settings
            .as_ref()
            .and_then(|s| s.signup_allowed_domains.clone()),
    );
    if !domain_allowed(&domains, email) {
        return Err(AppError::BadRequest("your email domain is not allowed"));
    }

    // Verification requires SMTP.
    let (cfg, _) = mailer::resolve(&db).await?.ok_or(AppError::BadRequest(
        "email is not configured; contact an administrator",
    ))?;

    if service::find_by_username(&db, username).await?.is_some() {
        return Err(AppError::Conflict("username already exists"));
    }
    if service::find_by_email(&db, email).await?.is_some() {
        return Err(AppError::Conflict("email already registered"));
    }

    let role_id = match settings.as_ref().and_then(|s| s.signup_default_role_id) {
        Some(id) => service::find_role_by_id(&db, id)
            .await?
            .map(|r| r.id)
            .ok_or(AppError::Internal("configured signup role missing"))?,
        None => service::find_role_by_name(&db, rbac::MEMBER_ROLE)
            .await?
            .map(|r| r.id)
            .ok_or(AppError::Internal("default role missing"))?,
    };

    let user = service::create_signup_user(&db, username, email, &body.password, role_id).await?;
    let token = create_reset_token(&db, user.id, EMAIL_VERIFY_TTL_HOURS).await?;
    let link = format!("{}/verify-email?token={token}", mailer::base_url(&headers));
    if let Err(e) = mailer::send_verification_link(&cfg, email, &link).await {
        tracing::warn!("verification email failed, rolling back signup: {e}");
        user::Entity::delete_by_id(user.id).exec(&db).await?;
        return Err(AppError::Internal("failed to send verification email"));
    }
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
}

/// `POST /api/auth/verify-email` — public. Redeems a verification token and
/// marks the account verified so it can log in.
pub async fn verify_email(
    State(db): State<DatabaseConnection>,
    Json(body): Json<VerifyEmailRequest>,
) -> Result<StatusCode, AppError> {
    let user = consume_reset_token(&db, &body.token).await?;
    let mut active: user::ActiveModel = user.into();
    active.email_verified = Set(true);
    active.update(&db).await?;
    Ok(StatusCode::OK)
}

// --- Settings ------------------------------------------------------------

#[derive(Serialize)]
pub struct SignupSettingsResponse {
    pub signup_enabled: bool,
    pub signup_allowed_domains: String,
    pub signup_default_role_id: Option<i32>,
}

fn response(row: Option<auth_settings::Model>) -> SignupSettingsResponse {
    match row {
        Some(r) => SignupSettingsResponse {
            signup_enabled: r.signup_enabled,
            signup_allowed_domains: r.signup_allowed_domains.unwrap_or_default(),
            signup_default_role_id: r.signup_default_role_id,
        },
        None => SignupSettingsResponse {
            signup_enabled: false,
            signup_allowed_domains: String::new(),
            signup_default_role_id: None,
        },
    }
}

pub async fn get_settings(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
) -> Result<Json<SignupSettingsResponse>, AppError> {
    caller.require("settings.read")?;
    Ok(Json(response(service::find_auth_settings(&db).await?)))
}

#[derive(Deserialize)]
pub struct UpdateSignupRequest {
    pub signup_enabled: bool,
    pub signup_allowed_domains: String,
    pub signup_default_role_id: Option<i32>,
}

pub async fn update_settings(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Json(body): Json<UpdateSignupRequest>,
) -> Result<Json<SignupSettingsResponse>, AppError> {
    caller.require("settings.write")?;
    if let Some(id) = body.signup_default_role_id {
        service::find_role_by_id(&db, id)
            .await?
            .ok_or(AppError::BadRequest("unknown role"))?;
    }
    let existing = service::find_auth_settings(&db).await?;
    let model = auth_settings::ActiveModel {
        id: Set(SETTINGS_ID),
        signup_enabled: Set(body.signup_enabled),
        signup_allowed_domains: Set(
            Some(body.signup_allowed_domains.trim().to_string()).filter(|s| !s.is_empty())
        ),
        signup_default_role_id: Set(body.signup_default_role_id),
        updated_at: Set(chrono::Utc::now()),
    };
    if existing.is_some() {
        model.update(&db).await?;
    } else {
        model.insert(&db).await?;
    }
    Ok(Json(response(service::find_auth_settings(&db).await?)))
}
