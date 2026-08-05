use sea_orm::entity::prelude::*;

/// Single-row workspace orchestration configuration (the row always has
/// `id = 1`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "workspace_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    /// Image reference with a `{version}` placeholder, e.g.
    /// `cityhall/aoe:{version}`.
    pub image_template: String,
    /// Version served to users without a pin; workspaces cannot start while
    /// unset.
    pub default_version: Option<String>,
    pub idle_stop_minutes: i32,
    /// Deployment-wide aoe telemetry policy, as one of
    /// [`crate::orchestrator::TelemetryPolicy`]'s wire values. Kept as a string
    /// here so a value written by a newer CityHall round-trips through an older
    /// one instead of failing the whole row's deserialization.
    pub telemetry_policy: String,
    pub updated_at: DateTimeUtc,
    /// Coding agents a workspace should arrive with, as the comma-joined
    /// canonical form produced by `crate::agents::canonicalize`. Empty means
    /// users install their own, which is the behaviour without this setting.
    pub agents: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
