use sea_orm_migration::prelude::*;

mod m0001_create_users;
mod m0002_create_sessions;
mod m0003_create_smtp_settings;
mod m0004_create_password_reset_tokens;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_create_users::Migration),
            Box::new(m0002_create_sessions::Migration),
            Box::new(m0003_create_smtp_settings::Migration),
            Box::new(m0004_create_password_reset_tokens::Migration),
        ]
    }
}
