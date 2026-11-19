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

[[ "$(status "$WEB/api/drafts")" == "401" ]] || fail "/api/drafts should reach the API (401 without a sign-in)"
curl -s -H 'Accept-Language: en' "$WEB/api/drafts" | grep -q '"code":"unauthorized"' || fail "/api/ errors must use the contract's body"
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

# A project, owned by the admin.
admin_id="$(field id <<<"$session")"
project="{\"name\":\"Smoke $stamp\",\"description\":\"\",\"icon\":\"box\",\"repository\":{\"provider\":\"gitlab\",\"url\":\"https://git.example.com/smoke\"},\"wikiPath\":\"docs/llm-wiki\",\"ownerIds\":[\"$admin_id\"],\"targets\":[{\"name\":\"Home\"}],\"constraints\":[],\"active\":true}"
r="$(call -X POST -d "$project" "$WEB/api/projects")"
[[ "$(code "$r")" == "201" ]] || fail "createProject: $r"
call "$WEB/api/projects" | grep -q "\"name\":\"Smoke $stamp\"" || fail "listProjects does not list the new project"
project_id="$(json "$r" | sed -n 's/^{"id":"\([^"]*\)".*/\1/p')"

# An attachment, uploaded through nginx and served back through its signed link.
# The file goes in on standard input, which every curl build reads the same way.
r="$(printf 'smoke %s' "$stamp" | curl -s -w '
%{http_code}' -H "Authorization: Bearer $token" -H 'Accept-Language: en'   -F "file=@-;filename=smoke.png;type=image/png" "$WEB/api/attachments")"
[[ "$(code "$r")" == "201" ]] || fail "uploadAttachment: $r"
attachment_id="$(json "$r" | field id)"
attachment_url="$(json "$r" | field url)"
[[ "$attachment_url" == /api/files/* ]] || fail "the attachment link does not go through /api: $r"
[[ "$(curl -fsS "$WEB$attachment_url")" == "smoke $stamp" ]] || fail "the attachment link does not serve the file"
curl -fsS -o /dev/null -D - "$WEB$attachment_url" | grep -qi '^content-type: image/png' || fail "the attachment is not served as image/png"
[[ "$(status "$WEB${attachment_url}x")" == "404" ]] || fail "an altered attachment link must be 404"

# A saved draft of the request, deleted when the request is filed from it.
r="$(call -X POST -d "{\"step\":6,\"content\":{\"title\":\"Smoke request $stamp\"},\"clientId\":\"smoke-$stamp\"}" "$WEB/api/drafts")"
[[ "$(code "$r")" == "201" ]] || fail "createDraft: $r"
draft_id="$(json "$r" | field id)"
call "$WEB/api/drafts" | grep -q "\"id\":\"$draft_id\"" || fail "listDrafts does not list the new draft"

# A request in that project, with the attachment: filed, then the AI reports on it.
req="{\"projectId\":\"$project_id\",\"type\":\"non_code\",\"title\":\"Smoke request $stamp\",\"targetIds\":[],\"currentBehavior\":\"\",\"desiredBehavior\":\"A smoke test files a request\",\"acceptanceCriteria\":[{\"text\":\"When the smoke test runs, the request is listed\"}],\"attachmentIds\":[\"$attachment_id\"],\"draftId\":\"$draft_id\"}"
r="$(call -X POST -d "$req" "$WEB/api/requests")"
[[ "$(code "$r")" == "201" ]] && grep -q '"status":"reporting"' <<<"$r" || fail "createRequest: $r"
grep -q "\"attachments\":\[{\"id\":\"$attachment_id\"" <<<"$r" || fail "the request does not carry its attachment: $r"
[[ "$(code "$(call "$WEB/api/drafts/$draft_id")")" == "404" ]] || fail "filing the request did not delete its draft"
request_id="$(json "$r" | sed -n 's/^{"id":\([0-9]*\),.*/\1/p')"
[[ "$(code "$(call "$WEB/api/requests/$request_id")")" == "200" ]] || fail "getRequest #$request_id"
call "$WEB/api/requests?scope=mine" | grep -q "\"title\":\"Smoke request $stamp\"" || fail "listRequests does not list the new request"
# The AI worker (the stub provider until one is wired) writes the report.
for _ in $(seq 1 15); do
  r="$(call "$WEB/api/requests/$request_id")"
  grep -q '"status":"review"' <<<"$r" && break
  sleep 1
done
grep -q '"status":"review"' <<<"$r" && grep -q '"verdict":"undetermined"' <<<"$r" ||
  fail "the AI worker did not report on request #$request_id: $r"
r="$(call -X POST -d '{"event":"cancel","reason":"smoke test"}' "$WEB/api/requests/$request_id/transitions")"
[[ "$(code "$r")" == "200" ]] && grep -q '"status":"cancelled"' <<<"$r" || fail "transitionRequest (cancel): $r"
[[ "$(code "$(call "$WEB/api/notifications")")" == "200" ]] || fail "listNotifications"
[[ "$(code "$(call "$WEB/api/me/notification-settings")")" == "200" ]] || fail "getNotificationSettings"
call "$WEB/api/dashboard/summary" | grep -q '"received":' || fail "the dashboard does not answer"
[[ "$(code "$(call "$WEB/api/releases")")" == "200" ]] || fail "the release schedule does not answer"

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

echo "stack OK: $WEB -> $API (signed in as $email; added a team, a project, an attachment, a draft, a request (reported on by the AI worker, then cancelled) and a person, who set a password through a setup link; signed out)"
