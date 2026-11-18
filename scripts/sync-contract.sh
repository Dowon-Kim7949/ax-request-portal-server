#!/usr/bin/env bash
# Pins the API contract to a frontend commit.
#
# The contract is the frontend's docs/openapi.json (generated there from
# src/api/contract.ts). This script copies it byte for byte, records the commit
# in contract/SOURCE, and points the compose demo's frontend build at the same
# commit, so the server, its tests and the demo always agree on one contract.
#
# Usage: scripts/sync-contract.sh <ref>     # branch, tag or commit of the frontend
#        scripts/sync-contract.sh --check   # CI: the copy and compose match contract/SOURCE
set -euo pipefail

REPO="Dowon-Kim7949/ax-request-portal"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/contract/SOURCE"
CONTRACT="$ROOT/contract/openapi.json"
COMPOSE="$ROOT/deploy/compose.yaml"

fail() { echo "FAIL: $*" >&2; exit 1; }
raw() { curl -fsSL "https://raw.githubusercontent.com/$REPO/$1/docs/openapi.json"; }
pinned() { sed -n 's/^commit=//p' "$SOURCE"; }

if [[ "${1:-}" == "--check" ]]; then
  sha="$(pinned)"
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || fail "contract/SOURCE must pin a full commit SHA, got '$sha'"
  raw "$sha" | cmp -s - "$CONTRACT" || fail "contract/openapi.json differs from $REPO@$sha (edit the frontend, then re-run this script)"
  grep -q "FRONTEND_REF:-$sha}" "$COMPOSE" || fail "deploy/compose.yaml builds a different frontend commit than contract/SOURCE"
  echo "contract matches $REPO@$sha"
  exit 0
fi

ref="${1:?usage: sync-contract.sh <frontend-ref> | --check}"
sha="$(curl -fsSL -H 'Accept: application/vnd.github.sha' "https://api.github.com/repos/$REPO/commits/$ref")"
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || fail "could not resolve '$ref' to a commit"

raw "$sha" > "$CONTRACT.tmp"
mv "$CONTRACT.tmp" "$CONTRACT"
printf 'repo=https://github.com/%s\ncommit=%s\npath=docs/openapi.json\n' "$REPO" "$sha" > "$SOURCE"
sed -i.bak -E "s/(FRONTEND_REF:-)[0-9a-f]{40}\}/\1$sha}/" "$COMPOSE" && rm -f "$COMPOSE.bak"

echo "pinned the contract to $REPO@$sha"
echo "next: cargo test (new operations answer 501 until implemented), then commit contract/ and deploy/compose.yaml"
