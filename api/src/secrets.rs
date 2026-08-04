//! Rotating `CITYHALL_SECRET_KEY`, and reporting what the current key ring can
//! still read (#54).
//!
//! Changing the key used to make every stored secret undecryptable. Now the old
//! key goes in `CITYHALL_SECRET_KEY_PREVIOUS`, the new one in
//! `CITYHALL_SECRET_KEY`, and `cityhall secrets rotate` re-encrypts everything
//! under the new one. The same command with no key change upgrades values written
//! before the envelope existed, which is what binds them to their row.
//!
//! Deliberately a CLI command and not a migration: a migration runs once and is
//! then recorded as applied, so it could serve the first rotation and no other.
//!
//! There is no transaction. Every value is self-describing, so a run that dies
//! halfway leaves some rows rotated and some not, all of them readable, and
//! re-running finishes the job.

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::crypto::{self, Aad, Provenance};
use crate::entities::{
    agent_credential, git_credential, git_ssh_key, oidc_settings, smtp_settings,
};
use crate::error::AppError;
use crate::{agent_credentials, mailer, oidc};

/// Every store, so one holding no secrets still reports a zero row rather than
/// vanishing from the output and reading as forgotten.
const STORES: [&str; 5] = [
    "SMTP password",
    "OIDC client secret",
    "git credentials",
    "git SSH keys",
    "agent credentials",
];

/// One encrypted value in the database, identified well enough to classify it,
/// name it to an operator, and write it back.
///
/// The stores have as many shapes as there are stores, and only what happens to
/// the ciphertext is common, so the shape-specific part is this enum and
/// everything else is shared.
#[derive(Clone, Debug)]
enum Slot {
    SmtpPassword,
    OidcClientSecret,
    GitCredential { user_id: i32 },
    GitSshKey { user_id: i32 },
    AgentCredential { user_id: i32, env_var: String },
}

impl Slot {
    fn store(&self) -> &'static str {
        match self {
            Self::SmtpPassword => STORES[0],
            Self::OidcClientSecret => STORES[1],
            Self::GitCredential { .. } => STORES[2],
            Self::GitSshKey { .. } => STORES[3],
            Self::AgentCredential { .. } => STORES[4],
        }
    }

    /// How this row is named in operator output. Never the value.
    fn label(&self) -> String {
        match self {
            Self::SmtpPassword => "SMTP password".to_string(),
            Self::OidcClientSecret => "OIDC client secret".to_string(),
            Self::GitCredential { user_id } => format!("git credential (user {user_id})"),
            Self::GitSshKey { user_id } => format!("git SSH key (user {user_id})"),
            Self::AgentCredential { user_id, env_var } => format!("{env_var} (user {user_id})"),
        }
    }

    fn aad(&self) -> Aad {
        match self {
            Self::SmtpPassword => Aad::SmtpPassword,
            Self::OidcClientSecret => Aad::OidcClientSecret,
            Self::GitCredential { user_id } => Aad::GitCredential { user_id: *user_id },
            Self::GitSshKey { user_id } => Aad::GitSshKey { user_id: *user_id },
            // Through the store's own helper, so rotation cannot bind a value to
            // something the workspace path will not recognise.
            Self::AgentCredential { user_id, env_var } => agent_credentials::aad(*user_id, env_var),
        }
    }

    /// Replace this row's ciphertext, but only if it still holds `old`.
    ///
    /// Compare and swap rather than a plain update: a user saving this credential
    /// while rotation runs would otherwise have their new value overwritten by a
    /// re-encryption of what rotation read earlier. A zero result means someone
    /// else wrote the row, so it is left alone and reported.
    ///
    /// `updated_at` is deliberately untouched. Rotation does not change the
    /// secret, and leaving the timestamp alone is what makes a second run
    /// observably a no-op.
    async fn write(&self, db: &DatabaseConnection, old: &str, new: &str) -> Result<u64, AppError> {
        let affected = match self {
            Self::SmtpPassword => {
                smtp_settings::Entity::update_many()
                    .col_expr(smtp_settings::Column::PasswordEncrypted, Expr::value(new))
                    .filter(smtp_settings::Column::Id.eq(mailer::SETTINGS_ID))
                    .filter(smtp_settings::Column::PasswordEncrypted.eq(old))
                    .exec(db)
                    .await?
                    .rows_affected
            }
            Self::OidcClientSecret => {
                oidc_settings::Entity::update_many()
                    .col_expr(
                        oidc_settings::Column::ClientSecretEncrypted,
                        Expr::value(new),
                    )
                    .filter(oidc_settings::Column::Id.eq(oidc::SETTINGS_ID))
                    .filter(oidc_settings::Column::ClientSecretEncrypted.eq(old))
                    .exec(db)
                    .await?
                    .rows_affected
            }
            Self::GitCredential { user_id } => {
                git_credential::Entity::update_many()
                    .col_expr(git_credential::Column::TokenEncrypted, Expr::value(new))
                    .filter(git_credential::Column::UserId.eq(*user_id))
                    .filter(git_credential::Column::TokenEncrypted.eq(old))
                    .exec(db)
                    .await?
                    .rows_affected
            }
            Self::GitSshKey { user_id } => {
                git_ssh_key::Entity::update_many()
                    .col_expr(git_ssh_key::Column::KeyEncrypted, Expr::value(new))
                    .filter(git_ssh_key::Column::UserId.eq(*user_id))
                    .filter(git_ssh_key::Column::KeyEncrypted.eq(old))
                    .exec(db)
                    .await?
                    .rows_affected
            }
            Self::AgentCredential { user_id, env_var } => {
                agent_credential::Entity::update_many()
                    .col_expr(agent_credential::Column::ValueEncrypted, Expr::value(new))
                    .filter(agent_credential::Column::UserId.eq(*user_id))
                    .filter(agent_credential::Column::EnvVar.eq(env_var.as_str()))
                    .filter(agent_credential::Column::ValueEncrypted.eq(old))
                    .exec(db)
                    .await?
                    .rows_affected
            }
        };
        Ok(affected)
    }
}

