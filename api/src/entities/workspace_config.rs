use sea_orm::entity::prelude::*;

/// Single-row aoe config bundle served to every workspace (the row always has
/// `id = 1`).
///
/// `bundle` is opaque TOML: aoe defines the settings schema, so aoe owns the
/// format and both directions of translation. CityHall stores it, composes the
/// per-user `[git]` section onto it when serving, and never parses the settings
/// keys itself.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "workspace_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub bundle: String,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
