---
title: Operations
tags:
  - llm-wiki
  - verified
status: verified
doc_type: operations
project: ax-request-portal-server
last_updated: 2026-11-27
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - scripts/cargo.sh
  - scripts/smoke.sh
  - deploy/api.Dockerfile
  - deploy/web.Dockerfile
  - deploy/runner.Dockerfile
evidence:
  - crates/api/src/config.rs#L1-L30
  - crates/api/src/config.rs#L236-L266
  - deploy/compose.yaml#L12-L24
  - deploy/compose.yaml#L77-L96
  - deploy/compose.yaml#L50-L54
  - README.md#section:Configuration
  - .github/workflows/ci.yml#L16-L45
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/project-profile.md
  - docs/llm-wiki/SECURITY.md
  - docs/llm-wiki/domains/11_runner.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-27
---

# Operations

## Configuration

Everything comes from environment variables (`deploy/.env`, from `deploy/.env.example`, git-ignored); a wrong value stops the server at start. A new setting goes into `.env.example`, `compose.yaml`, `config.rs` and README "Configuration" together.

| Setting | Default | Meaning |
| --- | --- | --- |
| `AX_BIND` | `0.0.0.0:8080` | listen address |
| `AX_BASE_PATH` | `/` | where the contract is mounted; the frontend's nginx strips `/api/`, so `/` is right behind it; use `/api` behind a proxy that keeps the prefix |
| `DATABASE_URL` | unset | PostgreSQL; unset starts without a database (data operations answer 500) |
| `AX_SESSION_IDLE` / `AX_SESSION_MAX` | `24h` / `7d` | session expiry ([Auth and Sessions](domains/01_auth_sessions.md)) |
| `AX_SETUP_LINK_TTL` | `72h` | setup link lifetime |
| `AX_LOGIN_ATTEMPTS` / `AX_LOGIN_LOCKOUT` | `5` / `15m` | login throttling |
| `AX_TIME_ZONE` | `Asia/Seoul` | business calendar; checked against PostgreSQL's zone names at start |
| `AX_AI_PROVIDER` | `stub` | `stub` or `anthropic` (needs a key) for reports; neither changes code — work needs the runner |
| `AX_SCOPE_CHECK` | `true` | ask the requester to confirm the AI's scope; keep equal to the frontend's `workflow.scopeCheck` |
| `AX_ANTHROPIC_API_KEY` (+ `_PRECHECK`, `_REPORT`, `_WORK`) | unset | secret; per-task keys override the shared one |
| `AX_RUNNER_TOKEN` | unset | secret, 32+ characters; set it and AI jobs wait for a runner |
| `AX_DRAFTS_PER_ACCOUNT` | `20` | draft cap; keep equal to the frontend's setting |
| `AX_UPLOAD_DIR` | `uploads` | attachment storage (the compose volume `uploads`) |
| `AX_PUBLIC_PATH` | `/api` | path the browser uses to reach this server, for attachment links |
| `AX_GITHUB_READ_TOKEN` | unset | reads a GitHub repository's branches for the project form; Contents read only, not the runner's token; unset, the form takes a typed branch name |
| `AX_GITHUB_API` | `https://api.github.com` | GitHub's REST API, for the branch list here and for the runner |

Runner settings (`AX_RUNNER_SERVER`, `AX_GITHUB_TOKEN`, `AX_GITHUB_API`, `AX_RUNNER_WORKDIR`, `AX_RUNNER_TIMEOUT`, `AX_CLAUDE_BIN`) are in [Runner](domains/11_runner.md). Values that must match the frontend's `appConfig` (time zone, scope check, draft cap) are not checked by code.

## The compose stack (`deploy/compose.yaml`)

