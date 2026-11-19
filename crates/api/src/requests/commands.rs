//! The contract's `TransitionCommand` (the twelve events a person may send),
//! `VerificationDraft`, `PreviewFeedbackInput`, `RegressionUpdate` and
//! `ReleaseInput`, checked like their zod schemas. AI and system events are
//! not accepted from clients: they fail on `event`.

use serde_json::{Value, json};

use super::draft::is_iso_date;
use super::lifecycle::Event;
use crate::body::{Body, Text, utf16_len};
use crate::error::ApiError;
use crate::lang::Lang;

/// One acceptance criterion's result, as the requester marked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub criterion_id: String,
    /// `pending`, `passed` or `failed`
    pub result: String,
    pub note: Option<String>,
}

impl Check {
    pub fn to_json(&self) -> Value {
        let mut c = json!({ "criterionId": self.criterion_id, "result": self.result });
        if let Some(note) = &self.note {
            c["note"] = json!(note);
        }
        c
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    RequestInfo { questions: Vec<String> },
    ProvideInfo { answer: String },
    ApproveScope { choices: Vec<(String, String)>, note: Option<String> },
    ReviseScope { note: String },
    Reject { reason: String },
    RunWork { instructions: Option<String>, checked_assumptions: Option<Vec<String>> },
    RequestRevision { note: String },
    ApproveMerge,
    ReportIssue { checks: Vec<Check>, note: Option<String> },
    ConfirmResult { checks: Vec<Check> },
    MarkReleased,
    Cancel { reason: Option<String> },
}

impl Command {
    pub const fn event(&self) -> Event {
        match self {
            Self::RequestInfo { .. } => Event::RequestInfo,
            Self::ProvideInfo { .. } => Event::ProvideInfo,
            Self::ApproveScope { .. } => Event::ApproveScope,
            Self::ReviseScope { .. } => Event::ReviseScope,
            Self::Reject { .. } => Event::Reject,
            Self::RunWork { .. } => Event::RunWork,
            Self::RequestRevision { .. } => Event::RequestRevision,
            Self::ApproveMerge => Event::ApproveMerge,
            Self::ReportIssue { .. } => Event::ReportIssue,
            Self::ConfirmResult { .. } => Event::ConfirmResult,
            Self::MarkReleased => Event::MarkReleased,
            Self::Cancel { .. } => Event::Cancel,
        }
    }

    /// The timeline note (the mock's `note` per event).
    pub fn note(&self) -> Option<String> {
        let note = match self {
            Self::RequestInfo { questions } => Some(questions.join("\n")),
            Self::ProvideInfo { answer } => Some(answer.clone()),
            Self::ApproveScope { note, .. } | Self::ReportIssue { note, .. } => note.clone(),
            Self::ReviseScope { note } | Self::RequestRevision { note } => Some(note.clone()),
            Self::Reject { reason } => Some(reason.clone()),
            Self::Cancel { reason } => reason.clone(),
            Self::RunWork { instructions, .. } => instructions.clone(),
            Self::ApproveMerge | Self::ConfirmResult { .. } | Self::MarkReleased => None,
        };
        note.filter(|n| !n.is_empty())
    }

