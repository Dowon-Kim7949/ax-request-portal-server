#!/usr/bin/env bash
# Runs cargo in the pinned Rust toolchain image, so no local Rust install is
# needed. Registry and build output live in Docker volumes, which keeps
# rebuilds fast on Windows and macOS bind mounts.
#
# `cargo test` also gets a throwaway PostgreSQL (tmpfs, container
# ax-server-test-db) and DATABASE_URL pointing at it: the database tests need
# one, and #[sqlx::test] makes a fresh database per test on that server.
# Stop it with: docker rm -f ax-server-test-db
#
# Usage: scripts/cargo.sh test
#        scripts/cargo.sh clippy --all-targets -- -D warnings
set -euo pipefail

IMAGE="ax-server-dev:1.100"
NETWORK="ax-server-dev"
TEST_DB="ax-server-test-db"
TEST_DB_IMAGE="postgres:18-alpine"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Git Bash on Windows: hand Docker a Windows path and stop MSYS from
# rewriting the container paths below.
if HOST_ROOT="$(cd "$ROOT" && pwd -W 2>/dev/null)"; then
  export MSYS_NO_PATHCONV=1
else
  HOST_ROOT="$ROOT"
fi

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  docker build -q -t "$IMAGE" -f "$HOST_ROOT/deploy/dev.Dockerfile" "$HOST_ROOT/deploy" >/dev/null
fi

tty=()
[[ -t 1 ]] && tty=(-t)

db=()
if [[ "${1:-}" == "test" ]]; then
  docker network inspect "$NETWORK" >/dev/null 2>&1 || docker network create "$NETWORK" >/dev/null
  if [[ "$(docker container inspect -f '{{.State.Running}}' "$TEST_DB" 2>/dev/null)" != "true" ]]; then
    docker rm -f "$TEST_DB" >/dev/null 2>&1 || true
    # A local, throwaway server on a private network: the password is not a secret.
    docker run -d --name "$TEST_DB" --network "$NETWORK" --tmpfs /var/lib/postgresql \
      -e POSTGRES_USER=ax -e POSTGRES_PASSWORD=ax -e POSTGRES_DB=ax "$TEST_DB_IMAGE" >/dev/null
  fi
  # Over TCP: during first start the entrypoint runs a socket-only server, then restarts.
  ready=""
  for _ in $(seq 1 30); do
    if docker exec "$TEST_DB" pg_isready -q -h 127.0.0.1 -U ax -d ax; then ready=1; break; fi
    sleep 1
  done
  [[ -n "$ready" ]] || { echo "the test database ($TEST_DB) did not start" >&2; exit 1; }
  db=(--network "$NETWORK" -e "DATABASE_URL=postgres://ax:ax@$TEST_DB:5432/ax")
fi

exec docker run --rm "${tty[@]}" "${db[@]}" \
  -v "$HOST_ROOT:/work" \
  -v ax-server-cargo-registry:/usr/local/cargo/registry \
  -v ax-server-target:/work/target \
  -w /work \
  -e CARGO_TERM_COLOR=always \
  "$IMAGE" cargo "$@"
