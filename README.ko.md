# AX Request Portal — Server

[![CI](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml/badge.svg)](https://github.com/Dowon-Kim7949/ax-request-portal-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md) · **한국어**

AI와 함께 일하는 업무 요청 포털 **[AX Request Portal](https://github.com/Dowon-Kim7949/ax-request-portal)** 의 참조 백엔드입니다. Rust(axum, PostgreSQL)로 작성합니다.

두 저장소는 하나의 계약으로 만납니다. 계약의 주인은 프런트엔드입니다. `src/api/contract.ts`가 `docs/openapi.json`, 목 API, 타입 클라이언트를 만듭니다. 이 서버는 그 문서를 고정한 프런트엔드 커밋에서 바이트 그대로 복사해 구현합니다. 어느 쪽도 상대의 코드를 가져다 쓰지 않습니다.

> **현재 상태: 계약의 operation 46개를 모두 구현했습니다.** 로그인, 사용자·팀, 프로젝트, 요청과 상태 전이, 첨부, 임시 저장, 알림, 대시보드와 배포 일정, 그리고 운영자의 AI 예산·모델 설정·가격·사용량 보고가 PostgreSQL과 함께 동작합니다. AI가 하는 두 가지 작업은 백그라운드 워커에서 돕니다. 실현 가능성 리포트는 Claude가 쓰고(`AX_AI_PROVIDER=anthropic`), 그렇지 않으면 스텁이 제공자가 연결되지 않았다고 알립니다. 다음은 AI 사전 점검, 그다음 코딩 에이전트로 하는 작업 실행입니다.

## 계약을 맞추는 방법

| 구성 | 역할 |
| --- | --- |
| `contract/openapi.json` | `contract/SOURCE`에 적힌 커밋의 프런트엔드 계약. 여기서는 고치지 않습니다. |
| `contract/lifecycle.json` | 같은 커밋의 요청 상태 전이를 데이터로 담은 파일(상태, 전이, 이벤트를 보내는 쪽, 이벤트별 가드, 차례, 단계). 서버의 포팅(`requests::lifecycle`)을 이 파일로 검증합니다. |
| `scripts/sync-contract.sh <ref>` | 프런트엔드의 브랜치·태그·커밋에서 두 문서를 복사하고, 데모의 프런트엔드 빌드도 같은 커밋으로 맞춥니다. |
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

## 요청

- `createRequest`(`request:create`)는 요청을 접수합니다. 요청은 `submitted`로 시작해 바로 `reporting`(`startReport`)으로 넘어가고, 이 단계에서 AI가 실현 가능성 리포트를 씁니다. 담당자는 프로젝트의 첫 번째 담당자입니다. 없거나 비활성인 프로젝트, 프로젝트에 없는 대상, 본인 것이 아닌 첨부는 `400`입니다.
- 사전 점검(`precheckRequest`, 새 요청에도 저장)은 프런트엔드 `precheck.ts`의 규칙을 그대로 옮겼습니다. 모호한 표현, "~하면 ~된다" 형식이 아닌 완료 기준, 빠진 재현 순서, 지난 날짜를 짚습니다. 접수를 막지 않으며 요청의 언어로 답합니다.
- 요청을 열 수 있는 사람은 요청한 본인, `request:read:all` 권한이 있는 사람, 그 요청의 담당 개발자(담당자와 프로젝트 담당자)입니다. 그 밖의 사람에게는 없는 요청과 똑같이 `404`를 돌려줍니다.
- `listRequests`는 `scope`(`mine`, `request:review`가 필요한 `assigned`, `request:read:all`이 필요한 `all`), 쉼표로 구분한 `status`, `projectId`, `q`(제목 또는 `#번호`)를 받고, 최근에 바뀐 순으로 돌려줍니다.
- `transitionRequest`는 사람이 보낼 수 있는 이벤트 12가지를 받습니다. 목 API와 같은 순서로 검사합니다.
  1. 본문을 검사합니다.
  2. 요청을 열 수 없는 사람이면 `404`입니다.
  3. 그 사람의 단계가 아니면 `403 notYourStep`입니다. 이 검사는 상태와 상관없이 먼저 합니다. 요청자의 이벤트는 요청자가 보내고(취소는 관리자도 할 수 있음), 개발자의 이벤트는 담당 개발자가 보냅니다.
  4. 지금 상태에서 할 수 없는 이벤트면 `409 invalid_transition`입니다.
  5. 가드에 걸리면 `409 guard_failed`입니다. 가드는 다음과 같습니다: 완료 기준 전부 통과, 하나 이상 실패, 가정 확인, 범위 질문 답변, 범위 다시 정리는 두 번까지, 배포 예정일 설정.

  AI와 시스템의 이벤트는 API로 받지 않습니다. 그래서 AI가 사람의 단계를 건너뛸 수 없습니다.
- 상태 변경, 타임라인 기록, 그에 따른 알림은 요청 행을 잠근 한 트랜잭션에서 처리합니다. 요청을 움직이는 것 말고도 할 수 있는 일이 있습니다. 요청자는 체크리스트를 저장하고 미리보기에 의견을 남깁니다. 담당 개발자는 회귀 확인 결과를 기록하고 배포 일정을 정합니다. 새 배포 예정일은 업무 달력 기준 오늘보다 앞설 수 없습니다.
- 알림은 메시지 키와 매개변수로 저장하고, 목록을 볼 때 읽는 사람의 언어로 씁니다. 문구는 목 API와 같습니다.
  - 알림을 일으킨 본인에게는 보내지 않습니다.
  - 설정으로 앱 알림을 끄거나, 조치가 필요한 알림만 받을 수 있습니다.
  - 한 사람당 최근 200개까지만 남깁니다.
  - 실제로 보내는 것은 앱 알림뿐입니다. 이메일과 메신저 설정은 그 채널을 보내는 백엔드를 위해 저장만 합니다.
- **AI 작업.** 요청이 `reporting`이 되면 리포트 작업이, `in_progress`가 되면 구현 작업이 큐(`ai_jobs`)에 들어갑니다.
  - 백그라운드 워커가 처리할 차례가 된 작업을 가져갑니다. `FOR UPDATE SKIP LOCKED`로 가져가므로 서버를 여러 대 띄워도 한 작업을 두 번 처리하지 않습니다.
  - AI 제공자는 트랜잭션 밖에서 호출합니다. 결과는 AI 자신의 이벤트(`reportReady`, `workFinished`)로 반영합니다. 구현 가능 또는 조건부 리포트라 요청자의 확인이 필요하면 `reportReady` 대신 `scopeReady`를 씁니다. 결과를 반영하는 건 요청이 작업을 시작할 때의 상태 그대로일 때뿐입니다.
  - 실패한 작업은 30초 뒤, 60초 뒤 두 번 더 시도합니다. 그래도 실패하면 사람이 처리하도록 남겨 둡니다.
  - AI는 사람의 이벤트를 보내지 않습니다. 그래서 머지를 승인하거나, 검토를 건너뛰거나, 사람이 확인할 단계를 넘길 수 없습니다.
- `AX_AI_PROVIDER=stub`(기본값)이면 작업을 넣은 사람의 언어로 "AI 제공자가 연결되지 않았다"고 알립니다. 가능성은 판단하지 않고(`undetermined`) 코드도 바꾸지 않습니다(`outcome: error`). 그래서 개발자가 직접 판단하면서 상태 전이 전체를 끝까지 쓸 수 있습니다.
- **Claude**(`AX_AI_PROVIDER=anthropic`)는 Messages API 호출 한 번(`POST /v1/messages`, 공식 Rust SDK가 없어 HTTP로 호출)으로 리포트를 씁니다.
  - 모델은 AI 설정이 프로젝트에 고른 것(기본 Opus)이고, effort는 `high`입니다. 구조화 출력(`output_config.format`)을 쓰고, 지시문은 캐시합니다. 모델이 지원하면 안전 거절 때 서버 측 fallback을 씁니다(Haiku는 제외).
  - 요청과 프로젝트의 설명·화면·규칙만 읽고, 코드와 위키는 읽지 않습니다. 그래서 읽지 않은 것은 주장하지 않습니다. 프런트엔드 목처럼 근거 없는 리포트는 `undetermined`(`non_code` 요청은 `out_of_scope`)이고, 계획은 비어 있으며 위키 신뢰도는 `low`입니다.
  - Claude가 더하는 내용은 판단하는 개발자를 위한 것입니다. 완료 기준마다 확인할 방법, 요청자에게 물을 것, 코드에서 확인할 가정, 요청이 어길 수 있는 프로젝트 규칙, 함께 다시 확인할 화면, 규모, 위험을 씁니다. 서버는 완료 기준과 규칙을 요청과 프로젝트의 순서대로 두고, 프로젝트에 없는 화면은 뺍니다.
  - 호출마다 그 작업을 시작한 사람(첫 리포트는 요청자)에게 `ai_usage`로 기록하고, 작업이 실패해도 기록합니다. 거절, 거부된 요청(`4xx`), 토큰 소진은 바로 실패로 끝내고, 장애와 rate limit은 다시 시도합니다.
  - 작업 실행에는 저장소를 다루는 코딩 에이전트가 필요합니다. 연결되기 전까지는 그렇다고 알리고 코드를 바꾸지 않습니다.
- **대시보드와 배포 일정.**
  - `dashboard`(`dashboard:view`)는 한 달 동안 접수된 요청과 반영된 요청을 셉니다.
    - 대상 달을 지정하지 않으면 이번 달입니다.
    - 이번 달처럼 아직 끝나지 않은 달은 지난달의 같은 날짜 범위와 비교합니다.
    - 평균 소요 기간, 진행 중인 일과 지연된 일, 이번 주 배포 예정, 확인이 필요한 일도 함께 보여 줍니다.
  - `releases`(`schedule:view`)는 회사 전체의 배포 일정입니다.
    - 각 요청은 실제로 반영된 날에 놓이고, 아직 반영되지 않았으면 예정일에 놓입니다.
    - 요청마다 보는 사람이 그 요청을 열 수 있는지 알려 줍니다.
  - 날짜와 달은 모두 업무 달력(`AX_TIME_ZONE`) 기준입니다.
  - AI 비용 항목은 그달에 기록된 AI 사용 금액과 월 한도입니다.
- 목록에서 거르는 값은 컬럼에, 계약의 `WorkRequest` 나머지는 JSONB 하나에 둡니다. 타임라인은 추가만 하는 별도 테이블입니다. 요청 번호는 1부터 시작합니다.
- **첨부.** `uploadAttachment`(`request:create`)는 multipart 필드 `file`을 받아 무작위 id로 `AX_UPLOAD_DIR`에 저장합니다. 10MB까지 받습니다. 파일이 없거나 한도를 넘으면 `400`이고, 목 API처럼 권한보다 먼저 검사합니다. 올린 사람만 요청에 붙일 수 있고, 요청에는 첨부마다 한 번씩, 보낸 순서대로 들어갑니다.
  - 계약에는 내려받기 operation이 없고, `<img>`는 bearer 토큰을 보낼 수 없습니다. 그래서 `Attachment.url`은 계약 밖의 유일한 경로인 `GET /files/{id}`로 가는 서명 링크입니다. id와 만료 시각의 HMAC이고, 서버가 첨부를 보여 줄 때마다 새로 만들며, 12~24시간 동안 쓸 수 있습니다. 바뀌었거나 만료된 링크는 `404`입니다. 키는 처음 시작할 때 만들어 데이터베이스(`server_secrets`)에 두므로 따로 설정할 비밀값이 없습니다.
  - 프런트엔드가 화면에서 바로 여는 형식(PNG, JPEG, GIF, WebP, AVIF, BMP, PDF)만 원래 형식으로 보냅니다. SVG와 HTML을 포함한 나머지는 내려받도록 `application/octet-stream`으로 보내고, 항상 `nosniff`를 붙입니다.
- **임시 저장**(`request:create`)은 다 쓰지 않은 요청을 저장해 두었다가 어느 기기에서든 이어 쓰는 기능입니다. 쓴 사람만 볼 수 있고, 다른 사람의 id는 `404`입니다.
  - 내용은 계약의 `DraftContent`이고, 보낸 그대로 JSONB 컬럼 하나에 둡니다. 모든 필드가 선택이며 남용을 막는 상한(10,000자, 50개)만 검사합니다. 첨부는 쓴 사람이 직접 올린 것이어야 합니다(색인마다 `400`, `content.attachmentIds.<i>`).
  - 같은 `clientId`로 다시 만들면 처음 만든 초안을 지금 상태 그대로 돌려줍니다. 한 사람이 `AX_DRAFTS_PER_ACCOUNT`개(20)까지 저장할 수 있고, 하나 더 저장하면 `409 limit_reached`입니다. 저장하는 동안 계정 행을 잠가서 두 저장이 마지막 자리를 함께 차지할 수 없습니다.
  - 수정할 때는 시작한 버전을 보냅니다. 그 사이 저장된 초안이면 `409 conflict`입니다. 확인과 쓰기는 잠긴 행에서 합니다.
  - `draftId`로 요청을 접수하면 같은 트랜잭션에서 그 초안을 지웁니다.

## AI 설정, 예산, 사용량

이 기능은 서비스 운영자(`operator`, CLI로만 만듭니다)의 것입니다. 필요한 권한은 `budget:manage`, `agent:manage`, `usage:view`입니다.

- **제공자 키는 설정으로만 받습니다.** `AX_ANTHROPIC_API_KEY`를 쓰거나, 작업마다 키를 따로 둡니다(`AX_ANTHROPIC_API_KEY_PRECHECK`, `_REPORT`, `_WORK`). 키를 나누면 제공자 콘솔에서 작업별 지출을 키별로 볼 수 있습니다. `getAiSettings`는 연결된 제공자를 키의 마지막 네 글자와 함께 보여 줍니다. `setProviderKey`와 `removeProviderKey`는 API로 운영자를 만들 때처럼 `403`입니다. 연결할 수 있는 제공자는 Anthropic뿐입니다. OpenAI는 계약에 남아 있지만 여기서는 연결되지 않습니다.
- **작업별 모델**(`getAiSettings`, `updateAiSettings`): `precheck`, `report`, `work`마다 기본 모델을 두고, 프로젝트별로 바꿀 수 있습니다. 고른 모델은 연결된 제공자의 것이어야 하고 가격이 있어야 합니다(`400`). 없는 프로젝트나, 한 프로젝트에 같은 작업을 두 번 지정하면 `400`입니다. AI 작업은 쓴 에이전트와 모델을 기록합니다(`work.agent`).
- **가격**(`getAiPricing`, `updateAiPricing`): 백만 토큰당 USD(입력, 캐시 읽기, 캐시 쓰기, 출력)와 환율입니다. 쓰고 있는 모델은 뺄 수 없습니다(`409`). 새 가격은 그 뒤 기록되는 사용량부터 적용되고, 이미 기록된 사용량은 기록할 때의 가격을 유지합니다.
- **예산**(`getBudget`, `updateBudget`): 월 한도, 요청당 한도(월 한도를 정했다면 그보다 클 수 없음), 경고 임계값입니다. 금액은 원 단위 정수이고, `0`은 한도 없음입니다. 새 서버는 한도 없음으로 시작합니다.
  - 목 API처럼 월 한도는 보여 주기만 하고 막지는 않습니다.
  - 요청당 한도는 실제로 막습니다. 요청에 기록된 AI 비용이 한도에 닿으면, 운영자가 한도를 올릴 때까지 `runWork`, `requestRevision`, `reviseScope`가 `409 guard_failed`입니다. 개발자의 질문에 답하는 일은 막지 않습니다.
- **사용량**(`aiUsage`, `?month=YYYY-MM`): 모델 호출이나 에이전트 실행 한 번마다 장부(`ai_usage`)에 한 줄을 남깁니다.
  - 기록할 때의 가격으로 계산하고, 그 작업을 시작한 사람과 그 순간 그 사람이 속한 팀에 기록합니다.
  - 보고서는 제공자, 모델, 작업, 날짜, 요청(상위 10개), 팀, 사람별로 목 API와 같은 방식으로 합산합니다.
  - 요청 화면은 전체 리포트를 볼 수 있는 사람(`report:read`)에게 그 요청의 사용량을 보여 줍니다.
- 설정은 `ai_config`에 항목마다 JSON 문서 하나로 둡니다. 없는 항목은 템플릿 기본값(`src/config/ai.ts`)을 씁니다. precheck는 Haiku, report와 work는 Opus이고, 가격표는 템플릿의 것을 쓰며 환율은 임시값입니다. 쓰기는 advisory lock 하나를 잡으므로, 가격을 빼는 일과 그 모델을 고르는 일이 엇갈리지 않습니다.

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
| `AX_ANTHROPIC_API_KEY` | 없음 | 모든 AI 작업에 쓰는 Claude Platform 키. API로는 절대 돌려주지 않고, 콘솔에는 마지막 네 글자만 보입니다. |
| `AX_ANTHROPIC_API_KEY_PRECHECK`, `AX_ANTHROPIC_API_KEY_REPORT`, `AX_ANTHROPIC_API_KEY_WORK` | 없음 | 한 작업에만 쓰는 키. 그 작업에는 `AX_ANTHROPIC_API_KEY` 대신 이 키를 씁니다. |
| `AX_AI_PROVIDER` | `stub` | 리포트를 쓰는 제공자: `stub`, 또는 `anthropic`(Claude, 키 필요). 작업 실행은 어느 쪽이든 코딩 에이전트를 기다립니다. |
| `AX_SCOPE_CHECK` | `true` | 구현 가능 또는 조건부 리포트가 나오면, 개발자 검토 전에 요청자가 AI가 이해한 범위를 확인합니다. 프런트엔드의 `appConfig.workflow.scopeCheck`와 같은 값으로 둡니다. |
| `AX_TIME_ZONE` | `Asia/Seoul` | 업무 달력의 시간대(IANA 이름). 희망일과 배포일, 지연, 월별 합계를 계산할 때 "오늘"의 기준입니다. 프런트엔드의 `appConfig.organization.timeZone`과 같은 값으로 둡니다. PostgreSQL이 모르는 이름이면 서버가 시작되지 않습니다. |
| `AX_DRAFTS_PER_ACCOUNT` | `20` | 한 사람이 저장할 수 있는 초안 수. 프런트엔드의 `drafts.maxPerAccount`와 같은 값으로 둡니다. |
| `AX_UPLOAD_DIR` | `uploads` | 올린 첨부를 두는 곳. 없으면 만듭니다. Compose에서는 `uploads` 볼륨에 둡니다. |
| `AX_PUBLIC_PATH` | `/api` | 브라우저가 이 서버의 경로 앞에 붙이는 경로(프런트엔드 빌드의 `VITE_API_BASE_URL`). 첨부 링크가 이 경로로 시작합니다. |
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
