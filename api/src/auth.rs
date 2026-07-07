use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;
use chrono::{Duration, Utc};
use rand::distr::Alphanumeric;
use rand::RngExt;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use crate::entities::{session, user};
use crate::error::AppError;

pub const SESSION_COOKIE: &str = "cityhall_session";
const SESSION_TTL_DAYS: i64 = 30;

/// Generate a random ASCII-alphanumeric string of `len` chars.
pub fn random_token(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AppError::Internal("failed to hash password"))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Create a session row for `user_id` and return its token.
pub async fn create_session(db: &DatabaseConnection, user_id: i32) -> Result<String, AppError> {
    let token = random_token(48);
    let now = Utc::now();
    session::ActiveModel {
        id: Set(token.clone()),
        user_id: Set(user_id),
        expires_at: Set(now + Duration::days(SESSION_TTL_DAYS)),
        created_at: Set(now),
    }
    .insert(db)
    .await?;
    Ok(token)
}

pub async fn delete_session(db: &DatabaseConnection, token: &str) -> Result<(), AppError> {
    session::Entity::delete_by_id(token.to_string())
        .exec(db)
        .await?;
    Ok(())
}

/// The authenticated user, resolved from the session cookie. Rejects with 401
/// when the cookie is missing, unknown, or expired (expired rows are pruned).
pub struct AuthUser(pub user::Model);

impl FromRequestParts<DatabaseConnection> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        db: &DatabaseConnection,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .map(|c| c.value().to_string())
            .ok_or(AppError::Unauthorized)?;

        let session = session::Entity::find_by_id(token.clone())
            .one(db)
            .await?
            .ok_or(AppError::Unauthorized)?;

        if session.expires_at < Utc::now() {
            let _ = delete_session(db, &token).await;
            return Err(AppError::Unauthorized);
        }

        let user = user::Entity::find_by_id(session.user_id)
            .one(db)
            .await?
            .ok_or(AppError::Unauthorized)?;

        Ok(AuthUser(user))
    }
}
