//! OIDC single sign-on: the login redirect, the callback, a public
//! provider-discovery endpoint, and the settings CRUD.
//!
//! The flow is authorization code + PKCE. Between the two requests the CSRF
//! state, nonce, and PKCE verifier are carried in a short-lived HttpOnly cookie
//! (the state is compared on return to defend against login CSRF).

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Redirect;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use serde::{Deserialize, Serialize};

use crate::auth::{create_session, random_token, session_cookie, AuthUser};
use crate::crypto;
use crate::entities::oidc_settings;
use crate::error::AppError;
use crate::mailer::base_url;
use crate::oidc::{self, SETTINGS_ID};
use crate::service;

/// Carries the OIDC flow across the redirect. Cleared on callback.
const FLOW_COOKIE: &str = "cityhall_oidc_flow";

#[derive(Serialize, Deserialize)]
struct Flow {
    state: String,
    nonce: String,
    verifier: String,
}

fn flow_cookie(value: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(FLOW_COOKIE, value);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie
}

/// Discover the provider's metadata. The client itself is built inline at each
/// call site: its type-state (which endpoints are set) is not nameable as a
/// return type in the openidconnect crate.
async fn discover(
    cfg: &oidc::OidcConfig,
    http: &reqwest::Client,
) -> Result<CoreProviderMetadata, String> {
    let issuer =
        IssuerUrl::new(cfg.issuer.clone()).map_err(|_| "invalid issuer URL".to_string())?;
    CoreProviderMetadata::discover_async(issuer, http)
        .await
        .map_err(|e| {
            tracing::warn!("OIDC discovery failed: {e}");
            "could not reach the identity provider".to_string()
        })
}

/// The redirect URI registered with the IdP, derived from the request host.
fn redirect_uri(headers: &HeaderMap) -> Result<RedirectUrl, String> {
    let url = format!("{}{}", base_url(headers), oidc::CALLBACK_PATH);
    RedirectUrl::new(url).map_err(|_| "invalid redirect URI".to_string())
}

