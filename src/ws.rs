//! WebSocket handler: join command vs broadcast (play, stop, pause, prev, next).

use crate::auth::KindeValidator;
use crate::room::{ConnectionId, RoomStore};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Top-level keys that are broadcast to the room (one per message).
const BROADCAST_COMMANDS: &[&str] = &["play", "stop", "pause", "prev", "next"];

/// Handle a single WebSocket connection.
/// Query param `token` must be the Kinde access token.
pub async fn handle_socket(
    mut socket: WebSocket,
    token: String,
    validator: Arc<KindeValidator>,
    store: Arc<RwLock<RoomStore>>,
) {
    let claims = match validator.validate(&token).await {
        Ok(c) => c,
        Err(e) => {
            warn!("invalid token: {}", e);
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({ "error": "invalid or expired token" }).to_string(),
                ))
                .await;
            return;
        }
    };
    let user_id = claims.sub.clone();
    let conn_id = ConnectionId(uuid::Uuid::new_v4());
    info!(conn_id = %conn_id.0, user_id = %user_id, "client connected");

    let store = store.clone();
    let store_leave = store.clone();
    let conn_id_leave = conn_id;
    let (mut sender, mut receiver) = socket.split();
    let mut room_rx = None::<tokio::sync::broadcast::Receiver<String>>;

    loop {
        tokio::select! {
            msg = receiver.next() => {
                let msg = match msg {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Err(e)) => {
                        error!("ws recv error: {}", e);
                        break;
                    }
                    Some(Ok(_)) => continue,
                    None => break,
                };

                let parsed: serde_json::Value = match serde_json::from_str(&msg) {
                    Ok(v) => v,
                    Err(_) => {
                        let _ = sender.send(Message::Text(serde_json::json!({ "error": "invalid JSON" }).to_string())).await;
                        continue;
                    }
                };

                let obj = match parsed.as_object() {
                    Some(o) => o,
                    None => {
                        let _ = sender.send(Message::Text(serde_json::json!({ "error": "expected JSON object" }).to_string())).await;
                        continue;
                    }
                };

                // Join: { "join": { "room": "...", "password": "..." } }
                if let Some(join_val) = obj.get("join") {
                    let join_obj = match join_val.as_object() {
                        Some(o) => o,
                        None => {
                            let _ = sender.send(Message::Text(serde_json::json!({ "error": "join must be an object with room and password" }).to_string())).await;
                            continue;
                        }
                    };
                    let room_name = join_obj
                        .get("room")
                        .and_then(|r| r.as_str())
                        .map(String::from);
                    let password = join_obj
                        .get("password")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    let (room_name, password) = match room_name {
                        Some(r) => (r, password.to_string()),
                        None => {
                            let _ = sender.send(Message::Text(serde_json::json!({ "error": "join requires room and password" }).to_string())).await;
                            continue;
                        }
                    };
                    let s = store.read().await;
                    match s.join(conn_id, user_id.clone(), room_name.clone(), &password).await {
                        Ok((rx, count)) => {
                            room_rx = Some(rx);
                            let _ = sender.send(Message::Text(serde_json::json!({
                                "ok": true,
                                "event": "joined",
                                "room": room_name,
                                "members": count
                            }).to_string())).await;
                        }
                        Err(e) => {
                            let _ = sender.send(Message::Text(serde_json::json!({ "error": e.to_string() }).to_string())).await;
                        }
                    }
                    continue;
                }

                // Broadcast commands: exactly one of play, stop, pause, prev, next (e.g. { "play": { "startAt": "...", "comment": "..." } })
                let broadcast_key = obj
                    .keys()
                    .find(|k| BROADCAST_COMMANDS.contains(&k.as_str()));
                match broadcast_key {
                    Some(key) if obj.len() == 1 => {
                        let store_r = store.read().await;
                        if store_r.broadcast_in_room(conn_id, &msg).await.is_none() {
                            let _ = sender.send(Message::Text(serde_json::json!({ "error": "join a room first" }).to_string())).await;
                        }
                    }
                    Some(_) => {
                        let _ = sender.send(Message::Text(serde_json::json!({
                            "error": "message must contain exactly one command: play, stop, pause, prev, or next"
                        }).to_string())).await;
                    }
                    None => {
                        let _ = sender.send(Message::Text(serde_json::json!({
                            "error": "unknown command; use join or one of: play, stop, pause, prev, next"
                        }).to_string())).await;
                    }
                }
            }

            broadcast_msg = async {
                if let Some(ref mut rx) = room_rx {
                    rx.recv().await.ok()
                } else {
                    std::future::pending().await
                }
            } => {
                if let Some(b) = broadcast_msg {
                    if sender.send(Message::Text(b)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    store_leave.read().await.leave(conn_id_leave).await;
    info!(conn_id = %conn_id_leave.0, "client disconnected");
}
