use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::auth::{
    consume_reset_token, create_reset_token, create_session, delete_session, verify_password,
    AuthUser, RESET_TOKEN_TTL_HOURS, SESSION_COOKIE,
};
use crate::error::AppError;
use crate::{mailer, service};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub must_change_password: bool,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub id: i32,
    pub username: String,
    pub email: Option<String>,
    pub must_change_password: bool,
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

fn session_cookie(token: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, token);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie
}

pub async fn login(
    State(db): State<DatabaseConnection>,
    jar: CookieJar,
    Json(body): Json<LoginRequest>,
) -> Result<(CookieJar, Json<LoginResponse>), AppError> {
    let user = service::find_by_username(&db, &body.username)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if !verify_password(&body.password, &user.password_hash) {
        return Err(AppError::Unauthorized);
    }

    let token = create_session(&db, user.id).await?;
    Ok((
        jar.add(session_cookie(token)),
        Json(LoginResponse {
            must_change_password: user.must_change_password,
        }),
    ))
}

pub async fn logout(
    State(db): State<DatabaseConnection>,
    jar: CookieJar,
) -> Result<CookieJar, AppError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        delete_session(&db, cookie.value()).await?;
    }
    Ok(jar.remove(Cookie::from(SESSION_COOKIE)))
}

pub async fn me(AuthUser(user): AuthUser) -> Json<MeResponse> {
    Json(MeResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        must_change_password: user.must_change_password,
    })
}

pub async fn change_password(
    State(db): State<DatabaseConnection>,
    AuthUser(user): AuthUser,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<MeResponse>, AppError> {
    if !verify_password(&body.current_password, &user.password_hash) {
        return Err(AppError::BadRequest("current password is incorrect"));
    }
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "new password must be at least 8 characters",
        ));
    }
    let updated = service::set_password(&db, user, &body.new_password, false).await?;
    Ok(Json(MeResponse {
        id: updated.id,
        username: updated.username,
        email: updated.email,
        must_change_password: updated.must_change_password,
    }))
}

#[derive(Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

/// Public endpoint. Always returns `200` regardless of whether the email
/// matches an account, so it cannot be used to enumerate registered addresses.
/// When it does match and SMTP is configured, a reset link is emailed.
pub async fn forgot_password(
    State(db): State<DatabaseConnection>,
    headers: HeaderMap,
    Json(body): Json<ForgotPasswordRequest>,
) -> Result<StatusCode, AppError> {
    let email = body.email.trim();
    if !email.is_empty() {
        if let Some(user) = service::find_by_email(&db, email).await? {
            match mailer::resolve(&db).await? {
                Some((cfg, _)) => {
                    let token = create_reset_token(&db, user.id, RESET_TOKEN_TTL_HOURS).await?;
                    let link = format!(
                        "{}/reset-password?token={token}",
                        mailer::base_url(&headers)
                    );
                    if let Err(e) = mailer::send_reset_link(&cfg, email, &link, false).await {
                        tracing::warn!("failed to send password reset email: {e}");
                    }
                }
                None => tracing::warn!("password reset requested but SMTP is not configured"),
            }
        }
    }
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

/// Public endpoint. Redeems a reset/setup token and sets the new password.
pub async fn reset_password(
    State(db): State<DatabaseConnection>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<StatusCode, AppError> {
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "new password must be at least 8 characters",
        ));
    }
    let user = consume_reset_token(&db, &body.token).await?;
    service::set_password(&db, user, &body.new_password, false).await?;
    Ok(StatusCode::OK)
}
