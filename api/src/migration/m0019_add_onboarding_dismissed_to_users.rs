use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Per-user first-run flag: whether this user has dismissed the
        // onboarding UI. Defaults false so every existing account is shown it
        // once, the same as a brand new one.
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(boolean(Users::OnboardingDismissed).default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::OnboardingDismissed)
                    .to_owned(),
            )
            .await
    }
}
