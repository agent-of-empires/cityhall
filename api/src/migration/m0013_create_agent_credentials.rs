use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Coding-agent credentials a user's workspace is started with, one row
        // per environment variable so a save or a delete touches only its own
        // credential. Values are encrypted with CITYHALL_SECRET_KEY like the git
        // token and the SMTP password; the column never holds plaintext.
        manager
            .create_table(
                Table::create()
                    .table(AgentCredentials::Table)
                    .if_not_exists()
                    .col(integer(AgentCredentials::UserId))
                    .col(string(AgentCredentials::EnvVar))
                    .col(text(AgentCredentials::ValueEncrypted))
                    .col(timestamp_with_time_zone(AgentCredentials::UpdatedAt))
                    .primary_key(
                        Index::create()
                            .col(AgentCredentials::UserId)
                            .col(AgentCredentials::EnvVar),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(AgentCredentials::Table, AgentCredentials::UserId)
                            .to(Users::Table, Users::Id)
                            // Deleting a user must revoke the provider keys they
                            // stored, not orphan them.
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AgentCredentials::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum AgentCredentials {
    Table,
    UserId,
    EnvVar,
    ValueEncrypted,
    UpdatedAt,
}
