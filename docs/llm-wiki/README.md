---
title: LLM-WIKI README
tags:
  - llm-wiki
  - verified
status: verified
doc_type: wiki_readme
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - llm-wiki.config.json
evidence:
  - .github/workflows/llm-wiki.yml#L30-L31
  - .github/workflows/llm-wiki.yml#L41-L44
related:
  - docs/llm-wiki/index.md
  - docs/llm-wiki/log.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# LLM-WIKI README

This directory is shared knowledge for people and coding agents working on `ax-request-portal-server`. It is maintained with [llm-wiki-governance](https://www.npmjs.com/package/llm-wiki-governance). This repository has no Node project, so the CLI is run with `npx` at a pinned version — always the full package name: the short `llm-wiki` on npm is an unrelated package.

## Operating Rules

- Every wiki document has YAML frontmatter. This repository is public, so documents use `visibility: public` and must never contain secrets, internal hostnames, organisation names, or real people's names or e-mail addresses.
- Documents created or edited by the CLI or an agent stay `needs_review`. Only a person promotes a document with `npx llm-wiki-governance@1.42.0 review --approve <path>`; the tool records who approved but cannot prove it.
- Claims cite their sources: broad ones in `source_files`, precise ones in `evidence` (`path#symbol:Name`, `path#L10-L20`, `path#section:Heading`) mirrored in the body's `## Evidence` section. Uncertain points go under `## Open Questions` instead of being guessed.
- Record changes append-only in [log.md](log.md).
- Write the durable conclusion here; keep the story of how it was found in the issue or pull request.

## Commands

- `npx llm-wiki-governance@1.42.0 validate --strict --agent claude` — structure, frontmatter and evidence locators (what CI runs).
- `npx llm-wiki-governance@1.42.0 impact --since origin/main --strict` — which `verified` documents a diff makes stale (CI fails on them).
- `npx llm-wiki-governance@1.42.0 search-docs "<query>"`, `… get-doc <path> --section "<heading>"` — read only what a task needs.
- `npx llm-wiki-governance@1.42.0 init --write --skills --refresh --type backend --agent claude` — regenerate the agent skills after changing the governance mode (then delete the `.agents/` and `.cursor/` copies it also writes; only Claude is configured).

The version is pinned in `.github/workflows/llm-wiki.yml`; bump it there and in these commands together.

## Files the tool manages

`CLAUDE.md` (the adapter block), `.claude/skills/llm-wiki-*`, `.llm-wiki/prompts/` and `llm-wiki.config.json` are written by the CLI. The config sets `type: backend` by hand: the CLI's detection reads this Rust/axum server as a `library`, so its `project.review_item` finding about the mismatch is lowered to `info`.

## Citing without false alarms

A file in `source_files`, or cited with `#symbol:`, is watched as a whole: once a document is `verified`, any change to that file makes CI demand a document update. Hub files that change for unrelated reasons — `Cargo.toml` on dependency bumps, `deploy/compose.yaml` on every contract sync (`FRONTEND_REF`), `config.rs`, `error.rs`, `main.rs`, `state.rs` whenever a setting, message or state field is added, workflow files on action bumps — are therefore cited only by line range or section, which is compared region by region. A domain document cites its own module's files; it links to other domains' documents rather than citing their files.

## Evidence

- `.github/workflows/llm-wiki.yml#L30-L31` — the pinned CLI version; `.github/workflows/llm-wiki.yml#L41-L44` — frontmatter, strict validation, the undocumented-change gate.
- `llm-wiki.config.json` — governance mode and the `impact.source_changed` rule.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
