---
title: LLM-WIKI Change Log
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: change_log
project: ax-request-portal-server
last_updated: 2026-11-23
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

