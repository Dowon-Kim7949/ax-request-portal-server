# AX Request Portal — Server

[![CI](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**English** · [한국어](README.ko.md)

The reference backend for **[AX Request Portal](https://github.com/Dowon-Kim7949/ax-request-portal)**, an AI-assisted work request portal. Written in Rust (axum, PostgreSQL).

The two repositories meet at one contract. The frontend owns it: `src/api/contract.ts` generates `docs/openapi.json`, the frontend's mock API and its typed client. This server copies that document byte for byte from a pinned frontend commit and implements it. Neither side imports the other's code.

> **Status: sign-in.** `login`, `me` and `logout` work against PostgreSQL. The other 41 contract operations are routed and answer `501` with the contract's error body until they are implemented. Next: people and teams, the request lifecycle, drafts, then the two AI jobs (feasibility report and AI work up to a merge request).

## How the contract stays in step

| Piece | What it does |
| --- | --- |
| `contract/openapi.json` | The frontend's contract at the commit in `contract/SOURCE`. Never edited here. |
| `scripts/sync-contract.sh <ref>` | Copies the contract from a frontend branch, tag or commit and pins the demo's frontend build to the same commit. |
| CI `contract` job | Fails if the copy differs from the pinned commit. |
| Tests | Every operation in the contract answers (`501` until implemented); the server's error codes and roles equal the contract's `ApiError.code` and `Role`. |

Changing the API therefore starts in the frontend repository: change `contract.ts`, merge, then run `scripts/sync-contract.sh main` here and implement.

## Run it

You need Docker. Nothing else has to be installed.

```bash
# Whole stack: PostgreSQL + API + the frontend built against this API
docker compose -f deploy/compose.yaml up --build
# open http://localhost:8088  (API: http://localhost:8080/healthz)
bash scripts/smoke.sh        # end-to-end check of the running stack, sign-in included
```

There are no built-in accounts. Create the first admin with the server's CLI; it prompts for the password (at least 12 characters):

```bash
docker compose -f deploy/compose.yaml exec ax-backend \
  ax-api user create --email you@example.com --name "Your Name" --role admin
```

Develop with cargo inside the pinned toolchain image (`rust:1.100`), or with a local `rustup` that reads `rust-toolchain.toml`:

```bash
bash scripts/cargo.sh test --workspace
bash scripts/cargo.sh clippy --workspace --all-targets -- -D warnings
bash scripts/cargo.sh fmt --all
```

The database tests need PostgreSQL: `scripts/cargo.sh test` starts a throwaway one (container `ax-server-test-db`, in memory) and CI runs one as a service. With a local `rustup`, set `DATABASE_URL` to a server where the user may create databases; each test gets a fresh one.

## Sign-in

- `POST /auth/login` returns an opaque bearer token (`axs_` and 256 random bits). The server stores only its SHA-256, in `sessions`.
- A sign-in ends after `AX_SESSION_IDLE` unused (default 24 hours) and `AX_SESSION_MAX` after signing in (default 7 days). There is no refresh; on `401` the frontend asks the person to sign in again.
- `logout` ends the session at once, and deactivating an account ends all of its sign-ins on their next request.
- Passwords are hashed with Argon2id (19 MiB, 2 passes). A failed sign-in answers the same way whatever the reason (no such account, deactivated, wrong password), and takes as long.
- Errors match the frontend's mock API: `401 unauthorized` for a failed sign-in or a missing, unknown or ended token; `403 forbidden` for a role without the permission; `400 bad_request` with `issues` for a body that does not match the contract. Messages follow `Accept-Language`.
- Roles and permissions are the frontend's `src/domain/roles.ts` (5 roles, 13 permissions). Operator accounts are provisioned by the service, never by an admin: only the CLI makes them.
- One difference from the contract: the contract lists only `200` and `400` for `login`, but a failed sign-in is `401`, as in the mock API the client is built against. The fix belongs in the frontend's `contract.ts`.

| Command | What it does |
| --- | --- |
| `ax-api` / `ax-api serve` | Applies pending migrations, then serves |
| `ax-api migrate` | Applies pending migrations and exits |
| `ax-api user create --email E --name N --role R` | Creates an active account; the only way to make an operator |
| `ax-api user set-password --email E` | Sets or resets a password and ends that account's sessions |

Passwords are never command-line arguments: the CLI prompts for them, or reads the first line of standard input with `--password-stdin`.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `AX_BIND` | `0.0.0.0:8080` | Listen address |
| `AX_BASE_PATH` | `/` | Where the contract is mounted. The frontend's nginx strips `/api/` before proxying, so `/` is right behind it; use `/api` if your proxy keeps the prefix. |
| `DATABASE_URL` | unset | PostgreSQL URL. Unset runs without a database (`/readyz` says `not_configured`; sign-in answers `500`). |
| `AX_SESSION_IDLE` | `24h` | A sign-in ends after this long unused (`s`, `m`, `h` or `d`) |
| `AX_SESSION_MAX` | `7d` | A sign-in ends this long after signing in, however much it is used |
| `RUST_LOG` | `info` | Log filter |

Probes: `GET /healthz` (process, version and pinned contract commit) and `GET /readyz` (database reachable) sit at the root whatever `AX_BASE_PATH` is.

## Layout

```
crates/contract   the vendored contract: operations, error codes, pinned commit
crates/api        the HTTP server (axum): routing, ApiError, Accept-Language, probes,
                  sign-in, roles, the ax-api CLI; migrations/ holds the SQL schema
contract/         openapi.json + SOURCE (pinned frontend commit)
deploy/           Dockerfiles and compose.yaml (db, ax-backend, web)
scripts/          cargo.sh, sync-contract.sh, smoke.sh
```

## Rules this repository keeps

- Nothing organisation-specific: no internal hosts, project names, request data or keys. Organisation settings come from configuration. CI scans the history for secrets.
- Error messages, notifications and AI-written text come back in the request's `Accept-Language` (`ko` or `en`).
- The AI never merges, never approves its own wiki edits, and never moves a request past a human checkpoint — the same rule as the frontend.

## License

MIT
