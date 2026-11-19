//! `createRequest`, `getRequest`, `listRequests` and `precheckRequest`, with
//! the frontend mock API's rules (`src/mocks/engine.ts`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use sqlx::Row;
use uuid::Uuid;

use super::draft::Draft;
use super::lifecycle::{Event, Status};
use super::precheck;
use super::store::{self, Actor, StoredRequest, iso};
use crate::ai;
use crate::attachments::{self, Attachment, Files};
use crate::auth::{self, CurrentUser};
use crate::error::{ApiError, Issue};
use crate::lang::Lang;
use crate::projects::Project;
use crate::roles::{Permission, Role};
use crate::state::AppState;
use crate::users::User;

/// The largest id JavaScript numbers hold exactly (the contract's `maximum`).
const MAX_ID: i64 = 9_007_199_254_740_991;

/// A path id: digits only, positive, within the contract's range.
pub fn parse_id(value: &str) -> Option<i64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<i64>().ok().filter(|id| (1..=MAX_ID).contains(id))
}

/// The request as the person may see it: attachments with fresh download
/// links, and `aiUsage` only with `report:read`. Until AI usage is recorded (a
/// later step) it is empty, with no limit.
pub fn present(user: &User, request: &StoredRequest, files: &Files) -> Value {
    let mut json = request.to_json();
    if let Some(attachments) = json.get_mut("attachments").and_then(Value::as_array_mut) {
        for attachment in attachments {
            *attachment = files.present(attachment);
        }
    }
    if user.role.can(Permission::ReportRead) {
        json["aiUsage"] = json!({ "spentKrw": 0, "limitKrw": 0, "entries": [] });
    }
    json
}

/// `POST /requests` (`request:create`): files the request. It starts as
/// `submitted` and moves to `reporting` at once (`startReport`), where the AI
/// writes the feasibility report.
///
/// `400` (`projectNotFound`) for an unknown or inactive project, `400`
/// (`unknownTarget`) for a target the project does not have, `400`
/// (`attachmentNotFound`, one issue per index) for attachments that are
/// missing or someone else's; the rest are kept once each, in order. The
/// request goes to the project's first owner. With `draftId`, the caller's
/// saved draft of that id is deleted in the same transaction (anyone else's,
/// or a missing one, is ignored).
pub async fn create(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let draft = Draft::parse(&bytes, lang, true)?;
    let user = &current.user;
    auth::require(user, Permission::RequestCreate, lang)?;
    let db = state.db(lang)?;
    let project = Project::find(db, &draft.project_id)
        .await
        .map_err(ApiError::db(lang))?
        .filter(|p| p.active)
        .ok_or_else(|| ApiError::project_not_found(lang, StatusCode::BAD_REQUEST))?;
    let targets: Vec<&str> = project.target_ids().collect();
    if let Some(unknown) = draft.target_ids.iter().find(|t| !targets.contains(&t.as_str())) {
        return Err(ApiError::unknown_target(lang, unknown));
    }
    let (attachments, missing) =
        attachments::owned(db, user.id, &draft.attachment_ids).await.map_err(ApiError::db(lang))?;
    if !missing.is_empty() {
        let paths = missing.iter().map(|i| format!("attachmentIds.{i}")).collect();
        return Err(ApiError::attachment_not_found(lang, paths));
    }

    let today = store::today(db, &state.time_zone.0).await.map_err(ApiError::db(lang))?;
    let details = details_of(&draft, &precheck::advise(&draft, &today, lang), &attachments);
    let assignee = project.owner_ids.first().copied();

    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    let rule = Event::StartReport.rule();
    let id: i64 = sqlx::query_scalar(
        "insert into requests (project_id, requester_id, assignee_id, status, type, title, desired_date, details)
         values ($1, $2, $3, $4, $5, $6, $7::date, $8)
         returning id",
    )
    .bind(project.id)
    .bind(user.id)
    .bind(assignee)
    .bind(rule.to.as_str())
    .bind(&draft.kind)
    .bind(&draft.title)
    .bind(&draft.desired_date)
    .bind(Value::Object(details))
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::db(lang))?;
    store::add_event(&mut tx, id, Actor::User(user.id), "created", None, Some(Status::Submitted), None)
        .await
        .map_err(ApiError::db(lang))?;
    store::add_event(
        &mut tx,
        id,
        Actor::System,
        Event::StartReport.as_str(),
        Some(Status::Submitted),
        Some(rule.to),
        None,
    )
    .await
    .map_err(ApiError::db(lang))?;
    if let Some(draft_id) = &draft.draft_id {
        sqlx::query("delete from drafts where id = $1 and owner_id = $2")
            .bind(draft_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::db(lang))?;
    }
    // The AI writes the feasibility report in the requester's language.
    ai::jobs::enqueue_for(&mut tx, id, rule.to, lang).await.map_err(ApiError::db(lang))?;
    let request = StoredRequest::find(&mut tx, id).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    let request = request.ok_or_else(|| ApiError::internal(lang))?;
    Ok((StatusCode::CREATED, Json(present(user, &request, &state.files))))
}

