use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0011_create_workspaces::WorkspaceSettings;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Empty by default, which is the behaviour every existing install
        // already has: the workspace ships no coding agent and the user installs
        // the one they want. Only an operator who fills this in changes anything.
        manager
            .alter_table(
                Table::alter()
                    .table(WorkspaceSettings::Table)
                    .add_column(string(WorkspaceSettings::Agents).default(""))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(WorkspaceSettings::Table)
                    .drop_column(WorkspaceSettings::Agents)
                    .to_owned(),
            )
            .await
    }
}
