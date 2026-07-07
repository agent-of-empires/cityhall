use sea_orm::entity::prelude::*;

/// Single-row self-signup configuration (the row always has `id = 1`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub signup_enabled: bool,
    pub signup_allowed_domains: Option<String>,
    pub signup_default_role_id: Option<i32>,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
