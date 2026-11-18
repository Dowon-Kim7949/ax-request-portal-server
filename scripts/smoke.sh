#!/usr/bin/env bash
# Checks a running stack end to end (after `docker compose -f deploy/compose.yaml up -d`):
# the API reaches PostgreSQL, nginx serves the app, /api/ on the web origin
# reaches the API with the contract's error body, and a person can sign in,
# look themselves up and sign out through it.
#
# Sign-in needs an account. The script provisions a fresh admin with a random
# password through the server's CLI (no fixed or demo credentials); each run
# adds one, which `docker compose ... down -v` clears.
#
# Usage: scripts/smoke.sh                     # defaults to the compose ports
#        API=http://host:8080 WEB=http://host:8088 scripts/smoke.sh
#        AX_CLI="ssh host docker exec -i ax-backend ax-api" scripts/smoke.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
API="${API:-http://localhost:8080}"
WEB="${WEB:-http://localhost:8088}"
read -r -a AX_CLI <<<"${AX_CLI:-docker compose -f $ROOT/deploy/compose.yaml exec -T ax-backend ax-api}"

fail() { echo "FAIL: $*" >&2; exit 1; }
status() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
# JSON string field of a flat response body, without needing jq.
field() { sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }

for _ in $(seq 1 60); do
  [[ "$(status "$API/readyz")" == "200" ]] && break
  sleep 1
done
curl -fsS "$API/readyz" | grep -q '"database":"ok"' || fail "the API does not reach the database"
curl -fsS "$API/healthz" | grep -q '"status":"ok"' || fail "/healthz is not ok"

curl -fsS "$WEB/" | grep -q '<div id="root">' || fail "nginx does not serve the app"
[[ "$(status "$WEB/mockServiceWorker.js")" == "404" ]] || fail "a real-backend build must not ship the mock service worker"

[[ "$(status "$WEB/api/users")" == "501" ]] || fail "/api/users should reach the API (501 until implemented)"
curl -s -H 'Accept-Language: en' "$WEB/api/users" | grep -q '"code":"internal"' || fail "/api/ errors must use the contract's body"
[[ "$(status "$WEB/api/no-such-route")" == "404" ]] || fail "unknown API paths must be 404"

# --- sign-in ---------------------------------------------------------------
email="smoke-$(date +%s)-$RANDOM@example.com"
password="$(head -c 24 /dev/urandom | base64 | tr -d '/+=\n')"
printf '%s\n' "$password" | "${AX_CLI[@]}" user create --email "$email" --name "Smoke Test" --role admin --password-stdin \
  >/dev/null || fail "could not provision an admin with the CLI (${AX_CLI[*]})"

login() { curl -s -H 'Content-Type: application/json' -H 'Accept-Language: en' -d "$1" "$WEB/api/auth/login"; }
me() { curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $1" "$WEB/api/auth/me"; }

session="$(login "{\"email\":\"$email\",\"password\":\"$password\"}")"
token="$(field token <<<"$session")"
[[ "$token" == axs_* ]] || fail "login did not return a session: $session"
[[ "$(field role <<<"$session")" == "admin" ]] || fail "login returned the wrong user: $session"

curl -fsS -H "Authorization: Bearer $token" "$WEB/api/auth/me" | grep -q "\"email\":\"$email\"" ||
  fail "/auth/me does not return the signed-in user"
[[ "$(me nope)" == "401" ]] || fail "/auth/me must be 401 for an unknown token"
[[ "$(status "$WEB/api/auth/me")" == "401" ]] || fail "/auth/me must be 401 without a token"

wrong="$(login "{\"email\":\"$email\",\"password\":\"not-the-password\"}")"
grep -q '"message":"The email or password is incorrect."' <<<"$wrong" || fail "a wrong password must be refused: $wrong"

[[ "$(status -X POST -H "Authorization: Bearer $token" "$WEB/api/auth/logout")" == "204" ]] || fail "logout must be 204"
[[ "$(me "$token")" == "401" ]] || fail "the token must stop working after logout"

echo "stack OK: $WEB -> $API (signed in, looked up and signed out $email)"
