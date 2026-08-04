//! Per-user coding-agent credentials: a self-service surface for a user's own
//! variables, plus an admin surface for setting them on someone else's behalf
//! (#16). Storage and delivery live in `crate::agent_credentials`; this module
//! is only the HTTP shape around it.
//!
//! A stored value is never read back out. What the client gets instead is,
//! for every entry in the catalog, whether a value is stored and whether it
//! actually decrypts, so the UI cannot claim a credential is configured while
//! the workspace would silently drop it.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::agent_credentials::{self, CATALOG, STRUCTURED_VIEW_LIMITATION};
use crate::auth::AuthUser;
use crate::crypto;
use crate::entities::{agent_credential, user};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct AgentCredentialsResponse {
    /// Whether `CITYHALL_SECRET_KEY` is configured; without it nothing can be
    /// stored, so the form says so instead of failing on save.
    pub secret_key_available: bool,
    /// The whole catalog, in catalog order, whether or not this user has
    /// stored that variable. The frontend renders exactly this list rather
    /// than hardcoding one of its own.
    pub credentials: Vec<AgentCredentialItem>,
}

#[derive(Serialize)]
pub struct AgentCredentialItem {
    pub env_var: &'static str,
    pub label: &'static str,
    pub structured_view: bool,
    pub limitation: Option<&'static str>,
    /// Whether a row exists for this variable.
    pub value_set: bool,
    /// Whether that row's ciphertext actually decrypts. False while
    /// `value_set` is true means a `CITYHALL_SECRET_KEY` rotation left this
    /// credential behind.
    pub usable: bool,
}

#[derive(Deserialize)]
pub struct UpdateAgentCredentialRequest {
    pub value: String,
}

/// GET /api/me/agent-credentials
pub async fn get_mine(
    State(state): State<AppState>,
    caller: AuthUser,
) -> Result<Json<AgentCredentialsResponse>, AppError> {
    // Gated on workspace use rather than a new permission: a user with no
    // workspace has nowhere for a credential to be injected into.
    caller.require("workspaces.use")?;
    Ok(Json(build_response(&state.db, caller.user.id).await?))
}

/// PUT /api/me/agent-credentials/{env_var}
pub async fn put_mine(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(env_var): Path<String>,
    Json(body): Json<UpdateAgentCredentialRequest>,
) -> Result<Json<AgentCredentialsResponse>, AppError> {
    caller.require("workspaces.use")?;
    set_credential(&state.db, caller.user.id, &env_var, &body.value).await?;
    // Every mutation is audited, self-service included, so the two paths
    // share one trail; the admin path (below) is the one that actually
    // matters, since there the actor and the affected workspace differ.
    tracing::info!(
        actor_id = caller.user.id,
        target_user_id = caller.user.id,
        action = "set",
        env_var = %env_var,
        "agent credential updated"
    );
    Ok(Json(build_response(&state.db, caller.user.id).await?))
}

/// DELETE /api/me/agent-credentials/{env_var}
pub async fn delete_mine(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(env_var): Path<String>,
) -> Result<Json<AgentCredentialsResponse>, AppError> {
    caller.require("workspaces.use")?;
    delete_credential(&state.db, caller.user.id, &env_var).await?;
    tracing::info!(
        actor_id = caller.user.id,
        target_user_id = caller.user.id,
        action = "delete",
        env_var = %env_var,
        "agent credential updated"
    );
    Ok(Json(build_response(&state.db, caller.user.id).await?))
}

/// GET /api/users/{user_id}/agent-credentials
pub async fn get_for_user(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_id): Path<i32>,
) -> Result<Json<AgentCredentialsResponse>, AppError> {
    caller.require("workspaces.write")?;
    ensure_user_exists(&state.db, user_id).await?;
    Ok(Json(build_response(&state.db, user_id).await?))
}

/// PUT /api/users/{user_id}/agent-credentials/{env_var}
pub async fn put_for_user(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((user_id, env_var)): Path<(i32, String)>,
    Json(body): Json<UpdateAgentCredentialRequest>,
) -> Result<Json<AgentCredentialsResponse>, AppError> {
    caller.require("workspaces.write")?;
    // Checked ahead of the user lookup: an unknown variable is a bad request
    // no matter who the target is, and there is no reason to spend a query
    // finding that out.
    agent_credentials::lookup(&env_var).ok_or_else(|| {
        AppError::BadRequestOwned(format!("unknown agent credential `{env_var}`"))
    })?;
    // An admin can name any user id; check it exists so a typo reports 404
    // instead of surfacing as a foreign-key failure from the insert below.
    ensure_user_exists(&state.db, user_id).await?;
    set_credential(&state.db, user_id, &env_var, &body.value).await?;
    // This is the line that matters: an admin changed what will run in
    // someone else's workspace without that person doing it themselves.
    tracing::info!(
        actor_id = caller.user.id,
        target_user_id = user_id,
        action = "set",
        env_var = %env_var,
        "agent credential updated"
    );
    Ok(Json(build_response(&state.db, user_id).await?))
}

