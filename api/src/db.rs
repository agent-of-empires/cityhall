use sea_orm::{Database, DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;

use crate::migration::Migrator;

/// Default SQLite database, created in the working directory. `mode=rwc`
/// opens read-write and creates the file if it is missing.
const DEFAULT_DATABASE_URL: &str = "sqlite://cityhall.db?mode=rwc";

/// Connect using `DATABASE_URL` (Postgres/MySQL/SQLite), falling back to a
/// local SQLite file, then run pending migrations.
pub async fn connect() -> Result<DatabaseConnection, DbErr> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    let db = Database::connect(&url).await?;
    Migrator::up(&db, None).await?;
    Ok(db)
}
