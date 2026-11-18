# AX Request Portal — Server

[![CI](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md) · **한국어**

AI와 함께 일하는 업무 요청 포털 **[AX Request Portal](https://github.com/Dowon-Kim7949/ax-request-portal)** 의 참조 백엔드입니다. Rust(axum, PostgreSQL)로 작성합니다.

두 저장소는 하나의 계약으로 만납니다. 계약의 주인은 프런트엔드입니다. `src/api/contract.ts`가 `docs/openapi.json`, 목 API, 타입 클라이언트를 만듭니다. 이 서버는 그 문서를 고정한 프런트엔드 커밋에서 바이트 그대로 복사해 구현합니다. 어느 쪽도 상대의 코드를 가져다 쓰지 않습니다.

> **현재 상태: 로그인, 사용자·팀.** 로그인(`login`, `me`, `logout`, `completeSetup`)과 사용자·팀(`listUsers`, `createUser`, `updateUser`, `createSetupLink`, `listTeams`, `createTeam`, `updateTeam`)이 PostgreSQL과 함께 동작합니다. 나머지 operation 35개도 연결돼 있고, 구현되기 전까지는 계약의 오류 형식으로 `501`을 돌려줍니다. 다음은 요청 상태 전이, 드래프트, 그리고 AI 작업 두 가지(실현 가능성 리포트, MR까지의 AI 작업)입니다.

## 계약을 맞추는 방법

| 구성 | 역할 |
| --- | --- |
| `contract/openapi.json` | `contract/SOURCE`에 적힌 커밋의 프런트엔드 계약. 여기서는 고치지 않습니다. |
| `scripts/sync-contract.sh <ref>` | 프런트엔드의 브랜치·태그·커밋에서 계약을 복사하고, 데모의 프런트엔드 빌드도 같은 커밋으로 맞춥니다. |
| CI `contract` 작업 | 복사본이 고정한 커밋과 다르면 실패합니다. |
| 테스트 | 계약의 모든 operation이 응답하는지(구현 전에는 `501`), 서버의 오류 코드와 역할이 계약의 `ApiError.code`, `Role`과 같은지 검사합니다. |

그래서 API를 바꿀 때는 프런트엔드 저장소에서 시작합니다. `contract.ts`를 고쳐 병합한 뒤, 여기서 `scripts/sync-contract.sh main`을 실행하고 구현합니다.

## 실행

Docker만 있으면 됩니다.

```bash
# 전체 스택: PostgreSQL + API + 이 API에 맞춰 빌드한 프런트엔드
docker compose -f deploy/compose.yaml up --build
# http://localhost:8088 접속 (API: http://localhost:8080/healthz)
bash scripts/smoke.sh        # 실행 중인 스택을 로그인까지 처음부터 끝까지 점검
```

기본 계정은 없습니다. 첫 관리자는 서버 CLI로 만듭니다. 비밀번호(12자 이상)는 프롬프트로 입력합니다.

```bash
docker compose -f deploy/compose.yaml exec ax-backend \
  ax-api user create --email you@example.com --name "이름" --role admin
```

개발은 고정된 툴체인 이미지(`rust:1.100`) 안의 cargo로 하거나, `rust-toolchain.toml`을 읽는 로컬 `rustup`으로 합니다.

```bash
bash scripts/cargo.sh test --workspace
bash scripts/cargo.sh clippy --workspace --all-targets -- -D warnings
bash scripts/cargo.sh fmt --all
```

DB 테스트에는 PostgreSQL이 필요합니다. `scripts/cargo.sh test`가 버리는 용도의 서버(컨테이너 `ax-server-test-db`, 메모리에만 저장)를 띄우고, CI는 서비스 컨테이너로 띄웁니다. 로컬 `rustup`으로 돌릴 때는 DB를 만들 수 있는 사용자의 `DATABASE_URL`을 지정합니다. 테스트마다 새 DB를 만듭니다.

## 로그인

- `POST /auth/login`은 불투명한 Bearer 토큰(`axs_`와 256비트 난수)을 돌려줍니다. 서버는 토큰의 SHA-256만 `sessions`에 저장합니다.
- 로그인은 `AX_SESSION_IDLE`(기본 24시간) 동안 쓰지 않으면 끝나고, 쓰더라도 로그인한 뒤 `AX_SESSION_MAX`(기본 7일)가 지나면 끝납니다. 토큰 갱신은 없습니다. `401`을 받으면 프런트엔드가 다시 로그인하게 합니다.
- `logout`은 세션을 바로 끝냅니다. 계정을 비활성화하면 그 계정의 모든 로그인이 다음 요청부터 끝납니다.
- 비밀번호는 Argon2id(19 MiB, 2회)로 해시합니다. 로그인 실패는 이유(없는 계정, 비활성 계정, 틀린 비밀번호)와 상관없이 같은 응답을 같은 시간에 돌려줍니다.
- 오류는 프런트엔드 목 API와 같습니다. 로그인 실패와 토큰이 없거나 모르거나 끝난 경우는 `401 unauthorized`, 권한이 없는 역할은 `403 forbidden`, 계약과 맞지 않는 본문은 `issues`가 붙은 `400 bad_request`입니다. 메시지는 `Accept-Language`를 따릅니다.
- 역할과 권한은 프런트엔드의 `src/domain/roles.ts`(역할 5개, 권한 13개)와 같습니다. 운영자 계정은 관리자가 아니라 서비스가 만듭니다. CLI만 운영자를 만들 수 있습니다.
- 계약과 다른 점이 하나 있습니다. 계약은 `login`의 응답으로 `200`과 `400`만 적었지만, 클라이언트가 기준으로 삼는 목 API처럼 로그인 실패는 `401`입니다. 고칠 곳은 프런트엔드의 `contract.ts`입니다.

| 명령 | 하는 일 |
| --- | --- |
| `ax-api` / `ax-api serve` | 남은 마이그레이션을 적용하고 서버를 띄웁니다 |
| `ax-api migrate` | 남은 마이그레이션을 적용하고 끝냅니다 |
| `ax-api user create --email E --name N --role R` | 활성 계정을 만듭니다. 운영자를 만드는 유일한 방법입니다 |
| `ax-api user set-password --email E` | 비밀번호를 정하거나 바꾸고, 그 계정의 세션을 모두 끝냅니다 |

비밀번호는 명령줄 인자로 받지 않습니다. 프롬프트로 입력하거나, `--password-stdin`으로 표준 입력의 첫 줄을 넘깁니다.

## 사용자와 팀

- 로그인한 사람은 누구나 사용자 목록(비활성 사용자 포함)과 팀 목록을 볼 수 있습니다. 추가와 변경에는 `user:manage`(관리자) 권한이 필요합니다.
- 규칙은 목 API와 같습니다.
  - 이메일은 대소문자를 무시하고 중복될 수 없습니다(`400`).
  - 팀 이름은 대소문자를 무시하고 중복될 수 없으며, 앞뒤 공백을 지운 뒤 1–40자여야 합니다(`400`).
  - 없는 사용자나 팀은 `404`입니다.
  - 관리자는 자기 역할을 바꾸거나 자신을 비활성화할 수 없습니다(`400`).
  - API로는 아무도 운영자를 만들거나 바꿀 수 없습니다(`403`).
- 삭제는 없습니다. 사용자를 비활성화하면 그 사람의 세션이 바로 끝납니다. 팀은 이름만 바꾸며, 기록이 팀을 id로 가리키므로 이름을 바꿔도 연결이 유지됩니다.
- 검사 순서도 목 API와 같습니다. 세션(`401`), 본문(`400`, 문제를 `issues`에 모두 나열), 권한(`403`), 그다음 operation 규칙 순입니다.
- API로 추가한 사람은 아직 비밀번호가 없습니다. 관리자가 그 사람의 일회용 **설정 링크**를 만들면(`POST /users/{id}/setup-link`, `201 { token, expiresAt }`) 프런트엔드가 `/setup#token=…` 링크로 보여 줍니다. 링크를 받은 사람은 그 화면에서 비밀번호를 정하고(`POST /auth/setup`, 공개) 바로 로그인됩니다.
  - 링크는 한 번만 쓸 수 있고 `AX_SETUP_LINK_TTL`(72시간) 동안 유효합니다. 한 사람에게는 가장 최근 링크 하나만 유효합니다. 비밀번호를 잊은 사람에게는 새 링크를 만들어 주면 되고, 그 링크를 쓰는 순간 그 사람의 다른 세션은 모두 끝납니다.
  - 운영자(`403`, 대신 `ax-api user set-password`를 씁니다)와 비활성 사용자(`400`)에게는 링크를 만들 수 없습니다. 사용자를 비활성화하면 아직 쓰지 않은 링크도 함께 지웁니다. 없는 링크, 이미 쓴 링크, 만료된 링크는 로그인 실패처럼 `401`입니다.
  - 토큰은 세션 토큰처럼 SHA-256 해시만 저장합니다. 브라우저는 URL 프래그먼트를 서버로 보내지 않으므로 서버 로그에도 남지 않습니다.

## 설정

Compose로 띄울 때는 설정을 `deploy/.env`에 둡니다. 각 항목의 설명은 [`deploy/.env.example`](deploy/.env.example)에 있으니 복사해서 쓰면 됩니다. `.env` 파일은 git과 Docker 빌드 컨텍스트 모두에서 제외됩니다.

| 변수 | 기본값 | 의미 |
| --- | --- | --- |
| `AX_BIND` | `0.0.0.0:8080` | 수신 주소 |
| `AX_BASE_PATH` | `/` | 계약을 올릴 경로. 프런트엔드의 nginx는 `/api/`를 떼고 넘기므로 그 뒤에서는 `/`가 맞습니다. 프록시가 접두어를 그대로 넘기면 `/api`로 둡니다. |
| `DATABASE_URL` | 없음 | PostgreSQL 주소. 없으면 DB 없이 동작합니다(`/readyz`가 `not_configured`, 로그인은 `500`). |
| `AX_SESSION_IDLE` | `24h` | 이 시간 동안 쓰지 않으면 로그인이 끝납니다(`s`, `m`, `h`, `d`) |
| `AX_SESSION_MAX` | `7d` | 얼마나 쓰든 로그인한 뒤 이 시간이 지나면 끝납니다 |
| `AX_SETUP_LINK_TTL` | `72h` | 일회용 설정 링크가 유효한 시간 |
| `RUST_LOG` | `info` | 로그 필터 |

상태 확인: `GET /healthz`(프로세스, 버전, 고정한 계약 커밋)와 `GET /readyz`(DB 연결)는 `AX_BASE_PATH`와 상관없이 루트에 있습니다.

## 구조

```
crates/contract   복사한 계약: operation 목록, 오류 코드, 고정 커밋
crates/api        HTTP 서버(axum): 라우팅, ApiError, Accept-Language, 상태 확인,
                  로그인, 역할, 사용자·팀, ax-api CLI.
                  migrations/에 SQL 스키마가 있습니다
contract/         openapi.json + SOURCE(고정한 프런트엔드 커밋)
deploy/           Dockerfile, compose.yaml(db, ax-backend, web), .env.example
scripts/          cargo.sh, sync-contract.sh, smoke.sh
```

## 이 저장소의 규칙

- 조직 고유의 정보는 넣지 않습니다. 사내 호스트, 프로젝트 이름, 요청 데이터, 키가 여기에 해당합니다. 조직 설정은 설정값으로 받고, CI가 이력 전체에서 비밀값을 검사합니다.
- 오류 메시지, 알림, AI가 쓴 글은 요청의 `Accept-Language`(`ko` 또는 `en`)로 돌려줍니다.
- AI는 코드를 합치지 않고, 자기 위키 수정을 스스로 승인하지 않으며, 사람이 결정할 단계를 넘기지 않습니다. 프런트엔드와 같은 규칙입니다.

## 라이선스

MIT