/// Every stored secret, in a stable order so output and tests do not depend on
/// what the database felt like returning.
async fn slots(db: &DatabaseConnection) -> Result<Vec<(Slot, String)>, AppError> {
    let mut out = Vec::new();

    if let Some(encrypted) = smtp_settings::Entity::find_by_id(mailer::SETTINGS_ID)
        .one(db)
        .await?
        .and_then(|row| row.password_encrypted)
    {
        out.push((Slot::SmtpPassword, encrypted));
    }
    if let Some(encrypted) = oidc_settings::Entity::find_by_id(oidc::SETTINGS_ID)
        .one(db)
        .await?
        .and_then(|row| row.client_secret_encrypted)
    {
        out.push((Slot::OidcClientSecret, encrypted));
    }
    for row in git_credential::Entity::find()
        .order_by_asc(git_credential::Column::UserId)
        .all(db)
        .await?
    {
        out.push((
            Slot::GitCredential {
                user_id: row.user_id,
            },
            row.token_encrypted,
        ));
    }
    for row in git_ssh_key::Entity::find()
        .order_by_asc(git_ssh_key::Column::UserId)
        .all(db)
        .await?
    {
        out.push((
            Slot::GitSshKey {
                user_id: row.user_id,
            },
            row.key_encrypted,
        ));
    }
    for row in agent_credential::Entity::find()
        .order_by_asc(agent_credential::Column::UserId)
        .order_by_asc(agent_credential::Column::EnvVar)
        .all(db)
        .await?
    {
        out.push((
            Slot::AgentCredential {
                user_id: row.user_id,
                env_var: row.env_var,
            },
            row.value_encrypted,
        ));
    }

    Ok(out)
}

/// What one store's rows look like under the current key ring.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreReport {
    pub store: &'static str,
    pub rows: usize,
    /// Bound to their row and readable with `CITYHALL_SECRET_KEY` alone. The only
    /// state that needs nothing further.
    pub current: usize,
    /// Readable, but only while `CITYHALL_SECRET_KEY_PREVIOUS` is still set.
    /// Dropping it with these outstanding loses them.
    pub needs_previous: usize,
    /// Predate the envelope, so not bound to the row holding them.
    pub legacy: usize,
    pub unreadable: usize,
}

