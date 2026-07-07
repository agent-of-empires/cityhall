use axum::extract::State;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::auth::{create_session, delete_session, verify_password, AuthUser, SESSION_COOKIE};
use crate::error::AppError;
use crate::service;

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
