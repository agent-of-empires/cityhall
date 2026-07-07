use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A role is a named bundle of permission keys. `permissions` is a JSON
        // array of strings stored as text so it stays backend-agnostic; the key
        // catalog itself lives in code (see `crate::rbac`). `is_system` marks the
        // built-in roles, which are protected from deletion.
        manager
            .create_table(
                Table::create()
                    .table(Roles::Table)
                    .if_not_exists()
                    .col(pk_auto(Roles::Id))
                    .col(string(Roles::Name).unique_key())
                    .col(string_null(Roles::Description))
                    .col(text(Roles::Permissions))
                    .col(boolean(Roles::IsSystem).default(false))
                    .col(timestamp_with_time_zone(Roles::CreatedAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Roles::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Roles {
    Table,
    Id,
    Name,
    Description,
    Permissions,
    IsSystem,
    CreatedAt,
}
