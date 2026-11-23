---
title: Project Profile
tags:
  - llm-wiki
  - verified
status: verified
doc_type: project_profile
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - rust-toolchain.toml
evidence:
  - Cargo.toml#L1-L11
  - crates/api/Cargo.toml#L11-L30
  - README.md#section:Run it
  - CLAUDE.md#section:Commands (no local Rust needed)
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/OPERATIONS.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# Project Profile

## Detected Project

- type: `backend` (HTTP API server and a job runner, one binary)
- confidence: explicit (`llm-wiki.config.json`; the tool's own detection says `library`, which is wrong for this repository)

## Summary

`ax-request-portal-server` is the MIT-licensed, public **reference backend** for the frontend template `ax-request-portal`: it implements that template's OpenAPI contract and lifecycle with the same rules as its in-browser mock API, so the template's screens work unchanged against a real server. It adds what a mock cannot: PostgreSQL storage, sessions, server-side permissions, AI jobs with Claude, and a runner that has Claude Code work on GitHub repositories and open pull requests. Nothing organisation-specific belongs in it; deployments configure themselves through environment variables.

## Stack

Rust (edition 2024, toolchain pinned to 1.100.0), axum 0.8, sqlx 0.8 on PostgreSQL 18, tokio, reqwest (rustls), Argon2id, HMAC-SHA256, clap. Workspace: `crates/contract` (`ax-contract`, the embedded contract) and `crates/api` (`ax-api`, server, CLI and runner). Unsafe code is forbidden; clippy runs with `-D warnings`.

## Runtime and commands

No local Rust is needed; `scripts/cargo.sh` runs cargo in a Docker image built from `deploy/dev.Dockerfile`.

- `bash scripts/cargo.sh test --workspace` — starts a throwaway PostgreSQL for the `#[sqlx::test]` database tests.
- `bash scripts/cargo.sh clippy --workspace --all-targets -- -D warnings`, `bash scripts/cargo.sh fmt --all`.
- `docker compose -f deploy/compose.yaml up -d --build --wait && bash scripts/smoke.sh` — the whole stack (PostgreSQL, API, the frontend built against this API) and a smoke test.
- `npx llm-wiki-governance@1.42.0 validate --strict --agent claude` — this wiki.

See [Operations](OPERATIONS.md) for settings, images and CI.

## Evidence

- `Cargo.toml#L1-L11`, `crates/api/Cargo.toml#L11-L30`, `rust-toolchain.toml` — workspace, dependencies, toolchain.
- `README.md#section:Run it`, `CLAUDE.md#section:Commands (no local Rust needed)` — how to build, test and run.

## Open Questions

- Not yet deployed or run against the real Claude API and GitHub; keys, tokens and the deployment host are pending.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
