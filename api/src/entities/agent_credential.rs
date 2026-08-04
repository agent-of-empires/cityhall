use sea_orm::entity::prelude::*;

/// One coding-agent credential a user's workspace is started with, injected as
/// an environment variable. The value is stored encrypted; see `crate::crypto`.
///
/// A row per variable rather than one blob per user, so a save or a delete
/// touches only its own credential and two concurrent writes to different
/// variables cannot lose each other.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_credentials")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: i32,
    /// Name of the environment variable, always one of
    /// `crate::agent_credentials::CATALOG`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub env_var: String,
    pub value_encrypted: String,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id",
        on_delete = "Cascade"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
