use sea_orm_migration::prelude::*;

mod m0001_create_users;
mod m0002_create_sessions;
mod m0003_create_smtp_settings;
mod m0004_create_password_reset_tokens;
mod m0005_create_roles;
mod m0006_add_role_to_users;
mod m0007_create_oidc_settings;
mod m0008_add_oidc_subject_to_users;
mod m0009_create_auth_settings;
mod m0010_add_email_verified_to_users;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_create_users::Migration),
            Box::new(m0002_create_sessions::Migration),
            Box::new(m0003_create_smtp_settings::Migration),
            Box::new(m0004_create_password_reset_tokens::Migration),
            Box::new(m0005_create_roles::Migration),
            Box::new(m0006_add_role_to_users::Migration),
            Box::new(m0007_create_oidc_settings::Migration),
            Box::new(m0008_add_oidc_subject_to_users::Migration),
            Box::new(m0009_create_auth_settings::Migration),
            Box::new(m0010_add_email_verified_to_users::Migration),
        ]
    }
}
