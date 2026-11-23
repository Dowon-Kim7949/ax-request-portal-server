---
title: Auth and Sessions
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
  - crates/api/src/auth
  - crates/api/src/accounts.rs
  - crates/api/migrations/20261118000000_users_teams_sessions.sql
  - crates/api/migrations/20261118000001_setup_links.sql
  - crates/api/migrations/20261119000007_login_attempts.sql
  - crates/api/tests/auth.rs
  - crates/api/tests/setup_links.rs
evidence:
  - crates/api/src/auth/handlers.rs#symbol:login
  - crates/api/src/auth/mod.rs#symbol:CurrentUser
  - crates/api/src/auth/mod.rs#symbol:bearer_token
  - crates/api/src/auth/token.rs#symbol:PREFIX
  - crates/api/src/auth/token.rs#symbol:LINK_PREFIX
  - crates/api/src/auth/sessions.rs#symbol:start
  - crates/api/src/auth/sessions.rs#symbol:authenticate
  - crates/api/src/auth/sessions.rs#symbol:end_all
  - crates/api/src/auth/sessions.rs#symbol:touch_interval
  - crates/api/src/auth/password.rs#symbol:check
  - crates/api/src/auth/password.rs#symbol:decoy
  - crates/api/src/auth/throttle.rs#symbol:attempt
  - crates/api/src/auth/setup.rs#symbol:complete
  - crates/api/src/auth/setup.rs#symbol:PASSWORD_MIN
  - crates/api/src/accounts.rs#symbol:set_password
related:
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/domains/02_users_teams_roles.md
  - docs/llm-wiki/SECURITY.md
  - docs/llm-wiki/OPERATIONS.md
visibility: public
contains_sensitive_info: false
---

# Auth and Sessions

## Responsibility

Signing in and out, the caller's identity (`CurrentUser`), bearer sessions, login throttling, and one-time setup links through which a person added in the app sets (or resets) their password. Operator accounts and CLI passwords come from [Users, Teams and Roles](02_users_teams_roles.md) and `crates/api/src/accounts.rs`.

## Behaviour

