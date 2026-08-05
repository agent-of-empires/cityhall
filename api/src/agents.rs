//! The coding agents an admin can have a workspace arrive with (#57).
//!
//! An operator picks a set here, CityHall hands the names to the workspace, and
//! the reference image's entrypoint installs any that are missing. The catalog
//! is closed for the same reason the credential catalog is: the value ends up in
//! a workspace's environment, and the entrypoint turns each name into an install
//! command, so a free-form list would be a way to run something of the
//! operator's choosing inside every workspace. Names are all this side sends;
//! the name-to-command map lives in the entrypoint, where an unrecognised name
//! matches no branch and is skipped.
//!
//! **The installed set is a default, not a restriction.** aoe applies no
//! allowlist to agent selection, and a terminal session can run any binary on
//! `PATH` regardless of what was installed here. Enforcement needs
//! agent-of-empires/agent-of-empires#3241.

use crate::error::AppError;

/// One agent an operator can ask for.
pub struct WorkspaceAgent {
    /// aoe's own name for the agent, which is also what the entrypoint's
    /// install map is keyed on.
    pub name: &'static str,
    pub label: &'static str,
}

/// Every agent an operator may select, and nothing else. Ordered the way the
/// settings form should list them, which is also the canonical order a stored
/// set is sorted into.
///
/// These four are exactly the agents `docs/workspaces.md` documents a user
/// installing by hand, and the four whose state directories the reference
/// image's entrypoint already keeps on the volume. An agent whose install is
/// manual upstream (`vibe`, `pi`, `omp`, `kimi`) has nothing to automate here.
pub const CATALOG: &[WorkspaceAgent] = &[
    WorkspaceAgent {
        name: "claude",
        label: "Claude",
    },
    WorkspaceAgent {
        name: "codex",
        label: "Codex",
    },
    WorkspaceAgent {
        name: "gemini",
        label: "Gemini",
    },
    WorkspaceAgent {
        name: "opencode",
        label: "OpenCode",
    },
];

/// The environment variable a workspace receives the set through. Read by the
/// reference image's entrypoint, not by aoe.
pub const ENV_VAR: &str = "CITYHALL_AGENTS";

fn known(name: &str) -> bool {
    CATALOG.iter().any(|a| a.name == name)
}

/// Canonicalize a submitted set into the single string that is stored,
/// injected, and recorded as a container label.
///
/// One canonical form matters because that string is compared against a running
/// workspace's label to decide whether it is out of date. Ticking two boxes in
/// the other order, or twice, must not read as a change, so the set is
/// deduplicated and sorted into catalog order rather than kept as submitted.
pub fn canonicalize(requested: &[String]) -> Result<String, AppError> {
    if let Some(unknown) = requested.iter().find(|n| !known(n)) {
        tracing::warn!(agent = %unknown, "rejected an unknown coding agent");
        return Err(AppError::BadRequest("unknown coding agent"));
    }
    Ok(CATALOG
        .iter()
        .filter(|a| requested.iter().any(|n| n == a.name))
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(","))
}

/// The stored value read back as a list.
///
/// A name that is no longer in the catalog is dropped rather than passed on, the
/// same way a stored credential for a retired variable is: the workspace would
/// otherwise keep being told to install something nothing knows how to install.
pub fn parse(stored: &str) -> Vec<String> {
    stored
        .split(',')
        .filter(|n| !n.is_empty())
        .filter(|n| {
            let ok = known(n);
            if !ok {
                tracing::warn!(agent = %n, "stored coding agent is not in the catalog, skipping it");
            }
            ok
        })
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_closed() {
        assert!(canonicalize(&["claude".to_string()]).is_ok());
        assert!(canonicalize(&["Claude".to_string()]).is_err());
        assert!(canonicalize(&["".to_string()]).is_err());
        // A name that would smuggle a second entry past the entrypoint's split,
        // or a shell fragment, is not a name in the catalog.
        assert!(canonicalize(&["claude,codex".to_string()]).is_err());
        assert!(canonicalize(&["claude; rm -rf /".to_string()]).is_err());
    }

    /// The stored value is comma-joined and split on commas at both ends, so a
    /// name containing one would silently become two entries.
    #[test]
    fn catalog_names_survive_the_encoding() {
        assert!(CATALOG
            .iter()
            .all(|a| !a.name.contains(',') && !a.name.trim().is_empty()));
        assert!(CATALOG.iter().all(|a| a.name.trim() == a.name));
    }

    #[test]
    fn catalog_has_no_duplicates() {
        let mut names: Vec<&str> = CATALOG.iter().map(|a| a.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    /// Order and repetition in the request must not change the stored value, or
    /// re-saving the same set would read as drift and recreate every workspace.
    #[test]
    fn canonical_form_is_order_and_duplicate_insensitive() {
        let expected = "claude,codex,gemini";
        for submitted in [
            vec!["claude", "codex", "gemini"],
            vec!["gemini", "claude", "codex"],
            vec!["codex", "codex", "gemini", "claude"],
        ] {
            let submitted: Vec<String> = submitted.into_iter().map(String::from).collect();
            assert_eq!(canonicalize(&submitted).unwrap(), expected);
        }
    }

    #[test]
    fn an_empty_set_round_trips_as_nothing() {
        assert_eq!(canonicalize(&[]).unwrap(), "");
        assert!(parse("").is_empty());
    }

    #[test]
    fn parse_drops_names_no_longer_in_the_catalog() {
        assert_eq!(parse("claude,retired-agent,codex"), vec!["claude", "codex"]);
    }

    #[test]
    fn parse_round_trips_a_canonical_value() {
        let stored = canonicalize(&["opencode".to_string(), "claude".to_string()]).unwrap();
        assert_eq!(parse(&stored), vec!["claude", "opencode"]);
    }
}