/// DELETE /api/users/{user_id}/agent-credentials/{env_var}
pub async fn delete_for_user(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((user_id, env_var)): Path<(i32, String)>,
) -> Result<Json<AgentCredentialsResponse>, AppError> {
    caller.require("workspaces.write")?;
    ensure_user_exists(&state.db, user_id).await?;
    delete_credential(&state.db, user_id, &env_var).await?;
    tracing::info!(
        actor_id = caller.user.id,
        target_user_id = user_id,
        action = "delete",
        env_var = %env_var,
        "agent credential updated"
    );
    Ok(Json(build_response(&state.db, user_id).await?))
}

async fn ensure_user_exists(db: &DatabaseConnection, user_id: i32) -> Result<(), AppError> {
    user::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound("user not found"))?;
    Ok(())
}

/// The full catalog, decorated with `user_id`'s stored rows.
async fn build_response(
    db: &DatabaseConnection,
    user_id: i32,
) -> Result<AgentCredentialsResponse, AppError> {
    let rows = agent_credential::Entity::find()
        .filter(agent_credential::Column::UserId.eq(user_id))
        .all(db)
        .await?;

    let credentials = CATALOG
        .iter()
        .map(|kind| {
            let row = rows.iter().find(|r| r.env_var == kind.env_var);
            AgentCredentialItem {
                env_var: kind.env_var,
                label: kind.label,
                structured_view: kind.structured_view,
                limitation: (!kind.structured_view).then_some(STRUCTURED_VIEW_LIMITATION),
                value_set: row.is_some(),
                usable: row.is_some_and(|r| {
                    agent_credentials::usable(
                        &r.value_encrypted,
                        &agent_credentials::aad(r.user_id, &r.env_var),
                    )
                }),
            }
        })
        .collect();

    Ok(AgentCredentialsResponse {
        secret_key_available: crypto::key_available(),
        credentials,
    })
}

/// Validate, encrypt, and upsert one credential. A blank submit is not "keep
/// the existing value" the way the git credential treats an empty token: each
/// variable here is a single field, so `validate_value` rejects it outright,
/// and deleting is the way to remove one.
async fn set_credential(
    db: &DatabaseConnection,
    user_id: i32,
    env_var: &str,
    value: &str,
) -> Result<(), AppError> {
    agent_credentials::lookup(env_var).ok_or_else(|| {
        AppError::BadRequestOwned(format!("unknown agent credential `{env_var}`"))
    })?;
    let value = agent_credentials::validate_value(value)?;
    // Through the store's own helper, like every read of one of these: the
    // encoding lives in one place, so the write cannot drift from what the
    // workspace path will accept.
    let encrypted = crypto::encrypt(&value, &agent_credentials::aad(user_id, env_var))?;

    // One upsert statement, like the git credential's: two concurrent saves
    // for the same user and variable cannot then both see no row and race to
    // insert.
    agent_credential::Entity::insert(agent_credential::ActiveModel {
        user_id: Set(user_id),
        env_var: Set(env_var.to_string()),
        value_encrypted: Set(encrypted),
        updated_at: Set(Utc::now()),
    })
    .on_conflict(
        OnConflict::columns([
            agent_credential::Column::UserId,
            agent_credential::Column::EnvVar,
        ])
        .update_columns([
            agent_credential::Column::ValueEncrypted,
            agent_credential::Column::UpdatedAt,
        ])
        .to_owned(),
    )
    .exec(db)
    .await?;
    Ok(())
}

