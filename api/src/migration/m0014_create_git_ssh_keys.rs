use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One optional SSH key per user, for the workspace remotes an HTTPS
        // token cannot reach (#52). The key is encrypted with
        // CITYHALL_SECRET_KEY, like the git token beside it.
        //
        // Its own table rather than two more columns on `git_credentials`:
        // that row's `token_encrypted` is NOT NULL, so a user with a key and no
        // token could not have one at all, and relaxing the constraint is a
        // table rebuild on SQLite. Separate rows also mean removing the HTTPS
        // token leaves the key in place, which is what a user switching from
        // one to the other expects.
        //
        // `known_hosts` is deliberately plaintext. A host's public key is
        // public, and encrypting it would buy nothing while making the value a
        // fifth thing key rotation has to rewrite.
        manager
            .create_table(
                Table::create()
                    .table(GitSshKeys::Table)
                    .if_not_exists()
                    .col(integer(GitSshKeys::UserId).primary_key())
                    .col(text(GitSshKeys::KeyEncrypted))
                    .col(text(GitSshKeys::KnownHosts))
                    .col(timestamp_with_time_zone(GitSshKeys::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(GitSshKeys::Table, GitSshKeys::UserId)
                            .to(Users::Table, Users::Id)
                            // Deleting a user must revoke the key they stored,
                            // not orphan it.
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(GitSshKeys::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum GitSshKeys {
    Table,
    UserId,
    KeyEncrypted,
    KnownHosts,
    UpdatedAt,
}
