---
title: Domain Overview
tags:
  - llm-wiki
  - verified
status: verified
doc_type: domain_overview
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/lib.rs
  - crates/api/src/requests/lifecycle.rs
evidence:
  - crates/api/src/requests/lifecycle.rs#symbol:rule
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/DOMAIN_FEATURES.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/domains/05_lifecycle.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Domain Overview

## The request's path through the system

1. A requester drafts a request ([Drafts and Attachments](06_drafts_attachments.md)), gets rule-based (and, when connected, Claude) advice from the precheck, and files it ([Requests](04_requests.md)). It goes to the project's first owner ([Projects](03_projects.md)).
2. Filing queues a **report** job (which statuses queue AI work is in [AI Jobs](09_ai_jobs.md)); the AI (stub, Claude, or Claude Code through the [Runner](11_runner.md)) writes a feasibility report ([AI Jobs](09_ai_jobs.md)). The requester may first confirm the AI's reading of the scope (`scope_check`).
3. A responsible developer decides: ask the requester, reject, or run the work ([Lifecycle](05_lifecycle.md)).
4. Running the work queues a **work** job; with the runner, Claude Code edits a branch and opens a pull request. Nothing merges automatically.
5. The developer reviews the change (`code_review`) and approves the merge or asks for a revision; the requester checks the acceptance criteria (`verification`), and the developer schedules and marks the release.
6. Along the way people are notified ([Notifications](07_notifications.md)), AI calls are booked against a budget ([AI Settings and Budget](10_ai_settings_budget.md)), and executives see the month and the release schedule ([Dashboard and Releases](08_dashboard_releases.md)).

Every step that commits to something — scope, work, merge, result, release — is a person's event; the AI only ever reports, works on a branch, or hands the request back ([Lifecycle](05_lifecycle.md#human-checkpoints)).

## Domains

| # | Domain | Owns |
| --- | --- | --- |
| 01 | [Auth and Sessions](01_auth_sessions.md) | sign-in, bearer sessions, login throttling, setup links |
| 02 | [Users, Teams and Roles](02_users_teams_roles.md) | people, teams, the five roles, operator provisioning |
| 03 | [Projects](03_projects.md) | projects, owners, targets, constraints, repository |
| 04 | [Requests](04_requests.md) | filing, reading, listing, precheck |
| 05 | [Lifecycle](05_lifecycle.md) | the state machine, guards, verification, regression, release |
| 06 | [Drafts and Attachments](06_drafts_attachments.md) | saved drafts, uploads, signed file links |
| 07 | [Notifications](07_notifications.md) | the inbox and its settings |
| 08 | [Dashboard and Releases](08_dashboard_releases.md) | monthly summary, release schedule |
| 09 | [AI Jobs](09_ai_jobs.md) | the job queue, providers, verdict rules |
| 10 | [AI Settings and Budget](10_ai_settings_budget.md) | models, prices, budget, the usage ledger |
| 11 | [Runner](11_runner.md) | Claude Code on GitHub clones, pull requests |
| 12 | [Platform](12_platform.md) | contract routing, handler pipeline, errors, language, CLI |

Dependencies point downwards in this order: lifecycle uses requests, notifications and AI jobs; AI jobs use settings and the ledger; everything uses the platform and roles.

## Evidence

- `crates/api/src/requests/lifecycle.rs#symbol:rule` — the transitions behind the path above.
- `crates/api/src/lib.rs` — the module list the domains follow.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
