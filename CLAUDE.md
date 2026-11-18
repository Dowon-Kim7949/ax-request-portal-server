# Project Instructions

Reference backend (Rust, axum, PostgreSQL) for the public frontend template
https://github.com/Dowon-Kim7949/ax-request-portal. Public, MIT, a portfolio piece.

## Contract first

- `contract/openapi.json` is a byte-for-byte copy of the frontend's `docs/openapi.json` at the commit in `contract/SOURCE`. Never edit it here. To change the API: change `src/api/contract.ts` in the frontend, merge there, then `bash scripts/sync-contract.sh main` here.
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

CI runs fmt, clippy (`-D warnings`), tests, the contract pin check, gitleaks and the compose smoke test.

## Rules

- Deployment settings live in `deploy/.env` (from `deploy/.env.example`; git-ignored). A new setting goes in `.env.example`, `compose.yaml`, `config.rs` and README "Configuration" together.
- Nothing organisation-specific in this repository: no internal hosts or IPs, project or customer names, real request data, keys or passwords. Organisation settings come from configuration.
- Commits use the GitHub noreply author (`127072329+Dowon-Kim7949@users.noreply.github.com`); check `git config user.email` in every clone.
- Server text (errors, notifications, AI output) follows `Accept-Language` (`ko` | `en`).
- The AI never merges, never approves its own wiki edits, and never moves a request past a human checkpoint.
