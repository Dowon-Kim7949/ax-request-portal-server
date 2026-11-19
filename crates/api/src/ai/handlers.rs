//! The operator's AI operations: budget (`budget:manage`), settings, keys and
//! prices (`agent:manage`), usage (`usage:view`), in the mock API's order:
//! session, path and query, body, permission, then the operation's rules.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use super::settings::{self, PROVIDERS};
use super::usage;
use crate::auth::{self, CurrentUser};
use crate::body::Body;
use crate::calendar::is_year_month;
use crate::error::{ApiError, Issue};
use crate::lang::Lang;
use crate::requests::store;
use crate::roles::Permission;
use crate::state::AppState;

/// This month on the business calendar.
async fn this_month(state: &AppState, lang: Lang) -> Result<String, ApiError> {
    let today = store::today(state.db(lang)?, &state.time_zone.0).await.map_err(ApiError::db(lang))?;
    Ok(today[0..7].to_owned())
}

async fn budget_of(state: &AppState, month: &str, lang: Lang) -> Result<Value, ApiError> {
    let mut conn = state.db(lang)?.acquire().await.map_err(ApiError::db(lang))?;
    let limits = settings::budget(&mut conn).await.map_err(ApiError::db(lang))?;
    let entries = usage::in_month(&mut conn, &state.time_zone.0, month).await.map_err(ApiError::db(lang))?;
    Ok(usage::budget_json(month, &limits, &entries))
}

/// `GET /budget`: this month's spending against the limits.
pub async fn get_budget(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    auth::require(&current.user, Permission::BudgetManage, lang)?;
    let month = this_month(&state, lang).await?;
    Ok(Json(budget_of(&state, &month, lang).await?))
}

/// `PUT /budget`: replaces the limits; answers with this month's `Budget`.
pub async fn update_budget(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let limits = settings::budget_input(&mut body);
    body.finish()?;
    let limits = limits.ok_or_else(|| ApiError::internal(lang))?;
    auth::require(&current.user, Permission::BudgetManage, lang)?;
    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    settings::lock(&mut tx).await.map_err(ApiError::db(lang))?;
    settings::save(&mut tx, "budget", &limits).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    let month = this_month(&state, lang).await?;
    Ok(Json(budget_of(&state, &month, lang).await?))
}

/// `GET /ai/settings`: the model per task and project, and which providers
/// have keys (the last four characters of each, never the key).
pub async fn get_settings(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    auth::require(&current.user, Permission::AgentManage, lang)?;
    let mut conn = state.db(lang)?.acquire().await.map_err(ApiError::db(lang))?;
    let selections = settings::selections(&mut conn).await.map_err(ApiError::db(lang))?;
    Ok(Json(settings::settings_json(&selections, &state.ai)))
}

/// `PUT /ai/settings`: replaces the defaults and overrides.
///
/// For each override in turn: `400 projectNotFound` for an unknown project,
/// `400 checkInput` for a project and task named twice. Then every selection
/// in use, defaults first: `400 providerNotConnected`, then `400
/// modelNotPriced`. Checked and written under the settings lock.
pub async fn update_settings(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let input = settings::selections_input(&mut body);
    body.finish()?;
    let input = input.ok_or_else(|| ApiError::internal(lang))?;
    auth::require(&current.user, Permission::AgentManage, lang)?;

    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    settings::lock(&mut tx).await.map_err(ApiError::db(lang))?;
    let ids: Vec<&str> = input.project_overrides.iter().map(|o| o.project_id.as_str()).collect();
    let known: HashSet<String> = sqlx::query_scalar("select id::text from projects where id::text = any($1)")
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(ApiError::db(lang))?
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    for o in &input.project_overrides {
        if !known.contains(&o.project_id) {
            return Err(ApiError::project_not_found(lang, StatusCode::BAD_REQUEST));
        }
        if !seen.insert((o.project_id.as_str(), o.task.as_str())) {
            return Err(ApiError::check_input(lang, Vec::new()));
        }
    }
    let pricing = settings::pricing(&mut tx).await.map_err(ApiError::db(lang))?;
    for selection in input.in_use() {
        if !settings::connected(&state.ai, &selection.provider) {
            return Err(ApiError::provider_not_connected(lang, &selection.provider));
        }
        if pricing.price_of(selection).is_none() {
            return Err(ApiError::model_not_priced(lang, &selection.model));
        }
    }
    settings::save(&mut tx, "selections", &input).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(settings::settings_json(&input, &state.ai)))
}

