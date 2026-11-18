//! `listUsers`, `createUser` and `updateUser`, with the frontend mock API's rules.
//!
//! Like the mock API, a request is checked in this order: session (`401`),
//! body (`400` with `issues`), permission (`403`), then the rules of the
//! operation.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use super::User;
use crate::auth::{self, CurrentUser, sessions};
use crate::body::{Body, Patch, Text, trimmed};
use crate::error::ApiError;
use crate::lang::Lang;
use crate::roles::{Permission, Role};
use crate::state::AppState;
use crate::teams::Team;
use crate::validate::is_email;

const NAME: Text = ("이름을 입력해 주세요.", "Enter a name.");
const EMAIL: Text = ("올바른 이메일 주소를 입력해 주세요.", "Enter a valid email address.");
const ROLE: Text = ("역할을 확인해 주세요.", "Choose one of the roles.");
const TEAM: Text = ("팀을 확인해 주세요.", "Choose a team.");
const ACTIVE: Text = ("true 또는 false여야 합니다.", "Must be true or false.");

const RETURNING: &str = "returning id, name, email, role, team_id, active";

fn role(value: &Value) -> Option<Role> {
    value.as_str().and_then(Role::parse)
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// `GET /users`: everyone, deactivated people included, oldest first. Anyone
/// signed in may read it, because names show on every request.
pub async fn list(State(state): State<AppState>, lang: Lang, _: CurrentUser) -> Result<Json<Vec<User>>, ApiError> {
    let db = state.db(lang)?;
    let rows = sqlx::query("select id, name, email, role, team_id, active from users order by created_at, id")
        .fetch_all(db)
        .await
        .map_err(ApiError::db(lang))?;
    let users = rows.iter().map(User::from_row).collect::<Result<Vec<_>, _>>().map_err(ApiError::db(lang))?;
    Ok(Json(users))
}

/// `POST /users` (`user:manage`): an active account with no password yet.
///
/// `403` (`operatorProvisioned`) for the operator role, which only the service
/// provisions; `400` (`emailTaken`) when the email exists in any case; `404`
/// (`teamNotFound`) for an unknown team.
pub async fn create(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<(StatusCode, Json<User>), ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let name = body.required("name", NAME, |v| trimmed(v, None));
    let email = body.required("email", EMAIL, |v| v.as_str().filter(|e| is_email(e)).map(str::to_owned));
    let role = body.required("role", ROLE, role);
    let team = body.optional("teamId", TEAM, text);
    body.finish()?;
    let (Some(name), Some(email), Some(role)) = (name, email, role) else {
        unreachable!("finish() fails when a required field is missing")
    };

    auth::require(&current.user, Permission::UserManage, lang)?;
    if !role.admin_may_assign() {
        return Err(ApiError::operator_provisioned(lang));
    }
    let db = state.db(lang)?;
    let taken: bool = sqlx::query_scalar("select exists (select 1 from users where lower(email) = lower($1))")
        .bind(&email)
        .fetch_one(db)
        .await
        .map_err(ApiError::db(lang))?;
    if taken {
        return Err(ApiError::email_taken(lang));
    }
    let team_id = match team {
        None => None,
        Some(id) => Some(existing_team(db, &id, lang).await?),
    };

    let row = sqlx::query(&format!(
        "insert into users (email, name, role, team_id) values ($1, $2, $3, $4)
         on conflict ((lower(email))) do nothing {RETURNING}"
    ))
    .bind(&email)
    .bind(&name)
    .bind(role.as_str())
    .bind(team_id)
    .fetch_optional(db)
    .await
    .map_err(ApiError::db(lang))?;
    // Another request took the email in between.
    let row = row.ok_or_else(|| ApiError::email_taken(lang))?;
    Ok((StatusCode::CREATED, Json(User::from_row(&row).map_err(ApiError::db(lang))?)))
}

/// `PATCH /users/{id}` (`user:manage`): role, team (`null` takes the person out
/// of their team) and active flag.
///
/// `404` (`userNotFound`); `403` (`operatorProvisioned`) when the person is an
/// operator or would become one; `400` (`cannotChangeSelf`) when an admin
/// deactivates themselves or changes their own role; `404` (`teamNotFound`).
/// Deactivating someone ends their sessions and deletes their unused setup link.
pub async fn update(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<User>, ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let new_role = body.optional("role", ROLE, role);
    let team = body.patch("teamId", TEAM, text);
    let active = body.optional("active", ACTIVE, Value::as_bool);
    body.finish()?;

    auth::require(&current.user, Permission::UserManage, lang)?;
    let db = state.db(lang)?;
    let target =
        User::find(db, &id).await.map_err(ApiError::db(lang))?.ok_or_else(|| ApiError::user_not_found(lang))?;
    if !target.role.admin_may_assign() || new_role.is_some_and(|role| !role.admin_may_assign()) {
        return Err(ApiError::operator_provisioned(lang));
    }
    let me = &current.user;
    if target.id == me.id && (active == Some(false) || new_role.is_some_and(|role| role != me.role)) {
        return Err(ApiError::cannot_change_self(lang));
    }
    let (set_team, team_id) = match team {
        Patch::Keep => (false, None),
        // The mock API leaves the team alone for an empty id.
        Patch::Set(id) if id.is_empty() => (false, None),
        Patch::Set(id) => (true, Some(existing_team(db, &id, lang).await?)),
        Patch::Clear => (true, None),
    };

    let row = sqlx::query(&format!(
        "update users
            set role = coalesce($2, role),
                active = coalesce($3, active),
                team_id = case when $4 then $5 else team_id end,
                updated_at = now()
          where id = $1
          {RETURNING}"
    ))
    .bind(target.id)
    .bind(new_role.map(Role::as_str))
    .bind(active)
    .bind(set_team)
    .bind(team_id)
    .fetch_one(db)
    .await
    .map_err(ApiError::db(lang))?;
    let user = User::from_row(&row).map_err(ApiError::db(lang))?;
    if !user.active {
        // Out at once, and an unused setup link cannot bring them back in after a reactivation.
        sessions::end_all(db, user.id).await.map_err(ApiError::db(lang))?;
        sqlx::query("delete from setup_links where user_id = $1")
            .bind(user.id)
            .execute(db)
            .await
            .map_err(ApiError::db(lang))?;
    }
    Ok(Json(user))
}

async fn existing_team(db: &sqlx::PgPool, id: &str, lang: Lang) -> Result<Uuid, ApiError> {
    let team = Team::find(db, id).await.map_err(ApiError::db(lang))?;
    team.map(|team| team.id).ok_or_else(|| ApiError::team_not_found(lang))
}
