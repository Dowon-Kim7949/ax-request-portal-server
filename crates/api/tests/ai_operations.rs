//! The operator's AI operations against a real PostgreSQL: budget, model
//! settings, provider keys, prices, the usage report, and the per-request
//! limit on AI runs.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use ax_api::ai::settings::Selection;
use ax_api::ai::usage::{self, Call, Tokens};
use ax_api::config::{AiSettings, Secret, TaskKeys};
use ax_api::roles::Role;
use common::*;

fn with_key() -> AiSettings {
    AiSettings {
        anthropic: TaskKeys { default: Some(Secret::new("sk-ant-test-0000-ab12")), ..TaskKeys::default() },
        ..AiSettings::default()
    }
}

async fn keyed(pool: &PgPool, method: &str, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_to(app_with(pool, with_key()), method, path, Some(token), "en", body).await
}

async fn call(pool: &PgPool, method: &str, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    send(pool, method, path, Some(token), "en", body).await
}

fn selection(provider: &str, model: &str) -> Value {
    json!({ "provider": provider, "model": model })
}

fn settings(overrides: Value) -> Value {
    json!({
        "defaults": {
            "precheck": selection("anthropic", "claude-haiku-5-5"),
            "report": selection("anthropic", "claude-opus-5-5"),
            "work": selection("anthropic", "claude-opus-5-5"),
        },
        "projectOverrides": overrides,
    })
}