- `db` — PostgreSQL 18 (alpine) with a health check and the `db-data` volume.
- `ax-backend` — the API (`deploy/api.Dockerfile`: release build on the pinned Rust image, Debian slim runtime, non-root user). Published on `127.0.0.1` only, at host port `AX_API_PORT` (default `8080`). The service name is fixed: the frontend's nginx proxies `/api/` to `http://ax-backend:8080/`.
- `web` — the frontend built at `FRONTEND_REF` (the same commit as `contract/SOURCE`) against this API (`deploy/web.Dockerfile`: mock off, nginx), open to the network at host port `AX_WEB_PORT` (default `8088`). Both ports are Compose-only settings in `deploy/.env`, for a host where another service already holds the default; `scripts/smoke.sh` then takes `WEB=` and `API=`.
- `ax-runner` — only with `--profile runner` ([Runner](domains/11_runner.md)).

`docker compose -f deploy/compose.yaml up -d --build --wait && bash scripts/smoke.sh` brings it up and smoke-tests probes, static serving, the `/api` proxy, the error shape, and a sign-in with a CLI-made admin (random password; no fixed accounts). Each run also adds accounts, a team, a project and a request that cannot be deleted (people and teams never are), so run it against a throwaway stack, not a live one; on a live one check the probes, the web root and `/api/` by hand.

## Accounts

There are no seeded accounts. The first admin (and every operator) comes from `ax-api user create --email … --name … --role admin` with the password on a prompt or stdin; `ax-api user set-password` resets one. In compose: `docker compose -f deploy/compose.yaml exec ax-backend ax-api user create …`.

## Backup and restore

A deployment's state is the database, the uploaded files and its `deploy/.env`; the images are rebuilt from the repository. Back up all three, and keep the copies private: the dump holds password digests and the attachment link key (`server_secrets`), and `.env` holds the provider keys and tokens.

From the repository directory on the host (`C` = `docker compose -f deploy/compose.yaml`):

1. Database: `$C exec -T db pg_dump -U ax -d ax -Fc > ax-$(date +%F).dump` — one consistent snapshot while the stack runs.
2. Uploads: `$C exec -T ax-backend tar czf - -C /var/lib/ax/uploads . > uploads-$(date +%F).tgz`.
3. Settings: a copy of `deploy/.env`, kept apart from the dumps.

Restore onto a stack started with the same `.env`:

1. Stop what writes: `$C stop ax-runner ax-backend`.
2. Database: `$C exec -T db pg_restore -U ax -d ax --clean --if-exists --no-owner < ax-<date>.dump`.
3. Uploads: `$C start ax-backend`, then `$C exec -T ax-backend tar xzf - -C /var/lib/ax/uploads < uploads-<date>.tgz`.
4. `$C --profile runner up -d --wait`; check `/readyz` and a sign-in.

To try a dump without touching the live database, restore it into a scratch database (`createdb`, `pg_restore -d <scratch>`, then `dropdb`). Scheduling backups (cron, copies off the host) is the deployment's own; this repository sets none up.

## Development and CI

- `scripts/cargo.sh` runs cargo in `deploy/dev.Dockerfile`'s image with cached registry and target volumes; for `test` it starts a throwaway PostgreSQL (`ax-server-test-db`, left running afterwards). On Git Bash it disables path conversion.
- `.github/workflows/ci.yml`: `fmt --check`, `clippy -D warnings`, `test` against a PostgreSQL service; the contract pin check; gitleaks over the whole history; the compose stack with the smoke test. `.github/workflows/llm-wiki.yml` checks this wiki.
- Probes: `/healthz` (version and contract commit) and `/readyz` (database reachable) at the root regardless of `AX_BASE_PATH`.

## Evidence

- `crates/api/src/config.rs#L1-L30` — the server settings table; `crates/api/src/config.rs#L236-L266` — the runner's.
- `deploy/compose.yaml#L12-L24` — the database service; `deploy/compose.yaml#L77-L96` — the runner service.
- `deploy/api.Dockerfile`, `deploy/web.Dockerfile`, `deploy/runner.Dockerfile` — images.
- `scripts/cargo.sh`, `scripts/smoke.sh` — development and smoke test.
- `.github/workflows/ci.yml#L16-L45` — the Rust job.
- `README.md#section:Configuration` — the documented settings.

## Open Questions

- A deployment's own settings (host, ports, keys) live in its `deploy/.env` on that host, never in this repository; how often it backs up, and where the copies go, is decided there.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
