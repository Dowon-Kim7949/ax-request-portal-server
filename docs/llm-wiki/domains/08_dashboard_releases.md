---
title: Dashboard and Releases
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
  - crates/api/src/dashboard.rs
  - crates/api/src/calendar.rs
  - crates/api/tests/dashboard.rs
evidence:
  - crates/api/src/dashboard.rs#symbol:summary
  - crates/api/src/dashboard.rs#symbol:releases
  - crates/api/src/dashboard.rs#symbol:is_delayed
  - crates/api/src/calendar.rs#symbol:add_days
  - crates/api/src/calendar.rs#symbol:is_year_month
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/05_lifecycle.md
  - docs/llm-wiki/domains/10_ai_settings_budget.md
visibility: public
contains_sensitive_info: false
---

# Dashboard and Releases

## Responsibility

Two read-only views computed from requests: the executive summary of a month and the release schedule. Every date is a **business date** in `AX_TIME_ZONE` (default `Asia/Seoul`, checked against PostgreSQL's time zone names at start-up).

## Summary (`dashboard`)

`GET /dashboard/summary?month=YYYY-MM` (default: this month), `dashboard:view` (executives and admins).

- `received`: requests filed in the month; `receivedPrevMonth` counts the previous month only up to the same day (today's day for the running month, the whole month otherwise), so a running month is compared fairly.
- `inProgress`: every non-terminal request. `delayed`: non-terminal and past its scheduled release date, or past its desired date unless it is already `release_scheduled` (`is_delayed`).
- `done`: `done` requests whose `releasedAt` falls in the month; `avgLeadTimeDays` the mean days from filing to release, one decimal, omitted when there are none.
- `aiCostKrw`, `aiBudgetKrw`, `aiAlertThresholdPct` from the AI ledger and budget ([AI Settings and Budget](10_ai_settings_budget.md)); a monthly limit of 0 means none.
- `byProject` (every project, oldest first), `upcomingReleases` (`release_scheduled` requests dated today through seven days ahead, inclusive, grouped by date and project), `attention` (`infoNeeded`, delayed ids).
- All requests are read once and counted in Rust; PostgreSQL only turns timestamps into business dates.

## Releases (`releases`)

`GET /releases?from&to` (inclusive ISO dates), `schedule:view` (every role but operator). A request's date is its `releasedAt` business date, else its scheduled date; requests with neither, and cancelled or rejected ones, are left out. Grouped by `(date, projectId)` in date order; each item says `canOpen`, true exactly when the caller could open the request, so the schedule never links to a `404`. `releases` repeats the rule of `StoredRequest::can_read` (requester, `request:read:all`, assignee, project owner) in its own closure rather than calling it.

Both operations check the query (400 naming `month`, `from` or `to`) **before** the permission (403), as the mock does.

## Evidence

- `crates/api/src/dashboard.rs#symbol:summary`, `crates/api/src/dashboard.rs#symbol:is_delayed` — the summary.
- `crates/api/src/dashboard.rs#symbol:releases` — the schedule and its `canOpen` rule (the same as in [Requests](04_requests.md)).
- `crates/api/src/calendar.rs#symbol:add_days`, `crates/api/src/calendar.rs#symbol:is_year_month` — business-calendar arithmetic without time zones.
- Tests: `crates/api/tests/dashboard.rs` (`months_follow_the_business_calendar`, `open_work_delays_and_the_weeks_releases`, `who_sees_the_summary`, `the_release_schedule`).

## Open Questions

- Reading every request per summary is fine at the expected size; a large installation would want SQL aggregation.
- `canOpen` duplicates the read rule of `StoredRequest::can_read`; a change to one must be made in the other.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
