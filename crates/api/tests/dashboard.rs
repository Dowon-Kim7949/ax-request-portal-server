//! The dashboard and the release schedule against a real PostgreSQL, on the
//! business calendar (Asia/Seoul by default): requests are dated in 2020 so
//! the counts do not depend on today.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::roles::Role;
use common::*;

struct World {
    executive: String,
    requester: String,
    other: String,
    project: String,
}

/// A project and `n` requests filed by `requester`.
async fn world(pool: &PgPool, n: usize) -> World {
    let (_, admin) = signed_in(pool, "admin@example.com", Role::Admin).await;
    let (dev, _) = signed_in(pool, "dev@example.com", Role::Developer).await;
    let (_, executive) = signed_in(pool, "exec@example.com", Role::Executive).await;
    let (_, requester) = signed_in(pool, "req@example.com", Role::Requester).await;
    let (_, other) = signed_in(pool, "req2@example.com", Role::Requester).await;
    let project = json!({
        "name": "Portal", "description": "", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [dev.id.to_string()],
        "targets": [], "constraints": [], "active": true,
    });
    let (_, project) = send(pool, "POST", "/projects", Some(&admin), "en", Some(project)).await;
    for i in 0..n {
        let request = json!({
            "projectId": project["id"], "type": "non_code", "title": format!("Request {i}"), "targetIds": [],
            "currentBehavior": "", "desiredBehavior": "Something", "acceptanceCriteria": [{ "text": "When X, Y shows" }],
            "attachmentIds": [],
        });
        assert_eq!(send(pool, "POST", "/requests", Some(&requester), "en", Some(request)).await.0, StatusCode::CREATED);
    }
    World { executive, requester, other, project: project["id"].as_str().unwrap().to_owned() }
}

