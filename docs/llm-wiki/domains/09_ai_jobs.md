---
title: AI Jobs
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
  - crates/api/src/ai/mod.rs
  - crates/api/src/ai/jobs.rs
  - crates/api/src/ai/agent.rs
  - crates/api/src/ai/claude.rs
  - crates/api/migrations/20261119000003_ai_jobs.sql
  - crates/api/tests/ai.rs
  - crates/api/tests/claude.rs
evidence:
  - crates/api/src/ai/jobs.rs#symbol:for_status
  - crates/api/src/ai/jobs.rs#symbol:enqueue_for
  - crates/api/src/ai/jobs.rs#symbol:claim
  - crates/api/src/ai/jobs.rs#symbol:complete
  - crates/api/src/ai/jobs.rs#symbol:MAX_ATTEMPTS
  - crates/api/src/ai/jobs.rs#symbol:given_up
  - crates/api/src/ai/jobs.rs#symbol:apply_report
  - crates/api/src/ai/jobs.rs#symbol:apply_work
  - crates/api/src/ai/jobs.rs#symbol:started_by
  - crates/api/src/ai/jobs.rs#symbol:request_for
  - crates/api/src/ai/jobs.rs#symbol:spawn
  - crates/api/src/ai/mod.rs#symbol:Provider
  - crates/api/src/ai/mod.rs#symbol:Reply
  - crates/api/src/ai/mod.rs#symbol:Failure
  - crates/api/src/ai/mod.rs#symbol:hand_over_report
  - crates/api/src/ai/mod.rs#symbol:hand_over_work
  - crates/api/src/ai/mod.rs#symbol:StubProvider
  - crates/api/src/ai/claude.rs#symbol:ClaudeProvider
  - crates/api/src/ai/claude.rs#symbol:API_VERSION
  - crates/api/src/ai/claude.rs#symbol:report_body
  - crates/api/src/ai/claude.rs#symbol:supports_fallbacks
  - crates/api/src/ai/agent.rs#symbol:report
  - crates/api/src/ai/agent.rs#symbol:wiki_trust
  - crates/api/src/ai/agent.rs#symbol:WIKI_DOCS_MAX
  - crates/api/src/main.rs#L90-L104
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/05_lifecycle.md
  - docs/llm-wiki/domains/10_ai_settings_budget.md
  - docs/llm-wiki/domains/11_runner.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# AI Jobs

## Responsibility

The queue of AI work on requests, the in-process worker, the providers that do the work (stub and Claude), and the rules that turn an AI answer into a report or a work result and an AI lifecycle event. The precheck is not a job; it is a synchronous call from the request handler ([Requests](04_requests.md)).

## Jobs

- **Two tasks**: `report` for a request entering `reporting`, `work` for one entering `in_progress` (`Task::for_status`). Jobs are queued in the same transaction as filing or the human transition, in the requester's language for reports. At most one open job per `(request, task)` (partial unique index; `on conflict do nothing`).
- **States** `queued → running → done | failed`. `claim` takes the oldest due job with `for update skip locked` and increments `attempts`. In-process jobs carry no claim or lease; runner jobs carry both ([Runner](11_runner.md)).
- **Retries**: a retryable failure goes back to `queued` after `attempts × 30` s while `attempts < MAX_ATTEMPTS` (3); otherwise the job is `failed` with its error.
- **Giving up hands the request to a person** (`given_up`), in the same transaction, so a request is never stuck in an AI step: a report becomes `hand_over_report` (verdict `undetermined`, every criterion left for a person) and `reportReady` moves the request to `review`; work becomes `hand_over_work` (outcome `error`, no code changed) and `workFinished` moves it to `code_review`.
- **A request that moved on is left alone**: `request_for` only returns the request while it still waits in the job's status; a cancelled or otherwise moved request closes the job as `done`. `complete` locks the request and applies only if the status still matches.
- On start the worker returns **every** `running` job without a claim to `queued` (left over from a crash); it polls every 2 s (5 s after a database error). Because the reset does not tell instances apart, run one in-process worker — or the runner — per database.
- **Where the job runs**: `AX_RUNNER_TOKEN` set → no in-process worker, jobs wait for the runner. Otherwise `AX_AI_PROVIDER=anthropic` runs `ClaudeProvider`, else `StubProvider` (`main.rs`).

## Applying results

- `apply_report` increments the report `revision`, stamps `generatedAt`, and emits `scopeReady` (request → `scope_check`) only when `AX_SCOPE_CHECK` is on, the verdict is feasible or conditional and a scope proposal exists; otherwise `reportReady` (→ `review`).
- `apply_work` sets `work.phase = finished`, outcome, summary, cost, merge request, tests, wiki changes, preview, and the regression list when non-empty; emits `workFinished` (→ `code_review`).
- The timeline records these events with actor `ai`. Spending (successful or not) is booked to `started_by`: the person who last moved the request into that status, else the requester ([AI Settings and Budget](10_ai_settings_budget.md)).
- `work` runs with the agent snapshotted on the request when work started; reports resolve the current selection for the project.

