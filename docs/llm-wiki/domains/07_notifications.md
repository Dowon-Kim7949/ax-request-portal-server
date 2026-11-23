---
title: Notifications
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
  - crates/api/src/notifications
  - crates/api/migrations/20261119000002_notifications.sql
  - crates/api/tests/transitions.rs
evidence:
  - crates/api/src/notifications/mod.rs#symbol:notify
  - crates/api/src/notifications/mod.rs#symbol:notify_status
  - crates/api/src/notifications/mod.rs#symbol:render
  - crates/api/src/notifications/mod.rs#symbol:readable_date
  - crates/api/src/notifications/mod.rs#symbol:PER_USER_MAX
  - crates/api/src/notifications/handlers.rs#symbol:list
  - crates/api/src/notifications/handlers.rs#symbol:mark_read
  - crates/api/src/notifications/handlers.rs#symbol:get_settings
  - crates/api/src/notifications/handlers.rs#symbol:update_settings
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/05_lifecycle.md
  - docs/llm-wiki/domains/09_ai_jobs.md
visibility: public
contains_sensitive_info: false
---

# Notifications

## Responsibility

The in-app inbox: notifications created by status changes, preview feedback and release dates; marking them read; and each person's notification settings. Session only — no permission, and everyone sees only their own.

## Stored as key and parameters

A notification row holds `key` and `params` (JSONB), never rendered text. `render` builds title and body on every read in the reader's `Accept-Language`, so the text follows the language even for notifications received earlier, as in the mock. A title gets a `[#id] ` prefix when it refers to a request. Dates in texts are written for people with `readable_date` (`11월 5일 (목)` / `Thu, Nov 5`). Adding a notification kind means adding its text in both languages to `text`; a key without one shows the key itself as the title.

## Who is told what

`notify_status` picks recipients by the new status. "Developers" are the request's responsible developers: assignee first, then project owners, without duplicates (`StoredRequest::responsible`).

| New status | Recipient | Key | Action required |
| --- | --- | --- | --- |
| `scope_check` | requester | `scopeCheck` | yes |
| `info_needed` | requester | `infoNeeded` | yes |
| `review` | developers | `scopeApproved` after `approveScope`, else `reportReady` | yes |
| `reporting` after `provideInfo` | developers | `requesterAnswered` | no |
| `in_progress` | requester | `workStarted` | no |
| `code_review` | developers; requester too when a preview exists | `workFinished`; `previewReady` | yes; no |
| `verification` | requester | `pleaseCheck` (with the criteria count) | yes |
| `changes_requested` | developers | `issueReported` | yes |
| `release_scheduled` | developers | `scheduleRelease` | yes |
| `done` / `rejected` | requester | `released` / `rejected` | no |
| `cancelled` | developers | `cancelled` | no |

Outside status changes: preview feedback tells developers `previewGood` or `previewChange` (action required only for a change), and setting a release date tells the requester `releaseDateSet`. `submitted` and other entries into `reporting` notify no one.

## Delivery rules

Applied when a notification is created, so changing settings does not touch stored ones: the person who caused it is never notified; nothing is stored when the recipient turned in-app off, or has "only when action is required" on and the notification does not require action. Without saved settings a person counts as in-app on, action-only off. **E-mail and messenger are settings only — the server delivers nothing on them.** Each person keeps the newest `PER_USER_MAX` (200); older rows are deleted in the same transaction. `notify` runs in the caller's transaction, so a rolled-back transition leaves no notification.

## API Services

| Operation | Endpoint | Notes |
| --- | --- | --- |
| `listNotifications` | `GET /notifications` | newest 100, rendered in the request's language |
| `markNotificationRead` | `POST /notifications/{id}/read` | someone else's, unknown or malformed id → 404; keeps the first `read_at` |
| `markAllNotificationsRead` | `POST /notifications/read-all` | 204 |
| `getNotificationSettings` | `GET /me/notification-settings` | default `{ inApp: true, email: true, messenger: false, onlyActionRequired: false }` |
| `updateNotificationSettings` | `PUT /me/notification-settings` | all four booleans required (one 400 listing the missing ones) |

## Evidence

- `crates/api/src/notifications/mod.rs#symbol:notify`, `crates/api/src/notifications/mod.rs#symbol:notify_status`, `crates/api/src/notifications/mod.rs#symbol:PER_USER_MAX` — recipients, delivery rules, cap.
- `crates/api/src/notifications/mod.rs#symbol:render`, `crates/api/src/notifications/mod.rs#symbol:readable_date` — render on read.
- `crates/api/src/notifications/handlers.rs#symbol:list`, `crates/api/src/notifications/handlers.rs#symbol:mark_read`, `crates/api/src/notifications/handlers.rs#symbol:get_settings`, `crates/api/src/notifications/handlers.rs#symbol:update_settings` — operations.
- Tests: `crates/api/tests/transitions.rs` (`notifications_are_read_and_follow_settings`); unit tests `readable_dates`, `notifications_read_like_the_mock` in `crates/api/src/notifications/mod.rs`.

## Open Questions

- E-mail and messenger delivery do not exist; the settings are stored for a future channel.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
