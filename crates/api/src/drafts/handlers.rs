//! The five saved-draft operations (`request:create`), in the mock API's
//! order: session, body, permission, then the operation's rules.

use std::collections::HashSet;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Map, Value};
use sqlx::postgres::PgRow;
use sqlx::{PgExecutor, Row};
use uuid::Uuid;

use super::StoredDraft;
use crate::attachments;
use crate::auth::{self, CurrentUser, token};
use crate::body::Body;
use crate::error::ApiError;
use crate::lang::Lang;
use crate::requests::store::iso;
use crate::roles::Permission;
use crate::state::AppState;

fn select() -> String {
    format!(
        "select id, step, content, version, {} as created_at, {} as updated_at from drafts",
        iso("created_at"),
        iso("updated_at")
    )
}

fn from_row(row: &PgRow) -> Result<StoredDraft, sqlx::Error> {
    let content: Value = row.try_get("content")?;
    Ok(StoredDraft {
        id: row.try_get("id")?,
        step: row.try_get("step")?,
        content: match content {
            Value::Object(map) => map,
            _ => Map::new(),
        },
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// The caller's draft with this id; someone else's is as good as missing.
async fn find(db: impl PgExecutor<'_>, owner: Uuid, id: &str, lock: bool) -> Result<Option<StoredDraft>, sqlx::Error> {
    let lock = if lock { " for update" } else { "" };
    let row = sqlx::query(&format!("{} where id = $1 and owner_id = $2{lock}", select()))
        .bind(id)
        .bind(owner)
        .fetch_optional(db)
        .await?;
    row.as_ref().map(from_row).transpose()
}

/// The draft as the contract's `SavedDraft`: its attachments are the caller's
/// existing uploads among its ids.
async fn full(state: &AppState, owner: Uuid, draft: &StoredDraft, lang: Lang) -> Result<Value, ApiError> {
    let db = state.db(lang)?;
    let (attachments, _) = attachments::owned(db, owner, &draft.attachment_ids()).await.map_err(ApiError::db(lang))?;
    Ok(draft.full(&attachments, &state.files))
}

/// `400 attachmentNotFound` for ids in `content.attachmentIds` that are not
/// the caller's uploads, one issue per index.
async fn check_attachments(
    db: impl PgExecutor<'_>,
    owner: Uuid,
    content: &Map<String, Value>,
    lang: Lang,
) -> Result<(), ApiError> {
    let ids: Vec<String> = content
        .get("attachmentIds")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    let (_, missing) = attachments::owned(db, owner, &ids).await.map_err(ApiError::db(lang))?;
    if missing.is_empty() {
        Ok(())
    } else {
        let paths = missing.iter().map(|i| format!("content.attachmentIds.{i}")).collect();
        Err(ApiError::attachment_not_found(lang, paths))
    }
}

/// `GET /drafts`: the caller's drafts, most recently changed first (the later
/// saved first on a tie).
pub async fn list(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Vec<Value>>, ApiError> {
    let user = &current.user;
    auth::require(user, Permission::RequestCreate, lang)?;
    let db = state.db(lang)?;
    let rows = sqlx::query(&format!("{} where owner_id = $1 order by updated_at desc, seq desc", select()))
        .bind(user.id)
        .fetch_all(db)
        .await
        .map_err(ApiError::db(lang))?;
    let drafts = rows.iter().map(from_row).collect::<Result<Vec<_>, _>>().map_err(ApiError::db(lang))?;
    let ids: Vec<String> = drafts.iter().flat_map(StoredDraft::attachment_ids).collect();
    let existing: HashSet<String> = if ids.is_empty() {
        HashSet::new()
    } else {
        sqlx::query_scalar::<_, String>("select id from attachments where owner_id = $1 and id = any($2)")
            .bind(user.id)
            .bind(&ids)
            .fetch_all(db)
            .await
            .map_err(ApiError::db(lang))?
            .into_iter()
            .collect()
    };
    Ok(Json(drafts.iter().map(|d| d.summary(super::count_existing(&d.attachment_ids(), &existing))).collect()))
}

/// `GET /drafts/{id}`: one of the caller's drafts; `404` for any other id.
pub async fn get(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let user = &current.user;
    auth::require(user, Permission::RequestCreate, lang)?;
    let db = state.db(lang)?;
    let draft = find(db, user.id, &id, false)
        .await
        .map_err(ApiError::db(lang))?
        .ok_or_else(|| ApiError::draft_not_found(lang))?;
    Ok(Json(full(&state, user.id, &draft, lang).await?))
}

/// `POST /drafts`: saves a new draft at version 1.
///
/// Repeated with a `clientId` that already made one of the caller's drafts,
/// it answers with that draft as it is now, before any other rule. Otherwise
/// `409 limit_reached` when the caller holds `AX_DRAFTS_PER_ACCOUNT` drafts,
/// then `400` for attachments that are not the caller's. The caller's account
/// row is locked meanwhile, so two saves cannot both take the last place.
pub async fn create(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let step = super::step(&mut body);
    let content = super::content(&mut body);
    let client_id = super::client_id(&mut body);
    body.finish()?;
    let (Some(step), Some(content)) = (step, content) else { return Err(ApiError::internal(lang)) };
    let user = &current.user;
    auth::require(user, Permission::RequestCreate, lang)?;

    let db = state.db(lang)?;
    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    sqlx::query("select 1 from users where id = $1 for update")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::db(lang))?;
    if let Some(client_id) = &client_id {
        let row = sqlx::query(&format!("{} where owner_id = $1 and client_id = $2", select()))
            .bind(user.id)
            .bind(client_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::db(lang))?;
        if let Some(row) = row {
            let draft = from_row(&row).map_err(ApiError::db(lang))?;
            tx.commit().await.map_err(ApiError::db(lang))?;
            return Ok((StatusCode::CREATED, Json(full(&state, user.id, &draft, lang).await?)));
        }
    }
    let held: i64 = sqlx::query_scalar("select count(*) from drafts where owner_id = $1")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::db(lang))?;
    let limit = state.drafts.per_account;
    if held >= i64::from(limit) {
        return Err(ApiError::draft_limit(lang, limit));
    }
    check_attachments(&mut *tx, user.id, &content, lang).await?;

    let row = sqlx::query(&format!(
        "insert into drafts (id, owner_id, client_id, step, content) values ($1, $2, $3, $4, $5)
         returning id, step, content, version, {} as created_at, {} as updated_at",
        iso("created_at"),
        iso("updated_at")
    ))
    .bind(token::generate_with(super::ID_PREFIX))
    .bind(user.id)
    .bind(&client_id)
    .bind(i32::try_from(step).map_err(|_| ApiError::internal(lang))?)
    .bind(Value::Object(content))
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::db(lang))?;
    let draft = from_row(&row).map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok((StatusCode::CREATED, Json(full(&state, user.id, &draft, lang).await?)))
}

/// `PUT /drafts/{id}`: replaces the step and content and moves to the next
/// version. `404` for a draft that is not the caller's, `409 conflict` when
/// `baseVersion` is not the stored version, then `400` for attachments that
/// are not the caller's. The row is locked from the check to the write.
pub async fn update(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let step = super::step(&mut body);
    let content = super::content(&mut body);
    let base_version = super::base_version(&mut body);
    body.finish()?;
    let (Some(step), Some(content), Some(base_version)) = (step, content, base_version) else {
        return Err(ApiError::internal(lang));
    };
    let user = &current.user;
    auth::require(user, Permission::RequestCreate, lang)?;

    let db = state.db(lang)?;
    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    let stored = find(&mut *tx, user.id, &id, true)
        .await
        .map_err(ApiError::db(lang))?
        .ok_or_else(|| ApiError::draft_not_found(lang))?;
    if stored.version != base_version {
        return Err(ApiError::draft_conflict(lang));
    }
    check_attachments(&mut *tx, user.id, &content, lang).await?;
    let row = sqlx::query(&format!(
        "update drafts set step = $2, content = $3, version = version + 1, updated_at = now()
          where id = $1
          returning id, step, content, version, {} as created_at, {} as updated_at",
        iso("created_at"),
        iso("updated_at")
    ))
    .bind(&stored.id)
    .bind(i32::try_from(step).map_err(|_| ApiError::internal(lang))?)
    .bind(Value::Object(content))
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::db(lang))?;
    let draft = from_row(&row).map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(full(&state, user.id, &draft, lang).await?))
}

/// `DELETE /drafts/{id}`: `204`, or `404` when it is gone or not the caller's
/// (a client may take that as already deleted). Its attachments stay.
pub async fn delete(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = &current.user;
    auth::require(user, Permission::RequestCreate, lang)?;
    let db = state.db(lang)?;
    let deleted = sqlx::query("delete from drafts where id = $1 and owner_id = $2")
        .bind(&id)
        .bind(user.id)
        .execute(db)
        .await
        .map_err(ApiError::db(lang))?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::draft_not_found(lang));
    }
    Ok(StatusCode::NO_CONTENT)
}
