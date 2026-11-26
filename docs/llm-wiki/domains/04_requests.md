---
title: Requests
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: domain
project: ax-request-portal-server
last_updated: 2026-11-26
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/requests/handlers.rs
  - crates/api/src/requests/store.rs
  - crates/api/src/requests/draft.rs
  - crates/api/src/requests/precheck.rs
  - crates/api/migrations/20261119000001_requests.sql
  - crates/api/tests/requests.rs
  - crates/api/tests/claude.rs
evidence:
  - crates/api/src/requests/handlers.rs#symbol:create
  - crates/api/src/requests/handlers.rs#symbol:list
  - crates/api/src/requests/handlers.rs#symbol:get
  - crates/api/src/requests/handlers.rs#symbol:precheck
  - crates/api/src/requests/handlers.rs#symbol:present
  - crates/api/src/requests/handlers.rs#symbol:truncate_utf16
  - crates/api/src/requests/draft.rs#symbol:Draft
  - crates/api/src/requests/store.rs#symbol:StoredRequest
  - crates/api/src/requests/store.rs#symbol:can_read
  - crates/api/src/requests/store.rs#symbol:may_open
  - crates/api/src/requests/store.rs#symbol:responsible
  - crates/api/src/requests/store.rs#symbol:field
  - crates/api/src/requests/store.rs#symbol:today
  - crates/api/src/requests/precheck.rs#symbol:advise
  - crates/api/src/requests/precheck.rs#symbol:VAGUE_KO
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/05_lifecycle.md
  - docs/llm-wiki/domains/06_drafts_attachments.md
  - docs/llm-wiki/domains/09_ai_jobs.md
  - docs/llm-wiki/DATA_MODEL.md
visibility: public
contains_sensitive_info: false
reviewed_at: 2026-11-23
---

# Requests

## Responsibility

Filing a request, reading one, listing them, and the precheck that advises on a draft before it is filed. State changes after filing belong to [Lifecycle](05_lifecycle.md).

## Behaviour

