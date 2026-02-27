# WebSocket server (Rust)

A WebSocket server built with **Tokio**, **Axum**, and **tokio-tungstenite**, with:

- **Auth**: [Kinde.com](https://kinde.com) (JWT validation via JWKS)
- **Rooms**: `room_name` + `password`; only authenticated users can join
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
| `PORT`           | No       | Bind port (default `8080`). |
| `RUST_LOG`       | No       | Log level (default `info`). |

## WebSocket API

- **Endpoint**: `GET /ws?token=<KINDE_ACCESS_TOKEN>`
  - The Kinde access token must be sent as the `token` query parameter. Validate it in your app (e.g. after Kinde login) and then open the WebSocket with that token.

- **Messages** (JSON text frames):

  1. **Join (or create) a room**
     - Send:
       ```json
       { "join": { "room": "CaryChoir2026", "password": "abc123" } }
       ```
     - If the room does not exist, it is created with the given password. If it exists, the password must match.
     - Response:
       - Success: `{ "ok": true, "event": "joined", "room": "room_name", "members": N }`
       - Error: `{ "error": "wrong password" }` or similar.

  2. **Room commands (broadcast to everyone in the room, including sender)**
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
  - Unknown command: `{ "error": "unknown command; use join or one of: play, stop, pause, prev, next" }`
  - Multiple commands in one message: `{ "error": "message must contain exactly one command: play, stop, pause, prev, or next" }`

## Example client flow

1. Log in with Kinde (e.g. SPA or backend callback) and obtain an access token.
2. Open WebSocket: `wss://your-server/ws?token=<access_token>`.
3. Send: `{"join":{"room":"CaryChoir2026","password":"abc123"}}`.
4. After success, send a room command to broadcast, e.g. `{"play":{"startAt":"2026-02-27T12:00:00Z","comment":"Starting"}}`; all room members (including you) receive it. Supported commands: **play**, **stop**, **pause**, **prev**, **next**.

## Tech stack

- **Runtime**: Tokio
- **HTTP/WS**: Axum (with built-in WebSocket support via tokio-tungstenite)
- **Auth**: Kinde JWT validation using JWKS from `https://<KINDE_DOMAIN>.kinde.com/.well-known/jwks`
- **Passwords**: bcrypt for room passwords
