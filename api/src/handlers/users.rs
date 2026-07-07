use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::auth::{
    create_reset_token, hash_password, random_token, AuthUser, SETUP_TOKEN_TTL_HOURS,
};
use crate::entities::user;
use crate::error::AppError;
use crate::rbac;
use crate::{mailer, service};

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,
    /// Omitted or empty generates a random password (returned once), unless
    /// `send_setup_email` is set.
    #[serde(default)]
    pub password: Option<String>,
    /// When true, email the user a setup link instead of setting a password.
    #[serde(default)]
    pub send_setup_email: bool,
    /// Role to assign; defaults to the `member` role when omitted.
    #[serde(default)]
    pub role_id: Option<i32>,
}

/// The created user, plus the generated password when one was generated (so the
/// admin can hand it over). Never set when a setup email was sent.
#[derive(Serialize)]
pub struct CreateUserResponse {
    #[serde(flatten)]
    pub user: user::Model,
    pub generated_password: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub role_id: Option<i32>,
}

/// Resolve the role id to assign at creation: the requested role if valid, else
/// the default `member` role.
async fn resolve_create_role(
    db: &DatabaseConnection,
    role_id: Option<i32>,
) -> Result<i32, AppError> {
    match role_id {
        Some(id) => service::find_role_by_id(db, id)
            .await?
            .map(|r| r.id)
            .ok_or(AppError::BadRequest("unknown role")),
        None => service::find_role_by_name(db, rbac::MEMBER_ROLE)
            .await?
            .map(|r| r.id)
            .ok_or(AppError::Internal("default role missing")),
    }
}

pub async fn list(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
) -> Result<Json<Vec<user::Model>>, AppError> {
    caller.require("users.read")?;
    Ok(Json(service::list(&db).await?))
}

pub async fn get(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<user::Model>, AppError> {
    caller.require("users.read")?;
    let user = user::Entity::find_by_id(id)
        .one(&db)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;
    Ok(Json(user))
}

pub async fn create(
    State(db): State<DatabaseConnection>,
    headers: HeaderMap,
    caller: AuthUser,
    Json(body): Json<CreateUserRequest>,
) -> Result<Json<CreateUserResponse>, AppError> {
    caller.require("users.write")?;
    let email = body.email.filter(|e| !e.trim().is_empty());
    let role_id = resolve_create_role(&db, body.role_id).await?;

    if body.send_setup_email {
        let address = email.clone().ok_or(AppError::BadRequest(
            "an email address is required to send a setup email",
        ))?;
        let (cfg, _) = mailer::resolve(&db).await?.ok_or(AppError::BadRequest(
            "SMTP is not configured; cannot send a setup email",
        ))?;

        // Create with an unknown random password (and must-change), then email a
        // setup link. If sending fails, roll the user back so a retry is clean.
        let user = service::create(
            &db,
            &body.username,
            email,
            &random_token(24),
            true,
            Some(role_id),
        )
        .await?;
        let token = create_reset_token(&db, user.id, SETUP_TOKEN_TTL_HOURS).await?;
        let link = format!(
            "{}/reset-password?token={token}",
            mailer::base_url(&headers)
        );
        if let Err(e) = mailer::send_reset_link(&cfg, &address, &link, true).await {
            tracing::warn!("setup email failed, rolling back user: {e}");
            user::Entity::delete_by_id(user.id).exec(&db).await?;
            return Err(AppError::Internal("failed to send setup email"));
        }
        return Ok(Json(CreateUserResponse {
            user,
            generated_password: None,
        }));
    }

    match body.password.filter(|p| !p.is_empty()) {
        Some(password) => {
            let user = service::create(&db, &body.username, email, &password, false, Some(role_id))
                .await?;
            Ok(Json(CreateUserResponse {
                user,
                generated_password: None,
            }))
        }
        None => {
            let generated = random_token(16);
            let user = service::create(&db, &body.username, email, &generated, true, Some(role_id))
                .await?;
            Ok(Json(CreateUserResponse {
                user,
                generated_password: Some(generated),
            }))
        }
    }
}

pub async fn update(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Path(id): Path<i32>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<user::Model>, AppError> {
    caller.require("users.write")?;
    let existing = user::Entity::find_by_id(id)
        .one(&db)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;

    let mut active: user::ActiveModel = existing.into();

    if let Some(username) = body.username {
        if username.trim().is_empty() {
            return Err(AppError::BadRequest("username cannot be empty"));
        }
        // Reject a rename that collides with a different user.
        if let Some(other) = user::Entity::find()
            .filter(user::Column::Username.eq(&username))
            .one(&db)
            .await?
        {
            if other.id != id {
                return Err(AppError::Conflict("username already exists"));
            }
        }
        active.username = Set(username);
    }
    if let Some(email) = body.email {
        active.email = Set(Some(email));
    }
    if let Some(password) = body.password {
        if password.is_empty() {
            return Err(AppError::BadRequest("password cannot be empty"));
        }
        active.password_hash = Set(hash_password(&password)?);
    }
    if let Some(role_id) = body.role_id {
        // Validate the role exists before assigning it.
        service::find_role_by_id(&db, role_id)
            .await?
            .ok_or(AppError::BadRequest("unknown role"))?;
        active.role_id = Set(Some(role_id));
    }

    Ok(Json(active.update(&db).await?))
}

pub async fn delete(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("users.write")?;
    if caller.user.id == id {
        return Err(AppError::BadRequest("cannot delete your own account"));
    }
    let res = user::Entity::delete_by_id(id).exec(&db).await?;
    if res.rows_affected == 0 {
        return Err(AppError::NotFound("user not found"));
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}
