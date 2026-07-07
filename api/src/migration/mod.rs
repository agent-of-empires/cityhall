use sea_orm_migration::prelude::*;

mod m0001_create_users;
mod m0002_create_sessions;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_create_users::Migration),
            Box::new(m0002_create_sessions::Migration),
        ]
    }
}
