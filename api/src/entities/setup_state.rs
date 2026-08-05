use sea_orm::entity::prelude::*;

/// Single-row admin setup checklist/wizard state (the row always has `id = 1`).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "setup_state")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    /// Comma-joined step keys the admin marked as handled or not needed, in
    /// `crate::handlers::setup::STEPS` order.
    pub dismissed_steps: String,
    pub wizard_finished: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
