use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single-row table (id is always 1) holding the OIDC configuration set
        // via the settings page. `OIDC_*` environment variables, when present,
        // take precedence over this row at runtime (mirrors smtp_settings). The
        // client secret is stored encrypted; see `crate::crypto`.
        manager
            .create_table(
                Table::create()
                    .table(OidcSettings::Table)
                    .if_not_exists()
                    .col(integer(OidcSettings::Id).primary_key())
                    .col(boolean(OidcSettings::Enabled))
                    .col(string(OidcSettings::Issuer))
                    .col(string(OidcSettings::ClientId))
                    .col(text_null(OidcSettings::ClientSecretEncrypted))
                    .col(string(OidcSettings::Scopes))
                    .col(string_null(OidcSettings::AllowedDomains))
                    .col(timestamp_with_time_zone(OidcSettings::UpdatedAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(OidcSettings::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum OidcSettings {
    Table,
    Id,
    Enabled,
    Issuer,
    ClientId,
    ClientSecretEncrypted,
    Scopes,
    AllowedDomains,
    UpdatedAt,
}
