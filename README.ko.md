# AX Request Portal — Server

[![CI](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md) · **한국어**

AI와 함께 일하는 업무 요청 포털 **[AX Request Portal](https://github.com/Dowon-Kim7949/ax-request-portal)** 의 참조 백엔드입니다. Rust(axum, PostgreSQL)로 작성합니다.

두 저장소는 하나의 계약으로 만납니다. 계약의 주인은 프런트엔드입니다. `src/api/contract.ts`가 `docs/openapi.json`, 목 API, 타입 클라이언트를 만듭니다. 이 서버는 그 문서를 고정한 프런트엔드 커밋에서 바이트 그대로 복사해 구현합니다. 어느 쪽도 상대의 코드를 가져다 쓰지 않습니다.

> **현재 상태: 뼈대.** 계약의 operation 44개가 모두 연결돼 있고, 구현되기 전까지는 계약의 오류 형식으로 `501`을 돌려줍니다. 다음 순서는 로그인, 요청 상태 전이, 드래프트, 그리고 AI 작업 두 가지(실현 가능성 리포트, MR까지의 AI 작업)입니다.

## 계약을 맞추는 방법

| 구성 | 역할 |
| --- | --- |
| `contract/openapi.json` | `contract/SOURCE`에 적힌 커밋의 프런트엔드 계약. 여기서는 고치지 않습니다. |
| `scripts/sync-contract.sh <ref>` | 프런트엔드의 브랜치·태그·커밋에서 계약을 복사하고, 데모의 프런트엔드 빌드도 같은 커밋으로 맞춥니다. |
| CI `contract` 작업 | 복사본이 고정한 커밋과 다르면 실패합니다. |
| 테스트 | 계약의 모든 operation이 응답하는지(구현 전에는 `501`), 서버의 오류 코드가 계약의 `ApiError.code`와 같은지 검사합니다. |

그래서 API를 바꿀 때는 프런트엔드 저장소에서 시작합니다. `contract.ts`를 고쳐 병합한 뒤, 여기서 `scripts/sync-contract.sh main`을 실행하고 구현합니다.

## 실행

Docker만 있으면 됩니다.

```bash
# 전체 스택: PostgreSQL + API + 이 API에 맞춰 빌드한 프런트엔드
docker compose -f deploy/compose.yaml up --build
# http://localhost:8088 접속 (API: http://localhost:8080/healthz)
bash scripts/smoke.sh        # 실행 중인 스택을 처음부터 끝까지 점검
```

개발은 고정된 툴체인 이미지(`rust:1.100`) 안의 cargo로 하거나, `rust-toolchain.toml`을 읽는 로컬 `rustup`으로 합니다.

```bash
bash scripts/cargo.sh test --workspace
bash scripts/cargo.sh clippy --workspace --all-targets -- -D warnings
bash scripts/cargo.sh fmt --all
```

## 설정

| 변수 | 기본값 | 의미 |
| --- | --- | --- |
| `AX_BIND` | `0.0.0.0:8080` | 수신 주소 |
| `AX_BASE_PATH` | `/` | 계약을 올릴 경로. 프런트엔드의 nginx는 `/api/`를 떼고 넘기므로 그 뒤에서는 `/`가 맞습니다. 프록시가 접두어를 그대로 넘기면 `/api`로 둡니다. |
| `DATABASE_URL` | 없음 | PostgreSQL 주소. 없으면 DB 없이 동작합니다(`/readyz`가 `not_configured`). |
| `RUST_LOG` | `info` | 로그 필터 |

상태 확인: `GET /healthz`(프로세스, 버전, 고정한 계약 커밋)와 `GET /readyz`(DB 연결)는 `AX_BASE_PATH`와 상관없이 루트에 있습니다.

## 구조

```
crates/contract   복사한 계약: operation 목록, 오류 코드, 고정 커밋
crates/api        HTTP 서버(axum): 라우팅, ApiError, Accept-Language, 상태 확인
contract/         openapi.json + SOURCE(고정한 프런트엔드 커밋)
deploy/           Dockerfile과 compose.yaml(db, ax-backend, web)
scripts/          cargo.sh, sync-contract.sh, smoke.sh
```

## 이 저장소의 규칙

- 조직 고유의 정보는 넣지 않습니다. 사내 호스트, 프로젝트 이름, 요청 데이터, 키가 여기에 해당합니다. 조직 설정은 설정값으로 받고, CI가 이력 전체에서 비밀값을 검사합니다.
- 오류 메시지, 알림, AI가 쓴 글은 요청의 `Accept-Language`(`ko` 또는 `en`)로 돌려줍니다.
- AI는 코드를 합치지 않고, 자기 위키 수정을 스스로 승인하지 않으며, 사람이 결정할 단계를 넘기지 않습니다. 프런트엔드와 같은 규칙입니다.

## 라이선스

MIT
