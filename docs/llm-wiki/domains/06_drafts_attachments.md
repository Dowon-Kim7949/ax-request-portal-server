---
title: Drafts and Attachments
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
  - crates/api/src/drafts
  - crates/api/src/attachments
  - crates/api/migrations/20261119000004_attachments.sql
  - crates/api/migrations/20261119000005_drafts.sql
  - crates/api/tests/drafts.rs
  - crates/api/tests/attachments.rs
evidence:
  - crates/api/src/drafts/handlers.rs#symbol:create
  - crates/api/src/drafts/handlers.rs#symbol:update
  - crates/api/src/drafts/mod.rs#symbol:TEXT_MAX
  - crates/api/src/drafts/mod.rs#symbol:StoredDraft
  - crates/api/src/attachments/handlers.rs#symbol:upload
  - crates/api/src/attachments/handlers.rs#symbol:download
  - crates/api/src/attachments/mod.rs#symbol:MAX_SIZE
  - crates/api/src/attachments/mod.rs#symbol:BODY_LIMIT
  - crates/api/src/attachments/mod.rs#symbol:INLINE_TYPES
  - crates/api/src/attachments/mod.rs#symbol:WINDOW_SECS
  - crates/api/src/attachments/mod.rs#symbol:Files
  - crates/api/src/attachments/mod.rs#symbol:signing_key
  - crates/api/src/attachments/mod.rs#symbol:owned
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/04_requests.md
  - docs/llm-wiki/SECURITY.md
visibility: public
contains_sensitive_info: false
---

# Drafts and Attachments

## Responsibility

Half-written requests saved per account (with optimistic versions and idempotent creation), and files attached to drafts and requests, stored on disk and served back through signed links. All operations need `request:create`.

## Drafts

- `content` is one JSONB column holding the known `DraftContent` fields only; unknown fields are dropped. Limits only stop abuse: text 10,000 UTF-16 units (`TEXT_MAX`), lists of 50, `step` 0–6, `clientId` 1–100. Ids look like `drf_…`.
- **Private**: someone else's draft answers `404` like a missing one.
- **Per-account cap** `AX_DRAFTS_PER_ACCOUNT` (20) → `409 limit_reached`. Creation locks the user row (`for update`) before counting so parallel saves cannot pass the cap.
- **Idempotent create**: a repeated `clientId` returns the first draft (`201`) before any other rule, the cap included (`unique (owner_id, client_id)`).
- **Optimistic update**: `updateDraft` is a `PUT` with `baseVersion`; a different stored version is `409 conflict`. The draft row is locked during the check.
- **Attachments are ids inside the JSON**, not foreign keys. Saving checks they are the caller's uploads (`attachmentNotFound` at `content.attachmentIds.{i}`); reading shows only attachments that still exist, and `attachmentCount` counts them once. Deleting a draft keeps its attachments.
- Filing a request with `draftId` deletes the draft in the filing transaction ([Requests](04_requests.md)).
- Order: body (400) → permission (403) → rules, as in the mock.

## Attachments

- `uploadAttachment` takes multipart field `file` and streams it to `AX_UPLOAD_DIR` (a temporary `.upload-{id}` renamed into place; partial files are removed on failure). At most `MAX_SIZE` (10 MiB, inclusive). The route alone raises axum's body limit to `BODY_LIMIT` (11 MiB) — that is why its arm in `routes::implemented` adds a `DefaultBodyLimit` layer — and a cut-off body is reported as `fileTooLarge`. Here the file checks (400) come **before** the permission check (403), as in the mock.
- **No type restriction on upload**: name and MIME are the browser's (`application/octet-stream` when empty). Safety is at serving time: only `INLINE_TYPES` (PNG, JPEG, GIF, WebP, AVIF, BMP, PDF — the frontend's list) open in place; everything else, SVG and HTML included, is sent as `application/octet-stream` with `Content-Disposition: attachment`. Every answer has `X-Content-Type-Options: nosniff` and `Cache-Control: private, max-age=3600`.
- **Signed links**: `{AX_PUBLIC_PATH}/files/{id}?exp=…&sig=…` where `sig` is HMAC-SHA256 of `id` and `exp`, base64url, checked in constant time. `exp` is aligned to 12-hour windows (`WINDOW_SECS`), so a link lives 12–24 hours and stays identical within a window (browser caching). Links are minted every time an attachment is shown; whoever sees the request gets the link. A bad, expired or unknown link is `404`.
- **The signing key** is 32 random bytes created on first start in `server_secrets` (`file_urls`) — no setting, survives restarts, shared by every instance. Deleting the row invalidates every link (they expire within a day anyway).
- `GET /files/{id}` is outside the contract and registered directly in `routes::router`.

## API Services

| Operation | Endpoint | Errors |
| --- | --- | --- |
| `listDrafts` | `GET /drafts` → summaries, most recently changed first | 403 |
| `createDraft` | `POST /drafts` → 201 | 400, `attachmentNotFound`; 409 `limit_reached`; 403 |
| `getDraft` | `GET /drafts/{id}` | 404 |
| `updateDraft` | `PUT /drafts/{id}` with `baseVersion` | 404; 409 `conflict`; 400 |
| `deleteDraft` | `DELETE /drafts/{id}` → 204 | 404 |
| `uploadAttachment` | `POST /attachments` → 201 `Attachment` (with link) | 400 `pickFile`, `fileTooLarge`; 403 |
| (outside the contract) | `GET /files/{id}?exp&sig` | 404 |

## Evidence

- `crates/api/src/drafts/handlers.rs#symbol:create`, `crates/api/src/drafts/handlers.rs#symbol:update`, `crates/api/src/drafts/mod.rs#symbol:TEXT_MAX`, `crates/api/src/drafts/mod.rs#symbol:StoredDraft` — drafts.
- `crates/api/src/attachments/handlers.rs#symbol:upload`, `crates/api/src/attachments/handlers.rs#symbol:download`, `crates/api/src/attachments/mod.rs#symbol:MAX_SIZE`, `crates/api/src/attachments/mod.rs#symbol:BODY_LIMIT`, `crates/api/src/attachments/mod.rs#symbol:INLINE_TYPES` — upload and serving.
- `crates/api/src/attachments/mod.rs#symbol:Files`, `crates/api/src/attachments/mod.rs#symbol:WINDOW_SECS`, `crates/api/src/attachments/mod.rs#symbol:signing_key`, `crates/api/src/attachments/mod.rs#symbol:owned` — links, key, ownership.
- Tests: `crates/api/tests/drafts.rs` (`a_repeated_create_answers_with_the_first_draft`, `an_account_keeps_at_most_twenty_drafts`, `drafts_hold_only_the_callers_attachments`, `filing_a_request_deletes_its_draft`), `crates/api/tests/attachments.rs` (`an_upload_is_served_back_through_its_link`, `only_safe_types_open_in_place`, `uploads_follow_the_mock_apis_rules`).

## Open Questions

- Uploaded files are never deleted (requests may show them); disk use grows with uploads.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
