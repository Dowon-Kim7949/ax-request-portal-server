---
title: Users, Teams and Roles
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
  - crates/api/src/users
  - crates/api/src/teams
  - crates/api/src/roles.rs
  - crates/api/src/cli.rs
  - crates/api/src/accounts.rs
  - crates/api/migrations/20261118000000_users_teams_sessions.sql
  - crates/api/tests/users_teams.rs
evidence:
  - crates/api/src/roles.rs#symbol:Role
  - crates/api/src/roles.rs#symbol:Permission
  - crates/api/src/roles.rs#symbol:permissions
  - crates/api/src/roles.rs#symbol:OPERATOR_PERMISSIONS
  - crates/api/src/roles.rs#symbol:admin_may_assign
  - crates/api/src/roles.rs#symbol:roles_match_the_contract
  - crates/api/src/users/handlers.rs#symbol:create
  - crates/api/src/users/handlers.rs#symbol:update
  - crates/api/src/users/mod.rs#symbol:User
  - crates/api/src/teams/handlers.rs#symbol:update
  - crates/api/src/cli.rs#symbol:UserCommand
  - crates/api/src/cli.rs#symbol:run_user
  - crates/api/src/accounts.rs#symbol:create
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/01_auth_sessions.md
  - docs/llm-wiki/SECURITY.md
  - docs/llm-wiki/GLOSSARY.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Users, Teams and Roles

## Responsibility

People and teams (listing for everyone, writes for admins), the five roles and their permissions, and the rule that operator accounts never come from the API. `roles.rs` is a port of the frontend's `src/domain/roles.ts`; every handler asks it through `auth::require` ([Platform](12_platform.md)).

## Roles and permissions

| Role | Permissions |
| --- | --- |
| `requester` | `request:create`, `request:read:own`, `schedule:view` |
| `executive` | requester's + `request:read:all`, `dashboard:view` |
| `developer` | `request:create`, `request:read:own`, `request:read:all`, `request:review`, `report:read`, `regression:view`, `schedule:view` |
| `admin` | developer's + `dashboard:view`, `project:manage`, `user:manage` |
| `operator` | `budget:manage`, `agent:manage`, `usage:view` only |

- Admins do **not** manage budget, agents or usage; operators see no requests (opening a request detail as an operator is a `404`, as in the mock).
- `Role::admin_may_assign()` is false only for `operator`: creating an operator, turning someone into one, or editing an operator through the API answers `403 operatorProvisioned`. Operators come only from `ax-api user create --role operator` ([Operations](../OPERATIONS.md)).
- The test `roles_match_the_contract` compares the role names with the contract's `Role` enum. A deployment that overrides role permissions in the frontend's `app.config.ts` must change this table too.

## Behaviour

- **Nothing is deleted.** Users are deactivated (`active = false`); teams are only renamed. Later tables reference both with `ON DELETE RESTRICT`.
- `createUser` makes an active account **without a password**; the person signs in after an admin gives them a setup link ([Auth and Sessions](01_auth_sessions.md)).
- `updateUser`: `teamId: null` removes the team, an empty string leaves it alone (as the mock does). An admin cannot deactivate themselves or change their own role (`400 cannotChangeSelf`). Deactivation ends all the person's sessions and deletes their unused setup link, so reactivating does not revive an old link.
- Emails are stored as typed and unique by `lower(email)`. Team names are 1–40 characters, trimmed, unique ignoring case (a racing duplicate is caught from the unique violation as `teamNameTaken`).
- `listUsers` returns everyone including the deactivated, by creation; `listTeams` sorts by name ignoring case. A user's `teamId` is omitted from JSON when they have none.
- **CLI accounts** (`ax-api user create | set-password`): any of the five roles, password only from `--password-stdin` or an interactive prompt (a `--password` argument is rejected), minimum 12 characters. `set-password` also works on deactivated accounts and signs the account out everywhere.

## API Services

| Operation | Endpoint | Permission | Errors |
| --- | --- | --- | --- |
| `listUsers` | `GET /users` | session | 401 |
| `createUser` | `POST /users` | `user:manage` | 403 `operatorProvisioned`; 400 `emailTaken`; 404 `teamNotFound` |
| `updateUser` | `PATCH /users/{id}` | `user:manage` | 404 `userNotFound`/`teamNotFound`; 403 `operatorProvisioned`; 400 `cannotChangeSelf` |
| `listTeams` | `GET /teams` | session | 401 |
| `createTeam` | `POST /teams` | `user:manage` | 400 `teamNameTaken`, `checkInput` |
| `updateTeam` | `PATCH /teams/{id}` | `user:manage` | 404 `teamNotFound`; 400 `teamNameTaken` |

## Data

`teams` (`name` 1–40, unique `lower(name)`) and `users` (`role` check over the five roles, unique `lower(email)`, `team_id` restrict, nullable `password_hash`, `active`) in `20261118000000_users_teams_sessions.sql`.

## Evidence

- `crates/api/src/roles.rs#symbol:Role`, `crates/api/src/roles.rs#symbol:Permission`, `crates/api/src/roles.rs#symbol:permissions`, `crates/api/src/roles.rs#symbol:OPERATOR_PERMISSIONS`, `crates/api/src/roles.rs#symbol:admin_may_assign`, `crates/api/src/roles.rs#symbol:roles_match_the_contract` — the port of `roles.ts`.
- `crates/api/src/users/handlers.rs#symbol:create`, `crates/api/src/users/handlers.rs#symbol:update`, `crates/api/src/users/mod.rs#symbol:User` — people.
- `crates/api/src/teams/handlers.rs#symbol:update` — team rename and duplicate handling.
- `crates/api/src/cli.rs#symbol:UserCommand`, `crates/api/src/cli.rs#symbol:run_user`, `crates/api/src/accounts.rs#symbol:create` — the only way to make an operator.
- Tests: `crates/api/tests/users_teams.rs` (`adding_people_follows_the_mock_apis_rules`, `changing_people_follows_the_mock_apis_rules`, `team_names_are_unique_ignoring_case`, `people_follow_a_renamed_team`).

## Open Questions

- The role table is fixed in code; a per-deployment permission override would need configuration that does not exist yet.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
