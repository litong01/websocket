//! WebSocket handler: join, leave, and broadcast (play, stop, pause, prev, next).
//! Includes idle timeout and server-side clock sync: join can send clientUtc, server stores offset and adjusts startAt when broadcasting.

use crate::auth::KindeValidator;
use crate::room::{ConnectionId, RoomStore};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tracing::{error, info, warn};

/// Top-level keys that are broadcast to the room (one per message).
const BROADCAST_COMMANDS: &[&str] = &["play", "stop", "pause", "prev", "next"];

/// If the message is a single-key command with startAt, convert startAt from client time to server time using offset_secs.
/// Returns the adjusted JSON string, or the original if no adjustment is needed or parsing fails.
fn adjust_startat_to_server_time(msg: &str, offset_secs: f64) -> String {
    let mut parsed: serde_json::Value = match serde_json::from_str(msg) {
        Ok(v) => v,
        Err(_) => return msg.to_string(),
    };
    let obj = match parsed.as_object_mut() {
        Some(o) => o,
        None => return msg.to_string(),
    };
    if obj.len() != 1 {
        return msg.to_string();
    }
    let (_cmd_key, cmd_val) = match obj.iter_mut().next() {
        Some((k, v)) => (k.clone(), v),
        None => return msg.to_string(),
    };
    let cmd_obj = match cmd_val.as_object_mut() {
        Some(o) => o,
        None => return msg.to_string(),
    };
    let start_at_str = match cmd_obj.get("startAt").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return msg.to_string(),
    };
    let client_dt = match chrono::DateTime::parse_from_rfc3339(start_at_str) {
        Ok(dt) => dt,
        Err(_) => return msg.to_string(),
    };
    let client_utc = client_dt.with_timezone(&chrono::Utc);
    let server_utc = client_utc + chrono::Duration::milliseconds((offset_secs * 1000.0) as i64);
    let server_str = server_utc.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    cmd_obj.insert("startAt".to_string(), serde_json::Value::String(server_str));
    serde_json::to_string(&parsed).unwrap_or_else(|_| msg.to_string())
}

/// Handle a single WebSocket connection.
/// Token is validated at upgrade; `idle_timeout_secs` closes the connection after no activity (0 = disabled).
pub async fn handle_socket(
    mut socket: WebSocket,
    token: String,
    validator: Arc<KindeValidator>,
    store: Arc<RwLock<RoomStore>>,
    idle_timeout_secs: u64,
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

    let idle_duration = Duration::from_secs(idle_timeout_secs);
    let mut next_deadline = Instant::now() + idle_duration;

    loop {
        tokio::select! {
            msg = receiver.next() => {
                // Reset idle deadline on any message from client
                if idle_timeout_secs > 0 {
                    next_deadline = Instant::now() + idle_duration;
                }
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

                // Join: { "join": { "room": "...", "password": "...", "clientUtc": "2026-02-27T12:00:00.000Z" (optional) } }
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
                    let clock_offset_secs = join_obj
                        .get("clientUtc")
                        .and_then(|v| v.as_str())
                        .and_then(|s| {
                            chrono::DateTime::parse_from_rfc3339(s).ok().map(|client_dt| {
                                let server_now = chrono::Utc::now();
                                let client_utc = client_dt.with_timezone(&chrono::Utc);
                                (server_now - client_utc).num_milliseconds() as f64 / 1000.0
                            })
                        });
                    let server_utc = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
                    let s = store.read().await;
                    match s.join(conn_id, user_id.clone(), room_name.clone(), &password, clock_offset_secs).await {
                        Ok((rx, count)) => {
                            room_rx = Some(rx);
                            let _ = sender.send(Message::Text(serde_json::json!({
                                "ok": true,
                                "event": "joined",
                                "room": room_name,
                                "members": count,
                                "serverUtc": server_utc
                            }).to_string())).await;
                        }
                        Err(e) => {
                            let _ = sender.send(Message::Text(serde_json::json!({ "error": e.to_string() }).to_string())).await;
                        }
                    }
                    continue;
                }

                // Leave: { "leave": {} }
                if obj.get("leave").is_some() {
                    let room_name = store.read().await.get_room(conn_id).await;
                    store.read().await.leave(conn_id).await;
                    room_rx = None;
                    let _ = sender
                        .send(Message::Text(
                            serde_json::json!({
                                "ok": true,
                                "event": "left",
                                "room": room_name
                            })
                            .to_string(),
                        ))
                        .await;
                    continue;
                }

                // Broadcast commands: exactly one of play, stop, pause, prev, next (e.g. { "play": { "startAt": "...", "comment": "..." } })
                let broadcast_key = obj
                    .keys()
                    .find(|k| BROADCAST_COMMANDS.contains(&k.as_str()));
                match broadcast_key {
                    Some(key) if obj.len() == 1 => {
                        let store_r = store.read().await;
                        let to_broadcast = match store_r.get_clock_offset(conn_id).await {
                            Some(offset) => adjust_startat_to_server_time(&msg, offset),
                            None => msg.clone(),
                        };
                        if store_r.broadcast_in_room(conn_id, &to_broadcast).await.is_none() {
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
                            "error": "unknown command; use join, leave, or one of: play, stop, pause, prev, next"
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
                    if idle_timeout_secs > 0 {
                        next_deadline = Instant::now() + idle_duration;
                    }
                    if sender.send(Message::Text(b)).await.is_err() {
                        break;
                    }
                }
            }

            _ = tokio::time::sleep_until(next_deadline), if idle_timeout_secs > 0 => {
                warn!(conn_id = %conn_id.0, "idle timeout, closing connection");
                break;
            }
        }
    }

    store_leave.read().await.leave(conn_id_leave).await;
    info!(conn_id = %conn_id_leave.0, "client disconnected");
}
