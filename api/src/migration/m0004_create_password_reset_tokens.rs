use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single-use, time-limited tokens for password reset and account setup.
        // `used_at` is set when redeemed so a token cannot be replayed.
        manager
            .create_table(
                Table::create()
                    .table(PasswordResetTokens::Table)
                    .if_not_exists()
                    .col(string(PasswordResetTokens::Token).primary_key())
                    .col(integer(PasswordResetTokens::UserId))
                    .col(timestamp_with_time_zone(PasswordResetTokens::ExpiresAt))
                    .col(timestamp_with_time_zone(PasswordResetTokens::CreatedAt))
                    .col(timestamp_with_time_zone_null(PasswordResetTokens::UsedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_password_reset_tokens_user")
                            .from(PasswordResetTokens::Table, PasswordResetTokens::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PasswordResetTokens::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum PasswordResetTokens {
    Table,
    Token,
    UserId,
    ExpiresAt,
    CreatedAt,
    UsedAt,
}
