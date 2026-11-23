---
title: Projects
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: domain
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/projects
  - crates/api/migrations/20261119000000_projects.sql
  - crates/api/tests/projects.rs
evidence:
  - crates/api/src/projects/handlers.rs#symbol:ProjectInput
  - crates/api/src/projects/handlers.rs#symbol:ICONS
  - crates/api/src/projects/handlers.rs#symbol:PROVIDERS
  - crates/api/src/projects/handlers.rs#symbol:CONSTRAINTS_MAX
  - crates/api/src/projects/handlers.rs#symbol:check_owners
  - crates/api/src/projects/handlers.rs#symbol:set_owners
  - crates/api/src/projects/handlers.rs#symbol:update
  - crates/api/src/projects/mod.rs#symbol:target_ids
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/04_requests.md
  - docs/llm-wiki/domains/11_runner.md
visibility: public
contains_sensitive_info: false
---

# Projects

## Responsibility

The systems requests are made against: name, icon, repository, wiki path, owners (developers in order), targets (screens or parts a request can point at) and constraints (rules the AI report checks a request against). Admins manage them; everyone signed in can list them.

## Behaviour

- `updateProject` is a **`PUT`** that replaces the whole project, not a patch. The owner check runs before the existence check, so a bad owner on an unknown project is `400`, not `404`.
- **Owners** must be existing `developer` or `admin` users (`400 ownersMustBeDevelopers`; a non-UUID id gets the same error). Duplicates keep the first. Their order is stored (`project_owners.position`) because a new request is assigned to the **first** owner ([Requests](04_requests.md)).
- **Targets and constraints keep their ids.** The server issues a UUID for an entry without one and keeps given ids, because requests store target ids. At most `CONSTRAINTS_MAX` (20) constraints; text 1–300, reference ≤ 60 characters.
- **Repository**: `provider` is `gitlab` or `github`, `url` an absolute http(s) URL. Only `github` projects are processed by the [Runner](11_runner.md); the server-side workers treat both alike.
- **Wiki numbers** (`wikiStats { verifiedDocs, totalDocs, lastSyncedAt? }`) are not input: they start at 0/0 and an update keeps them. Nothing fills them yet (see Open Questions).
- Validation errors use the frontend's zod paths (`repository.provider`, `targets.{i}`, `constraints.{i}.text`) in one `400` ([Platform](12_platform.md)). Name 1–60 trimmed, description ≤ 300 UTF-16 units, `icon` one of `ICONS`, `wikiPath` and `ownerIds` non-empty.
- There is no delete. `active` is a required boolean in the body of both create and update; deactivating is an update.

## API Services

| Operation | Endpoint | Permission | Errors |
| --- | --- | --- | --- |
| `listProjects` | `GET /projects` | session | 401 |
| `createProject` | `POST /projects` | `project:manage` | 400 `checkInput`, `ownersMustBeDevelopers` |
| `updateProject` | `PUT /projects/{id}` | `project:manage` | 400 (first), 404 `projectNotFound` |

## Data

`projects` (`icon` and `repository_provider` checks, `targets` and `rules` JSONB arrays, `wiki_verified_docs`, `wiki_total_docs`, `wiki_synced_at`, `active`) and `project_owners` (primary key `(project_id, user_id)`, unique `(project_id, position)`, user restrict) in `20261119000000_projects.sql`.

## Evidence

- `crates/api/src/projects/handlers.rs#symbol:ProjectInput`, `crates/api/src/projects/handlers.rs#symbol:ICONS`, `crates/api/src/projects/handlers.rs#symbol:PROVIDERS`, `crates/api/src/projects/handlers.rs#symbol:CONSTRAINTS_MAX` — input rules.
- `crates/api/src/projects/handlers.rs#symbol:check_owners`, `crates/api/src/projects/handlers.rs#symbol:set_owners` — owners and their order.
- `crates/api/src/projects/handlers.rs#symbol:update` — whole replacement, ids and wiki numbers kept.
- `crates/api/src/projects/mod.rs#symbol:target_ids` — used by request validation.
- Tests: `crates/api/tests/projects.rs` (`owners_are_developers_or_admins_in_order`, `an_update_replaces_the_project_but_keeps_ids_and_wiki_numbers`, `only_project_managers_change_projects`).

## Open Questions

- No code syncs a project's wiki into `wikiStats` yet; the numbers stay 0/0 until a wiki sync step exists.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
