//! `transitionRequest`: a person moves a request along its lifecycle (the
//! frontend's `applyTransition` and the mock engine's `transition`).
//!
//! Order, as in the mock API: body (`400`), then the request (`404` when it
//! does not exist or the person may not open it), whose step it is (`403
//! notYourStep`, before anything about the status), the status (`409
//! invalid_transition`), the guards (`409 guard_failed`). The change, its
//! timeline entry and its notifications are one transaction.

use std::collections::HashMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::commands::{Check, Command};
use super::errors;
use super::handlers::{parse_id, present};
use super::lifecycle::{Event, Guard, Party, SCOPE_REVISIONS_MAX, Status};
use super::store::{self, Actor, StoredRequest, field};
use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::lang::Lang;
use crate::notifications;
use crate::roles::{Permission, Role};
use crate::state::AppState;
use crate::users::User;

/// `POST /requests/{id}/transitions`.
pub async fn transition(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let id = parse_id(&id).ok_or_else(|| ApiError::bad_path_param(lang, "id"))?;
    let command = Command::parse(&bytes, lang)?;
    let user = &current.user;
    let db = state.db(lang)?;
    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    let (mut request, owners) = load(&mut tx, id, user, lang).await?;

    let event = command.event();
    if !is_authorized(event, user, &request, &owners) {
        return Err(errors::not_your_step(lang));
    }
    if !event.allowed_from(request.status) {
        return Err(errors::invalid_transition(lang));
    }
    check_guards(&request, &command).map_err(|guard| errors::guard_failed(lang, guard))?;
    // The per-request AI budget guard (runWork, requestRevision, reviseScope)
    // joins once AI usage is recorded; until then nothing has been spent.

    let at = store::now_iso(&mut *tx).await.map_err(ApiError::db(lang))?;
    let from = request.status;
    apply(&mut request, &command, user.id, &at);
    if matches!(event, Event::RunWork | Event::RequestRevision) && request.assignee_id.is_none() {
        request.assignee_id = Some(user.id);
    }
    let note = command.note();
    store::add_event(
        &mut tx,
        id,
        Actor::User(user.id),
        event.as_str(),
        Some(from),
        Some(request.status),
        note.as_deref(),
    )
    .await
    .map_err(ApiError::db(lang))?;
    request.save(&mut tx).await.map_err(ApiError::db(lang))?;
    notifications::notify_status(&mut tx, &request, &owners, event.as_str(), Some(user.id))
        .await
        .map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(present(user, &request)))
}

/// The request, locked, with its project's owners; `404` when it does not
/// exist or `user` may not open it.
pub async fn load(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    user: &User,
    lang: Lang,
) -> Result<(StoredRequest, Vec<Uuid>), ApiError> {
    let request = StoredRequest::find_for_update(tx, id)
        .await
        .map_err(ApiError::db(lang))?
        .ok_or_else(|| ApiError::request_not_found(lang, id))?;
    let owners = store::owners_of(&mut **tx, request.project_id).await.map_err(ApiError::db(lang))?;
    if !request.can_read(user, &owners) {
        return Err(ApiError::request_not_found(lang, id));
    }
    Ok((request, owners))
}

/// A developer who may act on the request: holds `request:review`, and is an
/// admin, its assignee or an owner of its project.
pub fn is_responsible_developer(user: &User, request: &StoredRequest, owners: &[Uuid]) -> bool {
    user.role.can(Permission::RequestReview)
        && (user.role == Role::Admin || request.assignee_id == Some(user.id) || owners.contains(&user.id))
}

/// Whose step the event is. Only the requester sends a requester's event
/// (an admin may also cancel); a responsible developer sends a developer's;
/// nobody sends the AI's or the system's.
pub fn is_authorized(event: Event, user: &User, request: &StoredRequest, owners: &[Uuid]) -> bool {
    match event.rule().by {
        Party::Requester => (event == Event::Cancel && user.role == Role::Admin) || request.requester_id == user.id,
        Party::Developer => is_responsible_developer(user, request, owners),
        Party::Ai | Party::System => false,
    }
}

fn criteria_ids(request: &StoredRequest) -> Vec<String> {
    field(&request.details, "acceptanceCriteria")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect()
}

/// The frontend's `assertGuards`.
fn check_guards(request: &StoredRequest, command: &Command) -> Result<(), Guard> {
    let d = &request.details;
    match command {
        Command::ConfirmResult { checks } | Command::ReportIssue { checks, .. } => {
            // The last check of a criterion counts; an unchecked one is pending.
            let by_id: HashMap<&str, &str> =
                checks.iter().map(|c| (c.criterion_id.as_str(), c.result.as_str())).collect();
            let results: Vec<&str> =
                criteria_ids(request).iter().map(|id| by_id.get(id.as_str()).copied().unwrap_or("pending")).collect();
            if matches!(command, Command::ConfirmResult { .. }) && results.iter().any(|r| *r != "passed") {
                return Err(Guard::CriteriaAllPassed);
            }
            if matches!(command, Command::ReportIssue { .. }) && !results.contains(&"failed") {
                return Err(Guard::CriteriaOneFailed);
            }
        }
        Command::ApproveScope { choices, .. } if !unanswered_scope_questions(request, choices).is_empty() => {
            return Err(Guard::ScopeUnanswered);
        }
        Command::ReviseScope { .. } if request.count_events(Event::ReviseScope.as_str()) >= SCOPE_REVISIONS_MAX => {
            return Err(Guard::ScopeRevisionsUsed);
        }
        Command::RunWork { checked_assumptions, .. } if request.status == Status::Review => {
            let checked = checked_assumptions.as_deref().unwrap_or_default();
            let unchecked = field(d, "report")["assumptions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a["id"].as_str())
                .any(|id| !checked.iter().any(|c| c == id));
            if unchecked {
                return Err(Guard::AssumptionsUnchecked);
            }
        }
        Command::MarkReleased if field(d, "release")["scheduledDate"].as_str().is_none_or(str::is_empty) => {
            return Err(Guard::ReleaseUnscheduled);
        }
        _ => {}
    }
    Ok(())
}

/// Scope questions without a valid answer among `choices` (the last answer
/// to a question counts). No scope, no questions.
fn unanswered_scope_questions(request: &StoredRequest, choices: &[(String, String)]) -> Vec<String> {
    let picked: HashMap<&str, &str> = choices.iter().map(|(q, o)| (q.as_str(), o.as_str())).collect();
    field(&request.details, "scope")["proposal"]["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|q| {
            let Some(answer) = q["id"].as_str().and_then(|id| picked.get(id).copied()) else { return true };
            !q["options"].as_array().into_iter().flatten().any(|o| o["id"].as_str() == Some(answer))
        })
        .filter_map(|q| q["id"].as_str().map(str::to_owned))
        .collect()
}

