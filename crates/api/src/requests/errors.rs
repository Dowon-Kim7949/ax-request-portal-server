//! The mock API's error messages for request actions (`src/mocks/copy.ts`).

use crate::error::{ApiError, ErrorCode};
use crate::lang::Lang;
use crate::notifications::readable_date;

use super::lifecycle::Guard;

fn keyed(code: ErrorCode, lang: Lang, ko: &str, en: &str) -> ApiError {
    ApiError::new(code, lang.pick(ko, en))
}

/// `403`: the event belongs to someone else (`notYourStep`).
pub fn not_your_step(lang: Lang) -> ApiError {
    keyed(ErrorCode::Forbidden, lang, "이 단계에서 할 수 있는 작업이 아닙니다.", "You cannot do this at this step.")
}

/// `409`: the event cannot leave the current status (`invalidTransition`).
pub fn invalid_transition(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::InvalidTransition,
        lang,
        "지금 단계에서는 할 수 없는 작업입니다.",
        "This is not possible at the current step.",
    )
}

/// `409 guard_failed`, with the mock's message for each guard (`GUARD_ERRORS`).
pub fn guard_failed(lang: Lang, guard: Guard) -> ApiError {
    let (ko, en) = match guard {
        Guard::CriteriaAllPassed => (
            "모든 완료 기준이 통과해야 완료할 수 있습니다.",
            "Every acceptance criterion must pass before you confirm.",
        ),
        Guard::CriteriaOneFailed => {
            ("안 되는 완료 기준을 하나 이상 표시해 주세요.", "Mark at least one acceptance criterion as not working.")
        }
        Guard::AssumptionsUnchecked => (
            "보고서의 가정을 모두 확인해야 AI 작업을 시작할 수 있습니다.",
            "Confirm every assumption in the report before the AI starts work.",
        ),
        Guard::ScopeUnanswered => ("골라야 할 항목에 모두 답해 주세요.", "Answer every question first."),
        Guard::ScopeRevisionsUsed => (
            "다시 정리는 더 요청할 수 없습니다. 덧붙일 말을 적어 진행하거나 요청을 취소해 주세요.",
            "The scope cannot be summed up again. Go ahead with a note, or cancel the request.",
        ),
        Guard::ReleaseUnscheduled => (
            "배포 예정일을 먼저 정해야 배포 완료로 바꿀 수 있습니다.",
            "Set the planned release date before marking the request released.",
        ),
    };
    keyed(ErrorCode::GuardFailed, lang, ko, en)
}

/// `409 guard_failed`: the request has spent its per-request AI limit, so no
/// AI run may start again (`requestBudgetExceeded`).
pub fn request_budget_exceeded(lang: Lang, limit: i64) -> ApiError {
    let limit = crate::error::thousands(limit);
    let message = match lang {
        Lang::Ko => format!(
            "이 요청의 AI 비용이 요청당 한도({limit}원)에 도달해 AI 작업을 더 실행할 수 없습니다. 운영자가 한도를 올리거나 직접 처리해 주세요."
        ),
        Lang::En => format!(
            "This request has used its per-request AI budget (₩{limit}), so AI work cannot run again. Ask an operator to raise the limit, or handle it by hand."
        ),
    };
    ApiError::new(ErrorCode::GuardFailed, message)
}

pub fn requester_only_verify(lang: Lang) -> ApiError {
    keyed(ErrorCode::Forbidden, lang, "요청자만 결과를 확인할 수 있습니다.", "Only the requester can check the result.")
}

pub fn requester_only_preview(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::Forbidden,
        lang,
        "요청자만 미리보기에 의견을 남길 수 있습니다.",
        "Only the requester can comment on the preview.",
    )
}

pub fn no_preview_now(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::InvalidTransition,
        lang,
        "지금은 미리보기에 의견을 남길 수 없습니다.",
        "There is no preview to comment on right now.",
    )
}

pub fn not_in_verification(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::InvalidTransition,
        lang,
        "지금은 결과 확인 단계가 아닙니다.",
        "The request is not waiting for a result check.",
    )
}

pub fn developer_only_regression(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::Forbidden,
        lang,
        "담당 개발자만 기록할 수 있습니다.",
        "Only the responsible developer can record this.",
    )
}

pub fn developer_only_schedule(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::Forbidden,
        lang,
        "담당 개발자만 일정을 정할 수 있습니다.",
        "Only the responsible developer can set the release date.",
    )
}

pub fn cannot_schedule_now(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::InvalidTransition,
        lang,
        "지금 단계에서는 배포 일정을 정할 수 없습니다.",
        "A release date cannot be set at this step.",
    )
}

pub fn request_closed(lang: Lang) -> ApiError {
    keyed(
        ErrorCode::InvalidTransition,
        lang,
        "끝난 요청은 바꿀 수 없습니다.",
        "This request is closed and can no longer be changed.",
    )
}

/// `400`: a new release date before today on the business calendar.
pub fn release_date_in_past(lang: Lang, today: &str) -> ApiError {
    let message = match lang {
        Lang::Ko => format!("배포 예정일은 오늘(본사 기준 {}) 이후로 정해 주세요.", readable_date(lang, today)),
        Lang::En => {
            format!(
                "The release date cannot be in the past. Today is {} at the head office.",
                readable_date(lang, today)
            )
        }
    };
    ApiError::new(ErrorCode::BadRequest, message)
}
