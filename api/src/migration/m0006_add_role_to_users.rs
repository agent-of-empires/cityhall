use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nullable role reference. No DB-level FK: SQLite cannot add one via
        // ALTER, so referential integrity is enforced in app code (a role in use
        // cannot be deleted). A null role means "no permissions". Startup seeding
        // backfills existing users to the admin role (pre-RBAC every logged-in
        // user could do everything) and assigns roles to new users.
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(integer_null(Users::RoleId))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::RoleId)
                    .to_owned(),
            )
            .await
    }
}
