---
title: Request Lifecycle
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
  - crates/api/src/requests/lifecycle.rs
  - crates/api/src/requests/transitions.rs
  - crates/api/src/requests/actions.rs
  - crates/api/src/requests/commands.rs
  - crates/api/src/requests/errors.rs
  - contract/lifecycle.json
  - crates/api/tests/transitions.rs
evidence:
  - crates/api/src/requests/lifecycle.rs#symbol:Status
  - crates/api/src/requests/lifecycle.rs#symbol:rule
  - crates/api/src/requests/lifecycle.rs#symbol:is_user_event
  - crates/api/src/requests/lifecycle.rs#symbol:Guard
  - crates/api/src/requests/lifecycle.rs#symbol:SCOPE_REVISIONS_MAX
  - crates/api/src/requests/lifecycle.rs#symbol:the_ai_never_sends_a_persons_event
  - crates/api/src/requests/transitions.rs#symbol:transition
  - crates/api/src/requests/transitions.rs#symbol:is_authorized
  - crates/api/src/requests/transitions.rs#symbol:is_responsible_developer
  - crates/api/src/requests/transitions.rs#symbol:check_guards
  - crates/api/src/requests/commands.rs#symbol:parse
  - crates/api/src/requests/actions.rs#symbol:save_verification
  - crates/api/src/requests/actions.rs#symbol:preview_feedback
  - crates/api/src/requests/actions.rs#symbol:update_regression
  - crates/api/src/requests/actions.rs#symbol:update_release
  - crates/api/src/requests/actions.rs#symbol:regression_queue
  - crates/api/src/requests/errors.rs#symbol:guard_failed
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/04_requests.md
  - docs/llm-wiki/domains/07_notifications.md
  - docs/llm-wiki/domains/09_ai_jobs.md
  - docs/llm-wiki/GLOSSARY.md
visibility: public
contains_sensitive_info: false
---

# Request Lifecycle

## Responsibility

The state machine of a request (13 statuses, 16 events, 6 guards), who may send which event, the side effects of each transition, and the record-keeping operations that do not change status (verification checklist, preview feedback, regression results, release date). The table is a port of the frontend's `src/domain/lifecycle.ts` and is tested against `contract/lifecycle.json`.

## The table

| By | Event | From → to |
| --- | --- | --- |
| system | `startReport` | submitted → reporting |
| ai | `reportReady` | reporting → review |
| ai | `scopeReady` | reporting → scope_check |
| ai | `workFinished` | in_progress → code_review |
| requester | `approveScope` / `reviseScope` | scope_check → review / reporting |
| requester | `provideInfo` | info_needed → reporting |
| requester | `confirmResult` / `reportIssue` | verification → release_scheduled / changes_requested |
| requester | `cancel` | submitted, reporting, scope_check, review, info_needed → cancelled |
| developer | `requestInfo` | review → info_needed |
| developer | `reject` | review, info_needed → rejected |
| developer | `runWork` | review, changes_requested → in_progress |
| developer | `requestRevision` / `approveMerge` | code_review → in_progress / verification |
| developer | `markReleased` | release_scheduled → done |

Terminal: `done`, `rejected`, `cancelled`. Whose turn (`Status::turn`): AI in `submitted`, `reporting`, `in_progress`; requester in `scope_check`, `info_needed`, `verification`; developer in `review`, `code_review`, `changes_requested`, `release_scheduled`.

## Human checkpoints

Only the 12 requester and developer events can be sent through `POST /requests/{id}/transitions` (`is_user_event`); the four system and AI events are refused with `400` by the command parser. AI events are applied only by the job code after a job finishes ([AI Jobs](09_ai_jobs.md)). So the AI can never approve a scope, approve a merge, confirm a result or release — the test `the_ai_never_sends_a_persons_event` keeps it that way.

## Who may send what

- Requester events: only the request's requester; an admin may also `cancel` anyone's request (`is_authorized`).
- Developer events: only a **responsible developer** — someone with `request:review` who is an admin, the assignee or a project owner (`is_responsible_developer`).
- **Check order differs from the other handlers**: body (400) → load the request (404, also when not readable) → step permission (403 `notYourStep`) → status (409 `invalid_transition`) → guards (409 `guard_failed`). There is no up-front `auth::require`, because who may act depends on the request.

## Guards

