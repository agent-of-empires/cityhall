use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use rand::distr::Alphanumeric;
use rand::RngExt;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use crate::entities::{password_reset_token, role, session, user};
use crate::error::AppError;
use crate::rbac::Perms;

pub const SESSION_COOKIE: &str = "cityhall_session";
const SESSION_TTL_DAYS: i64 = 30;

/// Build the HttpOnly session cookie carrying `token`.
pub fn session_cookie(token: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, token);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie
}

/// How long a self-service password-reset token is valid, in hours.
pub const RESET_TOKEN_TTL_HOURS: i64 = 1;
/// How long an admin-issued account-setup token is valid, in hours.
pub const SETUP_TOKEN_TTL_HOURS: i64 = 72;
/// How long a self-signup email-verification token is valid, in hours.
pub const EMAIL_VERIFY_TTL_HOURS: i64 = 24;

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

/// Create a single-use password-reset/setup token for `user_id`, valid for
/// `ttl_hours`, and return it.
pub async fn create_reset_token(
    db: &DatabaseConnection,
    user_id: i32,
    ttl_hours: i64,
) -> Result<String, AppError> {
    let token = random_token(48);
    let now = Utc::now();
    password_reset_token::ActiveModel {
        token: Set(token.clone()),
        user_id: Set(user_id),
        expires_at: Set(now + Duration::hours(ttl_hours)),
        created_at: Set(now),
        used_at: Set(None),
    }
    .insert(db)
    .await?;
    Ok(token)
}

/// Validate a reset token and mark it used, returning the owning user. Rejects
/// tokens that are unknown, already used, or expired.
pub async fn consume_reset_token(
    db: &DatabaseConnection,
    token: &str,
) -> Result<user::Model, AppError> {
    let row = password_reset_token::Entity::find_by_id(token.to_string())
        .one(db)
        .await?
        .ok_or(AppError::BadRequest("invalid or expired reset token"))?;

    if row.used_at.is_some() || row.expires_at < Utc::now() {
        return Err(AppError::BadRequest("invalid or expired reset token"));
    }

    let user = user::Entity::find_by_id(row.user_id)
        .one(db)
        .await?
        .ok_or(AppError::BadRequest("invalid or expired reset token"))?;

    let mut active: password_reset_token::ActiveModel = row.into();
    active.used_at = Set(Some(Utc::now()));
    active.update(db).await?;

    Ok(user)
}

/// Callers must have completed a forced password change before touching any
/// authenticated route. RBAC (admin-only) will layer on top of this later.
pub fn require_active(user: &user::Model) -> Result<(), AppError> {
    if user.must_change_password {
        return Err(AppError::Forbidden("password change required"));
    }
    Ok(())
}

/// The authenticated user with the permission set resolved from their role.
/// Rejects with 401 when the session cookie is missing, unknown, or expired
/// (expired rows are pruned).
pub struct AuthUser {
    pub user: user::Model,
    pub perms: Perms,
}

impl AuthUser {
    /// Whether the caller holds `permission`.
    pub fn can(&self, permission: &str) -> bool {
        self.perms.can(permission)
    }

    /// Require `permission`, returning `403` when the caller lacks it. Also
    /// enforces the forced-password-change gate: a user who must change their
    /// password cannot exercise any permission.
    pub fn require(&self, permission: &str) -> Result<(), AppError> {
        require_active(&self.user)?;
        if self.can(permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden("insufficient permissions"))
        }
    }
}

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

        let role = match user.role_id {
            Some(id) => role::Entity::find_by_id(id).one(db).await?,
            None => None,
        };

        Ok(AuthUser {
            user,
            perms: Perms::from_role(role.as_ref()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::Migrator;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    async fn setup() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn make_user(db: &DatabaseConnection) -> i32 {
        crate::service::create(db, "u", None, "password123", false, None)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn reset_token_valid_then_single_use() {
        let db = setup().await;
        let uid = make_user(&db).await;
        let token = create_reset_token(&db, uid, RESET_TOKEN_TTL_HOURS)
            .await
            .unwrap();
        assert_eq!(consume_reset_token(&db, &token).await.unwrap().id, uid);
        // A second redemption is rejected.
        assert!(consume_reset_token(&db, &token).await.is_err());
    }

    #[tokio::test]
    async fn expired_token_rejected() {
        let db = setup().await;
        let uid = make_user(&db).await;
        let token = create_reset_token(&db, uid, -1).await.unwrap();
        assert!(consume_reset_token(&db, &token).await.is_err());
    }

    #[tokio::test]
    async fn unknown_token_rejected() {
        let db = setup().await;
        assert!(consume_reset_token(&db, "does-not-exist").await.is_err());
    }
}