/// The JSONB part of a new request: everything but the columns, keys left out
/// rather than empty, as the mock API builds it.
fn details_of(draft: &Draft, advice: &[precheck::Advice], attachments: &[Attachment]) -> Map<String, Value> {
    let mut d = Map::new();
    d.insert("targetIds".into(), json!(draft.target_ids));
    if let Some(other) = draft.target_other.as_deref().filter(|o| !o.is_empty()) {
        d.insert("targetOther".into(), json!(other));
    }
    d.insert("currentBehavior".into(), json!(draft.current_behavior));
    d.insert("desiredBehavior".into(), json!(draft.desired_behavior));

    let checklist = (draft.kind == "checklist").then(|| {
        draft
            .checklist
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, item)| {
                let mut k =
                    json!({ "id": format!("k{}", i + 1), "criterionId": format!("c{}", i + 1), "ref": item.reference });
                if let Some(clause) = item.clause.as_deref().filter(|c| !c.is_empty()) {
                    k["clause"] = json!(clause);
                }
                k["finding"] = json!(item.finding);
                if let Some(expected) = item.expected.as_deref().filter(|e| !e.is_empty()) {
                    k["expected"] = json!(expected);
                }
                k
            })
            .collect::<Vec<_>>()
    });
    let criteria: Vec<Value> = match &checklist {
        Some(items) => items
            .iter()
            .map(|k| {
                let what = k.get("expected").unwrap_or(&k["finding"]).as_str().unwrap_or_default();
                let text = format!("[{}] {what}", k["ref"].as_str().unwrap_or_default());
                json!({ "id": k["criterionId"], "text": truncate_utf16(&text, 300) })
            })
            .collect(),
        None => draft
            .acceptance_criteria
            .iter()
            .enumerate()
            .map(|(i, text)| json!({ "id": format!("c{}", i + 1), "text": text }))
            .collect(),
    };
    d.insert("acceptanceCriteria".into(), Value::Array(criteria));
    if let Some(items) = checklist {
        d.insert("checklist".into(), Value::Array(items));
        if let Some(source) = draft.checklist_source.as_deref().filter(|s| !s.is_empty()) {
            d.insert("checklistSource".into(), json!(source));
        }
    }
    if let Some(out) = draft.out_of_scope.as_deref().filter(|o| !o.is_empty()) {
        d.insert("outOfScope".into(), json!(out));
    }
    d.insert("attachments".into(), attachments.iter().map(Attachment::to_json).collect());
    if let Some(reason) = draft.desired_date_reason.as_deref().filter(|r| !r.is_empty()) {
        d.insert("desiredDateReason".into(), json!(reason));
    }
    d.insert("precheck".into(), json!(advice));
    d.insert("regression".into(), json!([]));
    d.insert("verification".into(), json!([]));
    d
}

/// At most `max` UTF-16 units, cut between characters (JavaScript's `slice`).
fn truncate_utf16(text: &str, max: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= max
        })
        .collect()
}

/// `GET /requests/{id}`: anyone who may open it (the requester, someone with
/// `request:read:all`, or a responsible developer). Anyone else gets the same
/// `404` as for a request that does not exist.
pub async fn get(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = parse_id(&id).ok_or_else(|| ApiError::bad_path_param(lang, "id"))?;
    let db = state.db(lang)?;
    let mut conn = db.acquire().await.map_err(ApiError::db(lang))?;
    let request = StoredRequest::find(&mut conn, id)
        .await
        .map_err(ApiError::db(lang))?
        .ok_or_else(|| ApiError::request_not_found(lang, id))?;
    let owners = store::owners_of(&mut *conn, request.project_id).await.map_err(ApiError::db(lang))?;
    if !request.can_read(&current.user, &owners) {
        return Err(ApiError::request_not_found(lang, id));
    }
    Ok(Json(present(&current.user, &request, &state.files)))
}

