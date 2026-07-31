//! The aoe config bundle CityHall stores and serves to workspaces (#9, #43,
//! #44).
//!
//! A workspace runs `aoe serve --cityhall`, which closes every route that could
//! configure it: settings, project CRUD, git clone. So configuration arrives as
//! one document the workspace fetches at boot. Three surfaces here:
//!
//! - **`/api/settings/workspace-config`** — the admin's copy of the document.
//!   Stored as opaque TOML: aoe defines the settings schema, so aoe owns the
//!   format, and CityHall never reimplements it.
//! - **`/api/me/git-credential`** — each user's own git credential, so commits
//!   and pushes from their workspace carry their identity rather than a shared
//!   robot's. Stored encrypted; the token is never read back out to a client.
//! - **`/api/workspace-bundle`** — what a workspace container fetches. The only
//!   place the two provenances meet: the admin's document plus that user's
//!   `[git]` section. This is why the stored bundle never has to hold a secret.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::crypto;
use crate::entities::{git_credential, user, workspace, workspace_config};
use crate::error::AppError;

/// The config row is a singleton, like `workspace_settings`.
pub const CONFIG_ID: i32 = 1;

/// Bundle format version this CityHall knows how to shape-check and compose.
/// Matches aoe's `cityhall_bundle::SCHEMA_VERSION`.
const SCHEMA_VERSION: i64 = 1;

/// A bundle with nothing configured. Served when no admin document is stored
/// yet, so a workspace still gets its git identity rather than failing its boot
/// fetch outright.
///
/// Built from `SCHEMA_VERSION` rather than spelled out, so bumping the version
/// cannot leave this declaring the old one and serving a document aoe rejects.
fn empty_bundle() -> String {
    format!("schema_version = {SCHEMA_VERSION}\n")
}

// --- Admin: the stored document -------------------------------------------

#[derive(Serialize)]
pub struct WorkspaceConfigResponse {
    pub bundle: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub summary: BundleSummary,
}

