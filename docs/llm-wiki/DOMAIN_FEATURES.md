---
title: Domain Features
tags:
  - llm-wiki
  - verified
status: verified
doc_type: domain_overview
project: ax-request-portal-server
last_updated: 2026-11-27
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/implemented.rs
evidence:
  - crates/api/src/implemented.rs#symbol:IMPLEMENTED
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/API_CONTRACTS.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-27
---

# Domain Features

What the server does, by who uses it. Details and evidence are in the domain documents; the operation names are the contract's `operationId`s (all 47 are implemented, `implemented::IMPLEMENTED`).

## Everyone signed in

- Sign in with email and password, see who they are, sign out (`login`, `me`, `logout`); first password through a one-time setup link (`completeSetup`) — [Auth and Sessions](domains/01_auth_sessions.md).
- List people, teams and projects (`listUsers`, `listTeams`, `listProjects`).
- The notification inbox and its settings (`listNotifications`, `markNotificationRead`, `markAllNotificationsRead`, `getNotificationSettings`, `updateNotificationSettings`) — [Notifications](domains/07_notifications.md).

## Requesters (and everyone who may file)

- Save, continue and delete drafts; upload attachments (`listDrafts`, `createDraft`, `getDraft`, `updateDraft`, `deleteDraft`, `uploadAttachment`) — [Drafts and Attachments](domains/06_drafts_attachments.md).
- Precheck a draft and file it; read and list their requests (`precheckRequest`, `createRequest`, `getRequest`, `listRequests`) — [Requests](domains/04_requests.md).
- Confirm the AI's scope, answer questions, cancel, check the result against the acceptance criteria, react to a preview (`transitionRequest`, `saveVerification`, `sendPreviewFeedback`) — [Lifecycle](domains/05_lifecycle.md).
- See the release schedule (`releases`) — [Dashboard and Releases](domains/08_dashboard_releases.md).

## Developers

- Work the requests they are responsible for: ask, reject, run AI work, request revisions, approve the merge, schedule and mark the release (`transitionRequest`, `updateRelease`), record regression results (`updateRegression`, `regressionQueue`) — [Lifecycle](domains/05_lifecycle.md).

## Executives

- The monthly summary (`dashboard`) and every request (`listRequests` with `scope=all`) — [Dashboard and Releases](domains/08_dashboard_releases.md).

## Admins

- Add and change people and teams, issue setup links (`createUser`, `updateUser`, `createTeam`, `updateTeam`, `createSetupLink`) — [Users, Teams and Roles](domains/02_users_teams_roles.md).
- Register and change projects (`createProject`, `updateProject`) — [Projects](domains/03_projects.md).

## Operators

- Budget, AI models per task, prices, usage (`getBudget`, `updateBudget`, `getAiSettings`, `updateAiSettings`, `getAiPricing`, `updateAiPricing`, `aiUsage`); keys are shown, never set (`setProviderKey`, `removeProviderKey` answer 403) — [AI Settings and Budget](domains/10_ai_settings_budget.md).
- Operators see no requests; their accounts come only from `ax-api user create` — [Users, Teams and Roles](domains/02_users_teams_roles.md).

## The AI (no API of its own)

- Writes feasibility reports and does the work on a branch through jobs; it never sends a person's event — [AI Jobs](domains/09_ai_jobs.md), [Runner](domains/11_runner.md).

## Evidence

- `crates/api/src/implemented.rs#symbol:IMPLEMENTED` — the implemented operations, in contract order.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
