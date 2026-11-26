---
title: Runner
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
  - crates/api/src/ai/runner.rs
  - crates/api/src/runner
  - crates/api/migrations/20261119000008_runner_claims.sql
  - deploy/runner.Dockerfile
  - crates/api/tests/runner.rs
evidence:
  - crates/api/src/ai/runner.rs#symbol:authorize
  - crates/api/src/ai/runner.rs#symbol:claim
  - crates/api/src/ai/runner.rs#symbol:heartbeat
  - crates/api/src/ai/runner.rs#symbol:result
  - crates/api/src/ai/runner.rs#symbol:claimed
  - crates/api/src/ai/runner.rs#symbol:LEASE
  - crates/api/src/runner/mod.rs#symbol:Runner
  - crates/api/src/runner/mod.rs#symbol:RunError
  - crates/api/src/runner/mod.rs#symbol:unverify_wiki
  - crates/api/src/runner/agent.rs#symbol:run
  - crates/api/src/runner/agent.rs#symbol:report_schema
  - crates/api/src/runner/agent.rs#symbol:work_prompt
  - crates/api/src/runner/github.rs#symbol:pull_request
  - crates/api/src/runner/git.rs#symbol:Git
  - crates/api/src/config.rs#L236-L266
  - crates/api/src/config.rs#L319-L320
  - deploy/compose.yaml#L75-L94
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/09_ai_jobs.md
  - docs/llm-wiki/domains/03_projects.md
  - docs/llm-wiki/OPERATIONS.md
  - docs/llm-wiki/SECURITY.md
visibility: public
contains_sensitive_info: false
reviewed_at: 2026-11-23
---

# Runner

## Responsibility

Running report and work jobs with Claude Code on a clone of the project's GitHub repository. The server side hands out jobs over HTTP; the `ax-api runner` process (no database access) claims one at a time, runs the agent, and posts the result. The server applies it through the same code as in-process jobs, so its rules hold for runner answers too ([AI Jobs](09_ai_jobs.md)).

## Server side

- **Mode switch**: with `AX_RUNNER_TOKEN` set (32+ characters) the server starts no AI worker; `report` and `work` jobs wait for a runner. Without it the runner routes answer `404`.
- **Routes outside the contract** (`routes::router`, under the base path): `POST /runner/claim`, `POST /runner/jobs/{id}/heartbeat`, `POST /runner/jobs/{id}/result`. Not in `implemented.rs`; contract syncs do not touch them.
- **Auth** (`authorize`): `Authorization: Bearer <AX_RUNNER_TOKEN>`, compared as SHA-256 digests; missing or wrong → `401`.
- **Claim**: `204` when nothing is due; else `{ job { id, claim, task, lang, attempt, leaseSeconds }, selection, budgetUsd, request, project }`. A claim leases the job for `LEASE` (10 minutes) under a fresh claim id; `budgetUsd` is what is left of the per-request limit in USD (null when unlimited). A job already past `MAX_ATTEMPTS` is failed as "the runner stopped answering" instead of handed out.
- **Heartbeat** `{ claim, phase? }` extends the lease and, for work, records `editing | testing | opening_mr` in `details.work.phase`.
- **Result** `{ claim, report | work | failure, spend[] }`; a body without the task's key fails retryably. Every `spend` item is booked with the provider's coding-agent runtime (`agent_runtime`, `claude_code` for anthropic), a report included: a runner always runs the agent, never a single API call. Only the live claim is accepted (`claimed`): a late answer from a runner whose lease expired is `409`.
- **Lease expiry**: an expired running job is claimable again by the next claim, counting another attempt; after three, it is given up and handed to a person.

## Runner process

