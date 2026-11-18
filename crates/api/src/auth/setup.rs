//! One-time setup links: `createSetupLink` and `completeSetup`.
//!
//! People added through the API have no password. An admin makes a link for
//! them; the frontend shows it as `/setup#token=…` (the token stays in the URL
//! fragment, so it never reaches a server log). Opening it, the person sets a
//! password and is signed in. A new link replaces an unused one, so making a
//! link is also how a forgotten password is reset.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::handlers::Session;
use super::{CurrentUser, password, sessions, token};
use crate::body::{Body, Text};
use crate::error::ApiError;
use crate::lang::Lang;
use crate::roles::Permission;
use crate::state::AppState;
use crate::users::User;

/// The contract's `SETUP_PASSWORD_MIN` and `SETUP_PASSWORD_MAX`, in UTF-16
/// units like zod. The CLI keeps the same minimum (`accounts::MIN_PASSWORD_CHARS`).
pub const PASSWORD_MIN: usize = 12;
pub const PASSWORD_MAX: usize = 128;

/// The contract's `SetupLink`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupLink {
    pub token: String,
    /// RFC 3339 in UTC with a `Z`, as the contract's `Timestamp` requires.
    pub expires_at: String,
}

/// `POST /users/{id}/setup-link` (`user:manage`): a new link for the person,
/// replacing an unused one.
///
/// `404` (`userNotFound`); `403` (`operatorProvisioned`) for an operator, whose
/// password the service sets (`ax-api user set-password`); `400`
/// (`setupLinkInactive`) for a deactivated person.
pub async fn create(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<SetupLink>), ApiError> {
    super::require(&current.user, Permission::UserManage, lang)?;
    let db = state.db(lang)?;
    let user = User::find(db, &id).await.map_err(ApiError::db(lang))?.ok_or_else(|| ApiError::user_not_found(lang))?;
    if !user.role.admin_may_assign() {
        return Err(ApiError::operator_provisioned(lang));
    }
    if !user.active {
        return Err(ApiError::setup_link_inactive(lang));
    }

    let token = token::generate_with(token::LINK_PREFIX);
    let expires_at: String = sqlx::query_scalar(
        r#"insert into setup_links (user_id, token_hash, expires_at)
           values ($1, $2, now() + make_interval(secs => $3))
           on conflict (user_id) do update
              set token_hash = excluded.token_hash, created_at = now(), expires_at = excluded.expires_at
           returning to_char(expires_at at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')"#,
    )
    .bind(user.id)
    .bind(token::digest(&token).as_slice())
    .bind(state.setup_links.ttl.as_secs_f64())
    .fetch_one(db)
    .await
    .map_err(ApiError::db(lang))?;
    Ok((StatusCode::CREATED, Json(SetupLink { token, expires_at })))
}

/// `POST /auth/setup` (public): sets the password of the link's person, ends
/// their other sessions, uses up the link and signs them in.
///
/// `400` (`checkInput`) for a body that is not the contract's `SetupBody`;
/// `401` (`setupLinkInvalid`) for an unknown, used or expired link, or a
/// deactivated account, all alike.
pub async fn complete(State(state): State<AppState>, lang: Lang, bytes: Bytes) -> Result<Json<Session>, ApiError> {
    const TOKEN: Text = ("링크가 올바르지 않습니다.", "The link is missing its token.");
    const PASSWORD: Text = ("비밀번호는 12–128자로 정해 주세요.", "Use a password of 12 to 128 characters.");
    let mut body = Body::parse(&bytes, lang)?;
    let link = body.required("token", TOKEN, |v| v.as_str().filter(|t| !t.is_empty()).map(str::to_owned));
    let new_password = body.required("password", PASSWORD, |v: &Value| {
        let text = v.as_str()?;
        (PASSWORD_MIN..=PASSWORD_MAX).contains(&text.encode_utf16().count()).then(|| text.to_owned())
    });
    body.finish()?;
    let (Some(link), Some(new_password)) = (link, new_password) else {
        unreachable!("finish() fails when a required field is missing")
    };

    let db = state.db(lang)?;
    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    // Using the link deletes it, so two tabs racing with the same link cannot both succeed.
    let row = sqlx::query(
        "delete from setup_links l
          using users u
          where l.token_hash = $1 and l.expires_at > now() and u.id = l.user_id and u.active
          returning u.id, u.name, u.email, u.role, u.team_id, u.active",
    )
    .bind(token::digest(&link).as_slice())
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::db(lang))?;
    let Some(row) = row else { return Err(ApiError::setup_link_invalid(lang)) };
    let user = User::from_row(&row).map_err(ApiError::db(lang))?;
    let id: Uuid = row.try_get("id").map_err(ApiError::db(lang))?;

    let hash = tokio::task::spawn_blocking(move || password::hash(&new_password))
        .await
        .map_err(|_| ApiError::internal(lang))?;
    sqlx::query("update users set password_hash = $2, updated_at = now() where id = $1")
        .bind(id)
        .bind(hash)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::db(lang))?;
    // A reset signs the person out everywhere else.
    sqlx::query("delete from sessions where user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;

    let token = sessions::start(db, &state.sessions, id).await.map_err(ApiError::db(lang))?;
    Ok(Json(Session { token, user }))
}
