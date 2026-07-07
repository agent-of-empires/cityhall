use axum::{routing::get, Router};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = router();

    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    tracing::info!("cityhall-api listening on http://{addr}");
    axum::serve(listener, app).await.expect("serve");
}

fn router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Smallest check: the router builds with the health route wired up.
    #[test]
    fn router_builds() {
        let _: Router = router();
    }
}
