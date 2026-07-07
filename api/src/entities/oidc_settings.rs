use sea_orm::entity::prelude::*;

/// Single-row OIDC configuration (the row always has `id = 1`). The client
/// secret is stored encrypted; see `crate::crypto`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oidc_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub enabled: bool,
    pub issuer: String,
    pub client_id: String,
    pub client_secret_encrypted: Option<String>,
    pub scopes: String,
    pub allowed_domains: Option<String>,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
