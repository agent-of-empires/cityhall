use sea_orm::entity::prelude::*;

/// Single-row SMTP configuration (the row always has `id = 1`). The password is
/// stored encrypted; see `crate::crypto`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "smtp_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub host: String,
    pub port: i32,
    pub encryption: String,
    pub username: Option<String>,
    pub password_encrypted: Option<String>,
    pub from_address: String,
    pub from_name: Option<String>,
    pub enabled: bool,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