/// Classify every stored secret.
///
/// "Readable" is not the question an operator has; "readable with the current key
/// alone" is, because that is what decides whether the previous keys can go. So
/// `needs_previous` is counted separately rather than folded into a readable
/// total, which is also what the issue asks for: secrets unreadable *under the
/// current key*.
pub async fn status(db: &DatabaseConnection) -> Result<Vec<StoreReport>, AppError> {
    let mut reports: Vec<StoreReport> = STORES
        .iter()
        .map(|store| StoreReport {
            store,
            ..Default::default()
        })
        .collect();

    for (slot, ciphertext) in slots(db).await? {
        let store = slot.store();
        let Some(report) = reports.iter_mut().find(|r| r.store == store) else {
            continue;
        };
        report.rows += 1;
        match crypto::open(&ciphertext, &slot.aad()) {
            Ok((_, Provenance::CurrentKey)) => report.current += 1,
            Ok((_, Provenance::PreviousKey)) => report.needs_previous += 1,
            Ok((_, Provenance::Legacy)) => report.legacy += 1,
            Err(_) => report.unreadable += 1,
        }
    }

    Ok(reports)
}

/// The outcome of a rotation.
pub struct RotateReport {
    pub rotated: usize,
    /// Rows whose plaintext is gone: no key in the ring opens them, so they can
    /// only be re-entered by hand.
    pub unreadable: Vec<String>,
    /// Rows something else wrote mid-rotation, so they were left alone. Only
    /// non-empty when a writer is still running with an old key, which is why the
    /// documented procedure restarts every replica first.
    pub skipped: Vec<String>,
    /// Recomputed from the database afterwards rather than accumulated, so it
    /// reports what is actually stored and not what rotation believed it did.
    pub after: Vec<StoreReport>,
}

/// Re-encrypt every stored secret that is not already bound and under the
/// current key.
pub async fn rotate(db: &DatabaseConnection) -> Result<RotateReport, AppError> {
    let mut rotated = 0;
    let mut unreadable = Vec::new();
    let mut skipped = Vec::new();

    for (slot, ciphertext) in slots(db).await? {
        let aad = slot.aad();
        let (plaintext, provenance) = match crypto::open(&ciphertext, &aad) {
            Ok(opened) => opened,
            // Nothing here can recover it; naming it is the most that is useful.
            Err(_) => {
                unreadable.push(slot.label());
                continue;
            }
        };
        if provenance == Provenance::CurrentKey {
            // Already bound and already under the key a rewrite would use, so
            // rewriting would only churn the row.
            continue;
        }
        let replacement = crypto::encrypt(&plaintext, &aad)?;
        if slot.write(db, &ciphertext, &replacement).await? == 0 {
            skipped.push(slot.label());
        } else {
            rotated += 1;
        }
    }

    Ok(RotateReport {
        rotated,
        unreadable,
        skipped,
        after: status(db).await?,
    })
}

/// Warn at startup when any stored secret predates the envelope.
///
/// A prefix test per row, so it needs no key and cannot fail on one. Worth saying
/// out loud because the row binding does not protect a legacy value, and an
/// operator who never read a release note would otherwise have no way to learn
/// that rotating is what turns the protection on.
pub async fn warn_about_legacy_secrets(db: &DatabaseConnection) {
    let legacy = match legacy_count(db).await {
        Ok(count) => count,
        // A nag, never a reason to refuse to start.
        Err(e) => {
            tracing::warn!("could not check stored secrets for the legacy format: {e}");
            return;
        }
    };
    if legacy > 0 {
        tracing::warn!(
            legacy_secrets = legacy,
            "{legacy} stored secret(s) are still in the pre-envelope format, so they are not bound \
             to the row holding them; run `cityhall secrets rotate` to upgrade them"
        );
    }
}

