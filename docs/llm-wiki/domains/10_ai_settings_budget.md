---
title: AI Settings and Budget
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
  - crates/api/src/ai/settings.rs
  - crates/api/src/ai/usage.rs
  - crates/api/src/ai/handlers.rs
  - crates/api/migrations/20261119000006_ai_settings_and_usage.sql
  - crates/api/tests/ai_operations.rs
evidence:
  - crates/api/src/ai/settings.rs#symbol:lock
  - crates/api/src/ai/settings.rs#symbol:resolve
  - crates/api/src/ai/settings.rs#symbol:connected
  - crates/api/src/ai/settings.rs#symbol:connections
  - crates/api/src/ai/settings.rs#symbol:budget_input
  - crates/api/src/ai/settings.rs#symbol:LIMIT_MAX_KRW
  - crates/api/src/ai/settings.rs#symbol:runtime_for
  - crates/api/src/ai/settings.rs#symbol:agent_runtime
  - crates/api/src/ai/usage.rs#symbol:book
  - crates/api/src/ai/usage.rs#symbol:round_money
  - crates/api/src/ai/usage.rs#symbol:in_month
  - crates/api/src/ai/usage.rs#symbol:for_request
  - crates/api/src/ai/usage.rs#symbol:spent_since
  - crates/api/src/ai/usage.rs#symbol:request_usage
  - crates/api/src/ai/usage.rs#symbol:report_json
  - crates/api/src/ai/handlers.rs#symbol:update_settings
  - crates/api/src/ai/handlers.rs#symbol:set_provider_key
  - crates/api/src/ai/handlers.rs#symbol:update_pricing
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/09_ai_jobs.md
  - docs/llm-wiki/domains/05_lifecycle.md
  - docs/llm-wiki/domains/08_dashboard_releases.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
---

# AI Settings and Budget

## Responsibility

What operators manage: the budget, the model per AI task (with per-project overrides), the price table, and the usage report. Also the ledger every AI call is booked to, and the per-request cost guard.

## Settings

