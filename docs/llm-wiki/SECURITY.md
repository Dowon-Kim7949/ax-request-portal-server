---
title: Security
tags:
  - llm-wiki
  - needs-review
status: needs_review
doc_type: security
project: ax-request-portal-server
last_updated: 2026-11-26
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/src/auth
  - crates/api/src/roles.rs
evidence:
  - crates/api/src/auth/token.rs#symbol:PREFIX
  - crates/api/src/auth/password.rs#symbol:decoy
  - crates/api/src/auth/throttle.rs#symbol:attempt
  - crates/api/src/auth/mod.rs#symbol:require
  - crates/api/src/attachments/mod.rs#symbol:INLINE_TYPES
  - crates/api/src/ai/runner.rs#symbol:authorize
  - crates/api/src/runner/git.rs#symbol:Git
  - crates/api/src/config.rs#L1-L30
  - CLAUDE.md#section:Rules
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/domains/01_auth_sessions.md
  - docs/llm-wiki/domains/06_drafts_attachments.md
  - docs/llm-wiki/domains/11_runner.md
  - docs/llm-wiki/OPERATIONS.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
---

# Security

## Identity

- **Bearer sessions, no cookies**: `axs_` + 256 random bits, only the SHA-256 stored; idle and absolute expiry; ended at once on logout, password reset and deactivation ([Auth and Sessions](domains/01_auth_sessions.md)). There is no cookie, so no CSRF surface; the frontend keeps the token.
- **Passwords**: Argon2id (19 MiB, 2 passes, 1 lane). Every sign-in failure looks the same, and a decoy hash keeps timing from revealing which emails exist.
- **Guessing**: per-email lockout counted before the password check, unknown emails alike. Trade-off: anyone can keep a known email locked; admins recover with a setup link.
- **Setup links**: `axl_` tokens, digest only, one per person, single use inside a transaction, 72 h by default, never for operators.

## Authorisation

- Every operation checks permissions on the server (`auth::require` with `roles.rs`, or request-dependent checks in the lifecycle). A request the caller may not read answers `404`, not `403`, so ids cannot be probed.
- **Operators are provisioned outside the API** (`ax-api user create`); the API refuses to create, promote or edit them (`403`). Operators hold only budget, agent and usage permissions; admins hold none of those ([Users, Teams and Roles](domains/02_users_teams_roles.md)).

## Secrets

- Provider keys, the runner token and the GitHub token come **only** from the environment (`deploy/.env`, git-ignored and Docker-ignored). The API never accepts or returns a key; it shows only "connected" and the last four characters. `Secret` values print as `Secret(…)` in logs.
- The attachment link key is generated on first start and kept in `server_secrets`; nothing to configure.
- The repository is public: no internal hosts, organisation or customer names, real data, keys or passwords. CI runs gitleaks over the whole history.

## Files

Uploads are not type-checked; serving is: only `INLINE_TYPES` (images and PDF) open in place, everything else (SVG and HTML included) downloads as `application/octet-stream`, always with `nosniff`. Links are HMAC-signed, expire in 12–24 h, and are verified in constant time ([Drafts and Attachments](domains/06_drafts_attachments.md)).

## The AI and the runner

- The AI cannot send a person's lifecycle event, so it cannot approve scope, merge, results or releases ([Lifecycle](domains/05_lifecycle.md#human-checkpoints)).
- Runner calls need `AX_RUNNER_TOKEN` (compared as digests) and a live claim; without the token the routes do not exist (`404`).
- Report runs are read-only (`dontAsk`, editing and shell tools disallowed). Work runs may edit and run commands inside the clone (`bypassPermissions`, web tools disallowed) but receive no git credentials: the runner injects them per git command and masks them in errors. Before the agent runs, a dry-run push checks that the token may push; a refused token fails the job without retry. There is no merge call; the GitHub token should hold only Contents and Pull requests write, with the default branch protected. Wiki pages the agent changes go back to `needs_review` ([Runner](domains/11_runner.md)).
- Whoever holds the runner token is trusted: the server accepts the runner's evidence (which wiki pages are verified) as given.

## Evidence

- `crates/api/src/auth/token.rs#symbol:PREFIX`, `crates/api/src/auth/password.rs#symbol:decoy`, `crates/api/src/auth/throttle.rs#symbol:attempt` — sessions, passwords, throttling.
- `crates/api/src/auth/mod.rs#symbol:require` — server-side permission checks.
- `crates/api/src/attachments/mod.rs#symbol:INLINE_TYPES` — safe serving.
- `crates/api/src/ai/runner.rs#symbol:authorize`, `crates/api/src/runner/git.rs#symbol:Git` — runner authentication, credentials kept from the agent.
- `crates/api/src/config.rs#L1-L30` — keys only from configuration.
- `CLAUDE.md#section:Rules` — repository rules on organisation data and the AI.

## Open Questions

- No per-IP rate limit exists; a deployment that needs one should add it at the proxy.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
