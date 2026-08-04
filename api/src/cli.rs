use clap::{Parser, Subcommand};

use crate::auth::random_token;
use crate::error::AppError;
use crate::{crypto, db, rbac, secrets, seed, server, service};

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
    /// Inspect and rotate the encryption of stored secrets.
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
}

#[derive(Subcommand)]
pub enum SecretsAction {
    /// Report what the current key can read, per store.
    Status,
    /// Re-encrypt stored secrets under the current CITYHALL_SECRET_KEY.
    Rotate,
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
    // Before anything else: a malformed CITYHALL_SECRET_KEY_PREVIOUS is a
    // configuration error, and refusing to start beats discovering it whenever
    // some unrelated request happens to decrypt a secret.
    crypto::validate_keyring()?;

    let db = db::connect().await?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            seed::ensure_roles(&db).await?;
            seed::ensure_admin(&db).await?;
            secrets::warn_about_legacy_secrets(&db).await;
            server::serve(db).await?;
        }
        Command::User { action } => {
            seed::ensure_roles(&db).await?;
            run_user_action(&db, action).await?
        }
        // Deliberately without seeding roles, unlike every other command: this is
        // what an operator reaches for when stored secrets are already unreadable,
        // so it must not write to unrelated tables or be able to fail on them.
        Command::Secrets { action } => run_secrets_action(&db, action).await?,
    }
    Ok(())
}

async fn run_secrets_action(
    db: &sea_orm::DatabaseConnection,
    action: SecretsAction,
) -> Result<(), AppError> {
    match action {
        SecretsAction::Status => {
            let reports = secrets::status(db).await?;
            print_store_table(&reports);
            print_guidance(&reports);
        }
        SecretsAction::Rotate => {
            let report = secrets::rotate(db).await?;
            println!("re-encrypted {} secret(s)", report.rotated);
            if !report.unreadable.is_empty() {
                println!(
                    "\nno key in CITYHALL_SECRET_KEY or CITYHALL_SECRET_KEY_PREVIOUS opens these,\n\
                     so they cannot be recovered and must be entered again:"
                );
                for label in &report.unreadable {
                    println!("  {label}");
                }
            }
            if !report.skipped.is_empty() {
                println!(
                    "\nsomething else wrote these while rotating, so they were left alone.\n\
                     A writer is probably still running with the old key; restart every\n\
                     replica and rotate again:"
                );
                for label in &report.skipped {
                    println!("  {label}");
                }
            }
            println!();
            print_store_table(&report.after);
            print_guidance(&report.after);
        }
    }
    Ok(())
}

fn print_store_table(reports: &[secrets::StoreReport]) {
    println!(
        "{:<20} {:>5} {:>8} {:>15} {:>7} {:>11}",
        "STORE", "ROWS", "CURRENT", "NEEDS-PREVIOUS", "LEGACY", "UNREADABLE"
    );
    for r in reports {
        println!(
            "{:<20} {:>5} {:>8} {:>15} {:>7} {:>11}",
            r.store, r.rows, r.current, r.needs_previous, r.legacy, r.unreadable
        );
    }
}

/// Turn the counts into the next action, so an operator does not have to know
/// which column means "not safe to drop the old key yet".
fn print_guidance(reports: &[secrets::StoreReport]) {
    let sum = |f: fn(&secrets::StoreReport) -> usize| reports.iter().map(f).sum::<usize>();
    let (needs_previous, legacy, unreadable) = (
        sum(|r| r.needs_previous),
        sum(|r| r.legacy),
        sum(|r| r.unreadable),
    );

    println!();
    if needs_previous > 0 {
        println!(
            "{needs_previous} secret(s) need CITYHALL_SECRET_KEY_PREVIOUS to be readable.\n\
             Run `cityhall secrets rotate`; removing it before that count is zero loses them."
        );
    }
    if legacy > 0 {
        println!(
            "{legacy} secret(s) predate the encryption envelope, so they are not bound to the\n\
             row holding them and could be moved between rows. Run `cityhall secrets rotate`."
        );
    }
    if unreadable > 0 {
        println!(
            "{unreadable} secret(s) cannot be read with any configured key. Either a previous\n\
             key is missing from CITYHALL_SECRET_KEY_PREVIOUS, or they must be entered again."
        );
    }
    if needs_previous == 0 && legacy == 0 && unreadable == 0 {
        println!(
            "Every stored secret is bound to its row and readable with the current key.\n\
             CITYHALL_SECRET_KEY_PREVIOUS is safe to remove."
        );
    }
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
