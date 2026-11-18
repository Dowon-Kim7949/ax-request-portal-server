# AX Request Portal — Server

[![CI](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**English** · [한국어](README.ko.md)

The reference backend for **[AX Request Portal](https://github.com/Dowon-Kim7949/ax-request-portal)**, an AI-assisted work request portal. Written in Rust (axum, PostgreSQL).

The two repositories meet at one contract. The frontend owns it: `src/api/contract.ts` generates `docs/openapi.json`, the frontend's mock API and its typed client. This server copies that document byte for byte from a pinned frontend commit and implements it. Neither side imports the other's code.

> **Status: skeleton.** All 44 contract operations are routed and answer `501` with the contract's error body until they are implemented. Next: sign-in, the request lifecycle, drafts, then the two AI jobs (feasibility report and AI work up to a merge request).

## How the contract stays in step

| Piece | What it does |
| --- | --- |
| `contract/openapi.json` | The frontend's contract at the commit in `contract/SOURCE`. Never edited here. |
| `scripts/sync-contract.sh <ref>` | Copies the contract from a frontend branch, tag or commit and pins the demo's frontend build to the same commit. |
| CI `contract` job | Fails if the copy differs from the pinned commit. |
| Tests | Every operation in the contract answers (`501` until implemented); the server's error codes equal the contract's `ApiError.code`. |

Changing the API therefore starts in the frontend repository: change `contract.ts`, merge, then run `scripts/sync-contract.sh main` here and implement.

## Run it

You need Docker. Nothing else has to be installed.

```bash
# Whole stack: PostgreSQL + API + the frontend built against this API
docker compose -f deploy/compose.yaml up --build
# open http://localhost:8088  (API: http://localhost:8080/healthz)
bash scripts/smoke.sh        # end-to-end check of the running stack
```

Develop with cargo inside the pinned toolchain image (`rust:1.100`), or with a local `rustup` that reads `rust-toolchain.toml`:

```bash
bash scripts/cargo.sh test --workspace
bash scripts/cargo.sh clippy --workspace --all-targets -- -D warnings
bash scripts/cargo.sh fmt --all
```

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `AX_BIND` | `0.0.0.0:8080` | Listen address |
| `AX_BASE_PATH` | `/` | Where the contract is mounted. The frontend's nginx strips `/api/` before proxying, so `/` is right behind it; use `/api` if your proxy keeps the prefix. |
| `DATABASE_URL` | unset | PostgreSQL URL. Unset runs without a database (`/readyz` says `not_configured`). |
| `RUST_LOG` | `info` | Log filter |

Probes: `GET /healthz` (process, version and pinned contract commit) and `GET /readyz` (database reachable) sit at the root whatever `AX_BASE_PATH` is.

## Layout

```
crates/contract   the vendored contract: operations, error codes, pinned commit
crates/api        the HTTP server (axum): routing, ApiError, Accept-Language, probes
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
