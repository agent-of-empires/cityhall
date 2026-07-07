use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};

use crate::auth::random_token;
use crate::entities::{role, user};
use crate::error::AppError;
use crate::rbac;
use crate::service;

/// Create the built-in roles if missing, then backfill any user without a role
/// to `admin`. Pre-RBAC every logged-in user could do everything, so existing
/// accounts keep that access. Safe to run on every startup.
pub async fn ensure_roles(db: &DatabaseConnection) -> Result<(), AppError> {
    for (name, description, keys) in rbac::system_roles() {
        if service::find_role_by_name(db, name).await?.is_none() {
            let keys: Vec<String> = keys.into_iter().map(String::from).collect();
            role::ActiveModel {
                name: Set(name.to_string()),
                description: Set(Some(description.to_string())),
                permissions: Set(rbac::encode(&keys)),
                is_system: Set(true),
                created_at: Set(chrono::Utc::now()),
                ..Default::default()
            }
            .insert(db)
            .await?;
        }
    }

    let admin = service::find_role_by_name(db, rbac::ADMIN_ROLE)
        .await?
        .ok_or(AppError::Internal("admin role missing after seeding"))?;
    user::Entity::update_many()
        .col_expr(user::Column::RoleId, admin.id.into())
        .filter(user::Column::RoleId.is_null())
        .exec(db)
        .await?;
    Ok(())
}

/// On an empty users table, create the default `admin` account with a random
/// password that must be changed on first login. The password is logged once
/// (there is no other way to recover it).
pub async fn ensure_admin(db: &DatabaseConnection) -> Result<(), AppError> {
    if user::Entity::find().count(db).await? > 0 {
        return Ok(());
    }
    let admin_role = service::find_role_by_name(db, rbac::ADMIN_ROLE)
        .await?
        .ok_or(AppError::Internal("admin role missing after seeding"))?;
    let password = random_token(24);
    service::create(db, "admin", None, &password, true, Some(admin_role.id)).await?;
    tracing::warn!(
        "seeded initial admin user | username: admin | password: {password} | change it on first login"
    );
    Ok(())
}