fn provider_param(provider: &str, lang: Lang) -> Result<(), ApiError> {
    if PROVIDERS.contains(&provider) { Ok(()) } else { Err(ApiError::bad_path_param(lang, "provider")) }
}

/// `PUT /ai/providers/{provider}/key`: keys come only from the server's
/// configuration, so a valid request from an operator is `403`
/// (`providerKeysFromConfig`), like an operator account the API cannot make.
pub async fn set_provider_key(
    lang: Lang,
    current: CurrentUser,
    Path(provider): Path<String>,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    provider_param(&provider, lang)?;
    let mut body = Body::parse(&bytes, lang)?;
    settings::api_key_input(&mut body);
    body.finish()?;
    auth::require(&current.user, Permission::AgentManage, lang)?;
    Err(ApiError::provider_keys_from_config(lang))
}

/// `DELETE /ai/providers/{provider}/key`: `403` as [`set_provider_key`].
pub async fn remove_provider_key(
    lang: Lang,
    current: CurrentUser,
    Path(provider): Path<String>,
) -> Result<StatusCode, ApiError> {
    provider_param(&provider, lang)?;
    auth::require(&current.user, Permission::AgentManage, lang)?;
    Err(ApiError::provider_keys_from_config(lang))
}

/// `GET /ai/pricing` (`agent:manage` or `usage:view`).
pub async fn get_pricing(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
) -> Result<Json<Value>, ApiError> {
    let user = &current.user;
    if !user.role.can(Permission::AgentManage) && !user.role.can(Permission::UsageView) {
        return Err(ApiError::forbidden(lang));
    }
    let mut conn = state.db(lang)?.acquire().await.map_err(ApiError::db(lang))?;
    let pricing = settings::pricing(&mut conn).await.map_err(ApiError::db(lang))?;
    Ok(Json(serde_json::to_value(pricing).map_err(|_| ApiError::internal(lang))?))
}

/// `PUT /ai/pricing`: replaces the table. `409 modelInUse` when a selection
/// in use would lose its price. Usage booked before keeps its price.
pub async fn update_pricing(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<Json<Value>, ApiError> {
    let mut body = Body::parse(&bytes, lang)?;
    let pricing = settings::pricing_input(&mut body);
    body.finish()?;
    let pricing = pricing.ok_or_else(|| ApiError::internal(lang))?;
    auth::require(&current.user, Permission::AgentManage, lang)?;

    let mut tx = state.db(lang)?.begin().await.map_err(ApiError::db(lang))?;
    settings::lock(&mut tx).await.map_err(ApiError::db(lang))?;
    let selections = settings::selections(&mut tx).await.map_err(ApiError::db(lang))?;
    if let Some(dropped) = selections.in_use().into_iter().find(|s| pricing.price_of(s).is_none()) {
        return Err(ApiError::model_in_use(lang, &dropped.model));
    }
    settings::save(&mut tx, "pricing", &pricing).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(serde_json::to_value(pricing).map_err(|_| ApiError::internal(lang))?))
}

/// `GET /ai/usage?month=YYYY-MM` (`usage:view`): the month's usage and cost
/// (this month by default) by provider, model, task, day, request, team and
/// person.
pub async fn report(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    // The last of a repeated parameter wins, as in the mock API.
    let month = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .filter(|(key, _)| key == "month")
        .map(|(_, value)| value.into_owned())
        .last();
    if month.as_deref().is_some_and(|m| !is_year_month(m)) {
        let message = lang.pick("YYYY-MM 형식의 달이어야 합니다.", "Must be a month as YYYY-MM.").to_owned();
        return Err(ApiError::bad_query(lang, vec![Issue { path: "month".into(), message }]));
    }
    auth::require(&current.user, Permission::UsageView, lang)?;
    let month = match month {
        Some(month) => month,
        None => this_month(&state, lang).await?,
    };
    let mut conn = state.db(lang)?.acquire().await.map_err(ApiError::db(lang))?;
    let limits = settings::budget(&mut conn).await.map_err(ApiError::db(lang))?;
    let pricing = settings::pricing(&mut conn).await.map_err(ApiError::db(lang))?;
    let entries = usage::in_month(&mut conn, &state.time_zone.0, &month).await.map_err(ApiError::db(lang))?;
    let ids: Vec<i64> = entries.iter().filter_map(|e| e.request_id).collect();
    let projects: HashMap<i64, Uuid> =
        sqlx::query_as::<_, (i64, Uuid)>("select id, project_id from requests where id = any($1)")
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await
            .map_err(ApiError::db(lang))?
            .into_iter()
            .collect();
    Ok(Json(usage::report_json(&month, &limits, &pricing, &entries, &projects)))
}
