//! Role-based access control: the permission-key catalog and the `Perms` set
//! resolved from a user's role.
//!
//! Permission keys are compile-time constants, not database rows: a new feature
//! adds a key here and gates its handlers on it. Roles (in the `roles` table)
//! hold a JSON array of these keys. The wildcard `"*"` grants everything.

use std::collections::HashSet;

use crate::entities::role;
use crate::error::AppError;

/// Wildcard permission: a role holding it is granted every permission.
pub const WILDCARD: &str = "*";

pub const ADMIN_ROLE: &str = "admin";
pub const MEMBER_ROLE: &str = "member";

/// The full catalog of permission keys with human-readable descriptions,
/// surfaced to the UI so it can render role editors. Add new keys here.
pub const CATALOG: &[(&str, &str)] = &[
    ("users.read", "View users"),
    ("users.write", "Create, edit, and delete users"),
    ("roles.read", "View roles"),
    ("roles.write", "Create, edit, and delete roles"),
    ("settings.read", "View settings"),
    ("settings.write", "Change settings"),
    ("workspaces.use", "Use your own workspace"),
    ("workspaces.read", "View all workspaces"),
    ("workspaces.write", "Manage workspaces and their versions"),
];

/// Built-in roles seeded on startup: (name, description, permission keys).
pub fn system_roles() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        (ADMIN_ROLE, "Full access to everything", vec![WILDCARD]),
        (
            MEMBER_ROLE,
            "Basic access",
            vec!["users.read", "workspaces.use"],
        ),
    ]
}

fn is_known(key: &str) -> bool {
    key == WILDCARD || CATALOG.iter().any(|(k, _)| *k == key)
}

/// Parse and validate a set of permission keys, rejecting anything not in the
/// catalog so a typo cannot silently create a dead permission.
pub fn validate_keys(keys: &[String]) -> Result<(), AppError> {
    if keys.iter().any(|k| !is_known(k)) {
        return Err(AppError::BadRequest("unknown permission key"));
    }
    Ok(())
}

/// Serialize permission keys for storage in the `roles.permissions` column.
pub fn encode(keys: &[String]) -> String {
    serde_json::to_string(keys).unwrap_or_else(|_| "[]".to_string())
}

fn decode(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// The permission set resolved from a user's role. A user with no role has no
/// permissions.
#[derive(Clone, Debug, Default)]
pub struct Perms(HashSet<String>);

impl Perms {
    pub fn from_role(role: Option<&role::Model>) -> Self {
        match role {
            Some(r) => Perms(decode(&r.permissions).into_iter().collect()),
            None => Perms::default(),
        }
    }

    /// Whether this set grants `key` (directly or via the wildcard).
    pub fn can(&self, key: &str) -> bool {
        self.0.contains(WILDCARD) || self.0.contains(key)
    }

    /// The concrete permission keys granted, expanding the wildcard to the full
    /// catalog so the frontend can gate UI with a simple membership check.
    pub fn effective_keys(&self) -> Vec<String> {
        if self.0.contains(WILDCARD) {
            return CATALOG.iter().map(|(k, _)| k.to_string()).collect();
        }
        let mut keys: Vec<String> = self.0.iter().cloned().collect();
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role_with(perms: &[&str]) -> role::Model {
        role::Model {
            id: 1,
            name: "r".into(),
            description: None,
            permissions: encode(&perms.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
            is_system: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn wildcard_grants_everything() {
        let p = Perms::from_role(Some(&role_with(&[WILDCARD])));
        assert!(p.can("users.write"));
        assert!(p.can("anything"));
        assert_eq!(p.effective_keys().len(), CATALOG.len());
    }

    #[test]
    fn explicit_keys_only() {
        let p = Perms::from_role(Some(&role_with(&["users.read"])));
        assert!(p.can("users.read"));
        assert!(!p.can("users.write"));
    }

    #[test]
    fn no_role_no_perms() {
        let p = Perms::from_role(None);
        assert!(!p.can("users.read"));
    }

    #[test]
    fn validate_rejects_unknown() {
        assert!(validate_keys(&["users.read".into()]).is_ok());
        assert!(validate_keys(&["bogus.key".into()]).is_err());
    }
}