- **Filing** (`createRequest`) validates the body like the frontend's zod schema (`Draft::parse`, lengths in UTF-16 units), then: the project must exist and be active (`projectNotFound`), every target id must belong to it (`unknownTarget`), every attachment must be the caller's upload (`attachmentNotFound` at `attachmentIds.{i}`). Body problems come before the `403` for callers without `request:create` (operators).
- In **one transaction** the request is inserted already in `reporting` and assigned to the project's **first owner**; the timeline records both steps (`created` by the user, to `submitted`, then `startReport` by `system`, to `reporting`), so `submitted` exists only on the timeline. A report job is queued in the requester's language ([AI Jobs](09_ai_jobs.md)). A given `draftId` deletes the caller's own draft in the same transaction; anyone else's or an unknown one is ignored.
- A `checklist` request turns its items into `k1…` checks and its acceptance criteria into `c1…`; criterion text is `[ref] expected|finding` cut to 300 UTF-16 units (`truncate_utf16`, JavaScript `slice` semantics). Empty optional fields are left out of the JSON rather than sent empty.
- **Reading** (`getRequest`): visible to its requester, anyone with `request:read:all`, and its responsible developers (assignee or project owners, `StoredRequest::responsible`). The rule is `store::may_open`, which the release schedule's `canOpen` uses too. Anyone else gets the same `404` as for a missing request, so ids are not probed; a malformed id (not a positive number) is a `404` naming `id`. `present` adds signed links to attachments and adds `aiUsage` only for callers with `report:read`.
- **Listing** (`listRequests`): `scope=mine` (own requests, no permission), `assigned` (needs `request:review`; assignee or project owner, admins see all), `all` (needs `request:read:all`). Filters: `status` (comma list; an unknown value matches nothing), `projectId`, `q` (title substring or number; a leading `#` is stripped, a lone `#` is no filter). Sorted by `updated_at desc`. A bad `scope` is a `400` naming `scope`.
- **Precheck** (`precheckRequest`, `request:create`) never files or blocks. Rules come first, ported from the frontend's `precheck.ts`: missing targets, thin current behaviour (a bug needs two non-empty lines), vague desired behaviour or criteria (`VAGUE_KO`/`VAGUE_EN`), criteria not in "when … then …" form, duplicate checklist refs, empty expectations, a past desired date (by `AX_TIME_ZONE`), a date without a reason.
- **`aiAdvice`** says whether a model added to the rules: `notConnected` when no model was asked (provider stub, so `state.claude` is absent; or the project's precheck selection is not `anthropic`), `included` when Claude answered (it may add nothing; its issues are appended only for fields the rules did not flag), `failed` when the call failed or exceeded `PRECHECK_TIMEOUT` (20 s) — then only the rules answer. The call is booked to the caller with no request id, as runtime `api` ([AI Settings and Budget](10_ai_settings_budget.md)).

## API Services

| Operation | Endpoint | Permission | Payload → response | Errors |
| --- | --- | --- | --- | --- |
| `listRequests` | `GET /requests` | by `scope` | query → `WorkRequestSummary[]` | 400 `scope`; 403 |
| `createRequest` | `POST /requests` | `request:create` | `RequestSubmission` → 201 `WorkRequest` | 400 body, `projectNotFound`, `unknownTarget`, `attachmentNotFound`; 403 |
| `getRequest` | `GET /requests/{id}` | readable | → `WorkRequest` | 404 (also when not readable, or a malformed id) |
| `precheckRequest` | `POST /requests/precheck` | `request:create` | `RequestDraft` → `{ issues, aiAdvice }` | 400; 403 |

## Storage

Columns hold what is filtered or joined on (`project_id`, `requester_id`, `assignee_id`, `status`, `type`, `title`, `desired_date`, timestamps); everything else of `WorkRequest` lives in the `details` JSONB in the contract's camelCase shape. The timeline is the separate `request_events` table (`actor_type` `user|ai|system`, a user event must name its actor). Numbers come from an identity column starting at 1. `StoredRequest::field` returns `null` for a missing key; index the map directly and it panics. See [Data Model](../DATA_MODEL.md).

## Evidence

- `crates/api/src/requests/handlers.rs#symbol:create`, `crates/api/src/requests/draft.rs#symbol:Draft`, `crates/api/src/requests/handlers.rs#symbol:truncate_utf16` — filing.
- `crates/api/src/requests/handlers.rs#symbol:list`, `crates/api/src/requests/handlers.rs#symbol:get`, `crates/api/src/requests/handlers.rs#symbol:present` — reading and listing.
- `crates/api/src/requests/store.rs#symbol:StoredRequest`, `crates/api/src/requests/store.rs#symbol:can_read`, `crates/api/src/requests/store.rs#symbol:may_open`, `crates/api/src/requests/store.rs#symbol:responsible`, `crates/api/src/requests/store.rs#symbol:field`, `crates/api/src/requests/store.rs#symbol:today` — storage and visibility.
- `crates/api/src/requests/precheck.rs#symbol:advise`, `crates/api/src/requests/precheck.rs#symbol:VAGUE_KO`, `crates/api/src/requests/handlers.rs#symbol:precheck` — precheck and `aiAdvice` (the Claude call and its `PRECHECK_TIMEOUT` are in [AI Jobs](09_ai_jobs.md)).
- Tests: `crates/api/tests/requests.rs` (`a_filed_request_is_reported_on_and_goes_to_the_first_owner`, `filing_follows_the_mock_apis_rules`, `only_those_who_may_open_a_request_see_it`, `lists_by_scope_status_project_and_search`, `precheck_advises_without_filing`), `crates/api/tests/claude.rs` (`the_precheck_adds_claudes_advice_to_the_rules`).

## Open Questions

- The 20-second precheck timeout is not exercised by a test; the failure path is tested with a `529` answer.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