async fn project(pool: &PgPool, admin: &str, owner: Uuid) -> String {
    let body = json!({
        "name": "Portal", "description": "", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [owner.to_string()],
        "targets": [], "constraints": [], "active": true,
    });
    let (status, project) = call(pool, "POST", "/projects", admin, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    project["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn models_per_task_need_a_connected_provider_and_a_price(pool: PgPool) {
    let (_, operator) = signed_in(&pool, "ops@example.com", Role::Operator).await;
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (dev, _) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let project = project(&pool, &admin, dev.id).await;

    let (status, got) = keyed(&pool, "GET", "/ai/settings", &operator, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, {
        let mut s = settings(json!([]));
        s["providers"] = json!([
            { "provider": "anthropic", "connected": true, "keyHint": "ab12" },
            { "provider": "openai", "connected": false },
        ]);
        s
    });
    assert_eq!(call(&pool, "GET", "/ai/settings", &admin, None).await.0, StatusCode::FORBIDDEN);

    // Without a key nothing can be chosen.
    assert_eq!(
        call(&pool, "PUT", "/ai/settings", &operator, Some(settings(json!([])))).await,
        (
            StatusCode::BAD_REQUEST,
            json!({ "code": "bad_request", "message": "Anthropic(Claude) is not connected. Add its API key first." })
        )
    );

    let report =
        |p: &str, model: &str| json!({ "projectId": p, "task": "report", "selection": selection("anthropic", model) });
    let (status, body) =
        keyed(&pool, "PUT", "/ai/settings", &operator, Some(settings(json!([report("p-none", "claude-sonnet-5-5")]))))
            .await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::BAD_REQUEST, Some("The selected project was not found."))
    );
    let twice = json!([report(&project, "claude-sonnet-5-5"), report(&project, "claude-haiku-5-5")]);
    assert_eq!(
        keyed(&pool, "PUT", "/ai/settings", &operator, Some(settings(twice))).await,
        (StatusCode::BAD_REQUEST, json!({ "code": "bad_request", "message": "Please check the input." }))
    );
    let mut openai = settings(json!([]));
    openai["defaults"]["work"] = selection("openai", "gpt-6.1-sol");
    let (_, body) = keyed(&pool, "PUT", "/ai/settings", &operator, Some(openai)).await;
    assert_eq!(body["message"], "OpenAI is not connected. Add its API key first.");
    let (_, body) =
        keyed(&pool, "PUT", "/ai/settings", &operator, Some(settings(json!([report(&project, "claude-next")])))).await;
    assert_eq!(body["message"], "This model is not in the price table: claude-next");

    let chosen = settings(json!([report(&project, "claude-sonnet-5-5")]));
    let (status, saved) = keyed(&pool, "PUT", "/ai/settings", &operator, Some(chosen.clone())).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["projectOverrides"], chosen["projectOverrides"]);
    let (_, got) = keyed(&pool, "GET", "/ai/settings", &operator, None).await;
    assert_eq!(got["projectOverrides"], chosen["projectOverrides"]);

    // The price table cannot drop a model in use.
    let (status, mut pricing) = call(&pool, "GET", "/ai/pricing", &operator, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((pricing["models"].as_array().unwrap().len(), &pricing["usdKrw"]), (7, &json!(1400.0)));
    let all = pricing["models"].as_array().unwrap().clone();
    pricing["models"] = all.iter().filter(|m| m["model"] != "claude-sonnet-5-5").cloned().collect();
    assert_eq!(
        call(&pool, "PUT", "/ai/pricing", &operator, Some(pricing.clone())).await,
        (
            StatusCode::CONFLICT,
            json!({ "code": "guard_failed", "message": "A model in use (claude-sonnet-5-5) cannot be removed from the price table." })
        )
    );
    pricing["models"] = Value::Array(all);
    pricing["usdKrw"] = json!(1500);
    pricing["asOf"] = json!("2026-11-19");
    let (status, saved) = call(&pool, "PUT", "/ai/pricing", &operator, Some(pricing)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, got) = call(&pool, "GET", "/ai/pricing", &operator, None).await;
    assert_eq!((got["usdKrw"].as_f64(), got["asOf"].as_str()), (Some(1500.0), Some("2026-11-19")));
    assert_eq!(call(&pool, "GET", "/ai/pricing", &admin, None).await.0, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn provider_keys_come_only_from_the_configuration(pool: PgPool) {
    let (_, operator) = signed_in(&pool, "ops@example.com", Role::Operator).await;
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let key = json!({ "apiKey": "sk-ant-api03-something" });

    let (status, body) =
        keyed(&pool, "PUT", "/ai/providers/anthropic/key", &operator, Some(json!({ "apiKey": " short " }))).await;
    assert_eq!((status, body["issues"][0]["path"].as_str()), (StatusCode::BAD_REQUEST, Some("apiKey")));
    assert_eq!(
        keyed(&pool, "PUT", "/ai/providers/google/key", &operator, Some(key.clone())).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = keyed(&pool, "PUT", "/ai/providers/anthropic/key", &admin, Some(key.clone())).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::FORBIDDEN, Some("You do not have permission to do this."))
    );

    let refused = json!({
        "code": "forbidden",
        "message": "Provider keys are managed in the server's configuration and cannot be added or removed here.",
    });
    assert_eq!(
        keyed(&pool, "PUT", "/ai/providers/openai/key", &operator, Some(key)).await,
        (StatusCode::FORBIDDEN, refused.clone())
    );
    assert_eq!(
        keyed(&pool, "DELETE", "/ai/providers/anthropic/key", &operator, None).await,
        (StatusCode::FORBIDDEN, refused)
    );
    let (_, body) = send(&pool, "DELETE", "/ai/providers/anthropic/key", Some(&operator), "ko", None).await;
    assert_eq!(body["message"], "AI 제공자 키는 서버 설정에서 관리합니다. 여기서 등록하거나 지울 수 없습니다.");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn budgets_follow_the_contracts_limits(pool: PgPool) {
    let (_, operator) = signed_in(&pool, "ops@example.com", Role::Operator).await;
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;

    let (status, budget) = call(&pool, "GET", "/budget", &operator, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&budget["monthlyLimitKrw"], &budget["perRequestLimitKrw"], &budget["alertThresholdPct"], &budget["spentKrw"]),
        (&json!(0), &json!(0), &json!(80), &json!(0)),
        "no limits until an operator sets them"
    );
    assert_eq!(budget["byModel"], json!([]));
    assert_eq!(budget["month"].as_str().map(str::len), Some(7));
    assert_eq!(call(&pool, "GET", "/budget", &admin, None).await.0, StatusCode::FORBIDDEN);

    let (status, body) = call(
        &pool,
        "PUT",
        "/budget",
        &operator,
        Some(json!({ "monthlyLimitKrw": 10_000, "perRequestLimitKrw": 20_000, "alertThresholdPct": 80 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["issues"],
        json!([{ "path": "perRequestLimitKrw", "message": "The per-request limit cannot be higher than the monthly limit" }])
    );

    let limits = json!({ "monthlyLimitKrw": 500_000, "perRequestLimitKrw": 20_000, "alertThresholdPct": 75 });
    let (status, budget) = call(&pool, "PUT", "/budget", &operator, Some(limits)).await;
    assert_eq!(status, StatusCode::OK, "{budget}");
    let (_, budget) = call(&pool, "GET", "/budget", &operator, None).await;
    assert_eq!((&budget["monthlyLimitKrw"], &budget["alertThresholdPct"]), (&json!(500_000), &json!(75)));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn usage_is_reported_and_stops_a_request_at_its_limit(pool: PgPool) {
    let (_, operator) = signed_in(&pool, "ops@example.com", Role::Operator).await;
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (dev_user, dev) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let (requester_user, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let project = project(&pool, &admin, dev_user.id).await;
    let (_, team) = call(&pool, "POST", "/teams", &admin, Some(json!({ "name": "Planning" }))).await;
    let path = format!("/users/{}", requester_user.id);
    let (status, _) = call(&pool, "PATCH", &path, &admin, Some(json!({ "teamId": team["id"] }))).await;
    assert_eq!(status, StatusCode::OK);
    let request = json!({
        "projectId": project, "type": "non_code", "title": "Search notices", "targetIds": [],
        "currentBehavior": "", "desiredBehavior": "Search notices by title",
        "acceptanceCriteria": [{ "text": "When I type, matching notices show" }], "attachmentIds": [],
    });
    let (status, _) = call(&pool, "POST", "/requests", &requester, Some(request)).await;
    assert_eq!(status, StatusCode::CREATED);

    // The report: 1M input tokens of Opus at $4 = ₩5,600; a precheck: ₩2.8.
    let mut conn = pool.acquire().await.unwrap();
    let opus = Selection { provider: "anthropic".into(), model: "claude-opus-5-5".into() };
    let haiku = Selection { provider: "anthropic".into(), model: "claude-haiku-5-5".into() };
    let report = Call {
        task: "report",
        selection: &opus,
        request_id: Some(1),
        user_id: requester_user.id,
        tokens: Tokens { input: 1_000_000, ..Tokens::default() },
        cost_usd: None,
    };
    let entry = usage::book(&mut conn, "Asia/Seoul", &report).await.unwrap().unwrap();
    assert_eq!((entry.cost_usd, entry.cost_krw, entry.runtime.as_str()), (4.0, 5600.0, "api"));
    assert_eq!(entry.team_id.map(|t| t.to_string()), team["id"].as_str().map(str::to_owned), "the team at call time");
    let precheck = Call {
        task: "precheck",
        selection: &haiku,
        request_id: None,
        user_id: requester_user.id,
        tokens: Tokens { input: 10_000, output: 2_000, ..Tokens::default() },
        cost_usd: None,
    };
    let entry = usage::book(&mut conn, "Asia/Seoul", &precheck).await.unwrap().unwrap();
    assert_eq!((entry.cost_usd, entry.cost_krw), (0.0, 2.8));
    let unpriced = Selection { provider: "anthropic".into(), model: "claude-next".into() };
    assert_eq!(
        usage::book(&mut conn, "Asia/Seoul", &Call { selection: &unpriced, ..report.clone() }).await.unwrap(),
        None
    );
    drop(conn);

    let (status, r) = call(&pool, "GET", "/ai/usage", &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!((&r["totalKrw"], &r["totalUsd"], &r["calls"]), (&json!(5603), &json!(4), &json!(2)));
    assert_eq!(
        r["byTask"],
        json!([{ "task": "precheck", "calls": 1, "costKrw": 3 }, { "task": "report", "calls": 1, "costKrw": 5600 }])
    );
    assert_eq!(r["topRequests"], json!([{ "requestId": 1, "projectId": project, "costKrw": 5600, "calls": 1 }]));
    assert_eq!(r["byTeam"], json!([{ "teamId": team["id"], "calls": 2, "costKrw": 5603 }]));
    assert_eq!(
        r["byPerson"],
        json!([{ "userId": requester_user.id, "teamId": team["id"], "calls": 2, "costKrw": 5603 }])
    );
    assert_eq!(call(&pool, "GET", "/ai/usage?month=2020-01", &operator, None).await.1["calls"], 0);
    let (status, body) = call(&pool, "GET", "/ai/usage?month=2026-13", &operator, None).await;
    assert_eq!((status, body["issues"][0]["path"].as_str()), (StatusCode::BAD_REQUEST, Some("month")));
    assert_eq!(call(&pool, "GET", "/ai/usage", &admin, None).await.0, StatusCode::FORBIDDEN);

    let (_, budget) = call(&pool, "GET", "/budget", &operator, None).await;
    assert_eq!(budget["spentKrw"], 5603);
    assert_eq!(budget["byModel"][0], json!({ "provider": "anthropic", "model": "claude-opus-5-5", "spentKrw": 5600 }));
    let (_, summary) = call(&pool, "GET", "/dashboard/summary", &admin, None).await;
    assert_eq!((&summary["aiCostKrw"], &summary["aiBudgetKrw"]), (&json!(5603), &json!(0)));
    let (_, shown) = call(&pool, "GET", "/requests/1", &admin, None).await;
    assert_eq!((&shown["aiUsage"]["spentKrw"], &shown["aiUsage"]["limitKrw"]), (&json!(5600), &json!(0)));
    assert_eq!(shown["aiUsage"]["entries"][0]["model"], "claude-opus-5-5");

    // Over the per-request limit, no AI run starts again.
    let limits =
        |per_request: i64| json!({ "monthlyLimitKrw": 0, "perRequestLimitKrw": per_request, "alertThresholdPct": 80 });
    call(&pool, "PUT", "/budget", &operator, Some(limits(5_000))).await;
    sqlx::query(
        "update requests set status = 'review', details = details || '{\"report\":{\"assumptions\":[]}}' where id = 1",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        call(&pool, "POST", "/requests/1/transitions", &dev, Some(json!({ "event": "runWork" }))).await,
        (
            StatusCode::CONFLICT,
            json!({ "code": "guard_failed", "message": "This request has used its per-request AI budget (₩5,000), so AI work cannot run again. Ask an operator to raise the limit, or handle it by hand." })
        )
    );
    let (_, body) =
        send(&pool, "POST", "/requests/1/transitions", Some(&dev), "ko", Some(json!({ "event": "runWork" }))).await;
    assert_eq!(
        body["message"],
        "이 요청의 AI 비용이 요청당 한도(5,000원)에 도달해 AI 작업을 더 실행할 수 없습니다. 운영자가 한도를 올리거나 직접 처리해 주세요."
    );

    call(&pool, "PUT", "/budget", &operator, Some(limits(6_000))).await;
    let (status, started) =
        call(&pool, "POST", "/requests/1/transitions", &dev, Some(json!({ "event": "runWork" }))).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(
        started["work"]["agent"],
        json!({ "provider": "anthropic", "model": "claude-opus-5-5", "runtime": "claude_code" }),
        "the run records the agent the settings chose"
    );
}
