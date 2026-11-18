//! `listTeams`, `createTeam` and `updateTeam`, with the frontend mock API's rules.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use super::{NAME_MAX, Team};
use crate::auth::{self, CurrentUser};
use crate::body::{Body, Text, trimmed};
use crate::error::ApiError;
use crate::lang::Lang;
use crate::roles::Permission;
use crate::state::AppState;

const NAME: Text = ("팀 이름은 1–40자로 입력해 주세요.", "Enter a team name of 1 to 40 characters.");

/// `GET /teams`: every team by name, ignoring case. Anyone signed in may read
/// it, because team names show with people and in AI usage.
pub async fn list(State(state): State<AppState>, lang: Lang, _: CurrentUser) -> Result<Json<Vec<Team>>, ApiError> {
    let db = state.db(lang)?;
    let teams = sqlx::query_as("select id, name from teams order by lower(name), name, id")
        .fetch_all(db)
        .await
        .map_err(ApiError::db(lang))?;
    Ok(Json(teams))
}

/// `POST /teams` (`user:manage`). `400` (`teamNameTaken`) when the name exists in any case.
pub async fn create(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Team>), ApiError> {
    let name = team_name(&bytes, lang)?;
    auth::require(&current.user, Permission::UserManage, lang)?;
    let db = state.db(lang)?;
    let team: Option<Team> = sqlx::query_as(
        "insert into teams (name) values ($1) on conflict ((lower(name))) do nothing returning id, name",
    )
    .bind(&name)
    .fetch_optional(db)
    .await
    .map_err(ApiError::db(lang))?;
    let team = team.ok_or_else(|| ApiError::team_name_taken(lang))?;
    Ok((StatusCode::CREATED, Json(team)))
}

/// `PATCH /teams/{id}` (`user:manage`): renames a team. `404` (`teamNotFound`);
/// `400` (`teamNameTaken`) when another team has the name in any case.
pub async fn update(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Team>, ApiError> {
    let name = team_name(&bytes, lang)?;
    auth::require(&current.user, Permission::UserManage, lang)?;
    let db = state.db(lang)?;
    let team = Team::find(db, &id).await.map_err(ApiError::db(lang))?.ok_or_else(|| ApiError::team_not_found(lang))?;
    let renamed = sqlx::query_as("update teams set name = $2, updated_at = now() where id = $1 returning id, name")
        .bind(team.id)
        .bind(&name)
        .fetch_one(db)
        .await;
    match renamed {
        Ok(team) => Ok(Json(team)),
        Err(error) if error.as_database_error().is_some_and(|e| e.is_unique_violation()) => {
            Err(ApiError::team_name_taken(lang))
        }
        Err(error) => Err(ApiError::database(lang, &error)),
    }
}

/// The contract's `TeamInput`: a name, trimmed, 1 to 40 characters.
fn team_name(bytes: &[u8], lang: Lang) -> Result<String, ApiError> {
    let mut body = Body::parse(bytes, lang)?;
    let name = body.required("name", NAME, |v| trimmed(v, Some(NAME_MAX)));
    body.finish()?;
    Ok(name.unwrap_or_else(|| unreachable!("finish() fails when a required field is missing")))
}