- Stored in `ai_config` as three JSON rows (`budget`, `selections`, `pricing`); a missing row means the template default. Every write first takes `pg_advisory_xact_lock(hashtext('ai_config'))` (`lock`), so cross-checks (removing a price vs selecting that model) cannot race.
- **Selections**: default `precheck` → `claude-haiku-5-5`, `report` and `work` → `claude-opus-5-5`; `resolve(project, task)` takes a project override first. `runtime_for` (the frontend's `runtimeFor`): work → the provider's coding agent (`agent_runtime`: anthropic → `claude_code`, another provider → `codex`), everything else → `api`. It names the work agent a run snapshots ([Lifecycle](05_lifecycle.md)); the ledger does not use it.
- `updateAiSettings` checks under the lock: each override's project exists (`projectNotFound`), no duplicate `(project, task)`; then every selection in use, defaults first, needs a connected provider (`providerNotConnected`) and a price (`modelNotPriced`).
- **"Connected"** (`connected`): only `anthropic`, only with a key configured, and only when something can use it — the server runs `AX_AI_PROVIDER=anthropic` or a runner token is set. With the stub, a configured key still shows "not connected". `openai` is never connected. The settings answer shows `connected` and a `keyHint` (last four characters of each distinct key) — never the key.
- **Keys come only from configuration** (`AX_ANTHROPIC_API_KEY`, `…_PRECHECK`, `…_REPORT`, `…_WORK` in `deploy/.env`): `setProviderKey` / `removeProviderKey` validate their input and permission, then always answer `403 forbidden` (built by `ApiError::provider_keys_from_config`), the same way operator accounts never come from the API.
- **Pricing**: USD per million tokens (input, cache read, cache write, output) per model plus a KRW rate and an as-of date. Removing the price of a model in use is `409 guard_failed` with the mock's "model in use" message.
- **Budget**: monthly limit and per-request limit in KRW (0 = none, at most `LIMIT_MAX_KRW`), alert threshold 1–100 %; with a monthly limit, the per-request limit may not exceed it (`perRequestLimitKrw`).

## The ledger (`ai_usage`)

- `book` writes one row per call with tokens, `cost_usd` and `cost_krw` priced **at booking time** — later price changes never rewrite history. Its `runtime` is how the call actually ran (`Call::runtime`, from `Spend::runtime`), not the task's default: a call this server makes is `api`; a runner's run is the provider's coding agent for every task, so a report a runner writes is `claude_code`. A model missing from the price table is logged and **not booked**. `team_id` is copied from the caller's user row at call time (a subquery in the insert), so moving a person between teams does not move past spending. Rows are never deleted.
- Precheck rows have no request id and are booked to the caller; job rows are booked to `started_by` ([AI Jobs](09_ai_jobs.md)). A failed attempt's calls are booked too; a work run's `costKrw` sums every attempt's (`spent_since`).
- Money: `round_money` rounds to 0.01; whole values serialise as JSON integers. Monthly totals and per-model sums are whole won; a request's `spentKrw` keeps two decimals. Months are business months in `AX_TIME_ZONE` (`in_month`).
- **Per-request limit blocks; the monthly limit only shows.** `runWork`, `requestRevision` and `reviseScope` are refused with `409 guard_failed` once the request's lifetime spending (`for_request`) has reached the per-request limit; `provideInfo` is never blocked. The monthly limit and threshold are displayed in the budget and dashboard. Request details show `aiUsage { spentKrw, limitKrw, entries }` to callers with `report:read` (`request_usage`).

## API Services

| Operation | Endpoint | Permission | Notes |
| --- | --- | --- | --- |
| `getBudget` / `updateBudget` | `GET` / `PUT /budget` | `budget:manage` | this month's spending and limits |
| `getAiSettings` / `updateAiSettings` | `GET` / `PUT /ai/settings` | `agent:manage` | key hints only |
| `setProviderKey` / `removeProviderKey` | `PUT` / `DELETE /ai/providers/{provider}/key` | `agent:manage` | always 403 after validation |
| `getAiPricing` | `GET /ai/pricing` | `agent:manage` or `usage:view` | |
| `updateAiPricing` | `PUT /ai/pricing` | `agent:manage` | 409 `guard_failed` (model in use) |
| `aiUsage` | `GET /ai/usage?month=YYYY-MM` | `usage:view` | by provider, model, task, day, request, team, person (`report_json`) |

Only operators hold these permissions ([Users, Teams and Roles](02_users_teams_roles.md)).

## Evidence

- `crates/api/src/ai/settings.rs#symbol:lock`, `crates/api/src/ai/settings.rs#symbol:resolve`, `crates/api/src/ai/settings.rs#symbol:runtime_for`, `crates/api/src/ai/settings.rs#symbol:agent_runtime`, `crates/api/src/ai/handlers.rs#symbol:update_settings` — selections and runtimes.
- `crates/api/src/ai/settings.rs#symbol:connected`, `crates/api/src/ai/settings.rs#symbol:connections`, `crates/api/src/ai/handlers.rs#symbol:set_provider_key` — connection display and keys from configuration.
- `crates/api/src/ai/settings.rs#symbol:budget_input`, `crates/api/src/ai/settings.rs#symbol:LIMIT_MAX_KRW`, `crates/api/src/ai/handlers.rs#symbol:update_pricing` — budget and pricing rules.
- `crates/api/src/ai/usage.rs#symbol:book`, `crates/api/src/ai/usage.rs#symbol:round_money`, `crates/api/src/ai/usage.rs#symbol:in_month`, `crates/api/src/ai/usage.rs#symbol:for_request`, `crates/api/src/ai/usage.rs#symbol:request_usage`, `crates/api/src/ai/usage.rs#symbol:report_json` — the ledger.
- Tests: `crates/api/tests/ai_operations.rs` (`models_per_task_need_a_connected_provider_and_a_price`, `provider_keys_come_only_from_the_configuration`, `budgets_follow_the_contracts_limits`, `usage_is_reported_and_stops_a_request_at_its_limit`).

## Open Questions

- Managing keys from the UI would need a contract change (keys in the API) and secret storage; today the design keeps them in configuration only.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
