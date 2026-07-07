use axum::extract::{Path, State};
use axum::Json;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::entities::{role, user};
use crate::error::AppError;
use crate::rbac;

#[derive(Serialize)]
pub struct RoleResponse {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_system: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Number of users assigned this role (a role in use cannot be deleted).
    pub user_count: u64,
}

async fn to_response(db: &DatabaseConnection, r: role::Model) -> Result<RoleResponse, AppError> {
    let user_count = user::Entity::find()
        .filter(user::Column::RoleId.eq(r.id))
        .count(db)
        .await?;
    Ok(RoleResponse {
        id: r.id,
        name: r.name,
        description: r.description,
        permissions: serde_json::from_str(&r.permissions).unwrap_or_default(),
        is_system: r.is_system,
        created_at: r.created_at,
        user_count,
    })
}

#[derive(Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub permissions: Option<Vec<String>>,
}

/// The permission-key catalog, so the UI can render a role editor.
#[derive(Serialize)]
pub struct PermissionEntry {
    pub key: String,
    pub description: String,
}

pub async fn permissions(caller: AuthUser) -> Result<Json<Vec<PermissionEntry>>, AppError> {
    caller.require("roles.read")?;
    Ok(Json(
        rbac::CATALOG
            .iter()
            .map(|(key, description)| PermissionEntry {
                key: key.to_string(),
                description: description.to_string(),
            })
            .collect(),
    ))
}

pub async fn list(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
) -> Result<Json<Vec<RoleResponse>>, AppError> {
    caller.require("roles.read")?;
    let rows = role::Entity::find()
        .order_by_asc(role::Column::Id)
        .all(&db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(to_response(&db, r).await?);
    }
    Ok(Json(out))
}

pub async fn create(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Json(body): Json<CreateRoleRequest>,
) -> Result<Json<RoleResponse>, AppError> {
    caller.require("roles.write")?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("role name is required"));
    }
    rbac::validate_keys(&body.permissions)?;
    if crate::service::find_role_by_name(&db, &name)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("role name already exists"));
    }
    let created = role::ActiveModel {
        name: Set(name),
        description: Set(body.description.filter(|s| !s.trim().is_empty())),
        permissions: Set(rbac::encode(&body.permissions)),
        is_system: Set(false),
        created_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    Ok(Json(to_response(&db, created).await?))
}

pub async fn update(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Path(id): Path<i32>,
    Json(body): Json<UpdateRoleRequest>,
) -> Result<Json<RoleResponse>, AppError> {
    caller.require("roles.write")?;
    let existing = role::Entity::find_by_id(id)
        .one(&db)
        .await?
        .ok_or(AppError::NotFound("role not found"))?;

    // The admin role is the wildcard safety net; it cannot be modified.
    if existing.name == rbac::ADMIN_ROLE {
        return Err(AppError::Forbidden("the admin role cannot be modified"));
    }
    let is_system = existing.is_system;
    let mut active: role::ActiveModel = existing.into();

    if let Some(name) = body.name {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::BadRequest("role name cannot be empty"));
        }
        // Renaming a built-in role would break code that references it by name.
        if is_system {
            return Err(AppError::Forbidden("a built-in role cannot be renamed"));
        }
        if let Some(other) = crate::service::find_role_by_name(&db, &name).await? {
            if other.id != id {
                return Err(AppError::Conflict("role name already exists"));
            }
        }
        active.name = Set(name);
    }
    if let Some(description) = body.description {
        active.description = Set(Some(description).filter(|s| !s.trim().is_empty()));
    }
    if let Some(permissions) = body.permissions {
        rbac::validate_keys(&permissions)?;
        active.permissions = Set(rbac::encode(&permissions));
    }

    Ok(Json(to_response(&db, active.update(&db).await?).await?))
}

pub async fn delete(
    State(db): State<DatabaseConnection>,
    caller: AuthUser,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    caller.require("roles.write")?;
    let existing = role::Entity::find_by_id(id)
        .one(&db)
        .await?
        .ok_or(AppError::NotFound("role not found"))?;
    if existing.is_system {
        return Err(AppError::Forbidden("a built-in role cannot be deleted"));
    }
    // No DB-level FK, so enforce referential integrity here.
    let in_use = user::Entity::find()
        .filter(user::Column::RoleId.eq(id))
        .count(&db)
        .await?;
    if in_use > 0 {
        return Err(AppError::Conflict("role is assigned to users"));
    }
    role::Entity::delete_by_id(id).exec(&db).await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
