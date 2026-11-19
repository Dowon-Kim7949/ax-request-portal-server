//! What people record on a request besides moving it: the requester's
//! checklist and preview feedback, the developer's regression checks and
//! release date, and the regression queue (the mock engine's
//! `saveVerification`, `previewFeedback`, `updateRegression`, `updateRelease`
//! and `regressionQueue`). Each checks the request (`404`), the person's role
//! (`403`), then the status (`409`), as the mock API does.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use serde_json::{Value, json};
use sqlx::Row;

use super::commands;
use super::errors;
use super::handlers::{parse_id, present};
use super::lifecycle::Status;
use super::store::{self, Actor, field};
use super::transitions::{is_responsible_developer, load, normalize_checks};
use crate::auth::{self, CurrentUser};
use crate::error::ApiError;
use crate::lang::Lang;
use crate::notifications;
use crate::roles::{Permission, Role};
use crate::state::AppState;

/// `PUT /requests/{id}/verification`: the requester saves their checklist
/// while the request is in verification. Nothing else changes.
pub async fn save_verification(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let id = parse_id(&id).ok_or_else(|| ApiError::bad_path_param(lang, "id"))?;
    let checks = commands::verification_draft(&bytes, lang)?;
    let user = &current.user;
    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    let (mut request, _) = load(&mut tx, id, user, lang).await?;
    if request.requester_id != user.id {
        return Err(errors::requester_only_verify(lang));
    }
    if request.status != Status::Verification {
        return Err(errors::not_in_verification(lang));
    }
    let verification = normalize_checks(&request, &checks);
    request.details.insert("verification".into(), verification);
    request.save(&mut tx).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(present(&state, user, &request, lang).await?))
}

/// `PUT /requests/{id}/preview-feedback`: the requester's reaction to the AI's
/// preview during code review. Advisory: the status stays; the developers are
/// told (asking for action when a change is wanted).
pub async fn preview_feedback(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let id = parse_id(&id).ok_or_else(|| ApiError::bad_path_param(lang, "id"))?;
    let (needs_change, note) = commands::preview_feedback(&bytes, lang)?;
    let user = &current.user;
    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    let (mut request, owners) = load(&mut tx, id, user, lang).await?;
    if request.requester_id != user.id {
        return Err(errors::requester_only_preview(lang));
    }
    if request.status != Status::CodeReview || field(&request.details, "work").get("preview").is_none() {
        return Err(errors::no_preview_now(lang));
    }
    let at = store::now_iso(&mut *tx).await.map_err(ApiError::db(lang))?;
    let verdict = if needs_change { "needs_change" } else { "looks_good" };
    let note = note.filter(|n| !n.is_empty());
    let mut feedback = json!({ "at": at, "by": user.id, "verdict": verdict });
    if let Some(note) = &note {
        feedback["note"] = json!(note);
    }
    if let Some(work) = request.details.get_mut("work").and_then(Value::as_object_mut) {
        work.insert("previewFeedback".into(), feedback);
    }
    let event = if needs_change { "preview_needs_change" } else { "preview_looks_good" };
    store::add_event(&mut tx, id, Actor::User(user.id), event, None, None, note.as_deref())
        .await
        .map_err(ApiError::db(lang))?;
    request.save(&mut tx).await.map_err(ApiError::db(lang))?;
    let key = if needs_change { "previewChange" } else { "previewGood" };
    let params = json!({ "title": request.title, "note": note.clone().unwrap_or_default() });
    notifications::notify(&mut tx, &request.responsible(&owners), key, params, Some(id), needs_change, Some(user.id))
        .await
        .map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(present(&state, user, &request, lang).await?))
}

