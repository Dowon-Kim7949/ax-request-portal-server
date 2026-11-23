---
title: Backend Profile
tags:
  - llm-wiki
  - verified
status: verified
doc_type: profile
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - crates/api/tests/common/mod.rs
evidence:
  - crates/api/src/body.rs#symbol:Body
  - crates/api/src/body.rs#symbol:utf16_len
  - crates/api/src/lang.rs#symbol:Lang
  - crates/api/tests/common/mod.rs#symbol:app
  - crates/api/tests/common/mod.rs#symbol:token_for
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/domains/12_platform.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Backend Profile

Practical rules for writing code here, beyond the [Architecture Conventions](../ARCHITECTURE_CONVENTIONS.md).

## Writing a handler

- Extract `State<AppState>`, `Lang`, `CurrentUser` (when signed in) and the raw body as `Bytes`; parse with `body::Body`, collect every issue, call `finish()`, then `auth::require`, then the rules. Keep the mock's order; when it differs, say why in the module comment.
- Messages: every `ApiError` and issue carries Korean and English text, copied from the frontend's `src/mocks/copy.ts`. Never return one language only.
- Lengths are UTF-16 units (`body::utf16_len`) and string cutting follows JavaScript `slice`, because the frontend validates with zod in the browser.
- Fields left empty are omitted from JSON rather than sent as empty strings, matching the mock.
- A change to a request takes `StoredRequest::find_for_update` and does everything in that transaction.

## Writing a test

- Integration tests live in `crates/api/tests/<domain>.rs` and use `#[sqlx::test]` (a fresh, migrated database each); `tests/common` builds the app (`app`, `app_with`), makes accounts (`account`) and tokens (`token_for`).
- Name tests by the behaviour they pin, in plain words (`a_new_link_replaces_the_old_one_and_resets_the_password`). Many pin the mock's rules (`…_follows_the_mock_apis_rules`).
- Unit tests sit next to the code (`mod tests`); contract-matching tests compare with `ax_contract` (`roles_match_the_contract`, the lifecycle tests).
- External services are faked locally: a fake Messages API for Claude, a fake GitHub and a fake `claude` binary for the runner (unix only). No test calls the real services.

## Style

- `cargo fmt` with the repository's `rustfmt.toml`; clippy with `-D warnings`; `unsafe` is forbidden.
- Module doc comments (`//!`) explain the domain's rules and their source in the frontend; keep them true when the code changes.

## Evidence

- `crates/api/src/body.rs#symbol:Body`, `crates/api/src/body.rs#symbol:utf16_len`, `crates/api/src/lang.rs#symbol:Lang` — the parsing and language helpers.
- `crates/api/tests/common/mod.rs#symbol:app`, `crates/api/tests/common/mod.rs#symbol:token_for` — test helpers.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
