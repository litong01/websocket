//! WebSocket server: Kinde auth, rooms (name + password), join then broadcast.

mod auth;
mod config;
mod room;
mod ws;

use axum::{
    extract::{
        ws::WebSocketUpgrade,
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use axum_extra::TypedHeader;
use headers::{authorization::Bearer, Authorization};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::auth::KindeValidator;
use crate::config::Config;
use crate::room::RoomStore;
use crate::ws::handle_socket;

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
        .route("/time", get(time_handler))
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::new().allow_origin(Any))
        .with_state((validator, store, config.idle_timeout_secs));

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    auth: Option<TypedHeader<Authorization<Bearer>>>,
    State((validator, store, idle_timeout_secs)): State<(
        Arc<KindeValidator>,
        Arc<RwLock<RoomStore>>,
        u64,
    )>,
) -> Response {
    let auth = match auth {
        Some(TypedHeader(a)) => a,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                "missing Authorization: Bearer <token> header",
            )
                .into_response();
        }
    };
    let token = auth.token().to_string();
    ws.on_upgrade(move |socket| {
        handle_socket(socket, token, validator, store, idle_timeout_secs)
    })
    .into_response()
}

/// Server time in UTC (ISO 8601). No auth required. Use for client clock sync.
async fn time_handler() -> impl IntoResponse {
    #[derive(serde::Serialize)]
    struct TimeResponse {
        utc: String,
    }
    let utc = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    axum::Json(TimeResponse { utc })
}
