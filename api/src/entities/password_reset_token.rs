use sea_orm::entity::prelude::*;

/// Single-use, time-limited token for password reset and account setup.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "password_reset_tokens")]
pub struct Model {
    // Opaque random token, sent in the reset link.
    #[sea_orm(primary_key, auto_increment = false)]
    pub token: String,
    pub user_id: i32,
    pub expires_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
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
