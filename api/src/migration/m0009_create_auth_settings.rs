use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single-row table (id is always 1) holding self-signup settings.
        // Signup is off by default; an admin enables it in the settings page.
        manager
            .create_table(
                Table::create()
                    .table(AuthSettings::Table)
                    .if_not_exists()
                    .col(integer(AuthSettings::Id).primary_key())
                    .col(boolean(AuthSettings::SignupEnabled).default(false))
                    .col(string_null(AuthSettings::SignupAllowedDomains))
                    .col(integer_null(AuthSettings::SignupDefaultRoleId))
                    .col(timestamp_with_time_zone(AuthSettings::UpdatedAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AuthSettings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum AuthSettings {
    Table,
    Id,
    SignupEnabled,
    SignupAllowedDomains,
    SignupDefaultRoleId,
    UpdatedAt,
}