`criteriaAllPassed` (`confirmResult`), `criteriaOneFailed` (`reportIssue`), `assumptionsUnchecked` (`runWork` from review: every report assumption checked), `scopeUnanswered` (`approveScope`: every scope question answered with a valid option), `scopeRevisionsUsed` (`reviseScope` at most `SCOPE_REVISIONS_MAX` = 2, counted from the timeline), `releaseUnscheduled` (`markReleased` needs a scheduled date). Also a `409 guard_failed`: `runWork`, `requestRevision` and `reviseScope` stop when the request has reached its AI cost limit; `provideInfo` never does, so a requester is never stuck ([AI Settings and Budget](10_ai_settings_budget.md)).

## Side effects

Everything happens under a row lock (`find_for_update`) in one transaction: status and `details` update, a timeline row (`Actor::User`, with the command's note), notifications for the new status ([Notifications](07_notifications.md)), and a job for `reporting` or `in_progress` ([AI Jobs](09_ai_jobs.md)).

- `runWork` / `requestRevision` create a new `work` record (`attempt + 1`, phase `queued`, branch `ax/request-{id}`) and snapshot the work agent (`provider`, `model`, `runtime`) from the AI settings; the job uses that snapshot even if settings change. A request without an assignee is assigned to the developer who started work.
- `approveScope` stores the decision, `requestInfo` the question, `reject` the reason; `approveMerge` starts the verification checklist with every criterion pending; `markReleased` records `releasedAt`.

## Record-keeping operations

| Operation | Endpoint | Who | Rules |
| --- | --- | --- | --- |
| `saveVerification` | `PUT /requests/{id}/verification` | requester | only in `verification` (409); no timeline, no notification |
| `sendPreviewFeedback` | `PUT /requests/{id}/preview-feedback` | requester | only in `code_review` with a preview (409); timeline `preview_looks_good`/`preview_needs_change`; developers notified |
| `updateRegression` | `PUT /requests/{id}/regression` | responsible developer | not on closed requests (409); unknown check ids are ignored |
| `updateRelease` | `PUT /requests/{id}/release` | responsible developer | in `code_review`, `verification`, `changes_requested`, `release_scheduled`; a new date before today (`AX_TIME_ZONE`) is `400`, resending the current date is allowed; requester notified when a date is set |
| `regressionQueue` | `GET /regression-checks` | `regression:view` | open requests with regression checks, by request id; responsible ones only, admins all |

## Evidence

- `crates/api/src/requests/lifecycle.rs#symbol:Status`, `crates/api/src/requests/lifecycle.rs#symbol:rule`, `crates/api/src/requests/lifecycle.rs#symbol:is_user_event`, `crates/api/src/requests/lifecycle.rs#symbol:Guard`, `crates/api/src/requests/lifecycle.rs#symbol:SCOPE_REVISIONS_MAX` — the table; its `mod tests` reads `contract/lifecycle.json` through `ax_contract::lifecycle()`.
- `crates/api/src/requests/lifecycle.rs#symbol:the_ai_never_sends_a_persons_event` — the checkpoint guarantee.
- `crates/api/src/requests/transitions.rs#symbol:transition`, `crates/api/src/requests/transitions.rs#symbol:is_authorized`, `crates/api/src/requests/transitions.rs#symbol:is_responsible_developer`, `crates/api/src/requests/transitions.rs#symbol:check_guards` — order, permission, guards, effects.
- `crates/api/src/requests/commands.rs#symbol:parse` — per-event body fields.
- `crates/api/src/requests/actions.rs#symbol:save_verification`, `crates/api/src/requests/actions.rs#symbol:preview_feedback`, `crates/api/src/requests/actions.rs#symbol:update_regression`, `crates/api/src/requests/actions.rs#symbol:update_release`, `crates/api/src/requests/actions.rs#symbol:regression_queue` — record-keeping.
- `crates/api/src/requests/errors.rs#symbol:guard_failed` — the mock's messages.
- Tests: `crates/api/tests/transitions.rs` (`from_review_to_release`, `questions_answers_and_rejection`, `scope_answers_and_rewrites`, `who_may_cancel_and_what_is_refused`, `preview_feedback_and_regression`).

## Open Questions

- When the frontend changes `lifecycle.json`, the unit tests fail until this port follows; the change has to start in the frontend ([API Contracts](../API_CONTRACTS.md)).

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
