---
title: Platform
tags:
  - llm-wiki
  - verified
status: verified
doc_type: domain
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/contract/src/lib.rs
  - crates/api/src/routes.rs
  - crates/api/src/implemented.rs
  - crates/api/src/body.rs
  - crates/api/src/lang.rs
  - crates/api/src/auth/mod.rs
  - crates/api/src/db.rs
evidence:
  - crates/contract/src/lib.rs#symbol:OPENAPI_JSON
  - crates/contract/src/lib.rs#symbol:operations
  - crates/contract/src/lib.rs#symbol:error_codes
  - crates/api/src/routes.rs#symbol:router
  - crates/api/src/routes.rs#symbol:contract_routes
  - crates/api/src/routes.rs#symbol:implemented
  - crates/api/src/routes.rs#symbol:healthz
  - crates/api/src/routes.rs#symbol:readyz
  - crates/api/src/implemented.rs#symbol:IMPLEMENTED
  - crates/api/src/implemented.rs#symbol:listed_in_contract_order
  - crates/api/src/body.rs#symbol:Body
  - crates/api/src/body.rs#symbol:Patch
  - crates/api/src/error.rs#L13-L23
  - crates/api/src/error.rs#L75-L81
  - crates/api/src/lang.rs#symbol:Lang
  - crates/api/src/auth/mod.rs#symbol:require
  - crates/api/src/state.rs#L13-L27
  - crates/api/src/db.rs#symbol:MIGRATOR
  - crates/api/src/main.rs#L33-L54
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/API_CONTRACTS.md
  - docs/llm-wiki/OPERATIONS.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Platform

## Responsibility

What every operation shares: routing from the contract, the handler pipeline (session, body, permission, rules), the error shape and language, application state and the database pool, migrations, and the `ax-api` command line. Configuration and deployment are in [Operations](../OPERATIONS.md); the contract workflow in [API Contracts](../API_CONTRACTS.md).

## Routing from the contract

- `ax-contract` embeds `contract/openapi.json`, `contract/lifecycle.json` and `contract/SOURCE` with `include_str!` and lists every `Operation { id, method, path }` (`operations`).
- `routes::contract_routes` routes **every** contract operation: the ones in `implemented::IMPLEMENTED` to their handler (the `match` in `routes::implemented`), the rest to `501` naming the operation. If the two lists disagree, building the router **panics** — and every test builds it, so the mismatch never ships. `listed_in_contract_order` keeps `IMPLEMENTED` in contract order. All 46 operations are implemented.
- Outside the contract, `routes::router` adds `GET /files/{id}` ([Drafts and Attachments](06_drafts_attachments.md)) and `POST /runner/…` ([Runner](11_runner.md)), a JSON `404` fallback, and request tracing. The API is nested under `AX_BASE_PATH` (default `/`); `/healthz` (version and pinned contract commit) and `/readyz` (`select 1`; `503` when the database is unreachable, `200 not_configured` without one) stay at the root.

## Handler pipeline

Handlers check in the mock API's order:

1. **Session** — the `CurrentUser` extractor (`401`) ([Auth and Sessions](01_auth_sessions.md)).
2. **Body** — `body::Body` parses a JSON object, collects every problem with its zod-style path and Korean and English text, and `finish()` answers one `400 checkInput` listing them all. Unknown fields are ignored, as the contract's schemas strip them. `Patch { Keep, Clear, Set }` gives `null` its PATCH meaning; lengths count UTF-16 units (`utf16_len`) like JavaScript.
3. **Permission** — `auth::require(&user, Permission)` (`403`) using `roles.rs` ([Users, Teams and Roles](02_users_teams_roles.md)).
4. **Rules** — the operation's own checks with the mock's messages.

Exceptions, each matching the mock: lifecycle operations load the request before deciding who may act ([Lifecycle](05_lifecycle.md)); uploads check the file before the permission; the dashboard and releases check the query before the permission.

## Errors and language

- Body `{ code, message, issues? }` (`ApiErrorBody`). `code` is one of nine (`ErrorCode`): `bad_request` 400, `unauthorized` 401, `forbidden` 403, `not_found` 404, `invalid_transition`/`guard_failed`/`conflict`/`limit_reached` 409, `internal` 500 — a unit test compares them with the contract. The specific case (the mock's message key such as `operatorProvisioned`) is carried by the localised `message`, not by `code`.
- `Lang` comes from `Accept-Language`: the supported language (`ko`, `en`) with the highest q wins, ties go to the earlier one, anything else is Korean. Every server text — errors, notifications, AI output — follows it. The texts mirror the frontend's `src/mocks/copy.ts`.

## State, database, CLI

- `AppState` holds an optional pool (no `DATABASE_URL` → the server starts and data operations answer `500`), the policies from configuration, the business time zone, the AI configuration, the file signer and, with `AX_AI_PROVIDER=anthropic`, the Claude client. Pool: 10 connections, 3 s acquire timeout, lazy connect.
- Migrations are embedded (`MIGRATOR`, `sqlx::migrate!`) and applied on start; sqlx's advisory lock makes concurrent starts safe. `build.rs` rebuilds when `migrations/` changes. An applied migration is never edited ([Data Model](../DATA_MODEL.md)).
- `ax-api` (`run`): no subcommand or `serve` migrates and serves (graceful shutdown on Ctrl-C/SIGTERM); `migrate`; `user create | set-password` (the only way to make an operator; passwords from a prompt or stdin, never an argument); `runner`.

## Evidence

- `crates/contract/src/lib.rs#symbol:OPENAPI_JSON`, `crates/contract/src/lib.rs#symbol:operations`, `crates/contract/src/lib.rs#symbol:error_codes` — the embedded contract.
- `crates/api/src/routes.rs#symbol:router`, `crates/api/src/routes.rs#symbol:contract_routes`, `crates/api/src/routes.rs#symbol:implemented`, `crates/api/src/routes.rs#symbol:healthz`, `crates/api/src/routes.rs#symbol:readyz` — routing.
- `crates/api/src/implemented.rs#symbol:IMPLEMENTED`, `crates/api/src/implemented.rs#symbol:listed_in_contract_order` — the implemented list.
- `crates/api/src/body.rs#symbol:Body`, `crates/api/src/body.rs#symbol:Patch`, `crates/api/src/auth/mod.rs#symbol:require` — the pipeline.
- `crates/api/src/error.rs#L13-L23` (`ErrorCode`), `crates/api/src/error.rs#L75-L81` (`ApiErrorBody`), `crates/api/src/lang.rs#symbol:Lang` — errors and language.
- `crates/api/src/state.rs#L13-L27` (`AppState`), `crates/api/src/db.rs#symbol:MIGRATOR`, `crates/api/src/main.rs#L33-L54` (`run`) — state, migrations, CLI.
- Tests: `crates/api/tests/http.rs` (`every_contract_operation_is_routed`, `signed_in_operations_need_a_bearer_token`, `the_login_body_is_checked_first`, `unknown_paths_get_the_contract_error_in_the_requested_language`, `a_base_path_moves_the_contract_but_not_the_probes`, `ready_without_a_database`).

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
