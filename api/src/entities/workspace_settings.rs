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
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
