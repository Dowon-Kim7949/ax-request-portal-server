---
title: API Contracts
tags:
  - llm-wiki
  - verified
status: verified
doc_type: api_contracts
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - scripts/sync-contract.sh
  - crates/contract/src/lib.rs
evidence:
  - crates/contract/src/lib.rs#symbol:operations
  - crates/contract/src/lib.rs#symbol:lifecycle
  - crates/contract/src/lib.rs#symbol:source_commit
  - crates/api/src/routes.rs#symbol:contract_routes
  - README.md#section:How the contract stays in step
  - .github/workflows/ci.yml#L46-L53
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/domains/12_platform.md
  - docs/llm-wiki/domains/05_lifecycle.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# API Contracts

## Where the contract lives

The API is defined by the frontend template, not here. Its `src/api/contract.ts` generates `docs/openapi.json` (every operation, schema and error) and `docs/lifecycle.json` (statuses, transitions, guards, turns, stages). This repository keeps byte-for-byte copies in `contract/` at the frontend commit named in `contract/SOURCE`, embeds them in the binary (`ax-contract`), and tests against them:

- every contract operation is routed (`routes::contract_routes`, `implemented::IMPLEMENTED`);
- the lifecycle port matches `lifecycle.json` entry by entry ([Lifecycle](domains/05_lifecycle.md));
- role names, error codes and request bodies match the contract's enums and schemas (unit and integration tests named `…_contract…`).

## Changing the API

1. Change `src/api/contract.ts` in the frontend and merge it there (contract changes are made in that repository by pull request).
2. Here: `bash scripts/sync-contract.sh main` fetches both files at that commit, rewrites `contract/SOURCE`, and moves the default `FRONTEND_REF` in `deploy/compose.yaml` to the same commit so the bundled web image matches.
3. Build and test: a new operation answers `501` until implemented; then add the handler, its `operationId` in `IMPLEMENTED` and its arm in `routes::implemented`.
4. CI's `contract` job (`sync-contract.sh --check`) fails when the copies, `SOURCE` and `FRONTEND_REF` disagree.

## Outside the contract

`GET /files/{id}` (signed attachment links) and `POST /runner/claim`, `POST /runner/jobs/{id}/heartbeat`, `POST /runner/jobs/{id}/result` (the runner protocol), plus `/healthz` and `/readyz` at the root. Syncing the contract does not change them ([Drafts and Attachments](domains/06_drafts_attachments.md), [Runner](domains/11_runner.md)).

## Conventions every operation follows

- JSON in the contract's camelCase; errors as `{ code, message, issues? }` with one of nine codes; `message` in the `Accept-Language` language ([Platform](domains/12_platform.md)).
- Bearer tokens in `Authorization` for everything but `login` and `completeSetup` ([Auth and Sessions](domains/01_auth_sessions.md)).
- Behaviour matches the frontend's mock API, so the template's screens and tests hold against the real server.

## Evidence

- `crates/contract/src/lib.rs#symbol:operations`, `crates/contract/src/lib.rs#symbol:lifecycle`, `crates/contract/src/lib.rs#symbol:source_commit` — the embedded contract.
- `crates/api/src/routes.rs#symbol:contract_routes` — every operation routed.
- `scripts/sync-contract.sh` — sync and `--check`.
- `.github/workflows/ci.yml#L46-L53` — the CI pin check.
- `README.md#section:How the contract stays in step` — the documented workflow.

## Open Questions

- `README.md` ("Sign-in") still says the contract lists only `200` and `400` for `login`; the pinned contract now also declares `401`.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