## Providers

- `Provider::report` / `work` return `Reply { outcome: Result<T, Failure>, spend }`; `Failure { message, retry }`. `Spend.cost_usd`, when given, is booked as is instead of from the price table.
- **Stub** (`AX_AI_PROVIDER=stub`, default): reports honestly that no provider is connected (a hand-over report) and does no work.
- **Claude** (`AX_AI_PROVIDER=anthropic`): calls the Messages API with `x-api-key` and `anthropic-version` `API_VERSION`; the key comes from `AX_ANTHROPIC_API_KEY_<TASK>`, else `AX_ANTHROPIC_API_KEY`. Reports use effort `high`, JSON-schema output, a cached system prompt, 16,000 max tokens, a 600 s timeout, and `fallbacks: "default"` only for models that accept it (`supports_fallbacks`; Haiku does not). 429, 5xx, network errors and unparsable answers retry; other 4xx do not; a `refusal` or `max_tokens` stop fails without retry but its cost is still booked.
- **Claude without the runner reads neither code nor wiki**, so the server fixes its verdict: `undetermined` (`out_of_scope` for `non_code` requests), `wikiTrust: low`, empty plan, no scope proposal — it writes the verification method, questions, assumptions, rule conflicts, screens to recheck, size and risks. Its `work` always answers "not connected"; real work needs the [Runner](11_runner.md).

## Rules the server enforces on agent answers (`ai::agent`)

Used for runner answers, so a model cannot claim more than its evidence:

- `wikiTrust` from the cited wiki pages' review state (decided by the runner from the files, not by the model): all verified → high, at least half → medium, else (or none) → low.
- Verdict: `non_code` → `out_of_scope`; no wiki evidence → `undetermined` (plan and scope cleared); a claimed `feasible` stays feasible only with no open questions, trust above low and no rule conflict, otherwise `conditional`.
- Limits: `WIKI_DOCS_MAX` 10, code files 20, plan 10, scope items 5, questions 3 with 2–4 options; scope ids (`q1`, `q1o1`) are assigned by the server.
- Work: unknown outcome → `error`; a merge request is kept only with a positive number and an http(s) URL; tests only when all four counts are present (passed clamped to total); `wikiChanges.pendingReview` is always true; regression checks are created only with a merge request, one per screen to recheck.

## Evidence

- `crates/api/src/ai/jobs.rs#symbol:for_status`, `crates/api/src/ai/jobs.rs#symbol:enqueue_for`, `crates/api/src/ai/jobs.rs#symbol:claim`, `crates/api/src/ai/jobs.rs#symbol:complete`, `crates/api/src/ai/jobs.rs#symbol:MAX_ATTEMPTS`, `crates/api/src/ai/jobs.rs#symbol:request_for`, `crates/api/src/ai/jobs.rs#symbol:spawn` — the queue.
- `crates/api/src/ai/jobs.rs#symbol:given_up`, `crates/api/src/ai/mod.rs#symbol:hand_over_report`, `crates/api/src/ai/mod.rs#symbol:hand_over_work` — hand-over on final failure.
- `crates/api/src/ai/jobs.rs#symbol:apply_report`, `crates/api/src/ai/jobs.rs#symbol:apply_work`, `crates/api/src/ai/jobs.rs#symbol:started_by` — results and booking.
- `crates/api/src/ai/mod.rs#symbol:Provider`, `crates/api/src/ai/mod.rs#symbol:Reply`, `crates/api/src/ai/mod.rs#symbol:Failure`, `crates/api/src/ai/mod.rs#symbol:StubProvider` — the provider seam.
- `crates/api/src/ai/claude.rs#symbol:ClaudeProvider`, `crates/api/src/ai/claude.rs#symbol:API_VERSION`, `crates/api/src/ai/claude.rs#symbol:report_body`, `crates/api/src/ai/claude.rs#symbol:supports_fallbacks` — the Claude provider.
- `crates/api/src/ai/agent.rs#symbol:report`, `crates/api/src/ai/agent.rs#symbol:wiki_trust`, `crates/api/src/ai/agent.rs#symbol:WIKI_DOCS_MAX` — server-side verdict rules.
- `crates/api/src/main.rs#L90-L104` — worker or runner.
- Tests: `crates/api/tests/ai.rs` (`the_stub_reports_honestly_and_the_developer_reviews`, `a_request_that_moved_on_is_left_alone`, `a_feasible_report_asks_the_requester_first`, `a_failing_job_is_retried_then_given_up`), `crates/api/tests/claude.rs` (`claude_writes_a_report_that_claims_nothing_it_did_not_read`, `a_refusal_or_a_rejected_call_is_not_retried`, `haiku_takes_no_fallback_and_a_non_code_request_is_out_of_scope`).

## Open Questions

- Neither the Claude provider nor the runner has been run against the real API or GitHub yet; tests use local fakes.
- Two server instances with in-process workers would reset each other's running jobs on start; the code assumes one.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