/// `GET /api/auth/oidc/login` — redirect the browser to the identity provider.
pub async fn login(
    State(db): State<DatabaseConnection>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AppError> {
    let (cfg, _) = oidc::resolve(&db)
        .await?
        .ok_or(AppError::BadRequest("SSO is not configured"))?;
    let http = oidc::http_client()?;
    let setup = async {
        let metadata = discover(&cfg, &http).await?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(cfg.client_id.clone()),
            cfg.client_secret.clone().map(ClientSecret::new),
        )
        .set_redirect_uri(redirect_uri(&headers)?);
        Ok::<_, String>(client)
    };
    let client = setup
        .await
        .map_err(|_| AppError::Internal("OIDC provider setup failed"))?;

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut req = client.authorize_url(
        CoreAuthenticationFlow::AuthorizationCode,
        CsrfToken::new_random,
        Nonce::new_random,
    );
    // The authorization flow already includes `openid`; skip it to avoid a
    // duplicate in the scope parameter.
    for scope in cfg.scopes.iter().filter(|s| s.as_str() != "openid") {
        req = req.add_scope(Scope::new(scope.clone()));
    }
    let (auth_url, csrf, nonce) = req.set_pkce_challenge(challenge).url();

    let flow = Flow {
        state: csrf.secret().clone(),
        nonce: nonce.secret().clone(),
        verifier: verifier.secret().clone(),
    };
    // ponytail: flow state in a plaintext HttpOnly cookie; the returned `state`
    // is compared against it to stop login CSRF. Encrypt with crypto.rs if this
    // ever needs integrity beyond HttpOnly + state binding.
    let value = serde_json::to_string(&flow).map_err(|_| AppError::Internal("flow encode"))?;
    Ok((jar.add(flow_cookie(value)), Redirect::to(auth_url.as_str())))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /api/auth/oidc/callback` — exchange the code, provision/link the user,
/// and start a session. Errors redirect to `/login?error=...` so the SPA can
/// surface them; the flow cookie is always cleared.
pub async fn callback(
    State(db): State<DatabaseConnection>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(q): Query<CallbackQuery>,
) -> (CookieJar, Redirect) {
    let flow = jar
        .get(FLOW_COOKIE)
        .and_then(|c| serde_json::from_str::<Flow>(c.value()).ok());
    let jar = jar.remove(Cookie::from(FLOW_COOKIE));

    match complete(&db, &headers, flow, q).await {
        Ok(user_id) => match create_session(&db, user_id).await {
            Ok(token) => (jar.add(session_cookie(token)), Redirect::to("/")),
            Err(_) => (jar, Redirect::to("/login?error=session")),
        },
        Err(msg) => {
            let enc: String = url_escape(&msg);
            (jar, Redirect::to(&format!("/login?error={enc}")))
        }
    }
}

/// Minimal percent-encoding for the error querystring (letters/digits kept).
fn url_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The callback's fallible core, returning the resolved user id or a
/// user-facing error message.
async fn complete(
    db: &DatabaseConnection,
    headers: &HeaderMap,
    flow: Option<Flow>,
    q: CallbackQuery,
) -> Result<i32, String> {
    if let Some(err) = q.error {
        return Err(format!("provider returned an error: {err}"));
    }
    let code = q.code.ok_or("missing authorization code")?;
    let state = q.state.ok_or("missing state")?;
    let flow = flow.ok_or("login session expired, please try again")?;
    if flow.state != state {
        return Err("state mismatch".to_string());
    }

    let (cfg, _) = oidc::resolve(db)
        .await
        .map_err(|_| "configuration error".to_string())?
        .ok_or("SSO is not configured")?;
    let http = oidc::http_client().map_err(|_| "http client error".to_string())?;
    let metadata = discover(&cfg, &http).await?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(cfg.client_id.clone()),
        cfg.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect_uri(headers)?);

    let token_response = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|_| "token exchange setup failed".to_string())?
        .set_pkce_verifier(PkceCodeVerifier::new(flow.verifier))
        .request_async(&http)
        .await
        .map_err(|e| {
            tracing::warn!("OIDC token exchange failed: {e}");
            "token exchange failed".to_string()
        })?;

    let id_token = token_response
        .id_token()
        .ok_or("provider did not return an ID token")?;
    let claims = id_token
        .claims(&client.id_token_verifier(), &Nonce::new(flow.nonce))
        .map_err(|e| {
            tracing::warn!("OIDC id_token verification failed: {e}");
            "ID token verification failed".to_string()
        })?;

    let email = claims
        .email()
        .map(|e| e.to_string())
        .ok_or("the identity provider did not provide an email address")?;
    let subject = claims.subject().to_string();

    let user = provision(db, &cfg, &email, &subject).await?;
    Ok(user.id)
}

/// Find-or-provision the local account for an OIDC identity.
async fn provision(
    db: &DatabaseConnection,
    cfg: &oidc::OidcConfig,
    email: &str,
    subject: &str,
) -> Result<crate::entities::user::Model, String> {
    let err = |_| "account lookup failed".to_string();

    // 1. Already linked by subject.
    if let Some(u) = service::find_by_oidc_subject(db, subject)
        .await
        .map_err(err)?
    {
        return Ok(u);
    }
    // 2. Existing local account with this email: link it.
    if let Some(u) = service::find_by_email(db, email).await.map_err(err)? {
        let mut active: crate::entities::user::ActiveModel = u.into();
        active.oidc_subject = Set(Some(subject.to_string()));
        return active
            .update(db)
            .await
            .map_err(|_| "could not link account".to_string());
    }
    // 3. Provision a new account. Creating new external accounts is gated by the
    //    self-signup toggle; when off, only accounts an admin already created
    //    (matched above by subject or email) may sign in via SSO.
    if !service::signup_enabled(db).await.map_err(err)? {
        return Err("sign-up is disabled; ask an administrator to create your account".to_string());
    }
    if !oidc::domain_allowed(cfg, email) {
        return Err("your email domain is not allowed to sign in".to_string());
    }
    let role_id = service::signup_role_id(db).await.map_err(err)?;

    // Username defaults to the email; on the rare collision, add a suffix.
    let mut username = email.to_string();
    if service::find_by_username(db, &username)
        .await
        .map_err(err)?
        .is_some()
    {
        username = format!("{email}-{}", random_token(4));
    }
    service::create_sso_user(db, &username, email, role_id, subject)
        .await
        .map_err(|_| "could not create account".to_string())
}

// --- Public provider discovery -------------------------------------------

#[derive(Serialize)]
pub struct ProvidersResponse {
    /// Whether OIDC SSO is configured and enabled (drives the login button).
    pub oidc: bool,
    /// Whether self-signup is enabled (drives the register link).
    pub signup: bool,
}

/// `GET /api/auth/providers` — public; lets the login page decide what to show.
pub async fn providers(
    State(db): State<DatabaseConnection>,
) -> Result<Json<ProvidersResponse>, AppError> {
    Ok(Json(ProvidersResponse {
        oidc: oidc::resolve(&db).await?.is_some(),
        signup: service::signup_enabled(&db).await?,
    }))
}

// --- Settings ------------------------------------------------------------

#[derive(Serialize)]
pub struct OidcSettingsResponse {
    pub env_managed: bool,
    pub enabled: bool,
    pub issuer: String,
    pub client_id: String,
    pub scopes: String,
    pub allowed_domains: String,
    pub client_secret_set: bool,
    pub secret_key_available: bool,
    /// The path to register with the IdP (appended to the public base URL).
    pub callback_path: String,
}

#[derive(Deserialize)]
pub struct UpdateOidcRequest {
    pub enabled: bool,
    pub issuer: String,
    pub client_id: String,
    /// Omitted (or empty) keeps the stored secret; a value replaces it.
    pub client_secret: Option<String>,
    pub scopes: String,
    pub allowed_domains: String,
}

fn response_from_env(cfg: &oidc::OidcConfig) -> OidcSettingsResponse {
    OidcSettingsResponse {
        env_managed: true,
        enabled: true,
        issuer: cfg.issuer.clone(),
        client_id: cfg.client_id.clone(),
        scopes: cfg.scopes.join(" "),
        allowed_domains: cfg.allowed_domains.join(","),
        client_secret_set: cfg.client_secret.is_some(),
        secret_key_available: crypto::key_available(),
        callback_path: oidc::CALLBACK_PATH.to_string(),
    }
}

fn response_from_row(row: Option<oidc_settings::Model>) -> OidcSettingsResponse {
    let secret_key_available = crypto::key_available();
    match row {
        Some(r) => OidcSettingsResponse {
            env_managed: false,
            enabled: r.enabled,
            issuer: r.issuer,
            client_id: r.client_id,
            scopes: r.scopes,
            allowed_domains: r.allowed_domains.unwrap_or_default(),
            client_secret_set: r.client_secret_encrypted.is_some(),
            secret_key_available,
            callback_path: oidc::CALLBACK_PATH.to_string(),
        },
        None => OidcSettingsResponse {
            env_managed: false,
            enabled: false,
            issuer: String::new(),
            client_id: String::new(),
            scopes: "openid email profile".to_string(),
            allowed_domains: String::new(),
            client_secret_set: false,
            secret_key_available,
            callback_path: oidc::CALLBACK_PATH.to_string(),
        },
    }
}

pub async fn get_settings(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
) -> Result<Json<OidcSettingsResponse>, AppError> {
    caller.require("settings.read")?;
    if let Some(cfg) = oidc::from_env() {
        return Ok(Json(response_from_env(&cfg)));
    }
    Ok(Json(response_from_row(oidc::load_row(&db).await?)))
}

pub async fn update_settings(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Json(body): Json<UpdateOidcRequest>,
) -> Result<Json<OidcSettingsResponse>, AppError> {
    caller.require("settings.write")?;
    if oidc::env_managed() {
        return Err(AppError::Conflict(
            "OIDC is managed via environment variables",
        ));
    }
    if body.issuer.trim().is_empty() {
        return Err(AppError::BadRequest("issuer is required"));
    }
    if body.client_id.trim().is_empty() {
        return Err(AppError::BadRequest("client id is required"));
    }

    let existing = oidc::load_row(&db).await?;
    let client_secret_encrypted = match body.client_secret.filter(|s| !s.is_empty()) {
        Some(secret) => Some(crypto::encrypt(&secret)?),
        None => existing
            .as_ref()
            .and_then(|r| r.client_secret_encrypted.clone()),
    };

    let model = oidc_settings::ActiveModel {
        id: Set(SETTINGS_ID),
        enabled: Set(body.enabled),
        issuer: Set(body.issuer.trim().to_string()),
        client_id: Set(body.client_id.trim().to_string()),
        client_secret_encrypted: Set(client_secret_encrypted),
        scopes: Set(body.scopes.trim().to_string()),
        allowed_domains: Set(
            Some(body.allowed_domains.trim().to_string()).filter(|s| !s.is_empty())
        ),
        updated_at: Set(chrono::Utc::now()),
    };
    if existing.is_some() {
        model.update(&db).await?;
    } else {
        model.insert(&db).await?;
    }
    Ok(Json(response_from_row(oidc::load_row(&db).await?)))
}
