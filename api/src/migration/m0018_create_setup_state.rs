use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single-row table (id is always 1), like `workspace_settings`: the
        // admin setup checklist's persisted state. `dismissed_steps` is
        // comma-joined rather than a second table because the catalog of steps
        // is closed and small, the same reasoning as `workspace_settings.agents`.
        manager
            .create_table(
                Table::create()
                    .table(SetupState::Table)
                    .if_not_exists()
                    .col(integer(SetupState::Id).primary_key())
                    .col(text(SetupState::DismissedSteps).default(""))
                    .col(boolean(SetupState::WizardFinished).default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SetupState::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum SetupState {
    Table,
    Id,
    DismissedSteps,
    WizardFinished,
}
