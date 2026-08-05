//! Persisted state for the admin setup checklist/wizard.
//!
//! A single row (like `workspace_settings`) records which checklist steps an
//! admin has dismissed as handled or not needed, and whether the wizard itself
//! has been finished, so the checklist does not keep reappearing once it has
//! been dealt with.

use axum::extract::State;
use axum::Json;
use sea_orm::sea_query::OnConflict;
use sea_orm::{DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::entities::setup_state;
use crate::error::AppError;

const STATE_ID: i32 = 1;

/// Every step key the checklist can show, in the order a stored set is
/// canonicalized into. Closed rather than free-form, the same reasoning as
/// `crate::agents::CATALOG`: the value only ever needs to round-trip through
/// this UI, and a typo left as-is would silently create a step nothing ever
/// un-dismisses.
const STEPS: &[&str] = &[
    "password", "version", "agents", "projects", "email", "sso", "invites",
];

/// Steps CityHall cannot run without, so they are never dismissable: no
/// workspace starts without a default version, and the seeded admin password was
/// written to the server log in plain text. Dismissing one would report a
/// deployment as set up while it cannot serve anybody.
const REQUIRED_STEPS: &[&str] = &["password", "version"];

fn known(step: &str) -> bool {
    STEPS.contains(&step)
}

/// Canonicalize a submitted set of dismissed steps into the comma-joined form
/// that is stored, deduplicated and sorted into catalog order so saving the
/// same set again (in another order, or with repeats) does not read as a
/// change.
fn canonicalize(requested: &[String]) -> Result<String, AppError> {
    if let Some(unknown) = requested.iter().find(|s| !known(s)) {
        return Err(AppError::BadRequestOwned(format!(
            "unknown setup step '{unknown}'"
        )));
    }
    if let Some(required) = requested
        .iter()
        .find(|s| REQUIRED_STEPS.contains(&s.as_str()))
    {
        return Err(AppError::BadRequestOwned(format!(
            "setup step '{required}' is required and cannot be dismissed"
        )));
    }
    Ok(STEPS
        .iter()
        .filter(|s| requested.iter().any(|r| r == *s))
        .copied()
        .collect::<Vec<_>>()
        .join(","))
}

/// The stored value read back as a list. A key no longer in the catalog is
/// dropped rather than passed on, so a row written by a newer build does not
/// hand this one back a step it has never heard of.
fn parse(stored: &str) -> Vec<String> {
    stored
        .split(',')
        .filter(|s| !s.is_empty() && known(s))
        .map(String::from)
        .collect()
}

#[derive(Serialize, Deserialize)]
pub struct SetupStateResponse {
    pub dismissed_steps: Vec<String>,
    pub wizard_finished: bool,
}

/// GET /api/settings/setup
pub async fn get(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
) -> Result<Json<SetupStateResponse>, AppError> {
    caller.require("settings.read")?;
    let row = setup_state::Entity::find_by_id(STATE_ID).one(&db).await?;
    Ok(Json(match row {
        Some(r) => SetupStateResponse {
            dismissed_steps: parse(&r.dismissed_steps),
            wizard_finished: r.wizard_finished,
        },
        // No row yet: nothing dismissed, wizard not finished.
        None => SetupStateResponse {
            dismissed_steps: Vec::new(),
            wizard_finished: false,
        },
    }))
}

/// PUT /api/settings/setup
pub async fn update(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Json(body): Json<SetupStateResponse>,
) -> Result<Json<SetupStateResponse>, AppError> {
    caller.require("settings.write")?;
    let dismissed_steps = canonicalize(&body.dismissed_steps)?;

    // One statement rather than find-then-insert-or-update: two concurrent
    // saves could otherwise both see no row and race to insert, and the loser
    // fails on the primary key.
    setup_state::Entity::insert(setup_state::ActiveModel {
        id: Set(STATE_ID),
        dismissed_steps: Set(dismissed_steps.clone()),
        wizard_finished: Set(body.wizard_finished),
    })
    .on_conflict(
        OnConflict::column(setup_state::Column::Id)
            .update_columns([
                setup_state::Column::DismissedSteps,
                setup_state::Column::WizardFinished,
            ])
            .to_owned(),
    )
    .exec(&db)
    .await?;

    Ok(Json(SetupStateResponse {
        dismissed_steps: parse(&dismissed_steps),
        wizard_finished: body.wizard_finished,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_closed() {
        assert!(canonicalize(&["email".to_string()]).is_ok());
        assert!(canonicalize(&["Email".to_string()]).is_err());
        assert!(canonicalize(&["".to_string()]).is_err());
        assert!(canonicalize(&["bogus".to_string()]).is_err());
    }

    /// Dismissing one of these would report a deployment as set up while no
    /// workspace can start, so the API refuses rather than trusting the caller.
    #[test]
    fn required_steps_cannot_be_dismissed() {
        for required in REQUIRED_STEPS {
            assert!(canonicalize(&[required.to_string()]).is_err());
            assert!(canonicalize(&["email".to_string(), required.to_string()]).is_err());
        }
    }

    /// Order and repetition in the request must not change the stored value.
    #[test]
    fn canonical_form_is_order_and_duplicate_insensitive() {
        let expected = "agents,email,sso";
        for submitted in [
            vec!["agents", "email", "sso"],
            vec!["sso", "agents", "email"],
            vec!["email", "email", "sso", "agents"],
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
    fn parse_drops_steps_no_longer_in_the_catalog() {
        assert_eq!(parse("email,retired-step,sso"), vec!["email", "sso"]);
    }

    #[test]
    fn parse_round_trips_a_canonical_value() {
        let stored = canonicalize(&["sso".to_string(), "email".to_string()]).unwrap();
        assert_eq!(parse(&stored), vec!["email", "sso"]);
    }
}