/// The criteria's results in their order: a check sent for each, or pending.
/// Checks of unknown criteria are dropped.
pub fn normalize_checks(request: &StoredRequest, checks: &[Check]) -> Value {
    let by_id: HashMap<&str, &Check> = checks.iter().map(|c| (c.criterion_id.as_str(), c)).collect();
    Value::Array(
        criteria_ids(request)
            .iter()
            .map(|id| match by_id.get(id.as_str()) {
                Some(check) => check.to_json(),
                None => json!({ "criterionId": id, "result": "pending" }),
            })
            .collect(),
    )
}

/// The frontend's `applyTransition`, minus the timeline entry: the new status
/// and what each event changes, dated `at`.
fn apply(request: &mut StoredRequest, command: &Command, user: Uuid, at: &str) {
    let from = request.status;
    request.status = command.event().rule().to;
    let verification = match command {
        Command::ReportIssue { checks, .. } | Command::ConfirmResult { checks } => {
            Some(normalize_checks(request, checks))
        }
        Command::ApproveMerge => Some(Value::Array(
            criteria_ids(request).iter().map(|id| json!({ "criterionId": id, "result": "pending" })).collect(),
        )),
        _ => None,
    };
    let id = request.id;
    let d = &mut request.details;
    if let Some(verification) = verification {
        d.insert("verification".into(), verification);
    }
    match command {
        Command::RequestInfo { questions } => {
            d.insert("infoRequest".into(), json!({ "questions": questions, "askedAt": at }));
        }
        Command::ProvideInfo { answer } => {
            if let Some(info) = d.get_mut("infoRequest").and_then(Value::as_object_mut) {
                info.insert("answer".into(), json!(answer));
            }
        }
        Command::ApproveScope { choices, note } => {
            let picked: HashMap<&str, &str> = choices.iter().map(|(q, o)| (q.as_str(), o.as_str())).collect();
            if let Some(scope) = d.get_mut("scope").and_then(Value::as_object_mut) {
                let ordered: Vec<Value> = scope["proposal"]["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|q| {
                        let qid = q["id"].as_str()?;
                        Some(json!({ "questionId": qid, "optionId": picked.get(qid)? }))
                    })
                    .collect();
                let mut decision = json!({ "at": at, "by": user, "choices": ordered });
                if let Some(note) = note.as_deref().filter(|n| !n.is_empty()) {
                    decision["note"] = json!(note);
                }
                scope.insert("decision".into(), decision);
            }
        }
        Command::Reject { reason } => {
            d.insert("rejectionReason".into(), json!(reason));
        }
        Command::RunWork { instructions, .. } => {
            let checked: Option<Vec<Value>> = if from == Status::Review {
                Some(
                    field(d, "report")["assumptions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|a| a["id"].clone())
                        .collect(),
                )
            } else {
                field(d, "work")["checkedAssumptions"].as_array().cloned()
            };
            let work = new_work(d, id, at, instructions.as_deref(), checked);
            d.insert("work".into(), work);
        }
        Command::RequestRevision { note } => {
            let checked = field(d, "work")["checkedAssumptions"].as_array().cloned();
            let work = new_work(d, id, at, Some(note.as_str()), checked);
            d.insert("work".into(), work);
        }
        Command::ConfirmResult { .. } => {
            let release = d.get("release").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
            d.insert("release".into(), release);
        }
        Command::MarkReleased => {
            let mut release = d.get("release").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
            release["releasedAt"] = json!(at);
            d.insert("release".into(), release);
        }
        Command::ReviseScope { .. } | Command::ApproveMerge | Command::ReportIssue { .. } | Command::Cancel { .. } => {}
    }
}

/// A new run of the AI's work: everything of the previous run is reset.
fn new_work(
    d: &Map<String, Value>,
    id: i64,
    at: &str,
    instructions: Option<&str>,
    checked: Option<Vec<Value>>,
) -> Value {
    let attempt = d.get("work").and_then(|w| w["attempt"].as_i64()).unwrap_or(0) + 1;
    let mut work =
        json!({ "attempt": attempt, "startedAt": at, "phase": "queued", "branch": format!("ax/request-{id}") });
    if let Some(instructions) = instructions.filter(|i| !i.is_empty()) {
        work["instructions"] = json!(instructions);
    }
    if let Some(checked) = checked.filter(|c| !c.is_empty()) {
        work["checkedAssumptions"] = Value::Array(checked);
    }
    work
}
