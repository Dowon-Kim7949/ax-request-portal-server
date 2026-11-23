# Project Instructions

Reference backend (Rust, axum, PostgreSQL) for the public frontend template
https://github.com/Dowon-Kim7949/ax-request-portal. Public, MIT, a portfolio piece.

## LLM-WIKI <!-- llm-wiki-adapter v2 -->

Read the project LLM-WIKI before making code or documentation changes.

Always loaded (kept deliberately small):

@docs/llm-wiki/index.md
@docs/llm-wiki/project-profile.md

Load the rest on demand — do not preload the whole wiki:

- docs/llm-wiki/README.md — wiki operating rules and how to cite
- docs/llm-wiki/ARCHITECTURE_CONVENTIONS.md — layout and the conventions to keep
- docs/llm-wiki/domains/00_overview.md — a request's path and the domain map; then the domain document for the area you change
- docs/llm-wiki/API_CONTRACTS.md, DATA_MODEL.md, OPERATIONS.md, SECURITY.md — before changing the API surface, the schema, settings or deployment, or anything security-related
- Locate before reading: `npx llm-wiki-governance@1.42.0 search-docs "<query>"`, then read only the needed section with `npx llm-wiki-governance@1.42.0 get-doc <path> --section "<heading>" --strict-section`, or open the file directly when the task depends on all of it.

Required rules:

- Keep LLM-created or LLM-edited wiki docs as `needs_review`.
- Use `verified` only after human review.
- If code or docs change, update related wiki docs and append `docs/llm-wiki/log.md`.
- Keep why a change was made and how it was found in the issue or PR, not in wiki docs; the wiki keeps the durable conclusion and may link the issue or PR.
- Do not record sensitive information in wiki docs, logs, reports, or prompts.
- Read and write Markdown as UTF-8.

<!-- /llm-wiki-adapter -->

## Contract first

- `contract/openapi.json` and `contract/lifecycle.json` are byte-for-byte copies of the frontend's `docs/openapi.json` and `docs/lifecycle.json` at the commit in `contract/SOURCE`; `requests::lifecycle` is tested against the latter. Never edit them here. To change the API: change `src/api/contract.ts` in the frontend, merge there, then `bash scripts/sync-contract.sh main` here.
- Domain rules come from the frontend's `src/domain/` (lifecycle transitions and guards in `lifecycle.ts`, roles in `roles.ts`, AI usage normalisation in `ai.ts`) and from its `README.md` "Connect a real backend". Port them; match the mock API's behaviour (`src/mocks/engine.ts`).
- Every contract operation is routed. Unimplemented ones answer `501`. When one gets a handler: add its `operationId` to `crates/api/src/implemented.rs` and an arm in `routes::implemented` (method and path come from the contract; building the router panics if the two lists disagree).
- Schema changes are new files in `crates/api/migrations/` (never edit an applied one). Teams and users are never deleted; reference them with `ON DELETE RESTRICT`. AI usage rows copy the caller's `team_id` at call time.
- Handlers check in the mock API's order: session (`CurrentUser`, 401), body (`body::Body`, one 400 listing every issue), permission (`auth::require`, 403), then the operation's rules, with the mock's messages (`src/mocks/copy.ts`).
- Check permissions on the server for every operation (`auth::require`), using `roles.rs` (a port of the frontend's `roles.ts`). Operator accounts come only from the `ax-api user` CLI, never from the API (`403`, `operatorProvisioned`).

## Commands (no local Rust needed)

- `bash scripts/cargo.sh test --workspace` (starts a throwaway PostgreSQL for the `#[sqlx::test]` database tests)
- `bash scripts/cargo.sh clippy --workspace --all-targets -- -D warnings`
- `bash scripts/cargo.sh fmt --all`
- `docker compose -f deploy/compose.yaml up -d --build --wait && bash scripts/smoke.sh`
- `npx llm-wiki-governance@1.42.0 validate --strict --agent claude` (the wiki; always the full package name)

CI runs fmt, clippy (`-D warnings`), tests, the contract pin check, gitleaks, the compose smoke test and the wiki gate (`.github/workflows/llm-wiki.yml`).

## Rules

- Deployment settings live in `deploy/.env` (from `deploy/.env.example`; git-ignored). A new setting goes in `.env.example`, `compose.yaml`, `config.rs` and README "Configuration" together.
- Nothing organisation-specific in this repository: no internal hosts or IPs, project or customer names, real request data, keys or passwords. Organisation settings come from configuration.
- Commits use the GitHub noreply author (`127072329+Dowon-Kim7949@users.noreply.github.com`); check `git config user.email` in every clone.
- Server text (errors, notifications, AI output) follows `Accept-Language` (`ko` | `en`).
- The AI never merges, never approves its own wiki edits, and never moves a request past a human checkpoint.
