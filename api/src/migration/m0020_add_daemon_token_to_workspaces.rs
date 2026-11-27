use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0011_create_workspaces::Workspaces;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nullable: existing rows are backfilled lazily on their next start, by
        // the same code path that mints the token for a new row.
        manager
            .alter_table(
                Table::alter()
                    .table(Workspaces::Table)
                    .add_column(string_null(Workspaces::DaemonToken))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Workspaces::Table)
                    .drop_column(Workspaces::DaemonToken)
                    .to_owned(),
            )
            .await
    }
}
