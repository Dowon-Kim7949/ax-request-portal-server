---
title: Glossary
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: glossary
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/requests/lifecycle.rs
  - crates/api/src/roles.rs
evidence:
  - crates/api/src/requests/lifecycle.rs#symbol:Status
  - crates/api/src/roles.rs#symbol:Role
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/05_lifecycle.md
visibility: public
contains_sensitive_info: false
---

# Glossary

Terms as this server uses them. Contract names (camelCase) are the frontend's; Rust names follow in parentheses where they differ.

| Term | Meaning |
| --- | --- |
| Contract | The frontend's `docs/openapi.json` and `docs/lifecycle.json`, copied into `contract/` at the commit in `contract/SOURCE` ([API Contracts](API_CONTRACTS.md)). |
| Mock API | The frontend's in-browser API (`src/mocks/engine.ts`). This server reproduces its rules, order of checks and messages. |
| Operation | One contract endpoint, named by its `operationId` (for example `transitionRequest`). |
| Request | A work request (`WorkRequest`; `StoredRequest` in Rust), numbered from 1. |
| Status / event | A request's state and the named transition that changes it; 13 statuses, 16 events ([Lifecycle](domains/05_lifecycle.md)). |
| Turn | Whose move a status waits for: AI, requester, developer or none. |
| Human checkpoint | A transition only a person can send (all requester and developer events); the AI never sends one. |
| Guard | A condition a transition checks besides the status (`criteriaAllPassed`, `scopeRevisionsUsed`, …); failing it is `409 guard_failed`. |
| Responsible developer | A developer or admin who is the request's assignee or one of its project's owners (admins always count for developer events). |
| Assignee | The developer a request is assigned to: the project's first owner at filing, or whoever starts the work. |
| Precheck | Advice on a draft before filing: rules first, then (when connected) Claude; never blocks ([Requests](domains/04_requests.md)). |
| `aiAdvice` | Whether the precheck model added its advice: `included`, `notConnected`, `failed`. |
| Report | The AI's feasibility report; its verdict is `feasible`, `conditional`, `undetermined` or `out_of_scope`. |
| Scope check | The optional step (`AX_SCOPE_CHECK`) where the requester confirms the AI's reading of the request before a developer reviews it. |
| Work | The AI's change on a branch (`ax/request-<id>`) with a pull request; `attempt` counts runs. |
| Job | A queued AI task (`report` or `work`) in `ai_jobs` ([AI Jobs](domains/09_ai_jobs.md)). |
| Provider | Who runs AI calls: `stub` (honest no-op), `anthropic` (Claude); `openai` exists only in settings and is never connected. |
| Runner | The `ax-api runner` process that claims jobs and runs Claude Code on GitHub clones ([Runner](domains/11_runner.md)). |
| Claim / lease | A runner's hold on a job: a claim id valid for a 10-minute lease, renewed by heartbeats. |
| Wiki trust | `high` / `medium` / `low` from how many of the cited wiki pages are `verified`. |
| Ledger | `ai_usage`: one row per AI call, priced at booking, with the caller's team at call time ([AI Settings and Budget](domains/10_ai_settings_budget.md)). |
| Business date | A date in `AX_TIME_ZONE` (default `Asia/Seoul`): "today", months, release dates. |
| Setup link | A one-time `axl_` link through which a person added in the app sets their password. |
| Operator | The role that manages AI budget, models and usage; sees no requests; created only by the CLI. |

## Evidence

- `crates/api/src/requests/lifecycle.rs#symbol:Status` — statuses, turns and stages.
- `crates/api/src/roles.rs#symbol:Role` — roles.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
