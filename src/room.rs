//! Room model: name + password; only authenticated users can join.

use bcrypt::{hash, verify, DEFAULT_COST};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// Room: name, password hash, and broadcast channel for members.
pub struct Room {
    #[allow(dead_code)]
    pub name: String,
    password_hash: String,
    /// Sender side of broadcast channel; each member holds a receiver.
    tx: broadcast::Sender<String>,
}

impl Room {
    fn new(name: String, password: &str) -> Result<Self, bcrypt::BcryptError> {
        let password_hash = hash(password, DEFAULT_COST)?;
        let (tx, _) = broadcast::channel(256);
        Ok(Self { name, password_hash, tx })
    }

    fn verify_password(&self, password: &str) -> bool {
        verify(password, &self.password_hash).unwrap_or(false)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, msg: &str) {
        let _ = self.tx.send(msg.to_string());
    }

    pub fn member_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// Connection id for a WebSocket client.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConnectionId(pub Uuid);

/// Per-connection state: which room (if any) and user id.
pub struct ConnectionState {
    pub room_name: Option<String>,
    #[allow(dead_code)]
    pub user_id: String,
}

/// In-memory store: rooms by name, and connection id -> state.
pub struct RoomStore {
    rooms: RwLock<HashMap<String, Arc<Room>>>,
    connections: RwLock<HashMap<ConnectionId, ConnectionState>>,
}

impl RoomStore {
    pub fn new() -> Self {
        Self {
            rooms: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// Join or create a room. Returns the room's broadcast receiver and current member count.
    pub async fn join(
        &self,
        conn_id: ConnectionId,
        user_id: String,
        room_name: String,
        password: &str,
    ) -> Result<(broadcast::Receiver<String>, usize), RoomError> {
        let room = {
            let mut rooms = self.rooms.write().await;
            let room = match rooms.get(&room_name) {
                Some(r) => {
                    if !r.verify_password(password) {
                        return Err(RoomError::WrongPassword);
                    }
                    Arc::clone(r)
                }
                None => {
                    let room = Arc::new(Room::new(room_name.clone(), password)?);
                    rooms.insert(room_name.clone(), Arc::clone(&room));
                    room
                }
            };
            room
        };

        let rx = room.subscribe();
        let member_count = room.member_count();

        let mut connections = self.connections.write().await;
        connections.insert(
            conn_id,
            ConnectionState {
                room_name: Some(room_name),
                user_id,
            },
        );

        Ok((rx, member_count))
    }

    /// Get the room name for a connection (if any).
    pub async fn get_room(&self, conn_id: ConnectionId) -> Option<String> {
        let connections = self.connections.read().await;
        connections.get(&conn_id).and_then(|c| c.room_name.clone())
    }

    /// Broadcast a message to all members of the room the connection is in (including sender).
    pub async fn broadcast_in_room(&self, conn_id: ConnectionId, msg: &str) -> Option<()> {
        let room_name = self.get_room(conn_id).await?;
        let rooms = self.rooms.read().await;
        let room = rooms.get(&room_name)?;
        room.broadcast(msg);
        Some(())
    }

    /// Remove connection from store (e.g. on disconnect).
    pub async fn leave(&self, conn_id: ConnectionId) {
        let mut connections = self.connections.write().await;
        connections.remove(&conn_id);
    }
}

impl Default for RoomStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    #[error("wrong password")]
    WrongPassword,
    #[error("bcrypt error: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),
}