- `ax-api runner` (`RunnerConfig`): `AX_RUNNER_SERVER`, `AX_RUNNER_TOKEN`, `AX_GITHUB_TOKEN`, a Claude key (`AX_ANTHROPIC_API_KEY` or the per-task ones) required; `AX_GITHUB_API` (default the public GitHub API), `AX_RUNNER_WORKDIR`, `AX_RUNNER_TIMEOUT` (30m), `AX_CLAUDE_BIN` optional. Polls every 5 s, heartbeats every 60 s, one job at a time in `job-<id>-<first 8 characters of the claim>` (deleted afterwards).
- Only `github` projects with an `http(s)://host/owner/repo` URL (a `.git` suffix is allowed) are processed; others fail without retry.
- **Git credentials never reach the agent**: they are injected per git command as an `http.<origin>.extraheader` through `GIT_CONFIG_*` environment variables, not written to the clone, and masked in errors.
- **Failures** (`RunError`): every failed git command retries (authentication errors included); GitHub REST calls retry on 429, 5xx and network errors and fail on other 4xx; Claude Code that cannot start or breaks off with an I/O error retries; a timeout, a run without a result, an agent that did not finish and a spent budget fail without retry.
- **Claude Code flags** (`runner/agent.rs#symbol:run`): always `--bare --no-session-persistence --output-format json --json-schema …`, plus `--max-budget-usd <budgetUsd>` when the request has a per-request limit (a remaining budget of 0 fails the run before it starts), the task's key as `ANTHROPIC_API_KEY`, auto-update disabled.
  - **report** — read-only: `--permission-mode dontAsk` with `Bash, Edit, Write, NotebookEdit, WebFetch, WebSearch` disallowed, on a shallow clone of the default branch. The answer follows a closed schema (`report_schema`). The runner keeps only wiki pages under the project's wiki path and code files that exist, inside the repository, and decides `verified` from each page's front matter — the model cannot vouch for a page.
  - **work** — edits: `--permission-mode bypassPermissions` with only web tools disallowed, so the agent can edit and run tests in the clone. The prompt (`work_prompt`) forbids committing, pushing, merging, opening pull requests, marking wiki pages reviewed or verified, and touching secrets; without git credentials it could not push anyway.
- **Work flow**: branch `ax/request-<id>` (or the one already on the request; a revision continues the same branch and pull request) → `editing` → agent → no changed files means outcome `error` with "no files changed" → `unverify_wiki` → `opening_mr` → commit `[AX #<id>] <title> (run <attempt>)` → push → reuse the open pull request for the branch or open one against the default branch (`pull_request`) → report size and changed wiki pages.
- **Wiki review state**: `unverify_wiki` turns `status: verified` into `needs_review` in every wiki page the agent changed or added, before committing. This rule lives in the runner, not the server.
- **Nothing merges**: there is no merge call; protect the default branch and give the token Contents and Pull requests write only ([Security](../SECURITY.md)).

## Image and deployment

`deploy/runner.Dockerfile`: the release binary on Node 22 slim with git, Python 3 and Claude Code (`CLAUDE_CODE_VERSION`, default latest), running as the non-root `node` user with entrypoint `ax-api runner`. In `deploy/compose.yaml` it is the `ax-runner` service under `--profile runner`, talking to `http://ax-backend:8080` ([Operations](../OPERATIONS.md)).

## Evidence

- `crates/api/src/ai/runner.rs#symbol:authorize`, `crates/api/src/ai/runner.rs#symbol:claim`, `crates/api/src/ai/runner.rs#symbol:heartbeat`, `crates/api/src/ai/runner.rs#symbol:result`, `crates/api/src/ai/runner.rs#symbol:claimed`, `crates/api/src/ai/runner.rs#symbol:LEASE` — server side.
- `crates/api/src/runner/mod.rs#symbol:Runner`, `crates/api/src/runner/mod.rs#symbol:RunError`, `crates/api/src/runner/mod.rs#symbol:unverify_wiki` — the loop, failures, wiki review state.
- `crates/api/src/runner/agent.rs#symbol:run`, `crates/api/src/runner/agent.rs#symbol:report_schema`, `crates/api/src/runner/agent.rs#symbol:work_prompt` — Claude Code invocation.
- `crates/api/src/runner/github.rs#symbol:pull_request`, `crates/api/src/runner/git.rs#symbol:Git` — pull requests and credentials.
- `crates/api/src/config.rs#L236-L266` — runner settings (`RunnerConfig`); `crates/api/src/config.rs#L319-L320` — poll and heartbeat intervals.
- `deploy/compose.yaml#L75-L94` — the `ax-runner` service.
- Tests: `crates/api/tests/runner.rs` (unix only; fake GitHub, fake `claude`, local bare repository): `a_runner_reports_with_evidence_then_opens_a_pull_request`, `runner_routes_need_the_token_and_a_live_claim`.

## Open Questions

- Not yet run against real GitHub and Claude.
- The server trusts the runner's `verified` flags; whoever holds the runner token is inside the trust boundary.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
