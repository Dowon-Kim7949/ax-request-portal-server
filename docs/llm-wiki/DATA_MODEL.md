---
title: Data Model
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: data_model
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/migrations
  - crates/api/src/db.rs
evidence:
  - crates/api/src/db.rs#symbol:MIGRATOR
  - crates/api/migrations/20261118000000_users_teams_sessions.sql
  - crates/api/migrations/20261119000001_requests.sql
  - crates/api/migrations/20261119000006_ai_settings_and_usage.sql
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/domains/04_requests.md
  - docs/llm-wiki/domains/09_ai_jobs.md
visibility: public
contains_sensitive_info: false
---

# Data Model

PostgreSQL 18, managed by sqlx migrations embedded in the binary and applied on start (`db::MIGRATOR`). Applied migrations are never edited; every change is a new file in `crates/api/migrations/`.

## Principles

- Ids are `uuid` (`gen_random_uuid()`), an identity `bigint` where people see a number (requests) or need an order (`seq` columns), or a prefixed text id where the contract shows one (`drf_…`, `att_…`, `ai_…`). Times are `timestamptz`.
- **Columns for what is queried, JSONB for the rest** in the contract's camelCase shape (`requests.details`, `drafts.content`, `projects.targets`/`rules`, `ai_config.value`, `notifications.params`), each with a `jsonb_typeof` check.
- **Teams and users are never deleted**; everything referencing them uses `ON DELETE RESTRICT`. `CASCADE` is used only for rows owned by their parent: sessions and setup links (by user), project owners (by project), timeline events, notifications and AI jobs (by request).
- Tokens are stored as SHA-256 digests (`sessions`, `setup_links`), never as issued.
- Counts and limits that matter for correctness are enforced by constraints and locks as well as code (unique lower-cased emails and team names, one open job per request and task, one setup link per user, `unique (owner_id, client_id)` for drafts).

## Tables

| Migration | Tables | Notes |
| --- | --- | --- |
| `20261118000000_users_teams_sessions` | `teams`, `users`, `sessions` | five roles by check; nullable `password_hash`; `expires_at <= absolute_expires_at` |
| `20261118000001_setup_links` | `setup_links` | primary key `user_id` (one link per person) |
| `20261119000000_projects` | `projects`, `project_owners` | owners ordered by `position`; targets and rules JSONB with stable ids |
| `20261119000001_requests` | `requests`, `request_events` | 13 statuses and 6 types by check; `details` JSONB; append-only timeline with `actor_type` `user/ai/system` |
| `20261119000002_notifications` | `notifications`, `notification_settings` | key + params, rendered on read |
| `20261119000003_ai_jobs` | `ai_jobs` | `report`/`work`, `queued/running/done/failed`, partial unique index for one open job per `(request, task)` |
| `20261119000004_attachments` | `attachments`, `server_secrets` | files on disk, metadata here; the link-signing key |
| `20261119000005_drafts` | `drafts` | `content` JSONB, `version` for optimistic updates |
| `20261119000006_ai_settings_and_usage` | `ai_config`, `ai_usage` | three JSON settings rows; the never-deleted ledger with `team_id` copied at call time |
| `20261119000007_login_attempts` | `login_attempts` | per lower-cased email |
| `20261119000008_runner_claims` | (`ai_jobs` columns) | `claim`, `lease_until` for the runner |

Where each table is used: [Auth and Sessions](domains/01_auth_sessions.md), [Users, Teams and Roles](domains/02_users_teams_roles.md), [Projects](domains/03_projects.md), [Requests](domains/04_requests.md), [Notifications](domains/07_notifications.md), [AI Jobs](domains/09_ai_jobs.md), [Drafts and Attachments](domains/06_drafts_attachments.md), [AI Settings and Budget](domains/10_ai_settings_budget.md), [Runner](domains/11_runner.md).

## Working with the schema

- `#[sqlx::test]` gives each database test a fresh migrated database; `bash scripts/cargo.sh test` starts the PostgreSQL it needs.
- `build.rs` makes cargo rebuild when a migration changes, so the embedded set never goes stale.
- Business dates (months, "today") are computed in SQL with `at time zone` the configured `AX_TIME_ZONE`, not in the database's own time zone.

## Evidence

- `crates/api/src/db.rs#symbol:MIGRATOR` — embedded migrations.
- `crates/api/migrations/20261118000000_users_teams_sessions.sql`, `crates/api/migrations/20261119000001_requests.sql`, `crates/api/migrations/20261119000006_ai_settings_and_usage.sql` — the `RESTRICT`/`CASCADE` choices and the ledger's team copy, explained in their comments.
- `crates/api/migrations` — every table.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