/// Idempotent: deleting a credential that was never stored still succeeds.
async fn delete_credential(
    db: &DatabaseConnection,
    user_id: i32,
    env_var: &str,
) -> Result<(), AppError> {
    agent_credential::Entity::delete_by_id((user_id, env_var.to_string()))
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::Migrator;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database};
    use sea_orm_migration::MigratorTrait;

    async fn setup() -> DatabaseConnection {
        let mut opts = ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1);
        let db = Database::connect(opts).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    /// The password is generated rather than written inline: these tests
    /// never authenticate, and a literal here is a hard-coded-credential
    /// finding for no benefit.
    async fn make_user(db: &DatabaseConnection) -> i32 {
        crate::service::create(db, "u", None, &crate::auth::random_token(24), false, None)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn unknown_env_var_is_rejected() {
        let db = setup().await;
        let user_id = make_user(&db).await;
        let err = set_credential(&db, user_id, "NOT_IN_CATALOG", "sk-x")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequestOwned(_)));
    }

    #[tokio::test]
    async fn full_catalog_is_always_listed() {
        let db = setup().await;
        let user_id = make_user(&db).await;
        let resp = build_response(&db, user_id).await.unwrap();
        assert_eq!(resp.credentials.len(), CATALOG.len());
        assert!(resp.credentials.iter().all(|c| !c.value_set && !c.usable));
    }

    #[tokio::test]
    async fn limitation_is_present_only_for_non_structured_view_entries() {
        let db = setup().await;
        let user_id = make_user(&db).await;
        let resp = build_response(&db, user_id).await.unwrap();
        for item in &resp.credentials {
            assert_eq!(
                item.limitation.is_some(),
                !item.structured_view,
                "{}",
                item.env_var
            );
        }
    }

    #[tokio::test]
    async fn delete_removes_and_is_idempotent() {
        let db = setup().await;
        let user_id = make_user(&db).await;
        // Inserted directly rather than through `set_credential`: deletion
        // does not need the value to decrypt, and this keeps the test free
        // of the CITYHALL_SECRET_KEY dependency.
        agent_credential::ActiveModel {
            user_id: Set(user_id),
            env_var: Set("ANTHROPIC_API_KEY".to_string()),
            value_encrypted: Set("not-real-ciphertext".to_string()),
            updated_at: Set(Utc::now()),
        }
        .insert(&db)
        .await
        .unwrap();

        delete_credential(&db, user_id, "ANTHROPIC_API_KEY")
            .await
            .unwrap();
        let resp = build_response(&db, user_id).await.unwrap();
        assert!(
            !resp
                .credentials
                .iter()
                .find(|c| c.env_var == "ANTHROPIC_API_KEY")
                .unwrap()
                .value_set
        );

        // Calling it again on the now-absent row must still succeed.
        delete_credential(&db, user_id, "ANTHROPIC_API_KEY")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn undecryptable_row_reports_value_set_but_not_usable() {
        let db = setup().await;
        let user_id = make_user(&db).await;
        agent_credential::ActiveModel {
            user_id: Set(user_id),
            env_var: Set("ANTHROPIC_API_KEY".to_string()),
            value_encrypted: Set("not-real-ciphertext".to_string()),
            updated_at: Set(Utc::now()),
        }
        .insert(&db)
        .await
        .unwrap();

        let resp = build_response(&db, user_id).await.unwrap();
        let item = resp
            .credentials
            .iter()
            .find(|c| c.env_var == "ANTHROPIC_API_KEY")
            .unwrap();
        assert!(item.value_set);
        assert!(!item.usable);
    }

    // Combined into one test because CITYHALL_SECRET_KEY is a process-global
    // env var; a second test setting it in parallel would race, the same
    // reason `crypto::tests::encrypt_decrypt_and_missing_key` is one test.
    // The lock keeps this from racing that one, which clears the key.
    //
    // A plain `#[test]` driving its own runtime rather than `#[tokio::test]`:
    // the guard is a std mutex, so holding it across an `.await` inside an
    // async fn would block a whole executor thread. Taking it outside the
    // async block keeps the serialization without that hazard.
    #[test]
    fn key_dependent_credential_lifecycle() {
        let _guard = crypto::lock_key_env();
        std::env::set_var("CITYHALL_SECRET_KEY", B64.encode([7u8; 32]));

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let db = setup().await;
                let user_id = make_user(&db).await;

                // Storing works for a user with no `workspaces` row: nothing
                // here ever touches that table.
                set_credential(&db, user_id, "ANTHROPIC_API_KEY", "sk-secret-value")
                    .await
                    .unwrap();
                let resp = build_response(&db, user_id).await.unwrap();
                let item = resp
                    .credentials
                    .iter()
                    .find(|c| c.env_var == "ANTHROPIC_API_KEY")
                    .unwrap();
                assert!(item.value_set);
                assert!(item.usable);
                let serialized = serde_json::to_string(&resp).unwrap();
                assert!(!serialized.contains("sk-secret-value"));

                // Storing the same variable again replaces rather than
                // duplicating.
                set_credential(&db, user_id, "ANTHROPIC_API_KEY", "sk-second-value")
                    .await
                    .unwrap();
                let rows = agent_credential::Entity::find()
                    .filter(agent_credential::Column::UserId.eq(user_id))
                    .all(&db)
                    .await
                    .unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    crypto::decrypt(
                        &rows[0].value_encrypted,
                        &agent_credentials::aad(user_id, "ANTHROPIC_API_KEY")
                    )
                    .unwrap(),
                    "sk-second-value"
                );
            });

        std::env::remove_var("CITYHALL_SECRET_KEY");
    }
}
