# WebSocket server (Rust)

A WebSocket server built with **Tokio**, **Axum**, and **tokio-tungstenite**, with:

- **Auth**: [Kinde.com](https://kinde.com) (JWT validation via JWKS)
- **Rooms**: `room_name` + `password`; only authenticated users can join
- **One room per connection (per device)**: A given WebSocket connection can be in at most one room at a time. Joining a different room (or sending **leave** then **join**) moves that connection to the new room. This is enforced by the data model (one `room_name` per connection) and does not add any extra checks or slowdown.  
  **Same Kinde user on multiple devices:** Each device has its own connection and is treated as a separate participant for room membership. So one person logged in with the same Kinde account on two devices gets two connections; each can be in one room. They can both join the same room (and act as two participants there) or join different rooms. We do not restrict by Kinde user id—only by connection.
- **Semantics**: If the room does not exist, it is created when the first user joins with a password. Once in a room, every message other than `join` is broadcast to everyone in the room (including the sender).

## Quick start

1. **Environment**

   Copy the example env and set your Kinde domain (and optionally audience):

   ```bash
   cp .env.example .env
   # Edit .env: set KINDE_DOMAIN (e.g. myapp for myapp.kinde.com)
   ```

2. **Run**

   ```bash
   cargo run
   ```

   Or use the helper script with Docker (no local Cargo): `./check-and-test.sh` then run the container (see **Build and test** below).

   Server listens on `HOST:PORT` (default `0.0.0.0:8080`).

## Build and test

- **`./check-and-test.sh`** — Run cargo check and tests via Docker (uses `scorelib-builder:latest`). No local Cargo needed.
- **`./check-and-test.sh check`** — Cargo check only.
- **`./check-and-test.sh test`** — Cargo test only.
- **`./check-and-test.sh build`** — Build a container image for the host architecture and load it into Docker. Image name: `websocket-server:latest` (override with `APP_IMAGE_NAME`, `APP_IMAGE_TAG`).
- **Multi-arch (amd64 + arm64)**: set `REGISTRY` to your registry host, then run `./check-and-test.sh build` to build and push both platforms, e.g. `REGISTRY=myreg.io ./check-and-test.sh build`.

The **Dockerfile** is a multi-stage build (Rust build on `rust:1-bookworm`, runtime on `debian:bookworm-slim`). To build manually:

```bash
# Host-arch image only, load into Docker
docker buildx build --load -t websocket-server:latest .

# Multi-arch and push (after docker login)
docker buildx build --platform linux/amd64,linux/arm64 -t myreg.io/websocket-server:latest --push .
```

Run the image with env vars (e.g. `KINDE_DOMAIN`) and port mapping:

```bash
docker run --rm -e KINDE_DOMAIN=myapp -p 8080:8080 websocket-server:latest
```

**Connection capacity and open-file limits**  
The server does not enforce a maximum number of connections. The main limit is the **per-process open file descriptor limit** (each TCP/WebSocket connection uses file descriptors). The default in many environments is 1024 (soft), which caps you at roughly hundreds of concurrent connections.

To support more connections, raise the limit when running the container. For example, to allow up to 65k open files (and thus many thousands of connections):

```bash
docker run --rm -e KINDE_DOMAIN=myapp -p 8080:8080 \
  --ulimit nofile=65535:65535 \
  websocket-server:latest
```

- `nofile=65535:65535` sets both soft and hard limit for open files in the container. Adjust as needed (e.g. 32768 for moderate load).
- On Kubernetes you can set `securityContext.ulimits` or ensure the pod/container has a higher limit if the default is low.
- In production, also consider memory (each connection has buffer overhead) and CPU; scale horizontally (more replicas) if you need more than one node can handle.

**Scaling to tens of thousands of users (horizontal and global)**  
This server keeps **room state and broadcast in memory per process**. So a single instance can handle many connections (subject to ulimits and resources), but to scale out you need a strategy so that "rooms" and "broadcast" work across multiple instances.

- **Single region, multiple replicas**
  - **Option 1 – Sticky routing by room**  
    Put a load balancer (or API gateway) in front of multiple replicas and route by **room name**: e.g. consistent hash of `room_name`, or a small directory service that maps `room_id` → server. Every connection that joins the same room is sent to the same replica. That way each room lives on one server; broadcast stays in-process. Limits: no single room can be larger than one replica’s capacity, and you must implement the router/directory.
  - **Option 2 – Distributed pub-sub**  
    Introduce a message broker (e.g. **Redis Pub/Sub**, or similar). When a user sends play/stop/etc., the server **publishes** the message to a channel keyed by room (e.g. `room:{room_name}`). Every replica **subscribes** to the channels for rooms it has local members in and **forwards** received messages to those WebSocket connections. Room membership is still per-replica (which connections are in which room on this node); only "broadcast" is shared via the broker. This requires code changes (replace in-memory broadcast with publish/subscribe to Redis). Scaling: add more replicas; no single-room limit.

- **Global (multiple regions)**  
  Run replicas in several regions behind a **global load balancer** (e.g. DNS + geo routing, or cloud LB). You can either:
  - Keep **rooms regional**: route users to the nearest region; a "room" exists only in one region (same as Option 1 per region), or
  - **Cross-region rooms**: use a **globally replicated pub-sub** (e.g. Redis with cross-region replication, or a managed pub/sub like AWS SNS, GCP Pub/Sub) so that a broadcast in one region is delivered to subscribers in all regions. Higher latency and operational complexity.

**Summary:** For tens of thousands of users, use **multiple replicas** and either **room-based sticky routing** (simpler, room size limited by one node) or **Redis (or similar) pub-sub** for broadcast across nodes (more work, no per-room node limit). For global deployment, add multiple regions and choose regional vs cross-region rooms and the matching pub-sub setup.

### Packaging the binary in another product's container (arm64 + amd64)

Other products can ship the **websocket-server** binary inside their own image and support both **linux/amd64** and **linux/arm64** in either of these ways.

**Option A: Copy the binary from our published image (no Rust build)**

Use a multi-stage Dockerfile and copy the binary from the websocket-server image. When you build your image with `docker buildx build --platform linux/amd64,linux/arm64 ...`, Docker pulls the matching variant of the websocket-server image per platform, so you get the correct binary for each arch.

Replace `docker.io/<DOCKERHUB_USERNAME>/<repo>:latest` with the actual image (e.g. from Docker Hub or your registry).

```dockerfile
# Stage 1: get the binary from the websocket-server image
ARG WS_IMAGE=docker.io/<DOCKERHUB_USERNAME>/<repo>:latest
FROM ${WS_IMAGE} AS ws

# Stage 2: your product's image
FROM debian:bookworm-slim
# ... your app files, env, etc. ...
COPY --from=ws /usr/local/bin/websocket-server /usr/local/bin/websocket-server
# Run websocket-server as a sidecar, or your entrypoint that starts it
```

Build multi-arch and push:

```bash
docker buildx build --platform linux/amd64,linux/arm64 \
  -t myproduct:latest --push .
```

**Option B: Build the binary from source in your Dockerfile**

Build this repo inside your Dockerfile and copy the binary out. Use the same Dockerfile as this repo (or a stage that runs `cargo build --release`) and build your image with `--platform linux/amd64` or `--platform linux/arm64` so the binary matches the target. No cross-compilation needed.

```dockerfile
# Stage 1: build websocket-server (build with buildx for each platform)
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .   # or git clone this repo
RUN cargo build --release

# Stage 2: your product's image
FROM debian:bookworm-slim
COPY --from=builder /app/target/release/websocket-server /usr/local/bin/
# ... rest of your image
```

Build for each architecture (or use buildx with `--platform linux/amd64,linux/arm64`; each platform builds natively in its own builder stage).

### GitHub Actions

On every **push** and **pull_request** to `main`:

1. **Check and test** — `cargo check` and `cargo test` run (Rust is installed by the workflow; no Docker builder image used in CI).
2. **Build and push** (push to `main` only) — Container image is built for **linux/amd64** and **linux/arm64** and pushed to **Docker Hub**:
   - `docker.io/<DOCKERHUB_USERNAME>/<repo>:latest`
   - `docker.io/<DOCKERHUB_USERNAME>/<repo>:<short-sha>`

**Required secrets** (Settings → Secrets and variables → Actions):

- **`DOCKERHUB_USERNAME`** — Your Docker Hub username.
- **`DOCKERHUB_TOKEN`** — Docker Hub access token (Account → Security → New Access Token) with read/write permissions.

Example: `docker pull <your-dockerhub-username>/websocket:latest`

## Configuration

| Variable         | Required | Description |
|------------------|----------|-------------|
| `KINDE_DOMAIN`   | Yes      | Kinde domain, e.g. `myapp` for `myapp.kinde.com` (used for JWKS and issuer). |
| `KINDE_AUDIENCE` | No       | If set, JWT `aud` is validated against this value. |
| `HOST`           | No       | Bind address (default `0.0.0.0`). |
| `PORT`               | No       | Bind port (default `8080`). |
| `IDLE_TIMEOUT_SECS`  | No       | Close WebSocket after this many seconds with no activity (default `7200` = 2 hours). Set to `0` to disable. |
| `RUST_LOG`           | No       | Log level (default `info`). |

## WebSocket API

- **Server time and clock sync**  
  Commands like **play** use a `startAt` UTC time. To keep everyone aligned without extra round-trips:
  - **On join**, the client may send its current UTC: **`clientUtc`** (ISO 8601, e.g. `"2026-02-27T12:00:00.000Z"`). The server records the offset between server time and that value and stores it for the connection.
  - When this connection sends **play** / **stop** / **pause** / **prev** / **next** with **startAt**, the server **converts** that time from the client’s clock to server time using the stored offset, then **broadcasts the adjusted** message. Everyone in the room therefore receives the same canonical (server) time in `startAt`.
  - The **join response** includes **`serverUtc`** (server’s current time at join). The client can use that to convert received `startAt` values (which are in server time) to its own clock for display or scheduling.
  - If **`clientUtc`** is omitted on join, no offset is stored and `startAt` is broadcast unchanged.
  - Optional: **`GET /time`** (no auth) returns `{ "utc": "..." }` if you need server time without joining (e.g. before opening the WebSocket).

- **Endpoint**: `GET /ws`
  - Clients must send the Kinde access token in the **`Authorization`** header when opening the WebSocket: **`Authorization: Bearer <KINDE_ACCESS_TOKEN>`**. The server validates the token during the upgrade; if it is missing or invalid, the request is rejected with **401 Unauthorized**. After the connection is established, send **join** and **play** (etc.) as JSON messages—no token in the message body.

- **Messages** (JSON text frames):

  1. **Join (or create) a room**
     - Send:
       ```json
       { "join": { "room": "CaryChoir2026", "password": "abc123", "clientUtc": "2026-02-27T12:00:00.000Z" } }
       ```
       `clientUtc` is optional; if present, the server uses it to convert this connection’s `startAt` to server time when broadcasting.
     - If the room does not exist, it is created with the given password. If it exists, the password must match.
     - Response:
       - Success: `{ "ok": true, "event": "joined", "room": "room_name", "members": N, "serverUtc": "2026-02-27T12:00:00.123Z" }`
       - Error: `{ "error": "wrong password" }` or similar.

  2. **Leave the current room** (optional; closing the WebSocket also leaves the room)
     - Send: `{ "leave": {} }`
     - Response: `{ "ok": true, "event": "left", "room": "room_name" }` (or `"room": null` if you were not in a room). You can then send **join** again to join the same or another room.

  3. **Room commands (broadcast to everyone in the room, including sender)**
     - Exactly one top-level key per message: **play**, **stop**, **pause**, **prev**, or **next**. The value is an object (e.g. with `startAt`, `comment`).
     - Examples:
       ```json
       { "play":   { "startAt": "2026-02-27T12:00:00Z", "comment": "Starting track" } }
       { "stop":   { "startAt": "2026-02-27T12:05:00Z", "comment": "Stopped" } }
       { "pause":  { "startAt": "2026-02-27T12:10:00Z", "comment": "Paused" } }
       { "prev":   { "startAt": "2026-02-27T12:15:00Z", "comment": "Previous" } }
       { "next":   { "startAt": "2026-02-27T12:20:00Z", "comment": "Next" } }
       ```
     - The **exact** JSON is broadcast to every member of the room. No separate response is sent; clients receive the same JSON as incoming messages.
     - You must have joined a room first (via `join`).

- **Errors**
  - Invalid or expired token: `{ "error": "invalid or expired token" }`
  - Not in a room: `{ "error": "join a room first" }`
  - Invalid JSON: `{ "error": "invalid JSON" }`
  - Unknown command: `{ "error": "unknown command; use join, leave, or one of: play, stop, pause, prev, next" }`
  - Multiple commands in one message: `{ "error": "message must contain exactly one command: play, stop, pause, prev, or next" }`

## Recovery after network problems

If a connection is interrupted (Wi‑Fi drop, mobile network switch, etc.):

1. **On the server**  
   The connection is treated as closed: the receive loop ends, and the server runs normal cleanup (`leave(conn_id)`), so the user is removed from the room. There is no in-band “session resume”; each connection is independent.

2. **Recovery is client-driven: reconnect and re-join**  
   The client should:
   - Detect the closed connection (e.g. `on_close`, `on_error`, or a failed send/recv).
   - Open a **new** WebSocket to `GET /ws` with **`Authorization: Bearer <token>`** (use a refreshed Kinde token if the old one expired during the outage).
   - Send **join** again with the same room and password: `{"join":{"room":"...","password":"..."}}`.

   The server will treat this as a new connection and add the client to the room again. No extra server support is required.

3. **Missed messages**  
   While disconnected, the client does not receive room broadcasts. The server does not queue or replay them. If your app needs to catch up (e.g. current play state), have the client request or derive that state after re-joining (e.g. from your own API or from the next broadcast after reconnect).

**Recommendation:** Implement reconnection in the client (e.g. retry with backoff when the socket closes unexpectedly) and always re-send **join** after a successful reconnect.

## Example client flow

1. Log in with Kinde (e.g. backend or CLI) and obtain an access token.
2. Open WebSocket to `wss://your-server/ws` with header **`Authorization: Bearer <access_token>`**.
3. Send: `{"join":{"room":"CaryChoir2026","password":"abc123"}}`.
4. After success, send a room command to broadcast, e.g. `{"play":{"startAt":"2026-02-27T12:00:00Z","comment":"Starting"}}`; all room members (including you) receive it. Supported commands: **play**, **stop**, **pause**, **prev**, **next**. To leave the room but stay connected, send `{"leave":{}}`; you can then join another room.

## Tech stack

- **Runtime**: Tokio
- **HTTP/WS**: Axum (with built-in WebSocket support via tokio-tungstenite)
- **Auth**: Kinde JWT validation using JWKS from `https://<KINDE_DOMAIN>.kinde.com/.well-known/jwks`
- **Passwords**: bcrypt for room passwords
