#!/usr/bin/env bash
# Checks a running stack end to end (after `docker compose -f deploy/compose.yaml up -d`):
# the API reaches PostgreSQL, nginx serves the app, and /api/ on the web origin
# reaches the API with the contract's error body.
#
# Usage: scripts/smoke.sh                     # defaults to the compose ports
#        API=http://host:8080 WEB=http://host:8088 scripts/smoke.sh
set -euo pipefail

API="${API:-http://localhost:8080}"
WEB="${WEB:-http://localhost:8088}"

fail() { echo "FAIL: $*" >&2; exit 1; }
status() { curl -s -o /dev/null -w '%{http_code}' "$1"; }

for _ in $(seq 1 60); do
  [[ "$(status "$API/readyz")" == "200" ]] && break
  sleep 1
done
curl -fsS "$API/readyz" | grep -q '"database":"ok"' || fail "the API does not reach the database"
curl -fsS "$API/healthz" | grep -q '"status":"ok"' || fail "/healthz is not ok"

curl -fsS "$WEB/" | grep -q '<div id="root">' || fail "nginx does not serve the app"
[[ "$(status "$WEB/mockServiceWorker.js")" == "404" ]] || fail "a real-backend build must not ship the mock service worker"

[[ "$(status "$WEB/api/auth/me")" == "501" ]] || fail "/api/auth/me should reach the API (501 until implemented)"
curl -s -H 'Accept-Language: en' "$WEB/api/auth/me" | grep -q '"code":"internal"' || fail "/api/ errors must use the contract's body"
[[ "$(status "$WEB/api/no-such-route")" == "404" ]] || fail "unknown API paths must be 404"

echo "stack OK: $WEB -> $API"
