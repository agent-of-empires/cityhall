use sea_orm::entity::prelude::*;

/// A user's SSH key for git, forwarded to their workspace so `git@host:...`
/// remotes work there. The key is stored encrypted; see `crate::crypto`.
///
/// `known_hosts` is stored as the user pasted it, in plaintext: a host's public
/// key is public. It is required rather than optional, because a key shipped
/// without one would leave the workspace trusting whatever answered.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "git_ssh_keys")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: i32,
    pub key_encrypted: String,
    pub known_hosts: String,
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
