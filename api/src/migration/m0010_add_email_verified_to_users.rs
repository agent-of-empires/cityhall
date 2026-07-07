use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Default true so existing accounts (and admin-created or SSO-provisioned
        // ones) are unaffected. Only self-signups start unverified and cannot log
        // in until they confirm their email.
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(boolean(Users::EmailVerified).default(true))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::EmailVerified)
                    .to_owned(),
            )
            .await
    }
}
