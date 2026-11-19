//! The contract's error body (`ApiError`) and how each code maps to a status.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::lang::Lang;

/// `ApiError.code`. A unit test keeps this list equal to the contract's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    InvalidTransition,
    GuardFailed,
    Conflict,
    LimitReached,
    Internal,
}

impl ErrorCode {
    pub const ALL: [Self; 9] = [
        Self::BadRequest,
        Self::Unauthorized,
        Self::Forbidden,
        Self::NotFound,
        Self::InvalidTransition,
        Self::GuardFailed,
        Self::Conflict,
        Self::LimitReached,
        Self::Internal,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::InvalidTransition => "invalid_transition",
            Self::GuardFailed => "guard_failed",
            Self::Conflict => "conflict",
            Self::LimitReached => "limit_reached",
            Self::Internal => "internal",
        }
    }

    /// The default status for the code. The contract puts all lifecycle and
    /// draft refusals on 409.
    pub const fn status(self) -> StatusCode {
        match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::InvalidTransition | Self::GuardFailed | Self::Conflict | Self::LimitReached => StatusCode::CONFLICT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// One field-level problem, e.g. `{ path: "title", message: "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

/// The JSON body of every 4xx/5xx response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiErrorBody {
    pub code: ErrorCode,
    /// Already in the request's language.
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: ApiErrorBody,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { status: code.status(), body: ApiErrorBody { code, message: message.into(), issues: Vec::new() } }
    }

    #[must_use]
    pub fn with_issues(mut self, issues: Vec<Issue>) -> Self {
        self.body.issues = issues;
        self
    }

    pub fn not_found(lang: Lang) -> Self {
        Self::new(ErrorCode::NotFound, lang.pick("요청한 주소를 찾을 수 없습니다.", "Not found."))
    }

    /// `400`: the body does not match the operation's schema (the mock's `checkInput`).
    pub fn check_input(lang: Lang, issues: Vec<Issue>) -> Self {
        Self::new(ErrorCode::BadRequest, lang.pick("입력값을 확인해 주세요.", "Please check the input."))
            .with_issues(issues)
    }

    /// `401`: no session, or one that has ended (the mock's `signInAgain`).
    pub fn sign_in_again(lang: Lang) -> Self {
        Self::new(ErrorCode::Unauthorized, lang.pick("다시 로그인해 주세요.", "Please sign in again."))
    }

    /// `401` from `login`, whatever the reason: no such account, a deactivated
    /// one, or a wrong password (the mock's `wrongCredentials`).
    pub fn wrong_credentials(lang: Lang) -> Self {
        Self::new(
            ErrorCode::Unauthorized,
            lang.pick("이메일 또는 비밀번호가 맞지 않습니다.", "The email or password is incorrect."),
        )
    }

    /// `403`: the role lacks the permission (the mock's `forbidden`).
    pub fn forbidden(lang: Lang) -> Self {
        Self::new(
            ErrorCode::Forbidden,
            lang.pick("이 작업을 할 권한이 없습니다.", "You do not have permission to do this."),
        )
    }

    /// `403`: an admin tried to create or change an operator account (the mock's `operatorProvisioned`).
    pub fn operator_provisioned(lang: Lang) -> Self {
        Self::new(
            ErrorCode::Forbidden,
            lang.pick(
                "운영자 계정은 서비스가 관리합니다. 여기서 만들거나 바꿀 수 없습니다.",
                "Operator accounts are managed by the service and cannot be created or changed here.",
            ),
        )
    }

    /// `400`: another account has this email, in any case (the mock's `emailTaken`).
    pub fn email_taken(lang: Lang) -> Self {
        Self::new(ErrorCode::BadRequest, lang.pick("이미 등록된 이메일입니다.", "This email is already registered."))
    }

    /// `400`: another team has this name, in any case (the mock's `teamNameTaken`).
    pub fn team_name_taken(lang: Lang) -> Self {
        Self::new(
            ErrorCode::BadRequest,
            lang.pick("같은 이름의 팀이 이미 있습니다.", "A team with this name already exists."),
        )
    }

    /// `400`: an admin tried to deactivate themselves or change their own role (the mock's `cannotChangeSelf`).
    pub fn cannot_change_self(lang: Lang) -> Self {
        Self::new(
            ErrorCode::BadRequest,
            lang.pick("자신의 역할이나 활성 상태는 바꿀 수 없습니다.", "You cannot change your own role or status."),
        )
    }

    /// `400`: a setup link for a deactivated person (the mock's `setupLinkInactive`).
    pub fn setup_link_inactive(lang: Lang) -> Self {
        Self::new(
            ErrorCode::BadRequest,
            lang.pick(
                "비활성 사용자에게는 링크를 만들 수 없습니다. 먼저 활성화해 주세요.",
                "Activate this person before making a link.",
            ),
        )
    }

    /// `401` from `completeSetup`: an unknown, used or expired link, or a
    /// deactivated account (the mock's `setupLinkInvalid`).
    pub fn setup_link_invalid(lang: Lang) -> Self {
        Self::new(
            ErrorCode::Unauthorized,
            lang.pick(
                "링크가 올바르지 않거나 이미 쓰였거나 만료되었습니다. 관리자에게 새 링크를 요청해 주세요.",
                "This link is not valid, was already used or has expired. Ask your admin for a new one.",
            ),
        )
    }

    /// `400`: a project owner who is not a developer or an admin (the mock's `ownersMustBeDevelopers`).
    pub fn owners_must_be_developers(lang: Lang) -> Self {
        Self::new(
            ErrorCode::BadRequest,
            lang.pick("담당자는 개발자 또는 관리자여야 합니다.", "Owners must be developers or admins."),
        )
    }

    /// `404` on a project's own address, `400` where a request names one (the mock's `projectNotFound`).
    pub fn project_not_found(lang: Lang, status: StatusCode) -> Self {
        let code = if status == StatusCode::NOT_FOUND { ErrorCode::NotFound } else { ErrorCode::BadRequest };
        Self {
            status,
            ..Self::new(code, lang.pick("선택한 프로젝트를 찾을 수 없습니다.", "The selected project was not found."))
        }
    }

    /// `404`: no such request, or one the person may not open (the mock's
    /// `requestNotFound`; existence is not given away).
    pub fn request_not_found(lang: Lang, id: impl std::fmt::Display) -> Self {
        let message = match lang {
            Lang::Ko => format!("요청 #{id}을 찾을 수 없습니다."),
            Lang::En => format!("Request #{id} was not found."),
        };
        Self::new(ErrorCode::NotFound, message)
    }

    /// `404` for a path parameter that is not a valid id (the mock's `notFound`
    /// with the parameter's issue).
    pub fn bad_path_param(lang: Lang, name: &str) -> Self {
        let message = lang.pick("찾을 수 없습니다.", "Not found.");
        Self::new(ErrorCode::NotFound, message)
            .with_issues(vec![Issue { path: name.to_owned(), message: message.to_owned() }])
    }

    /// `400`: query parameters the operation does not accept (the mock's `badQuery`).
    pub fn bad_query(lang: Lang, issues: Vec<Issue>) -> Self {
        Self::new(ErrorCode::BadRequest, lang.pick("잘못된 조회 조건입니다.", "The query parameters are invalid."))
            .with_issues(issues)
    }

    /// `400`: a target that is not one of the project's (the mock's `unknownTarget`).
    pub fn unknown_target(lang: Lang, target: &str) -> Self {
        let message = match lang {
            Lang::Ko => format!("알 수 없는 대상입니다: {target}"),
            Lang::En => format!("Unknown screen: {target}"),
        };
        Self::new(ErrorCode::BadRequest, message)
    }

    /// `400`: attachments that are missing or someone else's, one issue per
    /// index (the mock's `attachmentNotFound`).
    pub fn attachment_not_found(lang: Lang, paths: Vec<String>) -> Self {
        let message = lang.pick(
            "첨부 파일을 찾을 수 없습니다. 첨부를 지우고 다시 올려 주세요.",
            "An attachment was not found. Remove it and upload it again.",
        );
        let issues = paths.into_iter().map(|path| Issue { path, message: message.to_owned() }).collect();
        Self::new(ErrorCode::BadRequest, message).with_issues(issues)
    }

    /// `404` (the mock's `userNotFound`).
    pub fn user_not_found(lang: Lang) -> Self {
        Self::new(ErrorCode::NotFound, lang.pick("사용자를 찾을 수 없습니다.", "User not found."))
    }

    /// `404` (the mock's `teamNotFound`).
    pub fn team_not_found(lang: Lang) -> Self {
        Self::new(ErrorCode::NotFound, lang.pick("팀을 찾을 수 없습니다.", "Team not found."))
    }

    /// `500` (the mock's `unknown`). The cause goes to the log, not to the client.
    pub fn internal(lang: Lang) -> Self {
        Self::new(ErrorCode::Internal, lang.pick("알 수 없는 오류가 발생했습니다.", "An unexpected error occurred."))
    }

    /// `500` for a database failure, logged here.
    pub fn database(lang: Lang, error: &sqlx::Error) -> Self {
        tracing::error!(%error, "database error");
        Self::internal(lang)
    }

    /// The same as [`ApiError::database`], shaped for `map_err`:
    /// `query.await.map_err(ApiError::db(lang))?`.
    pub fn db(lang: Lang) -> impl Fn(sqlx::Error) -> Self {
        move |error| Self::database(lang, &error)
    }

    /// `500` for an operation that needs the database when `DATABASE_URL` is unset.
    pub fn no_database(lang: Lang) -> Self {
        Self::new(
            ErrorCode::Internal,
            lang.pick("서버에 데이터베이스가 설정되지 않았습니다.", "The server has no database configured."),
        )
    }

    /// A contract operation this server does not implement yet: `501` with
    /// the contract's `internal` code, naming the operation.
    pub fn not_implemented(lang: Lang, operation_id: &str) -> Self {
        let message = match lang {
            Lang::Ko => format!("아직 구현되지 않은 기능입니다: {operation_id}"),
            Lang::En => format!("Not implemented yet: {operation_id}"),
        };
        Self { status: StatusCode::NOT_IMPLEMENTED, ..Self::new(ErrorCode::Internal, message) }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_codes_match_the_contract() {
        let ours: BTreeSet<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
        let contract: BTreeSet<&str> = ax_contract::error_codes().into_iter().collect();
        assert_eq!(ours, contract, "ErrorCode drifted from ApiError.code in contract/openapi.json");
    }

    #[test]
    fn serde_names_match_as_str() {
        for code in ErrorCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        }
    }

    #[test]
    fn issues_are_left_out_when_empty() {
        let body = serde_json::to_value(ApiError::not_found(Lang::En).body).unwrap();
        assert_eq!(body, serde_json::json!({ "code": "not_found", "message": "Not found." }));
    }

    #[test]
    fn auth_errors_use_the_mock_apis_status_and_code() {
        for (error, status, code) in [
            (ApiError::check_input(Lang::En, Vec::new()), 400, ErrorCode::BadRequest),
            (ApiError::sign_in_again(Lang::En), 401, ErrorCode::Unauthorized),
            (ApiError::wrong_credentials(Lang::En), 401, ErrorCode::Unauthorized),
            (ApiError::forbidden(Lang::En), 403, ErrorCode::Forbidden),
            (ApiError::operator_provisioned(Lang::En), 403, ErrorCode::Forbidden),
            (ApiError::internal(Lang::En), 500, ErrorCode::Internal),
            (ApiError::no_database(Lang::En), 500, ErrorCode::Internal),
            (ApiError::email_taken(Lang::En), 400, ErrorCode::BadRequest),
            (ApiError::team_name_taken(Lang::En), 400, ErrorCode::BadRequest),
            (ApiError::cannot_change_self(Lang::En), 400, ErrorCode::BadRequest),
            (ApiError::user_not_found(Lang::En), 404, ErrorCode::NotFound),
            (ApiError::team_not_found(Lang::En), 404, ErrorCode::NotFound),
            (ApiError::setup_link_inactive(Lang::En), 400, ErrorCode::BadRequest),
            (ApiError::owners_must_be_developers(Lang::En), 400, ErrorCode::BadRequest),
            (ApiError::project_not_found(Lang::En, StatusCode::NOT_FOUND), 404, ErrorCode::NotFound),
            (ApiError::project_not_found(Lang::En, StatusCode::BAD_REQUEST), 400, ErrorCode::BadRequest),
            (ApiError::setup_link_invalid(Lang::En), 401, ErrorCode::Unauthorized),
        ] {
            assert_eq!(error.status.as_u16(), status, "{}", error.body.message);
            assert_eq!(error.body.code, code, "{}", error.body.message);
        }
        assert_eq!(ApiError::forbidden(Lang::Ko).body.message, "이 작업을 할 권한이 없습니다.");
    }
}
