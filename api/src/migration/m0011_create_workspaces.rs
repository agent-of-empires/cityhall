use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One row per user workspace. Only intent lives here (pinned version,
        // activity checkpoint); runtime state is read from the orchestrator.
        manager
            .create_table(
                Table::create()
                    .table(Workspaces::Table)
                    .if_not_exists()
                    .col(integer(Workspaces::UserId).primary_key())
                    .col(string_null(Workspaces::PinnedVersion))
                    .col(timestamp_with_time_zone_null(Workspaces::LastActiveAt))
                    .col(timestamp_with_time_zone(Workspaces::CreatedAt))
                    .col(timestamp_with_time_zone(Workspaces::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Workspaces::Table, Workspaces::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Single-row table (id is always 1) holding orchestration settings.
        // Workspaces are off by default; an admin enables them in settings.
        manager
            .create_table(
                Table::create()
                    .table(WorkspaceSettings::Table)
                    .if_not_exists()
                    .col(integer(WorkspaceSettings::Id).primary_key())
                    .col(boolean(WorkspaceSettings::Enabled).default(false))
                    .col(string(WorkspaceSettings::ImageTemplate).default("cityhall/aoe:{version}"))
                    .col(string_null(WorkspaceSettings::DefaultVersion))
                    .col(integer(WorkspaceSettings::IdleStopMinutes).default(30))
                    .col(timestamp_with_time_zone(WorkspaceSettings::UpdatedAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(WorkspaceSettings::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Workspaces::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Workspaces {
    Table,
    UserId,
    PinnedVersion,
    LastActiveAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum WorkspaceSettings {
    Table,
    Id,
    Enabled,
    ImageTemplate,
    DefaultVersion,
    IdleStopMinutes,
    UpdatedAt,
}
