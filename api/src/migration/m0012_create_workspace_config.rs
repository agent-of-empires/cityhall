use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;
use super::m0011_create_workspaces::Workspaces;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single-row table (id is always 1) holding the aoe config bundle every
        // workspace is provisioned with: settings plus the project list. Stored
        // as opaque TOML because aoe owns the format; CityHall stores, serves,
        // and edits it without reimplementing aoe's settings schema.
        manager
            .create_table(
                Table::create()
                    .table(WorkspaceConfig::Table)
                    .if_not_exists()
                    .col(integer(WorkspaceConfig::Id).primary_key())
                    .col(text(WorkspaceConfig::Bundle).default(""))
                    .col(timestamp_with_time_zone(WorkspaceConfig::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        // One optional git credential per user, so commits and pushes from a
        // workspace carry that person's identity rather than a shared robot's.
        // The token is encrypted with CITYHALL_SECRET_KEY, like the SMTP
        // password; the column never holds plaintext.
        manager
            .create_table(
                Table::create()
                    .table(GitCredentials::Table)
                    .if_not_exists()
                    .col(integer(GitCredentials::UserId).primary_key())
                    .col(string(GitCredentials::Host))
                    .col(string(GitCredentials::Username))
                    .col(text(GitCredentials::TokenEncrypted))
                    .col(timestamp_with_time_zone(GitCredentials::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(GitCredentials::Table, GitCredentials::UserId)
                            .to(Users::Table, Users::Id)
                            // Deleting a user must revoke their git access, not
                            // orphan their credential.
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Bearer token a workspace presents to fetch its own bundle. Per
        // workspace so a leaked token exposes exactly one user's document, and
        // nullable so rows created before this migration get one lazily on
        // their next start.
        manager
            .alter_table(
                Table::alter()
                    .table(Workspaces::Table)
                    .add_column(string_null(Workspaces::BundleToken))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Workspaces::Table)
                    .drop_column(Workspaces::BundleToken)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(GitCredentials::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(WorkspaceConfig::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum WorkspaceConfig {
    Table,
    Id,
    Bundle,
    UpdatedAt,
}

#[derive(DeriveIden)]
pub enum GitCredentials {
    Table,
    UserId,
    Host,
    Username,
    TokenEncrypted,
    UpdatedAt,
}
