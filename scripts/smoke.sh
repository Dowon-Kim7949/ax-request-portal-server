#!/usr/bin/env bash
# Checks a running stack end to end (after `docker compose -f deploy/compose.yaml up -d`):
# the API reaches PostgreSQL, nginx serves the app, /api/ on the web origin
# reaches the API with the contract's error body, and a person can sign in,
# manage people and teams, let a new person set a password through a setup
# link, and sign out through it.
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

[[ "$(status "$WEB/api/projects")" == "501" ]] || fail "/api/projects should reach the API (501 until implemented)"
curl -s -H 'Accept-Language: en' "$WEB/api/projects" | grep -q '"code":"internal"' || fail "/api/ errors must use the contract's body"
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

# --- people and teams --------------------------------------------------------
auth=(-H "Authorization: Bearer $token" -H 'Content-Type: application/json' -H 'Accept-Language: en')
call() { curl -s -w '
%{http_code}' "${auth[@]}" "$@"; }   # body, then the status on its own line
code() { tail -n 1 <<<"$1"; }
json() { sed '$d' <<<"$1"; }

stamp="$(date +%s)-$RANDOM"
r="$(call -X POST -d "{\"name\":\"Smoke $stamp\"}" "$WEB/api/teams")"
[[ "$(code "$r")" == "201" ]] || fail "createTeam: $r"
team_id="$(json "$r" | field id)"

member="smoke-member-$stamp@example.com"
r="$(call -X POST -d "{\"name\":\"Smoke Member\",\"email\":\"$member\",\"role\":\"requester\",\"teamId\":\"$team_id\"}" "$WEB/api/users")"
[[ "$(code "$r")" == "201" ]] || fail "createUser: $r"
member_id="$(json "$r" | field id)"
[[ "$(json "$r" | field teamId)" == "$team_id" ]] || fail "createUser did not put the person in the team: $r"

call "$WEB/api/users" | grep -q "\"email\":\"$member\"" || fail "listUsers does not list the new person"
call "$WEB/api/teams" | grep -q "\"name\":\"Smoke $stamp\"" || fail "listTeams does not list the new team"

# The person sets a password through a one-time setup link, then signs in with it.
r="$(call -X POST "$WEB/api/users/$member_id/setup-link")"
[[ "$(code "$r")" == "201" ]] || fail "createSetupLink: $r"
link="$(json "$r" | field token)"
member_password="$(head -c 24 /dev/urandom | base64 | tr -d '/+=\n')"
setup() { curl -s -w '\n%{http_code}' -H 'Content-Type: application/json' -H 'Accept-Language: en' \
  -d "{\"token\":\"$link\",\"password\":\"$member_password\"}" "$WEB/api/auth/setup"; }
r="$(setup)"
[[ "$(code "$r")" == "200" ]] && [[ "$(json "$r" | field token)" == axs_* ]] || fail "completeSetup: $r"
[[ "$(code "$(setup)")" == "401" ]] || fail "a setup link must work once"
r="$(login "{\"email\":\"$member\",\"password\":\"$member_password\"}")"
[[ "$(field role <<<"$r")" == "requester" ]] || fail "the password set through the link does not sign in: $r"

r="$(call -X PATCH -d '{"active":false,"teamId":null}' "$WEB/api/users/$member_id")"
[[ "$(code "$r")" == "200" ]] && grep -q '"active":false' <<<"$r" || fail "updateUser: $r"

r="$(call -X POST -d "{\"name\":\"Op\",\"email\":\"op-$stamp@example.com\",\"role\":\"operator\"}" "$WEB/api/users")"
[[ "$(code "$r")" == "403" ]] || fail "an admin must not create an operator: $r"

[[ "$(status -X POST -H "Authorization: Bearer $token" "$WEB/api/auth/logout")" == "204" ]] || fail "logout must be 204"
[[ "$(me "$token")" == "401" ]] || fail "the token must stop working after logout"

echo "stack OK: $WEB -> $API (signed in as $email; added a team and a person, who set a password through a setup link; signed out)"
