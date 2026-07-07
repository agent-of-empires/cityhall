use axum::extract::{Path, State};
use axum::Json;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::Deserialize;

use crate::auth::{hash_password, require_active, AuthUser};
use crate::entities::user;
use crate::error::AppError;
use crate::service;

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: Option<String>,
    pub password: String,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
}

pub async fn list(
    State(db): State<DatabaseConnection>,
    AuthUser(caller): AuthUser,
) -> Result<Json<Vec<user::Model>>, AppError> {
    require_active(&caller)?;
    Ok(Json(service::list(&db).await?))
}

pub async fn get(
    State(db): State<DatabaseConnection>,
    AuthUser(caller): AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<user::Model>, AppError> {
    require_active(&caller)?;
    let user = user::Entity::find_by_id(id)
        .one(&db)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;
    Ok(Json(user))
}

pub async fn create(
    State(db): State<DatabaseConnection>,
    AuthUser(caller): AuthUser,
    Json(body): Json<CreateUserRequest>,
) -> Result<Json<user::Model>, AppError> {
    require_active(&caller)?;
    let user = service::create(&db, &body.username, body.email, &body.password, false).await?;
    Ok(Json(user))
}

pub async fn update(
    State(db): State<DatabaseConnection>,
    AuthUser(caller): AuthUser,
    Path(id): Path<i32>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<user::Model>, AppError> {
    require_active(&caller)?;
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

    Ok(Json(active.update(&db).await?))
}

pub async fn delete(
    State(db): State<DatabaseConnection>,
    AuthUser(caller): AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_active(&caller)?;
    if caller.id == id {
        return Err(AppError::BadRequest("cannot delete your own account"));
    }
    let res = user::Entity::delete_by_id(id).exec(&db).await?;
    if res.rows_affected == 0 {
        return Err(AppError::NotFound("user not found"));
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}
