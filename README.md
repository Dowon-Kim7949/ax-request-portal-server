# AX Request Portal — Server

[![CI](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**English** · [한국어](README.ko.md)

The reference backend for **[AX Request Portal](https://github.com/Dowon-Kim7949/ax-request-portal)**, an AI-assisted work request portal. Written in Rust (axum, PostgreSQL).

The two repositories meet at one contract. The frontend owns it: `src/api/contract.ts` generates `docs/openapi.json`, the frontend's mock API and its typed client. This server copies that document byte for byte from a pinned frontend commit and implements it. Neither side imports the other's code.

> **Status: sign-in, people, teams and projects.** Sign-in (`login`, `me`, `logout`, `completeSetup`), people and teams (`listUsers`, `createUser`, `updateUser`, `createSetupLink`, `listTeams`, `createTeam`, `updateTeam`) projects (`listProjects`, `createProject`, `updateProject`) requests (`createRequest`, `getRequest`, `listRequests`, `precheckRequest`), their lifecycle (`transitionRequest`, `saveVerification`, `sendPreviewFeedback`, `updateRegression`, `updateRelease`, `regressionQueue`) and notifications (`listNotifications`, `markNotificationRead`, `markAllNotificationsRead`, `getNotificationSettings`, `updateNotificationSettings`) work against PostgreSQL. The other 17 contract operations are routed and answer `501` with the contract's error body until they are implemented. Next: the request lifecycle, drafts, then the two AI jobs (feasibility report and AI work up to a merge request).

## How the contract stays in step

| Piece | What it does |
| --- | --- |
| `contract/openapi.json` | The frontend's contract at the commit in `contract/SOURCE`. Never edited here. |
| `contract/lifecycle.json` | The frontend's request lifecycle as data (statuses, transitions, who sends each event, guards by event, turns and stages), from the same commit. The server's port (`requests::lifecycle`) is tested against it. |
| `scripts/sync-contract.sh <ref>` | Copies both documents from a frontend branch, tag or commit and pins the demo's frontend build to the same commit. |
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

## People and teams

- Anyone signed in can list people, including deactivated ones, and teams. Adding or changing either needs `user:manage` (admins).
- The rules are the mock API's:
  - Emails are unique ignoring case (`400`).
  - Team names are unique ignoring case and 1–40 characters after trimming (`400`).
  - An unknown person or team is `404`.
  - Admins cannot change their own role or deactivate themselves (`400`).
  - Nobody creates or changes an operator through the API (`403`).
- Nothing is deleted. A deactivated person's sessions end at once. A renamed team keeps everything that points at it, because records refer to teams by id.
- Checks run in the mock API's order: session (`401`), body (`400`, listing every problem in `issues`), permission (`403`), then the operation's rules.
- People added through the API have no password yet. An admin makes a one-time **setup link** for them (`POST /users/{id}/setup-link`, `201 { token, expiresAt }`); the frontend shows it as `/setup#token=…`, and the person sets a password there (`POST /auth/setup`, public), which signs them in.
  - A link works once and for `AX_SETUP_LINK_TTL` (72 hours). Only the newest link of a person works, so making a new one is how a forgotten password is reset; using it ends the person's other sessions.
  - Not for operators (`403`, use `ax-api user set-password`) or deactivated people (`400`); deactivating someone deletes their unused link. An unknown, used or expired link is `401`, like a failed sign-in.
  - The token is stored as its SHA-256 only, like a session's, and rides in the URL fragment, which never reaches a server log.

## Requests

- `createRequest` (`request:create`) files a request: it starts `submitted` and moves to `reporting` at once (`startReport`), where the AI writes the feasibility report. It goes to the project's first owner. An unknown or inactive project, a target the project does not have, or an attachment that is not the caller's is `400`.
- The precheck advice (`precheckRequest`, and stored with every new request) is a rule port of the frontend's `precheck.ts`: vague words, criteria not written as "when X, Y happens", missing reproduction steps, past dates. It never blocks, and speaks the request's language.
- Who may open a request: whoever filed it, anyone with `request:read:all`, and its responsible developers (the assignee and the project's owners). Anyone else gets the same `404` as for a request that does not exist.
- `listRequests` takes `scope` (`mine`, `assigned` with `request:review`, `all` with `request:read:all`), a comma-separated `status`, `projectId` and `q` (title, or `#id`), most recently changed first.
- `transitionRequest` takes the twelve events a person may send. It checks, in the mock API's order: the body, whether the person may open the request (`404`), whether the step is theirs (`403 notYourStep`, before anything about the status: the requester sends a requester's event, an admin may also cancel, a responsible developer sends a developer's), the status (`409 invalid_transition`), then the guards (`409 guard_failed`: every criterion passed, one failed, assumptions confirmed, scope questions answered, at most two scope rewrites, a release date set). The AI's and the system's events are never accepted from the API, so the AI cannot step past a person.
- The change, its timeline entry and the notifications it sends are one transaction, on the request's locked row. Besides moving a request, the requester saves their checklist and reacts to the preview; a responsible developer records regression results and plans the release (a new date not before today on the business calendar).
- Notifications are kept as a message key and parameters, and written out in the reader's language when listed (the mock's texts, word for word). The person who caused one is not told; settings can turn in-app notifications off or keep only those that ask for action; each person keeps at most 200. Only in-app notifications are sent; e-mail and messenger settings are stored for a backend that sends those.
- Requests keep what lists filter on in columns and the rest of the contract's `WorkRequest` as one JSONB object; the timeline is its own append-only table. Numbers start at 1.

## Configuration

With Compose, put settings in `deploy/.env`: copy [`deploy/.env.example`](deploy/.env.example), which explains each one. Git ignores `.env` files, and so does the Docker build context.

| Variable | Default | Meaning |
| --- | --- | --- |
| `AX_BIND` | `0.0.0.0:8080` | Listen address |
| `AX_BASE_PATH` | `/` | Where the contract is mounted. The frontend's nginx strips `/api/` before proxying, so `/` is right behind it; use `/api` if your proxy keeps the prefix. |
| `DATABASE_URL` | unset | PostgreSQL URL. Unset runs without a database (`/readyz` says `not_configured`; sign-in answers `500`). |
| `AX_SESSION_IDLE` | `24h` | A sign-in ends after this long unused (`s`, `m`, `h` or `d`) |
| `AX_SESSION_MAX` | `7d` | A sign-in ends this long after signing in, however much it is used |
| `AX_SETUP_LINK_TTL` | `72h` | How long a one-time setup link works |
| `AX_TIME_ZONE` | `Asia/Seoul` | The business calendar (IANA name): "today" for desired and release dates, delays and monthly totals. Keep it equal to the frontend's `appConfig.organization.timeZone`; the server refuses to start with a name PostgreSQL does not know. |
| `RUST_LOG` | `info` | Log filter |

Probes: `GET /healthz` (process, version and pinned contract commit) and `GET /readyz` (database reachable) sit at the root whatever `AX_BASE_PATH` is.

## Layout

```
crates/contract   the vendored contract: operations, error codes, pinned commit
crates/api        the HTTP server (axum): routing, ApiError, Accept-Language, probes,
                  sign-in, roles, people and teams, the ax-api CLI;
                  migrations/ holds the SQL schema
contract/         openapi.json + SOURCE (pinned frontend commit)
deploy/           Dockerfiles, compose.yaml (db, ax-backend, web) and .env.example
scripts/          cargo.sh, sync-contract.sh, smoke.sh
```

## Rules this repository keeps

- Nothing organisation-specific: no internal hosts, project names, request data or keys. Organisation settings come from configuration. CI scans the history for secrets.
- Error messages, notifications and AI-written text come back in the request's `Accept-Language` (`ko` or `en`).
- The AI never merges, never approves its own wiki edits, and never moves a request past a human checkpoint — the same rule as the frontend.

## License

MIT
