---
title: LLM-WIKI Change Log
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: change_log
project: ax-request-portal-server
last_updated: 2026-11-26
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - Cargo.toml
evidence:

related:
  - docs/llm-wiki/index.md
visibility: public
contains_sensitive_info: false
---

# LLM-WIKI Change Log

This is an append-only change log. Do not edit existing entries; add new changes at the top.

## 2026-11-26 - Compose's published ports are settings

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - deploy/compose.yaml, deploy/.env.example: the web's and the API's host ports come from `AX_WEB_PORT` (default 8088) and `AX_API_PORT` (default 8080, still on 127.0.0.1 only)
  - README.md, README.ko.md: the Compose-only settings, ports included
  - docs/llm-wiki: OPERATIONS, domains/11_runner (the runner service's line range moved)
- evidence:
  - deploy/compose.yaml
- caveats:
  - The edited documents are back to needs_review.
  - Ports are read by Compose only; the server's own `AX_BIND` is unchanged.

## 2026-11-26 - A run that changed nothing is judged by the open pull request

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/runner/mod.rs: a work run with no changed file keeps the open pull request's link and is `success` when tests ran and pass, `tests_failed` when they fail, `error` otherwise or without an open pull request (`Unchanged`)
  - crates/api/src/runner/github.rs: `open_pull_request` looks the pull request up without opening one
  - docs/llm-wiki: domains/11_runner
- evidence:
  - crates/api/tests/runner.rs (`a_runner_reports_with_evidence_then_opens_a_pull_request`), crates/api/src/runner/mod.rs (`a_run_that_changed_nothing_is_judged_by_the_open_pull_request`)
- caveats:
  - The edited document is back to needs_review.
  - The contract's outcomes are unchanged; a check-only success reads like any other success, told apart by its summary.

## 2026-11-26 - A work run installs dependencies before it tests

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/runner/agent.rs: the work prompt has the agent install dependencies first with the project's lockfile, restore tracked files the install changes, fix the cause when tests cannot run, and answer `error` when they still cannot
  - docs/llm-wiki: domains/11_runner
- evidence:
  - crates/api/src/runner/agent.rs (`a_work_run_installs_before_it_tests`)
- caveats:
  - The edited document is back to needs_review.
  - This is an instruction to the agent; whether the tests really ran is still read from its answer.

## 2026-11-26 - A work run is held to the scope the requester confirmed

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/runner/agent.rs: the request brief carries the confirmed scope in words (`confirmed_scope`) instead of the decision's ids; the work prompt says the confirmed scope wins over the plan and open questions
  - docs/llm-wiki: domains/11_runner
- evidence:
  - crates/api/src/runner/agent.rs (`the_confirmed_scope_reaches_the_agent_in_words`), crates/api/tests/runner.rs (`a_runner_reports_with_evidence_then_opens_a_pull_request`)
- caveats:
  - The edited document is back to needs_review.
  - Staying within the scope is asked of the agent, not enforced; the developer still reviews the pull request.

## 2026-11-26 - A refused token costs nothing; a work run shows its whole cost

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/runner/mod.rs: a work run checks with `git push --dry-run` that the token may push before the agent runs
  - crates/api/src/runner/git.rs: a refused token (401, 403, 404, failed sign-in, no write access) fails without retry (`refused`); git runs with `LC_ALL=C`
  - crates/api/src/ai/jobs.rs, crates/api/src/ai/usage.rs: `work.costKrw` is every attempt's cost since the run started (`spent_since`), not the last attempt's
  - docs/llm-wiki: domains/09_ai_jobs, 10_ai_settings_budget, 11_runner, SECURITY
- evidence:
  - crates/api/tests/runner.rs (`a_token_that_cannot_push_ends_the_work_before_the_agent_runs`), crates/api/tests/ai.rs (`a_failing_job_is_retried_then_given_up`), crates/api/src/runner/git.rs (`a_refused_token_is_not_tried_again`)
- caveats:
  - The edited documents are back to needs_review.
  - Recognising a refusal reads git's English error text; a message not in the list still retries.

## 2026-11-26 - Wiki trust counts only pages about the software

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/runner/mod.rs: a report's wiki evidence leaves out the wiki's own navigation and bookkeeping pages (`navigation_page`, by `doc_type` or standard path); `front_matter` also reads `doc_type`
  - crates/api/src/runner/agent.rs: the report prompt asks for pages about the software, not the index, README, profiles, log, templates or guides
  - docs/llm-wiki: domains/09_ai_jobs, 11_runner, GLOSSARY
- evidence:
  - crates/api/tests/runner.rs (`a_runner_reports_with_evidence_then_opens_a_pull_request`), crates/api/src/runner/mod.rs (`navigation_pages_are_no_evidence`)
- caveats:
  - The edited documents are back to needs_review.
  - The trust thresholds themselves (the frontend's `deriveWikiTrust`) are unchanged.

## 2026-11-26 - The usage ledger books how a call actually ran

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/ai/mod.rs, crates/api/src/ai/usage.rs: `Spend` and `Call` carry the call's `runtime`; `book` stores it instead of deriving it from the task
  - crates/api/src/ai/settings.rs: `agent_runtime(provider)`; `runtime_for` keeps the frontend's per-task default
  - crates/api/src/ai/runner.rs: a runner's spend is booked as the provider's coding agent for every task, a report included (it was booked as `api`)
  - crates/api/src/ai/claude.rs, crates/api/src/ai/jobs.rs, crates/api/src/requests/handlers.rs: the server's own calls are booked as `api`
  - docs/llm-wiki: domains/04_requests, 09_ai_jobs, 10_ai_settings_budget, 11_runner, SECURITY
- evidence:
  - crates/api/tests/runner.rs (`a_runner_reports_with_evidence_then_opens_a_pull_request`), crates/api/src/ai/settings.rs (unit tests)
- caveats:
  - The edited documents are back to needs_review. SECURITY only cites `runner.rs`; its statements still hold.
  - Rows booked before this change keep runtime `api` for runner reports; the ledger is never rewritten.

## 2026-11-23 - Open questions from the first write-up resolved

- status: needs_review
- actor: claude-code
- scope: code, docs
- changed:
  - crates/api/src/ai/jobs.rs: in-process jobs now hold a lease (`WORKER_LEASE`); a result applies only while the claim is live; jobs claimed past `MAX_ATTEMPTS` are given up
  - crates/api/src/requests/store.rs, crates/api/src/dashboard.rs: `store::may_open` is the one read rule behind `can_read` and the schedule's `canOpen`
  - README.md, README.ko.md, deploy/.env.example, crates/api/src/config.rs: stale sentences about `login` responses and work runs corrected
  - docs/llm-wiki: domains/00_overview, 04_requests, 08_dashboard_releases, 09_ai_jobs, 11_runner, API_CONTRACTS, OPERATIONS
- evidence:
  - crates/api/tests/ai.rs (`a_job_another_server_holds_waits_for_its_lease`, `a_job_its_workers_keep_dying_on_is_given_up`)
- caveats:
  - The edited documents are back to needs_review.

## 2026-11-23 - Wiki written from the code

- status: needs_review
- actor: claude-code
- scope: docs
- changed:
  - docs/llm-wiki/ (every document), domains renumbered 01-12 in dependency order (auth to platform)
  - CLAUDE.md (adapter block, wiki command, CI line)
  - .github/workflows/llm-wiki.yml (new; CLI pinned to llm-wiki-governance@1.42.0)
  - llm-wiki.config.json (type backend, mode standard, impact.source_changed error)
- summary:
  - Replaced every placeholder with source-backed content: 12 domain documents, overview, features, architecture conventions, API contracts, data model, operations, security, glossary, backend profile.
  - Hub files (`Cargo.toml`, `config.rs`, `error.rs`, `main.rs`, `state.rs`, `deploy/compose.yaml`, workflows) are cited by line range or section only, so unrelated edits do not trip `impact.source_changed` once documents are verified.
- evidence:
  - crates/api/src (per-domain `#symbol:` citations), crates/api/migrations, crates/api/tests
- caveats:
  - Nothing has run against the real Claude API or GitHub yet; the AI and runner documents describe tested behaviour against local fakes.
  - `README.md` ("Sign-in") says the contract lists only 200 and 400 for `login`; the pinned contract also declares 401 (noted in API_CONTRACTS.md).
  - `deploy/.env.example` says work runs wait for a coding agent either way; with a runner token the runner does them (noted in domains/11_runner.md).
- review items:
  - Every document is needs_review; a person approves each with `npx llm-wiki-governance@1.42.0 review --approve <path>`.
  - Two code observations recorded as open questions: `dashboard::releases` repeats the read rule of `StoredRequest::can_read` instead of calling it (domains/08); the in-process worker resets every unclaimed running job on start, so it assumes one instance per database (domains/09).
  - The statements were checked against the code by a second pass; its corrections are included.

## 2026-11-23 - LLM-WIKI initial documents created

- status: needs_review
- actor: llm-wiki-cli
- scope: docs
- changed:
  - docs/llm-wiki/
- summary:
  - Created the initial LLM-WIKI document structure with `llm-wiki init --write`.
- evidence:
  - Cargo.toml
- caveats:
  - This is a CLI-generated draft and requires human review.

