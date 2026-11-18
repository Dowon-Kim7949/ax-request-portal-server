#!/usr/bin/env bash
# Runs cargo in the pinned Rust toolchain image, so no local Rust install is
# needed. Registry and build output live in Docker volumes, which keeps
# rebuilds fast on Windows and macOS bind mounts.
#
# Usage: scripts/cargo.sh test
#        scripts/cargo.sh clippy --all-targets -- -D warnings
set -euo pipefail

IMAGE="ax-server-dev:1.100"
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

exec docker run --rm "${tty[@]}" \
  -v "$HOST_ROOT:/work" \
  -v ax-server-cargo-registry:/usr/local/cargo/registry \
  -v ax-server-target:/work/target \
  -w /work \
  -e CARGO_TERM_COLOR=always \
  "$IMAGE" cargo "$@"
