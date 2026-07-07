use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait};

use crate::auth::random_token;
use crate::entities::user;
use crate::error::AppError;
use crate::service;

/// On an empty users table, create the default `admin` account with a random
/// password that must be changed on first login. The password is logged once
/// (there is no other way to recover it).
pub async fn ensure_admin(db: &DatabaseConnection) -> Result<(), AppError> {
    if user::Entity::find().count(db).await? > 0 {
        return Ok(());
    }
    let password = random_token(24);
    service::create(db, "admin", None, &password, true).await?;
    tracing::warn!(
        "seeded initial admin user | username: admin | password: {password} | change it on first login"
    );
    Ok(())
}
