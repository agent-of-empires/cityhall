use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m0001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One dashboard layout per user, so a customized dashboard follows the
        // admin to another browser instead of living in that browser's
        // localStorage (#12).
        //
        // The layout is stored as JSON text rather than as columns: it is a
        // list of widget rectangles whose shape belongs to the frontend
        // catalog, and modelling grid coordinates relationally would buy
        // nothing while making every new widget a migration. The handler still
        // validates it against a typed schema before it lands here, so the
        // column holds a known shape and not arbitrary caller bytes.
        //
        // Its own table rather than a general `user_preferences(namespace,
        // value)` one: there is no second preference today, and a generic
        // table needs a server-side namespace allowlist to stop a caller
        // writing unbounded rows, which is this table with more indirection.
        manager
            .create_table(
                Table::create()
                    .table(DashboardLayouts::Table)
                    .if_not_exists()
                    .col(integer(DashboardLayouts::UserId).primary_key())
                    .col(text(DashboardLayouts::Layout))
                    .col(timestamp_with_time_zone(DashboardLayouts::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(DashboardLayouts::Table, DashboardLayouts::UserId)
                            .to(Users::Table, Users::Id)
                            // A deleted user's layout is meaningless.
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DashboardLayouts::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum DashboardLayouts {
    Table,
    UserId,
    Layout,
    UpdatedAt,
}
