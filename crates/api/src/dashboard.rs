//! `dashboard` (the company-wide summary) and `releases` (the release
//! schedule), computed like the frontend mock's `dashboard` and `releases`.
//! Every day and month is on the business calendar (`AX_TIME_ZONE`):
//! PostgreSQL turns instants into its dates, the counting happens here.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::{RawQuery, State};
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::ai::{settings, usage};
use crate::auth::{self, CurrentUser};
use crate::calendar::{add_days, is_year_month, previous_month};
use crate::error::{ApiError, Issue};
use crate::lang::Lang;
use crate::requests::draft::is_iso_date;
use crate::requests::lifecycle::Status;
use crate::requests::store;
use crate::roles::Permission;
use crate::state::AppState;
use crate::users::User;

/// One request, as these summaries need it, its instants already turned into
/// business dates.
struct Item {
    id: i64,
    project_id: Uuid,
    requester_id: Uuid,
    assignee_id: Option<Uuid>,
    title: String,
    status: Status,
    /// The business date it was filed.
    filed: String,
    created_s: f64,
    /// The business date it went live, and when.
    released: Option<String>,
    released_s: Option<f64>,
    scheduled: Option<String>,
    desired: Option<String>,
}

async fn rows(db: &PgPool, time_zone: &str) -> Result<Vec<Item>, sqlx::Error> {
    let rows = sqlx::query(
        "select r.id, r.project_id, r.requester_id, r.assignee_id, r.title, r.status,
                to_char((r.created_at at time zone $1)::date, 'YYYY-MM-DD') as filed,
                extract(epoch from r.created_at)::float8 as created_s,
                to_char(((r.details->'release'->>'releasedAt')::timestamptz at time zone $1)::date, 'YYYY-MM-DD') as released,
                extract(epoch from (r.details->'release'->>'releasedAt')::timestamptz)::float8 as released_s,
                nullif(r.details->'release'->>'scheduledDate', '') as scheduled,
                to_char(r.desired_date, 'YYYY-MM-DD') as desired
           from requests r
          order by r.id",
    )
    .bind(time_zone)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|row| {
            let status: String = row.try_get("status")?;
            Ok(Item {
                id: row.try_get("id")?,
                project_id: row.try_get("project_id")?,
                requester_id: row.try_get("requester_id")?,
                assignee_id: row.try_get("assignee_id")?,
                title: row.try_get("title")?,
                status: Status::parse(&status).unwrap_or(Status::Submitted),
                filed: row.try_get("filed")?,
                created_s: row.try_get("created_s")?,
                released: row.try_get("released")?,
                released_s: row.try_get("released_s")?,
                scheduled: row.try_get("scheduled")?,
                desired: row.try_get("desired")?,
            })
        })
        .collect()
}

/// The last of each query parameter (as the mock reads them).
fn query_params(query: Option<String>) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).into_owned().collect()
}

fn bad_query(lang: Lang, path: &str) -> ApiError {
    let message = lang.pick("형식을 확인해 주세요.", "Check the format.");
    ApiError::bad_query(lang, vec![Issue { path: path.into(), message: message.into() }])
}

/// Late: not closed, and its release date or (before the release is planned)
/// its wanted-by date is before today.
fn is_delayed(r: &Item, today: &str) -> bool {
    if r.status.is_terminal() {
        return false;
    }
    if r.scheduled.as_deref().is_some_and(|d| d < today) {
        return true;
    }
    r.desired.as_deref().is_some_and(|d| d < today) && r.status != Status::ReleaseScheduled
}

