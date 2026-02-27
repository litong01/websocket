#!/usr/bin/env bash
# Usage: ./check-and-test.sh [check|test|build|run|all]
#   check  - run cargo check (default if no arg)
#   test   - run cargo test
#   build  - build container image (amd64 + arm64 via buildx)
#   run    - run the app container with .env.local
#   (no arg or "all") - run check then test
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BUILDER_IMAGE="scorelib-builder:latest"
APP_IMAGE_NAME="${APP_IMAGE_NAME:-websocket-server}"
APP_IMAGE_TAG="${APP_IMAGE_TAG:-latest}"

run_check() {
  echo "==> Running cargo check..."
  docker run --rm -v "$(pwd)":/workspace -w /workspace "$BUILDER_IMAGE" cargo check
}

run_test() {
  echo "==> Running cargo test..."
  docker run --rm -v "$(pwd)":/workspace -w /workspace "$BUILDER_IMAGE" cargo test
}

run_build() {
  echo "==> Building container image..."
  if ! docker buildx version &>/dev/null; then
    echo "Error: docker buildx is required. Create a builder: docker buildx create --use"
    exit 1
  fi

  if [[ -n "${REGISTRY:-}" ]]; then
    # Multi-arch build and push to registry
    echo "   platforms: linux/amd64, linux/arm64"
    echo "   pushing to ${REGISTRY}/${APP_IMAGE_NAME}:${APP_IMAGE_TAG}"
    docker buildx build \
      --platform linux/amd64,linux/arm64 \
      -t "${REGISTRY}/${APP_IMAGE_NAME}:${APP_IMAGE_TAG}" \
      --push \
      .
    echo ""
    echo "==> Pushed ${REGISTRY}/${APP_IMAGE_NAME}:${APP_IMAGE_TAG} (amd64 + arm64)"
  else
    # Single-arch build for current platform and load into Docker
    local platform
    platform="$(docker version --format '{{.Server.Os}}/{{.Server.Arch}}' 2>/dev/null || echo 'linux/amd64')"
    echo "   platform: ${platform} (set REGISTRY= to push multi-arch instead)"
    docker buildx build \
      --platform "${platform}" \
      -t "${APP_IMAGE_NAME}:${APP_IMAGE_TAG}" \
      --load \
      .
    echo ""
    echo "==> Image built and loaded: ${APP_IMAGE_NAME}:${APP_IMAGE_TAG}"
  fi
}

# Read PORT from .env.local (default 8080) for -p mapping
get_port_from_env_file() {
  local f="${1:-.env.local}"
  if [[ -r "$f" ]]; then
    local p
    p=$(grep -E '^PORT=[0-9]+' "$f" 2>/dev/null | head -n1 | cut -d= -f2)
    echo "${p:-8080}"
  else
    echo "8080"
  fi
}

run_app() {
  if [[ ! -f .env.local ]]; then
    echo "Error: .env.local not found. Create it with at least KINDE_DOMAIN=..."
    exit 1
  fi
  local port
  port=$(get_port_from_env_file .env.local)
  echo "==> Starting ${APP_IMAGE_NAME}:${APP_IMAGE_TAG} (env: .env.local, port ${port})..."
  docker run -d --name "${APP_IMAGE_NAME}" --rm -p "${port}:${port}" --env-file .env.local "${APP_IMAGE_NAME}:${APP_IMAGE_TAG}"
}

case "${1:-all}" in
  check)
    run_check
    echo ""
    echo "==> Done (check passed)."
    ;;
  test)
    run_test
    echo ""
    echo "==> Done (tests passed)."
    ;;
  build)
    run_build
    ;;
  run)
    run_app
    ;;
  all|"")
    run_check
    echo ""
    run_test
    echo ""
    echo "==> Done (check + tests passed)."
    ;;
  *)
    echo "Usage: $0 [check|test|build|run|all]"
    echo "  check  - cargo check"
    echo "  test   - cargo test"
    echo "  build  - build container image (host arch, loaded). Set REGISTRY=<host> to push multi-arch."
    echo "  run    - run the app container with .env.local (run 'build' first if needed)"
    echo "  all    - check then test (default)"
    exit 1
    ;;
esac
