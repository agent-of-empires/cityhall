//! Per-user coding-agent credentials, forwarded into a workspace as
//! environment variables (#16).
//!
//! Storage mirrors the git credential (#43): one encrypted row per user per
//! variable, never read back out to a client. Delivery differs, because the aoe
//! config bundle cannot carry these: aoe deserializes the bundle into a fixed
//! struct with no `deny_unknown_fields`, so an invented section is parsed and
//! silently dropped. Environment injection at container create is what reaches
//! the agent processes without an aoe-side change.
//!
//! Which variables are storable is a closed catalog rather than a denylist. The
//! values become the workspace's environment, so anything not named here (from
//! `PATH` and `LD_PRELOAD` to CityHall's own `AOE_CITYHALL_BUNDLE_TOKEN`) would
//! be a way to reconfigure the workspace from a credential form.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use sha2::{Digest, Sha256};

use crate::crypto::{self, Secret};
use crate::entities::agent_credential;
use crate::error::AppError;

/// Longest value accepted. Provider keys and OAuth tokens are far shorter; the
/// cap exists so a credential form cannot be used to push an unbounded blob
/// through into a container environment.
const MAX_VALUE_LEN: usize = 8 * 1024;

/// One storable credential.
pub struct CredentialKind {
    pub env_var: &'static str,
    pub label: &'static str,
    /// Whether aoe forwards this variable into structured-view (ACP) agent
    /// processes. Those spawns `env_clear()` and then forward a fixed
    /// allowlist, so a variable outside it reaches terminal sessions and is
    /// stripped from structured-view agents. False means partially functional,
    /// which the API reports and the UI badges rather than leaving to the docs.
    pub structured_view: bool,
}

/// Every variable a user may store, and nothing else.
pub const CATALOG: &[CredentialKind] = &[
    CredentialKind {
        env_var: "ANTHROPIC_API_KEY",
        label: "Anthropic API key (Claude)",
        structured_view: true,
    },
    CredentialKind {
        env_var: "ANTHROPIC_AUTH_TOKEN",
        label: "Anthropic auth token (Claude via a gateway)",
        structured_view: true,
    },
    CredentialKind {
        env_var: "CLAUDE_CODE_OAUTH_TOKEN",
        label: "Claude Code OAuth token (subscription login)",
        structured_view: true,
    },
    CredentialKind {
        env_var: "OPENAI_API_KEY",
        label: "OpenAI API key (Codex)",
        structured_view: false,
    },
    CredentialKind {
        env_var: "GEMINI_API_KEY",
        label: "Gemini API key",
        structured_view: false,
    },
    CredentialKind {
        env_var: "OPENROUTER_API_KEY",
        label: "OpenRouter API key (OpenCode)",
        structured_view: false,
    },
];

/// The message shown next to a credential that a structured-view agent will not
/// receive. Stated once here so the API and the UI cannot drift apart.
pub const STRUCTURED_VIEW_LIMITATION: &str =
    "Available in terminal sessions only. Structured-view agents do not receive this variable yet.";

/// The catalog entry for `env_var`, matched exactly. Case-sensitive: environment
/// variable names are, and a lenient match here would let `path` through as
/// `PATH` on a case-insensitive comparison.
pub fn lookup(env_var: &str) -> Option<&'static CredentialKind> {
    CATALOG.iter().find(|k| k.env_var == env_var)
}

/// Normalize and check a submitted value, or explain why it is unusable.
///
/// Outer whitespace goes: keys are pasted, and a trailing newline is a
/// copy-paste artifact rather than part of the secret. A NUL byte cannot exist
/// in a process environment at all, so it is rejected here instead of failing
/// the container start much later.
pub fn validate_value(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest("a value is required"));
    }
    if value.contains('\0') {
        return Err(AppError::BadRequest(
            "a credential value cannot contain a NUL byte",
        ));
    }
    if value.len() > MAX_VALUE_LEN {
        return Err(AppError::BadRequest("credential value is too long"));
    }
    Ok(value.to_string())
}

/// A user's credentials, resolved into what a backend needs to inject.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentEnv {
    /// Sorted by variable name, so the fingerprint of an unchanged set is
    /// stable no matter what order the rows came back in.
    pub pairs: Vec<(String, Secret)>,
    /// Identifies this exact set of values to a backend, so a credential change
    /// is detectable as drift. Empty when there are no credentials, which is
    /// what a container predating this feature reports, so those are not
    /// recreated on upgrade.
    pub fingerprint: String,
}

impl AgentEnv {
    fn new(pairs: Vec<(String, Secret)>) -> Self {
        let fingerprint = fingerprint(&pairs);
        Self { pairs, fingerprint }
    }
}