/// `PUT /requests/{id}/regression`: a responsible developer records regression
/// results. Unknown check ids are ignored; a closed request is `409`.
pub async fn update_regression(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let id = parse_id(&id).ok_or_else(|| ApiError::bad_path_param(lang, "id"))?;
    let results = commands::regression_update(&bytes, lang)?;
    let user = &current.user;
    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    let (mut request, owners) = load(&mut tx, id, user, lang).await?;
    if !is_responsible_developer(user, &request, &owners) {
        return Err(errors::developer_only_regression(lang));
    }
    if request.status.is_terminal() {
        return Err(errors::request_closed(lang));
    }
    if let Some(checks) = request.details.get_mut("regression").and_then(Value::as_array_mut) {
        for check in checks {
            // The last result sent for a check counts.
            if let Some((_, result)) = results.iter().rev().find(|(cid, _)| check["id"].as_str() == Some(cid)) {
                check["result"] = json!(result);
            }
        }
    }
    request.save(&mut tx).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(present(&state, user, &request, lang).await?))
}

/// `PUT /requests/{id}/release`: a responsible developer sets the planned
/// release date and note, from code review until the release. A new date must
/// not be before today on the business calendar; the date already set may be
/// sent again (to change only the note). The requester is told of a date.
pub async fn update_release(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let id = parse_id(&id).ok_or_else(|| ApiError::bad_path_param(lang, "id"))?;
    let (date, note) = commands::release_input(&bytes, lang)?;
    let user = &current.user;
    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    let (mut request, owners) = load(&mut tx, id, user, lang).await?;
    if !is_responsible_developer(user, &request, &owners) {
        return Err(errors::developer_only_schedule(lang));
    }
    if !matches!(
        request.status,
        Status::CodeReview | Status::Verification | Status::ChangesRequested | Status::ReleaseScheduled
    ) {
        return Err(errors::cannot_schedule_now(lang));
    }
    let today = store::today(&mut *tx, &state.time_zone.0).await.map_err(ApiError::db(lang))?;
    let current_date = field(&request.details, "release")["scheduledDate"].as_str().map(str::to_owned);
    if let Some(date) = &date
        && Some(date) != current_date.as_ref()
        && date.as_str() < today.as_str()
    {
        return Err(errors::release_date_in_past(lang, &today));
    }
    let mut release = request.details.get("release").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    if let Some(date) = &date {
        release["scheduledDate"] = json!(date);
    }
    if let Some(note) = &note {
        release["note"] = json!(note);
    }
    request.details.insert("release".into(), release);
    store::add_event(&mut tx, id, Actor::User(user.id), "release_updated", None, None, date.as_deref())
        .await
        .map_err(ApiError::db(lang))?;
    request.save(&mut tx).await.map_err(ApiError::db(lang))?;
    if let Some(date) = &date {
        notifications::notify(
            &mut tx,
            &[request.requester_id],
            "releaseDateSet",
            json!({ "date": date }),
            Some(id),
            false,
            Some(user.id),
        )
        .await
        .map_err(ApiError::db(lang))?;
    }
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(present(&state, user, &request, lang).await?))
}

/// `GET /regression-checks` (`regression:view`): open requests with regression
/// checks that the person is responsible for (every one for an admin).
pub async fn regression_queue(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Vec<Value>>, ApiError> {
    let user = &current.user;
    auth::require(user, Permission::RegressionView, lang)?;
    let db = state.db(lang)?;
    let rows = sqlx::query(
        "select r.id, r.title, r.project_id, r.status, r.details->'regression' as checks
           from requests r
          where jsonb_array_length(coalesce(r.details->'regression', '[]')) > 0
            and r.status not in ('done', 'rejected', 'cancelled')
            and ($1 or r.assignee_id = $2
                 or exists (select 1 from project_owners o where o.project_id = r.project_id and o.user_id = $2))
          order by r.id",
    )
    .bind(user.role == Role::Admin)
    .bind(user.id)
    .fetch_all(db)
    .await
    .map_err(ApiError::db(lang))?;
    let queue = rows
        .iter()
        .map(|row| -> Result<Value, sqlx::Error> {
            Ok(json!({
                "requestId": row.try_get::<i64, _>("id")?,
                "title": row.try_get::<String, _>("title")?,
                "projectId": row.try_get::<uuid::Uuid, _>("project_id")?,
                "status": row.try_get::<String, _>("status")?,
                "checks": row.try_get::<Value, _>("checks")?,
            }))
        })
        .collect::<Result<_, _>>()
        .map_err(ApiError::db(lang))?;
    Ok(Json(queue))
}
