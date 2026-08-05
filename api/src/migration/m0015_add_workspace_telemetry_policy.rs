use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0011_create_workspaces::WorkspaceSettings;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Deployment-wide aoe telemetry policy: `user_choice`, `force_on`, or
        // `force_off` (#40). Text rather than an enum type: SQLite has none, and
        // the values are validated at the Rust boundary anyway.
        //
        // `user_choice` is the default so an upgrade changes nothing. A stored
        // value is what an admin intended; a value the running CityHall does not
        // recognize is read as `user_choice`, which is the state that touches
        // nothing rather than one that forces a decision on every user.
        manager
            .alter_table(
                Table::alter()
                    .table(WorkspaceSettings::Table)
                    .add_column(string(WorkspaceSettings::TelemetryPolicy).default("user_choice"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(WorkspaceSettings::Table)
                    .drop_column(WorkspaceSettings::TelemetryPolicy)
                    .to_owned(),
            )
            .await
    }
}
