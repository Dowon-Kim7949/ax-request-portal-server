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
}