/// `GET /dashboard/summary?month=YYYY-MM` (`dashboard:view`). `month` (this
/// month by default) picks what was filed, released and spent; open work,
/// delays, the week's releases and what needs attention are as of today.
pub async fn summary(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    let params = query_params(query);
    let month = params.get("month").cloned();
    if month.as_deref().is_some_and(|m| !is_year_month(m)) {
        return Err(bad_query(lang, "month"));
    }
    auth::require(&current.user, Permission::DashboardView, lang)?;
    let db = state.db(lang)?;
    let tz = &state.time_zone.0;
    let today = store::today(db, tz).await.map_err(ApiError::db(lang))?;
    let this_month = today[0..7].to_owned();
    let month = month.unwrap_or_else(|| this_month.clone());
    let prev = previous_month(&month).unwrap_or_default();
    // A month still running is compared with the same days of the one before.
    let last_day = if month == this_month { today[8..10].to_owned() } else { "31".to_owned() };
    let week_end = add_days(&today, 7).unwrap_or_else(|| today.clone());

    let all = rows(db, tz).await.map_err(ApiError::db(lang))?;
    let projects: Vec<Uuid> = sqlx::query_scalar("select id from projects order by created_at, id")
        .fetch_all(db)
        .await
        .map_err(ApiError::db(lang))?;

    let received = all.iter().filter(|r| r.filed.starts_with(&month)).count();
    let received_prev = all.iter().filter(|r| r.filed.starts_with(&prev) && r.filed[8..10] <= *last_day).count();
    let open: Vec<&Item> = all.iter().filter(|r| !r.status.is_terminal()).collect();
    let done_in_month: Vec<&Item> = all
        .iter()
        .filter(|r| r.status == Status::Done && r.released.as_deref().is_some_and(|d| d.starts_with(&month)))
        .collect();
    let leads: Vec<f64> =
        done_in_month.iter().filter_map(|r| r.released_s.map(|s| (s - r.created_s) / 86_400.0)).collect();
    let mut delayed: Vec<i64> = open.iter().filter(|r| is_delayed(r, &today)).map(|r| r.id).collect();
    delayed.sort_unstable();

    let by_project: Vec<Value> = projects
        .iter()
        .map(|p| {
            json!({
                "projectId": p,
                "done": done_in_month.iter().filter(|r| r.project_id == *p).count(),
                "inProgress": open.iter().filter(|r| r.project_id == *p).count(),
            })
        })
        .collect();

    // This week's planned releases, by day and project, in date order.
    let mut upcoming: Vec<(String, Uuid, Vec<i64>)> = Vec::new();
    for r in &all {
        let Some(date) = r.scheduled.as_deref() else { continue };
        if r.status != Status::ReleaseScheduled || date < today.as_str() || date > week_end.as_str() {
            continue;
        }
        match upcoming.iter_mut().find(|(d, p, _)| d == date && *p == r.project_id) {
            Some((_, _, ids)) => ids.push(r.id),
            None => upcoming.push((date.to_owned(), r.project_id, vec![r.id])),
        }
    }
    upcoming.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Map::new();
    out.insert("month".into(), json!(month));
    out.insert("received".into(), json!(received));
    out.insert("receivedPrevMonth".into(), json!(received_prev));
    out.insert("inProgress".into(), json!(open.len()));
    out.insert("delayed".into(), json!(delayed.len()));
    out.insert("done".into(), json!(done_in_month.len()));
    if !leads.is_empty() {
        #[allow(clippy::cast_precision_loss)]
        let average = leads.iter().sum::<f64>() / leads.len() as f64;
        out.insert("avgLeadTimeDays".into(), json!((average * 10.0).round() / 10.0));
    }
    // The month's AI spending against the monthly limit (0 = no limit).
    let mut conn = db.acquire().await.map_err(ApiError::db(lang))?;
    let limits = settings::budget(&mut conn).await.map_err(ApiError::db(lang))?;
    let spent = usage::in_month(&mut conn, tz, &month).await.map_err(ApiError::db(lang))?;
    out.insert("aiCostKrw".into(), json!(usage::month_spent_krw(&spent)));
    out.insert("aiBudgetKrw".into(), json!(limits.monthly_limit_krw));
    out.insert("aiAlertThresholdPct".into(), json!(limits.alert_threshold_pct));
    out.insert("byProject".into(), Value::Array(by_project));
    out.insert(
        "upcomingReleases".into(),
        Value::Array(
            upcoming
                .into_iter()
                .map(|(date, project, ids)| json!({ "date": date, "projectId": project, "requestIds": ids }))
                .collect(),
        ),
    );
    out.insert(
        "attention".into(),
        json!({
            "infoNeeded": open.iter().filter(|r| r.status == Status::InfoNeeded).count(),
            "delayedRequestIds": delayed,
        }),
    );
    Ok(Json(Value::Object(out)))
}

/// `GET /releases?from&to` (`schedule:view`): the company-wide schedule, by
/// day and project. A request sits on the day it went live, or else its
/// planned day; cancelled and rejected ones are left out. Each says whether
/// this person may open it (`canOpen`), by the same rule as `getRequest`.
pub async fn releases(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<Value>>, ApiError> {
    let params = query_params(query);
    for key in ["from", "to"] {
        if params.get(key).is_some_and(|d| !is_iso_date(d)) {
            return Err(bad_query(lang, key));
        }
    }
    let user = &current.user;
    auth::require(user, Permission::ScheduleView, lang)?;
    let db = state.db(lang)?;
    let all = rows(db, &state.time_zone.0).await.map_err(ApiError::db(lang))?;
    let owners: Vec<(Uuid, Uuid)> = sqlx::query_as("select project_id, user_id from project_owners")
        .fetch_all(db)
        .await
        .map_err(ApiError::db(lang))?;
    let can_open = |r: &Item, user: &User| {
        r.requester_id == user.id
            || user.role.can(Permission::RequestReadAll)
            || r.assignee_id == Some(user.id)
            || owners.iter().any(|(p, u)| *p == r.project_id && *u == user.id)
    };

    let (from, to) = (params.get("from"), params.get("to"));
    let mut groups: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
    for r in &all {
        if matches!(r.status, Status::Cancelled | Status::Rejected) {
            continue;
        }
        let Some(date) = r.released.clone().or_else(|| r.scheduled.clone()) else { continue };
        if from.is_some_and(|f| date < *f) || to.is_some_and(|t| date > *t) {
            continue;
        }
        groups.entry((date, r.project_id.to_string())).or_default().push(json!({
            "id": r.id,
            "title": r.title,
            "status": r.status,
            "canOpen": can_open(r, user),
        }));
    }
    Ok(Json(
        groups
            .into_iter()
            .map(|((date, project), requests)| json!({ "date": date, "projectId": project, "requests": requests }))
            .collect(),
    ))
}
