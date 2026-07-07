use clap::{Parser, Subcommand};

use crate::auth::random_token;
use crate::error::AppError;
use crate::{db, rbac, seed, server, service};

#[derive(Parser)]
#[command(
    name = "cityhall",
    version,
    about = "CityHall: user management server + CLI"
)]
pub struct Cli {
    /// Log level for the app and its dependencies (error/warn/info/debug/trace).
    /// Cascades to sub-crates: e.g. `trace` also traces sqlx queries. For
    /// per-target control, set `RUST_LOG` instead (e.g. `info,sqlx::query=debug`).
    #[arg(long, global = true, env = "CITYHALL_LOG")]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the web server (default when no subcommand is given).
    Serve,
    /// Manage users from the command line.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
}

#[derive(Subcommand)]
pub enum UserAction {
    /// Create a user. Prints a random password when --password is omitted.
    Create {
        #[arg(long)]
        username: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        password: Option<String>,
        /// Role name to assign (defaults to `member`).
        #[arg(long)]
        role: Option<String>,
    },
    /// List all users.
    List,
    /// Delete a user by username.
    Delete {
        #[arg(long)]
        username: String,
    },
    /// Reset a user's password. Prints a random password when --password is omitted.
    Passwd {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: Option<String>,
    },
}

pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let db = db::connect().await?;
    seed::ensure_roles(&db).await?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            seed::ensure_admin(&db).await?;
            server::serve(db).await?;
        }
        Command::User { action } => run_user_action(&db, action).await?,
    }
    Ok(())
}

async fn run_user_action(
    db: &sea_orm::DatabaseConnection,
    action: UserAction,
) -> Result<(), AppError> {
    match action {
        UserAction::Create {
            username,
            email,
            password,
            role,
        } => {
            let role_name = role.unwrap_or_else(|| rbac::MEMBER_ROLE.to_string());
            let role = service::find_role_by_name(db, &role_name)
                .await?
                .ok_or(AppError::BadRequest("unknown role"))?;
            let (password, generated) = resolve_password(password);
            let must_change = generated;
            service::create(db, &username, email, &password, must_change, Some(role.id)).await?;
            println!("created user '{username}' with role '{role_name}'");
            if generated {
                println!("generated password: {password}");
            }
        }
        UserAction::List => {
            let users = service::list(db).await?;
            println!(
                "{:<5} {:<20} {:<30} MUST_CHANGE_PW",
                "ID", "USERNAME", "EMAIL"
            );
            for u in users {
                println!(
                    "{:<5} {:<20} {:<30} {}",
                    u.id,
                    u.username,
                    u.email.unwrap_or_default(),
                    u.must_change_password
                );
            }
        }
        UserAction::Delete { username } => {
            service::delete_by_username(db, &username).await?;
            println!("deleted user '{username}'");
        }
        UserAction::Passwd { username, password } => {
            let user = service::find_by_username(db, &username)
                .await?
                .ok_or(AppError::NotFound("user not found"))?;
            let (password, generated) = resolve_password(password);
            service::set_password(db, user, &password, generated).await?;
            println!("updated password for '{username}'");
            if generated {
                println!("generated password: {password}");
            }
        }
    }
    Ok(())
}

/// Returns the given password, or a random one; the bool flags "generated"
/// (in which case the user must change it on next login).
fn resolve_password(password: Option<String>) -> (String, bool) {
    match password {
        Some(p) => (p, false),
        None => (random_token(16), true),
    }
}
