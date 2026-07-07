use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Links a local account to an OIDC identity (the IdP's `sub` claim). No
        // DB-level unique constraint: SQLite cannot add one via ALTER, so
        // uniqueness is enforced in app code (look up by subject before linking
        // or provisioning).
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(string_null(Users::OidcSubject))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::OidcSubject)
                    .to_owned(),
            )
            .await
    }
}