- **Bearer tokens, not cookies.** The client sends `Authorization: Bearer axs_…` (scheme case-insensitive). The token is 256 random bits, base64url, behind the prefix `axs_` (setup links use `axl_`). Only its SHA-256 is stored (`sessions.token_hash`), so a database dump does not yield live sessions.
- **`CurrentUser` is the first check of every handler.** A missing token, a wrong prefix, an unknown or expired session, or a deactivated user all answer `401` with the mock's "sign in again" text.
- **Two expiries.** A session ends `AX_SESSION_IDLE` (24h) after last use, and never later than `AX_SESSION_MAX` (7d) after sign-in. Use moves `expires_at` ahead to `least(now + idle, absolute)`, but writes at most once per `touch_interval` (min(idle/10, 60 s)) to keep reads cheap. Starting a session first deletes expired rows.
- **Every sign-in failure looks the same.** Unknown email, deactivated account, account without a password, and a wrong password all answer `401 wrongCredentials`. With no stored hash the password is still verified against a decoy hash, so timing does not reveal which emails exist. Emails compare as `lower(email)`, untrimmed.
- **Passwords** are Argon2id PHC strings (19 MiB, 2 passes, 1 lane), verified on a blocking thread. A user whose `password_hash` is `NULL` cannot sign in until they use a setup link.
- **Login throttling is per email.** The attempt is counted **before** the password is checked, for registered and unknown emails alike, keyed by the lower-cased email. After `AX_LOGIN_ATTEMPTS` (5) failures within `AX_LOGIN_LOCKOUT` (15m) the email is locked for `AX_LOGIN_LOCKOUT`, and even the right password is refused. A success deletes the row. The lock answers **401** (not 429; the contract's `login` declares 200/400/401) with "try again in N minutes". A known trade-off: anyone can keep a known email locked; the remedy is an admin issuing a new setup link.
- **Setup links** (`createSetupLink`, then `completeSetup`): one per user (`setup_links.user_id` is the primary key), so a new link replaces the old one, which is how passwords are reset. Valid for `AX_SETUP_LINK_TTL` (72h). Only for active non-operators. Completing deletes the link inside the transaction (`delete … returning`), so it works once even under races. Unknown, used, expired and deactivated links all answer the same `401 setupLinkInvalid`. Password length is 12–128 counted in UTF-16 units (the contract's `SETUP_PASSWORD_MIN/MAX`). Completing sets the password, ends that user's other sessions and returns a new `Session { token, user }`.
- **Sessions end at once** on logout (`end`), on a password reset or CLI `set-password`, and on deactivation (`end_all`).

## API Services

| Operation | Endpoint | Permission | Payload → response | Errors |
| --- | --- | --- | --- | --- |
| `login` | `POST /auth/login` | public | `{ email, password }` → `Session` | 400 `checkInput`; 401 `wrongCredentials`; 401 locked |
| `logout` | `POST /auth/logout` | session | → 204 | 401 |
| `me` | `GET /auth/me` | session | → `User` (with `teamId`) | 401 |
| `completeSetup` | `POST /auth/setup` | public | `{ token, password }` → `Session` | 400 `checkInput`; 401 `setupLinkInvalid` |
| `createSetupLink` | `POST /users/{id}/setup-link` | `user:manage` | → 201 `{ token, expiresAt }` | 404 `userNotFound`; 403 `operatorProvisioned`; 400 `setupLinkInactive` |

The error body's `code` is a generic class (`unauthorized`, `forbidden`, …); names like `wrongCredentials` are the mock's message keys, which the server reproduces as the localised `message` ([Platform](12_platform.md)).

## Data

- `sessions`: `token_hash bytea unique`, `user_id` (cascade), `expires_at`, `absolute_expires_at`, `last_seen_at`, with `check (expires_at <= absolute_expires_at)`.
- `setup_links`: primary key `user_id`, unique `token_hash`, expiry.
- `login_attempts`: primary key `email_key`, `attempts >= 1`, `first_at`, `locked_until`.

## Evidence

- `crates/api/src/auth/handlers.rs#symbol:login` — sign-in order: body, throttle, password, session.
- `crates/api/src/auth/mod.rs#symbol:CurrentUser`, `crates/api/src/auth/mod.rs#symbol:bearer_token` — the session extractor.
- `crates/api/src/auth/token.rs#symbol:PREFIX`, `crates/api/src/auth/token.rs#symbol:LINK_PREFIX` — token shapes.
- `crates/api/src/auth/sessions.rs#symbol:start`, `crates/api/src/auth/sessions.rs#symbol:authenticate`, `crates/api/src/auth/sessions.rs#symbol:end_all`, `crates/api/src/auth/sessions.rs#symbol:touch_interval` — expiry and renewal.
- `crates/api/src/auth/password.rs#symbol:check`, `crates/api/src/auth/password.rs#symbol:decoy` — Argon2id and the decoy.
- `crates/api/src/auth/throttle.rs#symbol:attempt` — lockout (its message is `ApiError::sign_in_locked` in `error.rs`).
- `crates/api/src/auth/setup.rs#symbol:complete`, `crates/api/src/auth/setup.rs#symbol:PASSWORD_MIN` — setup links.
- `crates/api/src/accounts.rs#symbol:set_password` — CLI password reset signs the account out.
- Tests: `crates/api/tests/auth.rs` (`every_sign_in_failure_looks_the_same`, `only_the_tokens_hash_is_stored`, `use_moves_the_end_ahead_but_not_past_the_absolute_end`, `password_guessing_is_locked_out`, `a_lock_outlasts_the_window_it_started_in`), `crates/api/tests/setup_links.rs` (`a_new_link_replaces_the_old_one_and_resets_the_password`, `a_link_for_a_deactivated_or_late_person_is_refused`).

## Open Questions

- There is no per-IP throttle; per-email locking can be abused to lock a known person out (accepted in `throttle.rs`).

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
