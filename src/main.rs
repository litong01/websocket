//! WebSocket server: Kinde auth, rooms (name + password), join then broadcast.

mod auth;
mod config;
mod room;
mod ws;

use axum::{
    extract::{Query, WebSocketUpgrade},
    response::Response,
    routing::get,
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::auth::KindeValidator;
use crate::config::Config;
use crate::room::RoomStore;
use crate::ws::handle_socket;

#[derive(serde::Deserialize)]
struct WsQuery {
    token: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env();
    let validator = Arc::new(KindeValidator::new(
        &config.kinde_domain,
        config.kinde_audience.clone(),
    ));
    validator.refresh_jwks().await?;

    let store = Arc::new(RwLock::new(RoomStore::new()));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::new().allow_origin(Any))
        .with_state((validator, store));

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(q): Query<WsQuery>,
    axum::extract::State((validator, store)): axum::extract::State<(
        Arc<KindeValidator>,
        Arc<RwLock<RoomStore>>,
    )>,
) -> Response {
    let token = q.token;
    ws.on_upgrade(move |socket| {
        handle_socket(socket, token, validator, store)
    })
}
