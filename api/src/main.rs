mod auth;
mod cli;
mod crypto;
mod db;
mod entities;
mod error;
mod handlers;
mod mailer;
mod migration;
mod rbac;
mod seed;
mod server;
mod service;

use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    init_tracing(cli.log_level.as_deref());

    if let Err(e) = cli::run(cli).await {
        tracing::error!("fatal: {e}");
        std::process::exit(1);
    }
}

/// An explicit `--log-level`/`CITYHALL_LOG` sets one level for the app and all
/// dependencies, so raising it to `trace` also traces sqlx queries. Otherwise
/// `RUST_LOG` wins (full per-target control), falling back to a default that
/// keeps sqlx's per-query logging quiet.
fn init_tracing(level: Option<&str>) {
    use tracing_subscriber::EnvFilter;
    let filter = match level {
        Some(level) => EnvFilter::new(level),
        None => EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,sqlx::query=warn")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg(test)]
mod tests {
    use crate::auth::{hash_password, verify_password};

    #[test]
    fn password_hash_round_trip() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong", &hash));
    }
}
