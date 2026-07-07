//! User operations shared by the HTTP handlers and the CLI.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};

use crate::auth::hash_password;
use crate::entities::user;
use crate::error::AppError;

pub async fn find_by_username(
    db: &DatabaseConnection,
    username: &str,
) -> Result<Option<user::Model>, AppError> {
    Ok(user::Entity::find()
        .filter(user::Column::Username.eq(username))
        .one(db)
        .await?)
}

pub async fn find_by_email(
    db: &DatabaseConnection,
    email: &str,
) -> Result<Option<user::Model>, AppError> {
    Ok(user::Entity::find()
        .filter(user::Column::Email.eq(email))
        .one(db)
        .await?)
}

pub async fn list(db: &DatabaseConnection) -> Result<Vec<user::Model>, AppError> {
    Ok(user::Entity::find()
        .order_by_asc(user::Column::Id)
        .all(db)
        .await?)
}

pub async fn create(
    db: &DatabaseConnection,
    username: &str,
    email: Option<String>,
    password: &str,
    must_change_password: bool,
) -> Result<user::Model, AppError> {
    if username.trim().is_empty() {
        return Err(AppError::BadRequest("username is required"));
    }
    if password.is_empty() {
        return Err(AppError::BadRequest("password is required"));
    }
    if find_by_username(db, username).await?.is_some() {
        return Err(AppError::Conflict("username already exists"));
    }
    let model = user::ActiveModel {
        username: Set(username.to_string()),
        email: Set(email),
        password_hash: Set(hash_password(password)?),
        must_change_password: Set(must_change_password),
        created_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(model)
}

pub async fn set_password(
    db: &DatabaseConnection,
    user: user::Model,
    new_password: &str,
    must_change_password: bool,
) -> Result<user::Model, AppError> {
    if new_password.is_empty() {
        return Err(AppError::BadRequest("password is required"));
    }
    let mut active: user::ActiveModel = user.into();
    active.password_hash = Set(hash_password(new_password)?);
    active.must_change_password = Set(must_change_password);
    Ok(active.update(db).await?)
}

pub async fn delete_by_username(db: &DatabaseConnection, username: &str) -> Result<(), AppError> {
    let user = find_by_username(db, username)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;
    user::Entity::delete_by_id(user.id).exec(db).await?;
    Ok(())
}
