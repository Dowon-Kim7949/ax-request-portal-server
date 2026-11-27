---
title: Projects
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: domain
project: ax-request-portal-server
last_updated: 2026-11-27
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/projects
  - crates/api/migrations/20261119000000_projects.sql
  - crates/api/migrations/20261127000000_project_branch.sql
  - crates/api/tests/branches.rs
  - crates/api/tests/projects.rs
evidence:
  - crates/api/src/projects/handlers.rs#symbol:ProjectInput
  - crates/api/src/projects/handlers.rs#symbol:ICONS
  - crates/api/src/projects/handlers.rs#symbol:PROVIDERS
  - crates/api/src/projects/handlers.rs#symbol:CONSTRAINTS_MAX
  - crates/api/src/projects/handlers.rs#symbol:check_owners
  - crates/api/src/projects/handlers.rs#symbol:set_owners
  - crates/api/src/projects/handlers.rs#symbol:update
  - crates/api/src/projects/handlers.rs#symbol:branch_name
  - crates/api/src/projects/branches.rs#symbol:list
  - crates/api/src/projects/branches.rs#symbol:read
  - crates/api/src/projects/mod.rs#symbol:target_ids
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/04_requests.md
  - docs/llm-wiki/domains/11_runner.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Projects

## Responsibility

The systems requests are made against: name, icon, repository, wiki path, owners (developers in order), targets (screens or parts a request can point at) and constraints (rules the AI report checks a request against). Admins manage them; everyone signed in can list them.

## Behaviour

- `updateProject` is a **`PUT`** that replaces the whole project, not a patch. The owner check runs before the existence check, so a bad owner on an unknown project is `400`, not `404`.
- **Owners** must be existing `developer` or `admin` users (`400 ownersMustBeDevelopers`; a non-UUID id gets the same error). Duplicates keep the first. Their order is stored (`project_owners.position`) because a new request is assigned to the **first** owner ([Requests](04_requests.md)).
- **Targets and constraints keep their ids.** The server issues a UUID for an entry without one and keeps given ids, because requests store target ids. At most `CONSTRAINTS_MAX` (20) constraints; text 1–300, reference ≤ 60 characters.
- **Repository**: `provider` is `gitlab` or `github`, `url` an absolute http(s) URL, and an optional `branch`: the one the AI reads, works from and opens its pull request into ([Runner](11_runner.md)); left out, the repository's default branch. An update without a branch clears it. The name is trimmed and must pass `branch_name`, the frontend's `BranchName` (git's branch-name rules, 1–200 characters; the contract's pattern uses lookaheads the `regex` crate lacks, so the rules are spelled out, with the frontend's test cases). Only `github` projects are processed by the runner; the server-side workers treat both alike.
- **Branch list** (`listRepositoryBranches`, `projects::branches`): session, then the query (`provider`, `url`; `400` with those paths), then `project:manage`. The server reads GitHub with its own read token (`AX_GITHUB_READ_TOKEN`, Contents read; never the runner's) at `AX_GITHUB_API`: the repository's `default_branch`, then up to 10 pages of 100 branches; more is `truncated`. The answer lists the default branch first, then the rest by name. Anything it cannot read is one `404 repositoryNotFound`, after which the form takes a typed name: no read token, `gitlab`, a URL that names no `owner/name`, or a GitHub refusal, failure or timeout (15 s a call; logged with the reason, never the token).
- **Wiki numbers** (`wikiStats { verifiedDocs, totalDocs, lastSyncedAt? }`) are not input: they start at 0/0 and an update keeps them. Nothing fills them yet (see Open Questions).
- Validation errors use the frontend's zod paths (`repository.provider`, `repository.branch`, `targets.{i}`, `constraints.{i}.text`) in one `400` ([Platform](12_platform.md)). Name 1–60 trimmed, description ≤ 300 UTF-16 units, `icon` one of `ICONS`, `wikiPath` and `ownerIds` non-empty.
- There is no delete. `active` is a required boolean in the body of both create and update; deactivating is an update.

## API Services

| Operation | Endpoint | Permission | Errors |
| --- | --- | --- | --- |
| `listProjects` | `GET /projects` | session | 401 |
| `createProject` | `POST /projects` | `project:manage` | 400 `checkInput`, `ownersMustBeDevelopers` |
| `updateProject` | `PUT /projects/{id}` | `project:manage` | 400 (first), 404 `projectNotFound` |
| `listRepositoryBranches` | `GET /repositories/branches?provider=&url=` | `project:manage` | 400 `badQuery` (before 403), 404 `repositoryNotFound` |

## Data

`projects` (`icon` and `repository_provider` checks, `targets` and `rules` JSONB arrays, `wiki_verified_docs`, `wiki_total_docs`, `wiki_synced_at`, `active`; `repository_branch`, null for the default branch, from `20261127000000_project_branch.sql`) and `project_owners` (primary key `(project_id, user_id)`, unique `(project_id, position)`, user restrict) in `20261119000000_projects.sql`.

## Evidence

- `crates/api/src/projects/handlers.rs#symbol:ProjectInput`, `crates/api/src/projects/handlers.rs#symbol:ICONS`, `crates/api/src/projects/handlers.rs#symbol:PROVIDERS`, `crates/api/src/projects/handlers.rs#symbol:CONSTRAINTS_MAX` — input rules.
- `crates/api/src/projects/handlers.rs#symbol:check_owners`, `crates/api/src/projects/handlers.rs#symbol:set_owners` — owners and their order.
- `crates/api/src/projects/handlers.rs#symbol:update` — whole replacement, ids and wiki numbers kept.
- `crates/api/src/projects/handlers.rs#symbol:branch_name` — the branch name rule.
- `crates/api/src/projects/branches.rs#symbol:list`, `crates/api/src/projects/branches.rs#symbol:read` — the branch list, its order of checks and how GitHub is read.
- `crates/api/src/projects/mod.rs#symbol:target_ids` — used by request validation.
- Tests: `crates/api/tests/projects.rs` (`owners_are_developers_or_admins_in_order`, `an_update_replaces_the_project_but_keeps_ids_and_wiki_numbers`, `only_project_managers_change_projects`, `a_project_names_the_branch_the_ai_works_from`), `crates/api/tests/branches.rs` (a fake GitHub: pages, truncation, every unreadable case).

## Open Questions

- No code syncs a project's wiki into `wikiStats` yet; the numbers stay 0/0 until a wiki sync step exists.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