    /// Reads a `TransitionCommand`.
    ///
    /// # Errors
    ///
    /// `400` (`checkInput`) listing every issue.
    pub fn parse(bytes: &[u8], lang: Lang) -> Result<Self, ApiError> {
        const EVENT: Text = ("보낼 수 없는 작업입니다.", "Not an event a person can send.");
        let mut body = Body::parse(bytes, lang)?;
        let event = body.required("event", EVENT, |v| v.as_str().and_then(Event::parse).filter(|e| e.is_user_event()));
        let Some(event) = event else {
            body.finish()?;
            unreachable!("finish() fails when a required field is missing")
        };
        let command = match event {
            Event::RequestInfo => questions(&mut body).map(|questions| Self::RequestInfo { questions }),
            Event::ProvideInfo => {
                body.required("answer", ANSWER, trimmed_between(1, 2000)).map(|answer| Self::ProvideInfo { answer })
            }
            Event::ApproveScope => {
                let choices = choices(&mut body);
                let note = optional(&mut body, "note", MAX_1000, trimmed_between(0, 1000));
                choices.zip(note).map(|(choices, note)| Self::ApproveScope { choices, note })
            }
            Event::ReviseScope => {
                body.required("note", NOTE_1000, trimmed_between(1, 1000)).map(|note| Self::ReviseScope { note })
            }
            Event::Reject => {
                body.required("reason", REASON, trimmed_between(1, 1000)).map(|reason| Self::Reject { reason })
            }
            Event::RunWork => {
                let instructions = optional(&mut body, "instructions", MAX_2000, trimmed_between(0, 2000));
                let checked = optional(&mut body, "checkedAssumptions", ASSUMPTIONS, |v| {
                    let list: Option<Vec<String>> =
                        v.as_array()?.iter().map(|s| s.as_str().map(str::to_owned)).collect();
                    list.filter(|l| l.len() <= 50)
                });
                instructions
                    .zip(checked)
                    .map(|(instructions, checked_assumptions)| Self::RunWork { instructions, checked_assumptions })
            }
            Event::RequestRevision => {
                body.required("note", NOTE_2000, trimmed_between(1, 2000)).map(|note| Self::RequestRevision { note })
            }
            Event::ApproveMerge => Some(Self::ApproveMerge),
            Event::ReportIssue => {
                let checks = checks(&mut body);
                let note = optional(&mut body, "note", MAX_2000, trimmed_between(0, 2000));
                checks.zip(note).map(|(checks, note)| Self::ReportIssue { checks, note })
            }
            Event::ConfirmResult => checks(&mut body).map(|checks| Self::ConfirmResult { checks }),
            Event::MarkReleased => Some(Self::MarkReleased),
            Event::Cancel => {
                optional(&mut body, "reason", MAX_1000, trimmed_between(0, 1000)).map(|reason| Self::Cancel { reason })
            }
            Event::StartReport | Event::ReportReady | Event::ScopeReady | Event::WorkFinished => None,
        };
        body.finish()?;
        Ok(command.unwrap_or_else(|| unreachable!("finish() fails when a field is invalid")))
    }
}

const ANSWER: Text = ("답변은 1–2000자로 입력해 주세요.", "Answer in 1 to 2,000 characters.");
const QUESTIONS: Text = ("질문은 1–5개입니다.", "Ask 1 to 5 questions.");
const QUESTION: Text = ("질문을 입력해 주세요.", "Write the question.");
const CHOICES: Text = ("고른 항목을 확인해 주세요.", "Check the choices.");
const NOTE_1000: Text = ("1–1000자로 입력해 주세요.", "Write 1 to 1,000 characters.");
const NOTE_2000: Text = ("1–2000자로 입력해 주세요.", "Write 1 to 2,000 characters.");
const REASON: Text = ("이유를 1–1000자로 입력해 주세요.", "Give the reason in 1 to 1,000 characters.");
const MAX_1000: Text = ("1000자 이하로 입력해 주세요.", "Use 1,000 characters or fewer.");
const MAX_2000: Text = ("2000자 이하로 입력해 주세요.", "Use 2,000 characters or fewer.");
const ASSUMPTIONS: Text = ("확인한 가정 목록을 확인해 주세요.", "Check the list of confirmed assumptions.");
const CHECKS: Text = ("확인 결과 목록을 확인해 주세요.", "Check the list of results.");
const CHECK: Text = ("확인 결과를 확인해 주세요.", "Check this result.");

pub const RESULTS: [&str; 3] = ["pending", "passed", "failed"];

/// A string trimmed, then `min..=max` UTF-16 units long.
fn trimmed_between(min: usize, max: usize) -> impl Fn(&Value) -> Option<String> {
    move |v| {
        let t = v.as_str()?.trim();
        (min..=max).contains(&utf16_len(t)).then(|| t.to_owned())
    }
}

/// An optional field: `Some(None)` when absent, `None` after recording an issue.
fn optional<T>(body: &mut Body, key: &str, message: Text, read: impl FnOnce(&Value) -> Option<T>) -> Option<Option<T>> {
    match body.field(key) {
        None => Some(None),
        Some(_) => body.optional(key, message, read).map(Some),
    }
}

fn questions(body: &mut Body) -> Option<Vec<String>> {
    let Some(list) = body.field("questions").and_then(Value::as_array).cloned() else {
        body.issue("questions", QUESTIONS);
        return None;
    };
    let mut out = Vec::new();
    for (i, q) in list.iter().enumerate() {
        match trimmed_between(1, usize::MAX)(q) {
            Some(q) => out.push(q),
            None => body.issue(&format!("questions.{i}"), QUESTION),
        }
    }
    if !(1..=5).contains(&list.len()) {
        body.issue("questions", QUESTIONS);
        return None;
    }
    (out.len() == list.len()).then_some(out)
}

fn choices(body: &mut Body) -> Option<Vec<(String, String)>> {
    let Some(list) = body.field("choices").and_then(Value::as_array).cloned() else {
        body.issue("choices", CHOICES);
        return None;
    };
    if list.len() > 3 {
        body.issue("choices", CHOICES);
        return None;
    }
    let mut out = Vec::new();
    for (i, c) in list.iter().enumerate() {
        match (c["questionId"].as_str(), c["optionId"].as_str()) {
            (Some(q), Some(o)) => out.push((q.to_owned(), o.to_owned())),
            _ => body.issue(&format!("choices.{i}"), CHOICES),
        }
    }
    (out.len() == list.len()).then_some(out)
}

/// `checks: [{ criterionId, result, note? (≤ 1000, not trimmed) }]`.
pub fn checks(body: &mut Body) -> Option<Vec<Check>> {
    let Some(list) = body.field("checks").and_then(Value::as_array).cloned() else {
        body.issue("checks", CHECKS);
        return None;
    };
    let mut out = Vec::new();
    for (i, c) in list.iter().enumerate() {
        let criterion = c["criterionId"].as_str();
        let result = c["result"].as_str().filter(|r| RESULTS.contains(r));
        let note = match c.get("note") {
            None => Some(None),
            Some(n) => n.as_str().filter(|n| utf16_len(n) <= 1000).map(|n| Some(n.to_owned())),
        };
        for (field, ok) in
            [("criterionId", criterion.is_some()), ("result", result.is_some()), ("note", note.is_some())]
        {
            if !ok {
                body.issue(&format!("checks.{i}.{field}"), CHECK);
            }
        }
        if let (Some(criterion), Some(result), Some(note)) = (criterion, result, note) {
            out.push(Check { criterion_id: criterion.to_owned(), result: result.to_owned(), note });
        }
    }
    (out.len() == list.len()).then_some(out)
}

/// The contract's `PreviewFeedbackInput`: `looks_good` with an optional note,
/// or `needs_change` with one.
pub fn preview_feedback(bytes: &[u8], lang: Lang) -> Result<(bool, Option<String>), ApiError> {
    const VERDICT: Text = ("looks_good 또는 needs_change여야 합니다.", "Must be looks_good or needs_change.");
    let mut body = Body::parse(bytes, lang)?;
    let verdict = body.required("verdict", VERDICT, |v| {
        v.as_str().filter(|v| ["looks_good", "needs_change"].contains(v)).map(str::to_owned)
    });
    let needs_change = verdict.as_deref() == Some("needs_change");
    let note = if needs_change {
        body.required("note", NOTE_1000, trimmed_between(1, 1000)).map(Some)
    } else {
        optional(&mut body, "note", MAX_1000, trimmed_between(0, 1000))
    };
    body.finish()?;
    let note = note.unwrap_or_else(|| unreachable!("finish() fails when a field is invalid"));
    Ok((needs_change, note))
}

/// The contract's `RegressionUpdate`: `{ checks: [{ id, result }] }`.
pub fn regression_update(bytes: &[u8], lang: Lang) -> Result<Vec<(String, String)>, ApiError> {
    let mut body = Body::parse(bytes, lang)?;
    let list = body.required("checks", CHECKS, |v| {
        v.as_array()?
            .iter()
            .map(|c| {
                Some((c["id"].as_str()?.to_owned(), c["result"].as_str().filter(|r| RESULTS.contains(r))?.to_owned()))
            })
            .collect::<Option<Vec<_>>>()
    });
    body.finish()?;
    Ok(list.unwrap_or_else(|| unreachable!("finish() fails when a field is invalid")))
}

/// The contract's `ReleaseInput`: `{ scheduledDate?: YYYY-MM-DD, note?: ≤ 500 }`.
pub fn release_input(bytes: &[u8], lang: Lang) -> Result<(Option<String>, Option<String>), ApiError> {
    const DATE: Text = ("날짜 형식을 확인해 주세요. (예: 2026-11-30)", "Use a date like 2026-11-30.");
    const NOTE: Text = ("500자 이하로 입력해 주세요.", "Use 500 characters or fewer.");
    let mut body = Body::parse(bytes, lang)?;
    let date = optional(&mut body, "scheduledDate", DATE, |v| v.as_str().filter(|d| is_iso_date(d)).map(str::to_owned));
    let note = optional(&mut body, "note", NOTE, |v| v.as_str().filter(|n| utf16_len(n) <= 500).map(str::to_owned));
    body.finish()?;
    match (date, note) {
        (Some(date), Some(note)) => Ok((date, note)),
        _ => unreachable!("finish() fails when a field is invalid"),
    }
}

/// The contract's `VerificationDraft`: `{ checks }`.
pub fn verification_draft(bytes: &[u8], lang: Lang) -> Result<Vec<Check>, ApiError> {
    let mut body = Body::parse(bytes, lang)?;
    let list = checks(&mut body);
    body.finish()?;
    Ok(list.unwrap_or_else(|| unreachable!("finish() fails when a field is invalid")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> Result<Command, Vec<String>> {
        Command::parse(v.to_string().as_bytes(), Lang::En)
            .map_err(|e| e.body.issues.into_iter().map(|i| i.path).collect())
    }

    #[test]
    fn reads_every_event() {
        assert_eq!(
            parse(json!({ "event": "requestInfo", "questions": [" Which screen? "] })),
            Ok(Command::RequestInfo { questions: vec!["Which screen?".into()] })
        );
        assert_eq!(
            parse(json!({ "event": "runWork", "instructions": " ", "checkedAssumptions": ["a1"] })),
            Ok(Command::RunWork { instructions: Some(String::new()), checked_assumptions: Some(vec!["a1".into()]) })
        );
        assert_eq!(parse(json!({ "event": "approveMerge" })), Ok(Command::ApproveMerge));
        assert_eq!(parse(json!({ "event": "cancel" })), Ok(Command::Cancel { reason: None }));
        assert_eq!(
            parse(
                json!({ "event": "confirmResult", "checks": [{ "criterionId": "c1", "result": "passed", "note": " ok " }] })
            ),
            Ok(Command::ConfirmResult {
                checks: vec![Check { criterion_id: "c1".into(), result: "passed".into(), note: Some(" ok ".into()) }]
            })
        );
    }

    #[test]
    fn refuses_bad_payloads_and_machine_events() {
        assert_eq!(parse(json!({ "event": "workFinished" })), Err(vec!["event".into()]));
        assert_eq!(parse(json!({ "event": "nope" })), Err(vec!["event".into()]));
        assert_eq!(parse(json!({ "event": "requestInfo", "questions": [] })), Err(vec!["questions".into()]));
        assert_eq!(parse(json!({ "event": "requestInfo", "questions": [" ", "x"] })), Err(vec!["questions.0".into()]));
        assert_eq!(parse(json!({ "event": "reject", "reason": "  " })), Err(vec!["reason".into()]));
        assert_eq!(
            parse(json!({ "event": "reportIssue", "checks": [{ "criterionId": "c1", "result": "bad" }] })),
            Err(vec!["checks.0.result".into()])
        );
    }

    #[test]
    fn timeline_notes() {
        let info = Command::RequestInfo { questions: vec!["a".into(), "b".into()] };
        assert_eq!(info.note().as_deref(), Some("a\nb"));
        assert_eq!(Command::RunWork { instructions: Some(String::new()), checked_assumptions: None }.note(), None);
        assert_eq!(Command::Cancel { reason: Some("dup".into()) }.note().as_deref(), Some("dup"));
    }
}