/// Files request `id` at `created` (UTC); `release` is merged into its release.
async fn date(pool: &PgPool, id: i64, status: &str, created: &str, release: Value) {
    sqlx::query(
        "update requests set status = $2, created_at = $3::timestamptz,
                details = jsonb_set(details, '{release}', coalesce(details->'release', '{}') || $4)
          where id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(created)
    .bind(release)
    .execute(pool)
    .await
    .unwrap();
}

async fn summary(pool: &PgPool, token: &str, query: &str) -> (StatusCode, Value) {
    send(pool, "GET", &format!("/dashboard/summary{query}"), Some(token), "en", None).await
}

/// Today on the business calendar plus `days`.
async fn in_days(pool: &PgPool, days: i32) -> String {
    sqlx::query_scalar("select to_char((now() at time zone 'Asia/Seoul')::date + $1, 'YYYY-MM-DD')")
        .bind(days)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn months_follow_the_business_calendar(pool: PgPool) {
    let w = world(&pool, 3).await;
    // 15:00 UTC is already the 1st in Seoul; 14:59 UTC is still the 31st.
    date(&pool, 1, "done", "2020-10-31T15:00:00Z", json!({ "releasedAt": "2020-11-02T21:14:24.000Z" })).await;
    date(&pool, 2, "done", "2020-10-31T14:59:00Z", json!({ "releasedAt": "2020-10-31T14:59:00.000Z" })).await;
    date(&pool, 3, "cancelled", "2020-11-20T00:00:00Z", json!({})).await;

    let (status, november) = summary(&pool, &w.executive, "?month=2020-11").await;
    assert_eq!(status, StatusCode::OK, "{november}");
    assert_eq!(november["month"], "2020-11");
    assert_eq!(
        (&november["received"], &november["receivedPrevMonth"]),
        (&json!(2), &json!(1)),
        "cancelled ones were received too"
    );
    assert_eq!(november["done"], 1);
    assert_eq!(november["avgLeadTimeDays"], 2.3, "2.256 days, one decimal");
    assert_eq!(november["byProject"], json!([{ "projectId": w.project, "done": 1, "inProgress": 0 }]));
    assert_eq!(
        (&november["aiCostKrw"], &november["aiBudgetKrw"], &november["aiAlertThresholdPct"]),
        (&json!(0), &json!(0), &json!(80))
    );

    let (_, october) = summary(&pool, &w.executive, "?month=2020-10").await;
    assert_eq!((&october["received"], &october["done"]), (&json!(1), &json!(1)));
    assert_eq!(october["avgLeadTimeDays"], 0.0);
    let (_, empty) = summary(&pool, &w.executive, "?month=2019-01").await;
    assert_eq!((&empty["received"], &empty["done"]), (&json!(0), &json!(0)));
    assert!(empty.get("avgLeadTimeDays").is_none(), "left out when nothing went live");

    // The default is this month.
    let (_, now) = summary(&pool, &w.executive, "").await;
    assert_eq!(now["month"].as_str().map(str::len), Some(7));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn open_work_delays_and_the_weeks_releases(pool: PgPool) {
    let w = world(&pool, 4).await;
    let (soon, later, yesterday) = (in_days(&pool, 3).await, in_days(&pool, 10).await, in_days(&pool, -1).await);
    sqlx::query("update requests set desired_date = '2000-01-01' where id = 1").execute(&pool).await.unwrap();
    date(&pool, 1, "review", "2020-11-01T00:00:00Z", json!({})).await;
    date(&pool, 2, "release_scheduled", "2020-11-01T00:00:00Z", json!({ "scheduledDate": soon })).await;
    date(&pool, 3, "release_scheduled", "2020-11-01T00:00:00Z", json!({ "scheduledDate": later })).await;
    date(&pool, 4, "info_needed", "2020-11-01T00:00:00Z", json!({ "scheduledDate": yesterday })).await;

    let (_, s) = summary(&pool, &w.executive, "?month=2020-11").await;
    assert_eq!(s["inProgress"], 4);
    assert_eq!(s["delayed"], 2);
    assert_eq!(s["attention"], json!({ "infoNeeded": 1, "delayedRequestIds": [1, 4] }));
    assert_eq!(s["upcomingReleases"], json!([{ "date": soon, "projectId": w.project, "requestIds": [2] }]));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn who_sees_the_summary(pool: PgPool) {
    let w = world(&pool, 0).await;
    assert_eq!(summary(&pool, &w.requester, "").await.0, StatusCode::FORBIDDEN);
    // The query is checked before the permission, as in the mock API.
    for bad in ["?month=2020-13", "?month=2020-1", "?month=abc"] {
        let (status, body) = summary(&pool, &w.requester, bad).await;
        assert_eq!((status, body["issues"][0]["path"].as_str()), (StatusCode::BAD_REQUEST, Some("month")), "{bad}");
    }
    let (_, s) = summary(&pool, &w.executive, "?month=2020-11").await;
    assert_eq!(s["byProject"], json!([{ "projectId": w.project, "done": 0, "inProgress": 0 }]));
    assert_eq!((&s["upcomingReleases"], &s["attention"]["delayedRequestIds"]), (&json!([]), &json!([])));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_release_schedule(pool: PgPool) {
    let w = world(&pool, 4).await;
    // Live on 1 Nov in Seoul (31 Oct in UTC): the release day wins over the plan.
    date(
        &pool,
        1,
        "done",
        "2020-10-01T00:00:00Z",
        json!({ "scheduledDate": "2020-11-05", "releasedAt": "2020-10-31T23:30:00.000Z" }),
    )
    .await;
    date(&pool, 2, "code_review", "2020-10-01T00:00:00Z", json!({ "scheduledDate": "2020-11-01" })).await;
    date(&pool, 3, "cancelled", "2020-10-01T00:00:00Z", json!({ "scheduledDate": "2020-11-01" })).await;
    date(&pool, 4, "review", "2020-10-01T00:00:00Z", json!({})).await;

    let get = |token: String, query: &'static str| {
        let pool = pool.clone();
        async move { send(&pool, "GET", &format!("/releases{query}"), Some(&token), "en", None).await }
    };
    let (status, list) = get(w.executive.clone(), "?from=2020-11-01&to=2020-11-01").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list,
        json!([{ "date": "2020-11-01", "projectId": w.project, "requests": [
            { "id": 1, "title": "Request 0", "status": "done", "canOpen": true },
            { "id": 2, "title": "Request 1", "status": "code_review", "canOpen": true }
        ] }])
    );
    assert_eq!(get(w.executive.clone(), "?to=2020-10-31").await.1, json!([]));
    assert_eq!(get(w.executive.clone(), "").await.1.as_array().unwrap().len(), 1);

    // Company-wide, but only what one may open says so.
    let (_, theirs) = get(w.other.clone(), "").await;
    assert_eq!(theirs[0]["requests"][0]["canOpen"], false);
    let (_, mine) = get(w.requester.clone(), "").await;
    assert_eq!(mine[0]["requests"][0]["canOpen"], true);

    let (status, body) = get(w.executive.clone(), "?from=2020-11-31").await;
    assert_eq!((status, body["issues"][0]["path"].as_str()), (StatusCode::BAD_REQUEST, Some("from")));
}
