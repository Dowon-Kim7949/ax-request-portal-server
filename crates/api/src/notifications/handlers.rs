//! `listNotifications`, `markNotificationRead`, `markAllNotificationsRead`,
//! `getNotificationSettings` and `updateNotificationSettings`. Each person
//! reads and changes only their own; no permission beyond signing in.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use super::render;
use crate::auth::CurrentUser;
use crate::body::{Body, Text};
use crate::error::{ApiError, ErrorCode};
use crate::lang::Lang;
use crate::requests::store::iso;
use crate::state::AppState;

fn columns() -> String {
    format!("id, request_id, key, params, {} as created_at, {} as read_at", iso("created_at"), iso("read_at"))
}

fn rendered(lang: Lang, row: &PgRow) -> Result<Value, sqlx::Error> {
    let params: Value = row.try_get("params")?;
    let created_at: String = row.try_get("created_at")?;
    let read_at: Option<String> = row.try_get("read_at")?;
    Ok(render(
        lang,
        row.try_get("id")?,
        row.try_get("request_id")?,
        &row.try_get::<String, _>("key")?,
        params.as_object().unwrap_or(&Map::new()),
        &created_at,
        read_at.as_deref(),
    ))
}

fn not_found(lang: Lang) -> ApiError {
    ApiError::new(ErrorCode::NotFound, lang.pick("알림을 찾을 수 없습니다.", "Notification not found."))
}

/// `GET /notifications`: my latest 100, newest first, in my language.
pub async fn list(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Vec<Value>>, ApiError> {
    let db = state.db(lang)?;
    let rows =
        sqlx::query(&format!("select {} from notifications where user_id = $1 order by seq desc limit 100", columns()))
            .bind(current.user.id)
            .fetch_all(db)
            .await
            .map_err(ApiError::db(lang))?;
    let list = rows.iter().map(|row| rendered(lang, row)).collect::<Result<_, _>>().map_err(ApiError::db(lang))?;
    Ok(Json(list))
}

/// `POST /notifications/{id}/read`: marks one of mine read (keeping the first
/// time it was read). Someone else's or a missing one is `404`.
pub async fn mark_read(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = Uuid::parse_str(&id).map_err(|_| not_found(lang))?;
    let db = state.db(lang)?;
    let row = sqlx::query(&format!(
        "update notifications set read_at = coalesce(read_at, now())
          where id = $1 and user_id = $2
          returning {}",
        columns()
    ))
    .bind(id)
    .bind(current.user.id)
    .fetch_optional(db)
    .await
    .map_err(ApiError::db(lang))?
    .ok_or_else(|| not_found(lang))?;
    Ok(Json(rendered(lang, &row).map_err(ApiError::db(lang))?))
}

/// `POST /notifications/read-all`: marks all of mine read. `204`.
pub async fn mark_all_read(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<StatusCode, ApiError> {
    let db = state.db(lang)?;
    sqlx::query("update notifications set read_at = coalesce(read_at, now()) where user_id = $1 and read_at is null")
        .bind(current.user.id)
        .execute(db)
        .await
        .map_err(ApiError::db(lang))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /me/notification-settings`: mine, or the defaults if never saved.
pub async fn get_settings(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let db = state.db(lang)?;
    let row: Option<(bool, bool, bool, bool)> = sqlx::query_as(
        "select in_app, email, messenger, only_action_required from notification_settings where user_id = $1",
    )
    .bind(current.user.id)
    .fetch_optional(db)
    .await
    .map_err(ApiError::db(lang))?;
    let (in_app, email, messenger, only) = row.unwrap_or((true, true, false, false));
    Ok(Json(json!({ "inApp": in_app, "email": email, "messenger": messenger, "onlyActionRequired": only })))
}

/// `PUT /me/notification-settings`: saves and returns them. The server sends
/// in-app notifications only; `email` and `messenger` are kept for a backend
/// that sends those too.
pub async fn update_settings(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    const FLAG: Text = ("true 또는 false여야 합니다.", "Must be true or false.");
    let mut body = Body::parse(&bytes, lang)?;
    let in_app = body.required("inApp", FLAG, Value::as_bool);
    let email = body.required("email", FLAG, Value::as_bool);
    let messenger = body.required("messenger", FLAG, Value::as_bool);
    let only = body.required("onlyActionRequired", FLAG, Value::as_bool);
    body.finish()?;
    let (Some(in_app), Some(email), Some(messenger), Some(only)) = (in_app, email, messenger, only) else {
        unreachable!("finish() fails when a required field is missing")
    };
    let db = state.db(lang)?;
    sqlx::query(
        "insert into notification_settings (user_id, in_app, email, messenger, only_action_required)
         values ($1, $2, $3, $4, $5)
         on conflict (user_id) do update
            set in_app = $2, email = $3, messenger = $4, only_action_required = $5, updated_at = now()",
    )
    .bind(current.user.id)
    .bind(in_app)
    .bind(email)
    .bind(messenger)
    .bind(only)
    .execute(db)
    .await
    .map_err(ApiError::db(lang))?;
    Ok(Json(json!({ "inApp": in_app, "email": email, "messenger": messenger, "onlyActionRequired": only })))
}
