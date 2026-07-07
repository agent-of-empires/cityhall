use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single-row table (id is always 1) holding the SMTP configuration set
        // via the settings page. Environment variables, when present, take
        // precedence over this row at runtime.
        manager
            .create_table(
                Table::create()
                    .table(SmtpSettings::Table)
                    .if_not_exists()
                    .col(integer(SmtpSettings::Id).primary_key())
                    .col(string(SmtpSettings::Host))
                    .col(integer(SmtpSettings::Port))
                    .col(string(SmtpSettings::Encryption))
                    .col(string_null(SmtpSettings::Username))
                    .col(text_null(SmtpSettings::PasswordEncrypted))
                    .col(string(SmtpSettings::FromAddress))
                    .col(string_null(SmtpSettings::FromName))
                    .col(boolean(SmtpSettings::Enabled))
                    .col(timestamp_with_time_zone(SmtpSettings::UpdatedAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SmtpSettings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum SmtpSettings {
    Table,
    Id,
    Host,
    Port,
    Encryption,
    Username,
    PasswordEncrypted,
    FromAddress,
    FromName,
    Enabled,
    UpdatedAt,
}
