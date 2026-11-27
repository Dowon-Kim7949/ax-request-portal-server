---
title: Architecture Conventions
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: architecture_conventions
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/lib.rs
  - crates/api/src/routes.rs
  - crates/api/src/implemented.rs
evidence:
  - crates/api/src/routes.rs#symbol:contract_routes
  - crates/api/src/implemented.rs#symbol:IMPLEMENTED
  - CLAUDE.md#section:Contract first
  - CLAUDE.md#section:Rules
  - README.md#section:Layout
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/domains/12_platform.md
  - docs/llm-wiki/API_CONTRACTS.md
  - docs/llm-wiki/DATA_MODEL.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Architecture Conventions

## Layout

- `crates/contract` — `ax-contract`: the frontend's contract files embedded at build time, with helpers to list operations, schemas and enums.
- `crates/api/src` — one module per domain (`auth`, `users`, `teams`, `projects`, `requests`, `drafts`, `attachments`, `notifications`, `dashboard`, `ai`, `runner`) plus shared modules (`routes`, `implemented`, `body`, `validate`, `error`, `lang`, `roles`, `config`, `state`, `db`, `calendar`, `cli`, `accounts`). A domain module usually splits into `mod.rs` (types, storage, rules) and `handlers.rs` (axum handlers).
- `crates/api/migrations` — sqlx migrations, embedded into the binary.
- `crates/api/tests` — integration tests per domain against a real PostgreSQL (`#[sqlx::test]`); `tests/common` builds the app and signs people in.
- `contract/` — byte-for-byte copies of the frontend's `docs/openapi.json` and `docs/lifecycle.json`, pinned by `contract/SOURCE`.
- `deploy/`, `scripts/` — images, compose stack, cargo-in-Docker, smoke test, contract sync ([Operations](OPERATIONS.md)).

## Conventions an agent must keep

1. **Contract first.** Never edit `contract/`. An API change starts in the frontend's `src/api/contract.ts`, is merged there, then `bash scripts/sync-contract.sh main` pins it here ([API Contracts](API_CONTRACTS.md)).
2. **Port, don't invent.** Domain rules come from the frontend's `src/domain/` (`lifecycle.ts`, `roles.ts`, `ai.ts`), its README "Connect a real backend", and the behaviour of its mock API (`src/mocks/engine.ts`), including message texts (`src/mocks/copy.ts`). Tests are named after the mock behaviour they pin (`…_follows_the_mock_apis_rules`).
3. **Every operation is routed.** A new handler means adding its `operationId` to `implemented::IMPLEMENTED` (in contract order) **and** an arm to `routes::implemented`; building the router panics if they disagree. Unimplemented operations answer `501`.
4. **Handler order**: session (`CurrentUser`, 401) → body (`body::Body`, one 400 listing every issue) → permission (`auth::require`, 403) → the operation's rules with the mock's messages. Where the mock differs (request-dependent permissions, file and query checks first), follow the mock and say so in the module ([Platform](domains/12_platform.md)).
5. **Permissions on the server for every operation**, from `roles.rs`. Operator accounts only from the `ax-api user` CLI.
6. **Schema changes are new migration files**; an applied migration is never edited. Teams and users are never deleted; reference them with `ON DELETE RESTRICT`. AI usage rows copy the caller's `team_id` at call time ([Data Model](DATA_MODEL.md)).
7. **One transaction per state change**: status, timeline, notifications and job enqueue commit together, under a row lock on the request.
8. **Server text follows `Accept-Language`** (`ko` | `en`); every message is written in both languages at the call site.
9. **The AI never merges, never approves its own wiki edits, and never moves a request past a human checkpoint** ([Lifecycle](domains/05_lifecycle.md#human-checkpoints), [Runner](domains/11_runner.md)).
10. **A new setting** goes into `deploy/.env.example`, `deploy/compose.yaml`, `config.rs` and the README "Configuration" section together ([Operations](OPERATIONS.md)).
11. **Nothing organisation-specific** in the repository: no internal hosts, project or customer names, real data, keys or passwords. Commits use the GitHub noreply author.

## Evidence

- `crates/api/src/routes.rs#symbol:contract_routes`, `crates/api/src/implemented.rs#symbol:IMPLEMENTED` — the routing rule.
- `crates/api/src/lib.rs` — the module list.
- `CLAUDE.md#section:Contract first`, `CLAUDE.md#section:Rules`, `README.md#section:Layout` — the repository's own rules and layout.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