/// What the stored document actually contains, so the UI can show it without
/// the admin having to read TOML.
#[derive(Serialize, Default)]
pub struct BundleSummary {
    pub schema_version: Option<i64>,
    /// Number of overridden settings leaves.
    pub settings_count: usize,
    pub projects: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdateWorkspaceConfigRequest {
    pub bundle: String,
}

/// GET /api/settings/workspace-config
pub async fn get_config(
    State(state): State<crate::state::AppState>,
    caller: AuthUser,
) -> Result<Json<WorkspaceConfigResponse>, AppError> {
    caller.require("settings.read")?;
    let row = workspace_config::Entity::find_by_id(CONFIG_ID)
        .one(&state.db)
        .await?;
    let bundle = row.as_ref().map(|r| r.bundle.clone()).unwrap_or_default();
    // Summarize best-effort: a stored document always passed validation, but a
    // load must never fail just because the summary could not be computed.
    let summary = parse_bundle(&bundle).map(summarize).unwrap_or_default();
    Ok(Json(WorkspaceConfigResponse {
        bundle,
        updated_at: row.map(|r| r.updated_at),
        summary,
    }))
}

/// PUT /api/settings/workspace-config
pub async fn update_config(
    State(state): State<crate::state::AppState>,
    caller: AuthUser,
    Json(body): Json<UpdateWorkspaceConfigRequest>,
) -> Result<Json<WorkspaceConfigResponse>, AppError> {
    caller.require("settings.write")?;

    let bundle = body.bundle.trim().to_string();
    let summary = if bundle.is_empty() {
        // Clearing the document is legitimate: it turns config provisioning off
        // without having to tear down the deployment's workspaces.
        BundleSummary::default()
    } else {
        summarize(validate_bundle(&bundle)?)
    };

    let now = Utc::now();
    // One statement rather than find-then-insert-or-update: two concurrent saves
    // could otherwise both see no row and race to insert, and the loser fails on
    // the primary key.
    workspace_config::Entity::insert(workspace_config::ActiveModel {
        id: Set(CONFIG_ID),
        bundle: Set(bundle.clone()),
        updated_at: Set(now),
    })
    .on_conflict(
        OnConflict::column(workspace_config::Column::Id)
            .update_columns([
                workspace_config::Column::Bundle,
                workspace_config::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec(&state.db)
    .await?;

    Ok(Json(WorkspaceConfigResponse {
        bundle,
        updated_at: Some(now),
        summary,
    }))
}

// --- Self-service: a user's git credential --------------------------------

#[derive(Serialize)]
pub struct GitCredentialResponse {
    pub host: String,
    pub username: String,
    /// Whether a token is stored. The token itself is never returned.
    pub token_set: bool,
    /// Whether `CITYHALL_SECRET_KEY` is configured; without it nothing can be
    /// stored, so the form says so instead of failing on save.
    pub secret_key_available: bool,
}

#[derive(Deserialize)]
pub struct UpdateGitCredentialRequest {
    pub host: String,
    pub username: String,
    /// Omitted (or empty) keeps the stored token; a value replaces it.
    pub token: Option<String>,
}

/// GET /api/me/git-credential
pub async fn get_git_credential(
    State(state): State<crate::state::AppState>,
    caller: AuthUser,
) -> Result<Json<GitCredentialResponse>, AppError> {
    // Gated on workspace use rather than a new permission: a user with no
    // workspace has nowhere for a credential to be forwarded to.
    caller.require("workspaces.use")?;
    let row = git_credential::Entity::find_by_id(caller.user.id)
        .one(&state.db)
        .await?;
    Ok(Json(GitCredentialResponse {
        host: row
            .as_ref()
            .map(|r| r.host.clone())
            .unwrap_or_else(|| "https://github.com".to_string()),
        username: row.as_ref().map(|r| r.username.clone()).unwrap_or_default(),
        token_set: row.is_some(),
        secret_key_available: crypto::key_available(),
    }))
}

/// PUT /api/me/git-credential
pub async fn update_git_credential(
    State(state): State<crate::state::AppState>,
    caller: AuthUser,
    Json(body): Json<UpdateGitCredentialRequest>,
) -> Result<Json<GitCredentialResponse>, AppError> {
    caller.require("workspaces.use")?;

    let host = body.host.trim().trim_end_matches('/').to_string();
    let username = body.username.trim().to_string();
    if host.is_empty() {
        return Err(AppError::BadRequest("host is required"));
    }
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required"));
    }

    let existing = git_credential::Entity::find_by_id(caller.user.id)
        .one(&state.db)
        .await?;
    let token = match body
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(token) => crypto::encrypt(token)?,
        None => existing
            .as_ref()
            .map(|r| r.token_encrypted.clone())
            .ok_or(AppError::BadRequest("a token is required"))?,
    };

    // The `existing` lookup above stays: it is what lets a save omit the token and
    // keep the stored one. The write itself is a single upsert, so two concurrent
    // saves cannot both decide to insert.
    git_credential::Entity::insert(git_credential::ActiveModel {
        user_id: Set(caller.user.id),
        host: Set(host),
        username: Set(username),
        token_encrypted: Set(token),
        updated_at: Set(Utc::now()),
    })
    .on_conflict(
        OnConflict::column(git_credential::Column::UserId)
            .update_columns([
                git_credential::Column::Host,
                git_credential::Column::Username,
                git_credential::Column::TokenEncrypted,
                git_credential::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec(&state.db)
    .await?;

    get_git_credential(State(state), caller).await
}

/// DELETE /api/me/git-credential
pub async fn delete_git_credential(
    State(state): State<crate::state::AppState>,
    caller: AuthUser,
) -> Result<Json<GitCredentialResponse>, AppError> {
    caller.require("workspaces.use")?;
    git_credential::Entity::delete_by_id(caller.user.id)
        .exec(&state.db)
        .await?;
    get_git_credential(State(state), caller).await
}

// --- Workspaces: the composed document ------------------------------------

/// GET /api/workspace-bundle
///
/// Authenticated by the per-workspace bearer token, not the session cookie: the
/// caller is a workspace container, which has no browser session. The token maps
/// to exactly one user, so a workspace can only ever fetch its own document.
pub async fn serve_bundle(
    State(state): State<crate::state::AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let token = bearer_token(&headers).ok_or(AppError::Unauthorized)?;
    let ws = workspace::Entity::find()
        .filter(workspace::Column::BundleToken.eq(token))
        .one(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let owner = user::Entity::find_by_id(ws.user_id)
        .one(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let composed = compose(&state.db, &owner).await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/toml"),
            // The body carries the user's decrypted git token, so no proxy or
            // client cache may keep a copy of it on disk.
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        composed,
    )
        .into_response())
}

/// The `Authorization: Bearer <token>` value, if present.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw.strip_prefix("Bearer ")?.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// The stored document with `owner`'s `[git]` section spliced in.
async fn compose(db: &DatabaseConnection, owner: &user::Model) -> Result<String, AppError> {
    let stored = workspace_config::Entity::find_by_id(CONFIG_ID)
        .one(db)
        .await?
        .map(|r| r.bundle)
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(empty_bundle);

    let mut table = parse_bundle(&stored)?;
    table.insert("git".to_string(), git_table(db, owner).await?);
    toml::to_string_pretty(&table)
        .map_err(|_| AppError::Internal("failed to serialize the workspace bundle"))
}

/// A user's `[git]` table: always their commit identity, plus their credential
/// when they have stored one.
///
/// The identity goes in even without a credential, so commits made inside a
/// workspace are attributed correctly whether or not the user ever set a token.
async fn git_table(db: &DatabaseConnection, owner: &user::Model) -> Result<toml::Value, AppError> {
    let mut git = toml::Table::new();
    git.insert(
        "user_name".to_string(),
        toml::Value::String(owner.username.clone()),
    );
    if let Some(email) = owner.email.as_deref().filter(|e| !e.is_empty()) {
        git.insert(
            "user_email".to_string(),
            toml::Value::String(email.to_string()),
        );
    }

    if let Some(cred) = git_credential::Entity::find_by_id(owner.id).one(db).await? {
        // A token that no longer decrypts (after a CITYHALL_SECRET_KEY rotation,
        // say) must not cost this user their whole bundle: without it the
        // workspace would boot with no settings and no projects either. Serve the
        // rest and drop the secret, loudly enough that an operator can tell the
        // user to re-enter it.
        match crypto::decrypt(&cred.token_encrypted) {
            Ok(token) => {
                git.insert(
                    "credential_host".to_string(),
                    toml::Value::String(cred.host),
                );
                git.insert(
                    "credential_username".to_string(),
                    toml::Value::String(cred.username),
                );
                git.insert("credential_token".to_string(), toml::Value::String(token));
            }
            Err(e) => tracing::warn!(
                user_id = owner.id,
                "git credential could not be decrypted, serving the bundle without it: {e}"
            ),
        }
    }

    Ok(toml::Value::Table(git))
}

// --- Validation -----------------------------------------------------------

fn parse_bundle(raw: &str) -> Result<toml::Table, AppError> {
    raw.parse::<toml::Table>()
        .map_err(|e| AppError::BadRequestOwned(format!("not valid TOML: {e}")))
}

/// Shape-check a submitted bundle.
///
/// Deliberately does **not** validate setting keys: CityHall does not have aoe's
/// settings schema, and duplicating it here would drift out of step with every
/// aoe release. aoe rejects an unknown key when it applies the document, and
/// that failure surfaces as a workspace that would not start. What this checks is
/// the structure a workspace depends on, plus the one rule only CityHall knows:
/// `[git]` is CityHall's to compose per user, never the admin's to set, because
/// a document holding one user's token would be served to everybody.
fn validate_bundle(raw: &str) -> Result<toml::Table, AppError> {
    let table = parse_bundle(raw)?;

    match table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
    {
        Some(SCHEMA_VERSION) => {}
        Some(other) => {
            return Err(AppError::BadRequestOwned(format!(
                "unsupported schema_version {other}; this CityHall understands {SCHEMA_VERSION}"
            )))
        }
        None => {
            return Err(AppError::BadRequest(
                "missing schema_version; export a bundle with `aoe cityhall export`",
            ))
        }
    }

    if table.contains_key("git") {
        return Err(AppError::BadRequest(
            "a bundle must not contain a [git] section; CityHall adds each user's git identity and credential when it serves the bundle",
        ));
    }

    if let Some(settings) = table.get("settings") {
        if !settings.is_table() {
            return Err(AppError::BadRequest("[settings] must be a table"));
        }
    }

    // `projects()` treats any non-array as absent, so without this a
    // `projects = "oops"` would store cleanly and then fail aoe's parse at boot,
    // which is the failure this shape check exists to catch.
    if let Some(projects) = table.get("projects") {
        if !projects.is_array() {
            return Err(AppError::BadRequest("[[projects]] must be an array"));
        }
    }

    for (index, project) in projects(&table).iter().enumerate() {
        let Some(project) = project.as_table() else {
            return Err(AppError::BadRequest(
                "each [[projects]] entry must be a table",
            ));
        };
        for key in ["name", "remote"] {
            let ok = project
                .get(key)
                .and_then(toml::Value::as_str)
                .is_some_and(|v| !v.trim().is_empty());
            if !ok {
                return Err(AppError::BadRequestOwned(format!(
                    "project #{} is missing a non-empty `{key}`",
                    index + 1
                )));
            }
        }
    }

    Ok(table)
}

fn projects(table: &toml::Table) -> Vec<toml::Value> {
    table
        .get("projects")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn summarize(table: toml::Table) -> BundleSummary {
    let settings_count = table
        .get("settings")
        .and_then(toml::Value::as_table)
        .map(|sections| {
            sections
                .values()
                .filter_map(toml::Value::as_table)
                .map(|fields| fields.len())
                .sum()
        })
        .unwrap_or(0);
    BundleSummary {
        schema_version: table
            .get("schema_version")
            .and_then(toml::Value::as_integer),
        settings_count,
        projects: projects(&table)
            .iter()
            .filter_map(|p| p.get("name").and_then(toml::Value::as_str))
            .map(str::to_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
schema_version = 1

[settings.acp]
default_agent = "claude-code"

[[projects]]
name = "cityhall"
remote = "https://github.com/agent-of-empires/cityhall.git"
"#;

    #[test]
    fn accepts_an_exported_bundle_and_summarizes_it() {
        let summary = summarize(validate_bundle(GOOD).unwrap());
        assert_eq!(summary.schema_version, Some(1));
        assert_eq!(summary.settings_count, 1);
        assert_eq!(summary.projects, vec!["cityhall".to_string()]);
    }

    #[test]
    fn rejects_a_missing_or_future_schema_version() {
        assert!(validate_bundle("[settings.acp]\ndefault_agent = \"x\"\n").is_err());
        let err = validate_bundle("schema_version = 99\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("99"), "must name the version: {err}");
    }

    /// A stored `[git]` would put one user's token in every user's bundle.
    #[test]
    fn rejects_an_admin_supplied_git_section() {
        let raw = "schema_version = 1\n\n[git]\ncredential_token = \"leaked\"\n";
        let err = validate_bundle(raw).unwrap_err().to_string();
        assert!(err.contains("[git]"), "{err}");
    }

    #[test]
    fn rejects_a_project_without_a_remote() {
        let raw = "schema_version = 1\n\n[[projects]]\nname = \"cityhall\"\n";
        let err = validate_bundle(raw).unwrap_err().to_string();
        assert!(err.contains("remote"), "{err}");
        assert!(err.contains("#1"), "must say which entry: {err}");
    }

    /// `projects` of the wrong type used to be read as "no projects" and stored
    /// happily, then failed aoe's parse at boot, which is the failure this
    /// shape check exists to catch.
    #[test]
    fn rejects_a_projects_key_that_is_not_an_array() {
        for raw in [
            "schema_version = 1\nprojects = \"oops\"\n",
            "schema_version = 1\nprojects = 3\n",
        ] {
            let err = validate_bundle(raw).unwrap_err().to_string();
            assert!(err.contains("array"), "{raw}: {err}");
        }
    }

    /// An unknown settings key is aoe's to reject, not CityHall's: duplicating
    /// aoe's schema here would drift with every aoe release.
    #[test]
    fn does_not_second_guess_aoe_settings_keys() {
        let raw = "schema_version = 1\n\n[settings.acp]\nsomething_new_in_aoe = 3\n";
        assert!(validate_bundle(raw).is_ok());
    }

    /// Splicing `[git]` in must produce a document aoe can still parse. TOML
    /// forbids a bare key after a table, and the git table is inserted last, so
    /// this is exactly where a naive serialize would emit something invalid.
    #[test]
    fn a_composed_bundle_round_trips() {
        let mut table = validate_bundle(GOOD).unwrap();
        let mut git = toml::Table::new();
        git.insert("user_name".into(), toml::Value::String("Someone".into()));
        git.insert(
            "credential_token".into(),
            toml::Value::String("secret".into()),
        );
        table.insert("git".to_string(), toml::Value::Table(git));

        let rendered = toml::to_string_pretty(&table).unwrap();
        let reparsed: toml::Table = rendered.parse().expect(&rendered);
        assert_eq!(reparsed["schema_version"].as_integer(), Some(1));
        assert_eq!(reparsed["git"]["user_name"].as_str(), Some("Someone"));
        assert_eq!(
            reparsed["projects"][0]["name"].as_str(),
            Some("cityhall"),
            "the project list must survive: {rendered}"
        );
        assert_eq!(
            reparsed["settings"]["acp"]["default_agent"].as_str(),
            Some("claude-code")
        );
    }

    #[test]
    fn bearer_token_is_read_from_the_authorization_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer_token(&headers), None);
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer abc".parse().unwrap(),
        );
        assert_eq!(bearer_token(&headers), Some("abc".to_string()));
        // Another scheme is not a bearer token.
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Basic abc".parse().unwrap(),
        );
        assert_eq!(bearer_token(&headers), None);
    }
}