/// Hex SHA-256 over the sorted `name=value` pairs, or empty for none.
///
/// Computed rather than stored as a revision counter on the workspace row: there
/// is then nothing to keep in step with the rows it describes, no increment to
/// make transactional, and no lost update when two writes race.
fn fingerprint(pairs: &[(String, Secret)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let mut hasher = Sha256::new();
    for (name, value) in pairs {
        hasher.update(name.as_bytes());
        hasher.update(b"=");
        hasher.update(value.expose().as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// Load and decrypt `user_id`'s credentials.
///
/// Called on the reconcile path only, never before the endpoint cache lookup:
/// the proxy calls `ensure_started` for every request and almost always returns
/// a cached address, so decrypting here would run a query and several AES-GCM
/// decryptions per proxied request for a value used only when a container is
/// actually created.
///
/// A row that will not decrypt is skipped rather than failing the workspace. A
/// `CITYHALL_SECRET_KEY` rotation would otherwise cost the user their whole
/// workspace, not just the credential; this matches how the bundle's git table
/// handles the same case.
pub async fn materialize(db: &DatabaseConnection, user_id: i32) -> Result<AgentEnv, AppError> {
    let rows = agent_credential::Entity::find()
        .filter(agent_credential::Column::UserId.eq(user_id))
        .order_by_asc(agent_credential::Column::EnvVar)
        .all(db)
        .await?;

    let mut pairs = Vec::with_capacity(rows.len());
    for row in rows {
        // A row for a variable dropped from the catalog since it was stored
        // must not keep being injected.
        if lookup(&row.env_var).is_none() {
            tracing::warn!(
                user_id,
                env_var = %row.env_var,
                "stored agent credential is not in the catalog, skipping it"
            );
            continue;
        }
        match crypto::decrypt(&row.value_encrypted) {
            Ok(value) => pairs.push((row.env_var, Secret::new(value))),
            Err(e) => tracing::warn!(
                user_id,
                env_var = %row.env_var,
                "agent credential could not be decrypted, starting the workspace without it: {e}"
            ),
        }
    }
    Ok(AgentEnv::new(pairs))
}

/// Whether `user_id`'s stored value for `env_var` decrypts.
///
/// Reported to the owner so the UI cannot claim a credential is configured while
/// the workspace silently drops it.
pub fn usable(value_encrypted: &str) -> bool {
    crypto::decrypt(value_encrypted).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_closed() {
        assert!(lookup("ANTHROPIC_API_KEY").is_some());
        // The variables that would let a credential form reconfigure the
        // workspace, or steal its bundle.
        assert!(lookup("PATH").is_none());
        assert!(lookup("LD_PRELOAD").is_none());
        assert!(lookup("HOME").is_none());
        assert!(lookup("AOE_CITYHALL_BUNDLE_TOKEN").is_none());
        // Case-sensitive, so a lowercase spelling is not a way in.
        assert!(lookup("anthropic_api_key").is_none());
    }

    #[test]
    fn catalog_has_no_duplicates() {
        let mut names: Vec<&str> = CATALOG.iter().map(|k| k.env_var).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn values_are_trimmed_and_bounded() {
        assert_eq!(validate_value("  sk-abc\n").unwrap(), "sk-abc");
        assert!(validate_value("   ").is_err());
        assert!(validate_value("sk-\0abc").is_err());
        assert!(validate_value(&"x".repeat(MAX_VALUE_LEN + 1)).is_err());
    }

    fn env(pairs: &[(&str, &str)]) -> AgentEnv {
        AgentEnv::new(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), Secret::new(v.to_string())))
                .collect(),
        )
    }

    #[test]
    fn no_credentials_fingerprints_empty() {
        // A container created before this feature carries no fingerprint at
        // all, which reads back as empty. If this were a hash of nothing,
        // every such container would be recreated once on upgrade.
        assert_eq!(env(&[]).fingerprint, "");
    }

    #[test]
    fn fingerprint_tracks_the_values() {
        let a = env(&[("ANTHROPIC_API_KEY", "one")]);
        assert_ne!(a.fingerprint, "");
        assert_eq!(
            a.fingerprint,
            env(&[("ANTHROPIC_API_KEY", "one")]).fingerprint
        );
        // A changed value, a renamed variable, and an added variable all differ.
        assert_ne!(
            a.fingerprint,
            env(&[("ANTHROPIC_API_KEY", "two")]).fingerprint
        );
        assert_ne!(a.fingerprint, env(&[("OPENAI_API_KEY", "one")]).fingerprint);
        assert_ne!(
            a.fingerprint,
            env(&[("ANTHROPIC_API_KEY", "one"), ("OPENAI_API_KEY", "two")]).fingerprint
        );
    }

    #[test]
    fn fingerprint_does_not_confuse_name_and_value_boundaries() {
        // Without a separator between pairs, ("AB", "C") and ("A", "BC") would
        // hash the same bytes.
        assert_ne!(
            fingerprint(&[("AB".to_string(), Secret::new("C".to_string()))]),
            fingerprint(&[("A".to_string(), Secret::new("BC".to_string()))])
        );
    }
}
