---
title: LLM-WIKI Index
tags:
  - llm-wiki
  - verified
status: verified
doc_type: wiki_index
project: ax-request-portal-server
last_updated: 2026-11-23
author: cli-generated
last_edited_by: claude-code
wiki_block_version: v1
source_files:
  - llm-wiki.config.json
evidence:
  - CLAUDE.md#section:Contract first
related:
  - docs/llm-wiki/README.md
  - docs/llm-wiki/project-profile.md
  - docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md
  - docs/llm-wiki/DOMAIN_FEATURES.md
  - docs/llm-wiki/API_CONTRACTS.md
  - docs/llm-wiki/DATA_MODEL.md
  - docs/llm-wiki/OPERATIONS.md
  - docs/llm-wiki/SECURITY.md
  - docs/llm-wiki/GLOSSARY.md
  - docs/llm-wiki/profiles/backend.md
  - docs/llm-wiki/domains/00_overview.md
  - docs/llm-wiki/log.md
visibility: public
contains_sensitive_info: false
reviewed_by: Kim Dowon
reviewed_at: 2026-11-23
---

# LLM-WIKI Index

The entry point to this project's LLM-WIKI: what an agent should know about `ax-request-portal-server` that it cannot cheaply re-derive from the code. Every claim cites its source files. The frontend template this server implements has its own wiki in its repository; the API contract between them is described in [API Contracts](API_CONTRACTS.md).

## Status

- Every document is `needs_review` until a person approves it with `npx llm-wiki-governance@1.42.0 review --approve <path>`. Agents never approve their own edits.
- Governance mode is `standard`, and `impact.source_changed` is an error (`llm-wiki.config.json`): once a document is `verified`, a pull request that changes a source file it cites must update the document too, or CI fails (`.github/workflows/llm-wiki.yml`).

## Recommended Read Order

1. [Project Profile](project-profile.md) — what this is, stack, commands.
2. [Domain Overview](domains/00_overview.md) — a request's path and the map of domains.
3. [Architecture Conventions](ARCHITECTURE_CONVENTIONS.md) — layout, the contract-first rule, the handler pipeline, how to add an operation or a migration.
4. The domain document for the area you will change.
5. Before touching the API surface: [API Contracts](API_CONTRACTS.md); the database: [Data Model](DATA_MODEL.md); settings or deployment: [Operations](OPERATIONS.md); sessions, keys or files: [Security](SECURITY.md).

## Map

| Area | Documents |
| --- | --- |
| Product | [Domain Features](DOMAIN_FEATURES.md), [Glossary](GLOSSARY.md) |
| Identity | [Auth and Sessions](domains/01_auth_sessions.md), [Users, Teams and Roles](domains/02_users_teams_roles.md) |
| Requests | [Projects](domains/03_projects.md), [Requests](domains/04_requests.md), [Lifecycle](domains/05_lifecycle.md), [Drafts and Attachments](domains/06_drafts_attachments.md) |
| Everyone | [Notifications](domains/07_notifications.md), [Dashboard and Releases](domains/08_dashboard_releases.md) |
| AI | [AI Jobs](domains/09_ai_jobs.md), [AI Settings and Budget](domains/10_ai_settings_budget.md), [Runner](domains/11_runner.md) |
| Shared | [Platform](domains/12_platform.md), [Backend Profile](profiles/backend.md) |
| Cross-cutting | [API Contracts](API_CONTRACTS.md), [Data Model](DATA_MODEL.md), [Operations](OPERATIONS.md), [Security](SECURITY.md) |
| Wiki itself | [README](README.md), [Change Log](log.md) |

## Evidence

- `llm-wiki.config.json` — governance mode and the `impact.source_changed` rule.
- `CLAUDE.md#section:Contract first` — the repository rules these documents expand on.

## Open Questions

- None known.

## Review Notes

- Keep this document as `needs_review` until it has been reviewed.
- Once reviewed, approve it with `llm-wiki review --approve <path>`. An agent does not approve its own edits.