/// `GET /requests?scope&status&projectId&q`: summaries, most recently changed
/// first. `mine` = filed by me; `assigned` (`request:review`) = requests I am
/// responsible for, every request for an admin; `all` (`request:read:all`).
/// `status` is a comma-separated list (unknown values match nothing); `q`
/// matches the title or, without its `#`, the number.
pub async fn list(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<Value>>, ApiError> {
    // The last of a repeated parameter wins, as in the mock API.
    let mut params: Map<String, Value> = Map::new();
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        params.insert(key.into_owned(), Value::from(value.into_owned()));
    }
    let scope = params.get("scope").and_then(Value::as_str).unwrap_or("mine");
    if !["mine", "assigned", "all"].contains(&scope) {
        let message = lang.pick("mine, assigned, all 중 하나여야 합니다.", "Must be mine, assigned or all.");
        return Err(ApiError::bad_query(lang, vec![Issue { path: "scope".into(), message: message.into() }]));
    }
    let user = &current.user;
    match scope {
        "all" => auth::require(user, Permission::RequestReadAll, lang)?,
        "assigned" => auth::require(user, Permission::RequestReview, lang)?,
        _ => {}
    }
    let text = |key: &str| params.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
    let statuses: Option<Vec<String>> = text("status").map(|s| s.split(',').map(str::to_owned).collect());
    let project = text("projectId");
    let q = text("q").map(|q| {
        let q = q.trim().to_lowercase();
        q.strip_prefix('#').map(str::to_owned).unwrap_or(q)
    });
    let q = q.filter(|q| !q.is_empty());
    let mine = (scope == "mine").then_some(user.id);
    let responsible = (scope == "assigned" && user.role != Role::Admin).then_some(user.id);

    let db = state.db(lang)?;
    let rows = sqlx::query(&format!(
        "select r.id, r.title, r.project_id, r.type, r.status, r.requester_id, r.assignee_id,
                {} as created_at, {} as updated_at, to_char(r.desired_date, 'YYYY-MM-DD') as desired_date,
                r.details->'report'->>'verdict' as verdict,
                r.details->'work'->>'phase' as work_phase,
                nullif(r.details->'release'->>'scheduledDate', '') as scheduled_date
           from requests r
          where ($1::uuid is null or r.requester_id = $1)
            and ($2::uuid is null or r.assignee_id = $2
                 or exists (select 1 from project_owners o where o.project_id = r.project_id and o.user_id = $2))
            and ($3::text[] is null or r.status = any($3))
            and ($4::text is null or r.project_id::text = $4)
            and ($5::text is null or strpos(lower(r.title), $5) > 0 or r.id::text = $5)
          order by r.updated_at desc, r.id",
        iso("r.created_at"),
        iso("r.updated_at")
    ))
    .bind(mine)
    .bind(responsible)
    .bind(statuses)
    .bind(project)
    .bind(q)
    .fetch_all(db)
    .await
    .map_err(ApiError::db(lang))?;

    let summaries = rows
        .iter()
        .map(|row| -> Result<Value, sqlx::Error> {
            let mut s = Map::new();
            s.insert("id".into(), json!(row.try_get::<i64, _>("id")?));
            s.insert("title".into(), json!(row.try_get::<String, _>("title")?));
            s.insert("projectId".into(), json!(row.try_get::<Uuid, _>("project_id")?));
            s.insert("type".into(), json!(row.try_get::<String, _>("type")?));
            s.insert("status".into(), json!(row.try_get::<String, _>("status")?));
            s.insert("requesterId".into(), json!(row.try_get::<Uuid, _>("requester_id")?));
            if let Some(assignee) = row.try_get::<Option<Uuid>, _>("assignee_id")? {
                s.insert("assigneeId".into(), json!(assignee));
            }
            s.insert("createdAt".into(), json!(row.try_get::<String, _>("created_at")?));
            s.insert("updatedAt".into(), json!(row.try_get::<String, _>("updated_at")?));
            for (key, column) in [
                ("desiredDate", "desired_date"),
                ("verdict", "verdict"),
                ("workPhase", "work_phase"),
                ("scheduledDate", "scheduled_date"),
            ] {
                if let Some(value) = row.try_get::<Option<String>, _>(column)? {
                    s.insert(key.into(), json!(value));
                }
            }
            Ok(Value::Object(s))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::db(lang))?;
    Ok(Json(summaries))
}

/// `POST /requests/precheck` (`request:create`): advice on a draft before it
/// is filed, in the request's language. Never blocks anything.
pub async fn precheck(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let draft = Draft::parse(&bytes, lang, false)?;
    auth::require(&current.user, Permission::RequestCreate, lang)?;
    let db = state.db(lang)?;
    let today = store::today(db, &state.time_zone.0).await.map_err(ApiError::db(lang))?;
    Ok(Json(json!({ "issues": precheck::advise(&draft, &today, lang) })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::utf16_len;

    #[test]
    fn path_ids() {
        assert_eq!(parse_id("1017"), Some(1017));
        for bad in ["", "0", "-1", "1.5", "1e3", "abc", "9007199254740992", " 1"] {
            assert_eq!(parse_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn truncates_like_javascript_slice() {
        assert_eq!(truncate_utf16("abcdef", 3), "abc");
        assert_eq!(truncate_utf16("가나다", 2), "가나");
        assert_eq!(truncate_utf16("a😀b", 2), "a", "a character is never cut in half");
        assert_eq!(utf16_len(&truncate_utf16(&"x".repeat(400), 300)), 300);
    }
}