/// How many stored values are still in the pre-envelope format. Needs no key,
/// which is the point: this runs at startup and must work when nothing decrypts.
///
/// Counts by format, so it can exceed `status`'s `legacy`, which counts only the
/// ones that also still decrypt. A row that is both is reported as unreadable
/// there, and its own guidance covers it.
async fn legacy_count(db: &DatabaseConnection) -> Result<usize, AppError> {
    Ok(slots(db)
        .await?
        .iter()
        .filter(|(_, ciphertext)| crypto::is_legacy(ciphertext))
        .count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Aad;
    use crate::migration::Migrator;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    use chrono::Utc;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, Set};
    use sea_orm_migration::MigratorTrait;

    const KEY_A: [u8; 32] = [3u8; 32];
    const KEY_B: [u8; 32] = [4u8; 32];

    async fn setup() -> DatabaseConnection {
        let mut opts = ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1);
        let db = Database::connect(opts).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    /// The password is generated rather than written inline: these tests never
    /// authenticate, and a literal here is a hard-coded-credential finding for no
    /// benefit.
    async fn make_user(db: &DatabaseConnection, username: &str) -> i32 {
        crate::service::create(
            db,
            username,
            None,
            &crate::auth::random_token(24),
            false,
            None,
        )
        .await
        .unwrap()
        .id
    }

    /// One row in every store. SMTP is seeded in the pre-envelope format, so each
    /// test covers the upgrade path as well as the key change.
    async fn seed(db: &DatabaseConnection, user_id: i32) {
        smtp_settings::ActiveModel {
            id: Set(mailer::SETTINGS_ID),
            host: Set("smtp.example.com".to_string()),
            port: Set(587),
            encryption: Set("starttls".to_string()),
            username: Set(None),
            password_encrypted: Set(Some(crypto::encrypt_legacy("smtp-secret").unwrap())),
            from_address: Set("cityhall@example.com".to_string()),
            from_name: Set(None),
            enabled: Set(true),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .unwrap();

        oidc_settings::ActiveModel {
            id: Set(oidc::SETTINGS_ID),
            enabled: Set(true),
            issuer: Set("https://idp.example.com".to_string()),
            client_id: Set("cityhall".to_string()),
            client_secret_encrypted: Set(Some(
                crypto::encrypt("oidc-secret", &Aad::OidcClientSecret).unwrap(),
            )),
            scopes: Set("openid email".to_string()),
            allowed_domains: Set(None),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .unwrap();

        git_credential::ActiveModel {
            user_id: Set(user_id),
            host: Set("https://github.com".to_string()),
            username: Set("someone".to_string()),
            token_encrypted: Set(
                crypto::encrypt("git-token", &Aad::GitCredential { user_id }).unwrap(),
            ),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .unwrap();

        git_ssh_key::ActiveModel {
            user_id: Set(user_id),
            key_encrypted: Set(crypto::encrypt("ssh-key", &Aad::GitSshKey { user_id }).unwrap()),
            known_hosts: Set("github.com ssh-ed25519 AAAA\n".to_string()),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .unwrap();

        agent_credential::ActiveModel {
            user_id: Set(user_id),
            env_var: Set("ANTHROPIC_API_KEY".to_string()),
            value_encrypted: Set(crypto::encrypt(
                "sk-agent",
                &agent_credentials::aad(user_id, "ANTHROPIC_API_KEY"),
            )
            .unwrap()),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    fn totals(reports: &[StoreReport]) -> (usize, usize, usize, usize, usize) {
        reports.iter().fold((0, 0, 0, 0, 0), |acc, r| {
            (
                acc.0 + r.rows,
                acc.1 + r.current,
                acc.2 + r.needs_previous,
                acc.3 + r.legacy,
                acc.4 + r.unreadable,
            )
        })
    }

    /// A plain `#[test]` driving its own runtime rather than `#[tokio::test]`:
    /// the key guard is a std mutex, so holding it across an `.await` inside an
    /// async fn would block a whole executor thread. Taking it outside the async
    /// block keeps the serialization without that hazard.
    fn with_key_env<F: std::future::Future<Output = ()>>(body: impl FnOnce() -> F) {
        let _guard = crypto::lock_key_env();
        std::env::set_var("CITYHALL_SECRET_KEY", B64.encode(KEY_A));
        std::env::remove_var("CITYHALL_SECRET_KEY_PREVIOUS");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(body());
        std::env::remove_var("CITYHALL_SECRET_KEY");
        std::env::remove_var("CITYHALL_SECRET_KEY_PREVIOUS");
    }

    #[test]
    fn rotating_a_key_leaves_every_store_readable_under_the_new_one() {
        with_key_env(|| async {
            let db = setup().await;
            let user_id = make_user(&db, "u").await;
            seed(&db, user_id).await;

            // Before: one legacy row, the rest bound and current.
            assert_eq!(totals(&status(&db).await.unwrap()), (5, 4, 0, 1, 0));
            assert_eq!(legacy_count(&db).await.unwrap(), 1);

            // The new key becomes current, the old one decrypt-only.
            std::env::set_var("CITYHALL_SECRET_KEY", B64.encode(KEY_B));
            std::env::set_var("CITYHALL_SECRET_KEY_PREVIOUS", B64.encode(KEY_A));
            assert_eq!(totals(&status(&db).await.unwrap()), (5, 0, 4, 1, 0));

            let report = rotate(&db).await.unwrap();
            assert_eq!(report.rotated, 5);
            assert!(report.unreadable.is_empty());
            assert!(report.skipped.is_empty());
            assert_eq!(totals(&report.after), (5, 5, 0, 0, 0));

            // The point of the exercise: the old key can now go away.
            std::env::remove_var("CITYHALL_SECRET_KEY_PREVIOUS");
            assert_eq!(totals(&status(&db).await.unwrap()), (5, 5, 0, 0, 0));
            assert_eq!(legacy_count(&db).await.unwrap(), 0);

            // And the plaintexts survived the round trip.
            let row = git_credential::Entity::find_by_id(user_id)
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                crypto::decrypt(&row.token_encrypted, &Aad::GitCredential { user_id }).unwrap(),
                "git-token"
            );
            let env = agent_credentials::materialize(&db, user_id).await.unwrap();
            assert_eq!(env.pairs.len(), 1);
            assert_eq!(env.pairs[0].1.expose(), "sk-agent");
        });
    }

    /// Without this, rotation would rewrite every row on every run: each
    /// encryption draws a fresh nonce, so the ciphertext always differs and the
    /// compare and swap always matches.
    #[test]
    fn a_second_rotation_writes_nothing() {
        with_key_env(|| async {
            let db = setup().await;
            let user_id = make_user(&db, "u").await;
            seed(&db, user_id).await;
            assert_eq!(rotate(&db).await.unwrap().rotated, 1); // just the legacy row

            let before = slots(&db).await.unwrap();
            let report = rotate(&db).await.unwrap();
            assert_eq!(report.rotated, 0);
            let after = slots(&db).await.unwrap();
            assert_eq!(
                before.iter().map(|(_, c)| c).collect::<Vec<_>>(),
                after.iter().map(|(_, c)| c).collect::<Vec<_>>(),
            );
        });
    }

    #[test]
    fn an_unreadable_row_is_named_and_does_not_stop_the_rest() {
        with_key_env(|| async {
            let db = setup().await;
            let user_id = make_user(&db, "u").await;
            seed(&db, user_id).await;

            // A value from a key that is not in the ring at all.
            agent_credential::Entity::update_many()
                .col_expr(
                    agent_credential::Column::ValueEncrypted,
                    Expr::value("v2.bm90LWEtcmVhbC1jaXBoZXJ0ZXh0LWF0LWFsbC1ub3BlIQ=="),
                )
                .filter(agent_credential::Column::UserId.eq(user_id))
                .exec(&db)
                .await
                .unwrap();

            let report = rotate(&db).await.unwrap();
            assert_eq!(
                report.unreadable,
                vec![format!("ANTHROPIC_API_KEY (user {user_id})")]
            );
            // The legacy SMTP row was still upgraded.
            assert_eq!(report.rotated, 1);
            assert_eq!(totals(&report.after), (5, 4, 0, 0, 1));
        });
    }

    /// The acceptance criterion for the binding, on the path a workspace takes:
    /// one user's ciphertext dropped into another user's row must not become that
    /// user's credential.
    #[test]
    fn a_credential_moved_to_another_user_is_not_injected() {
        with_key_env(|| async {
            let db = setup().await;
            let alice = make_user(&db, "alice").await;
            let bob = make_user(&db, "bob").await;
            seed(&db, alice).await;

            let stolen =
                agent_credential::Entity::find_by_id((alice, "ANTHROPIC_API_KEY".to_string()))
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap()
                    .value_encrypted;
            agent_credential::ActiveModel {
                user_id: Set(bob),
                env_var: Set("ANTHROPIC_API_KEY".to_string()),
                value_encrypted: Set(stolen),
                updated_at: Set(Utc::now()),
            }
            .insert(&db)
            .await
            .unwrap();

            // Alice still has hers; Bob's workspace starts without the variable
            // rather than with Alice's key.
            assert_eq!(
                agent_credentials::materialize(&db, alice)
                    .await
                    .unwrap()
                    .pairs
                    .len(),
                1
            );
            assert!(agent_credentials::materialize(&db, bob)
                .await
                .unwrap()
                .pairs
                .is_empty());
            // And it is visible as unreadable rather than silently missing.
            let agents = status(&db)
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.store == STORES[4])
                .unwrap();
            assert_eq!((agents.rows, agents.current, agents.unreadable), (2, 1, 1));
        });
    }
}
