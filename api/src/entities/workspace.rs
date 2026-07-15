use sea_orm::entity::prelude::*;

/// A user's workspace: intent only (pinned version, activity checkpoint).
/// Runtime state (running/stopped, address) always comes from the
/// orchestrator; the row exists as soon as the user first uses a workspace.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "workspaces")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: i32,
    /// Pinned aoe version; `None` follows the default from workspace settings.
    pub pinned_version: Option<String>,
    /// Coarse activity checkpoint (the live value is in-memory).
    pub last_active_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
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
