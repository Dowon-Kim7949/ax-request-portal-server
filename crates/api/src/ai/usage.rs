//! The ledger of AI usage (`ai_usage`) and what is read from it: a request's
//! spending, the month's budget, and the operator's usage report. The sums and
//! orderings are the frontend mock API's (`src/mocks/engine.ts`), computed the
//! same way so the numbers agree to the won.

use std::cmp::Reverse;
use std::collections::HashMap;

use serde_json::{Map, Value, json};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::settings::{self, BudgetLimits, ModelPrice, PROVIDERS, Pricing, Selection, TASKS};
use crate::auth::token;
use crate::requests::store::iso;

/// The contract's `TokenUsage`: each count billed at its own rate, none
/// containing another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output: i64,
    /// Part of `output`, when the provider reports it.
    pub reasoning: Option<i64>,
}

impl Tokens {
    /// The frontend's `addUsage`.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self {
            input: self.input + other.input,
            cache_read: self.cache_read + other.cache_read,
            cache_write: self.cache_write + other.cache_write,
            output: self.output + other.output,
            reasoning: match (self.reasoning, other.reasoning) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
            },
        }
    }

    pub fn to_json(self) -> Value {
        let mut t = json!({
            "input": self.input, "cacheRead": self.cache_read, "cacheWrite": self.cache_write, "output": self.output,
        });
        if let Some(reasoning) = self.reasoning {
            t["reasoning"] = json!(reasoning);
        }
        t
    }
}

/// Money is booked at 1/100 of the unit so small calls do not round to zero
/// (the frontend's `roundMoney`; amounts are never negative).
pub fn round_money(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// An amount as JSON: a whole number as an integer (`5600`, not `5600.0`),
/// as JavaScript writes it.
pub fn money(value: f64) -> Value {
    #[allow(clippy::cast_possible_truncation)] // whole and within ±2^53, checked first
    let whole = (value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0).then_some(value as i64);
    whole.map_or_else(|| json!(value), |n| json!(n))
}

/// Adds amounts from 0 (`Iterator::sum` of `f64` starts at `-0.0`).
fn total(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0, |sum, v| sum + v)
}

/// USD for one usage at one price, unrounded (the frontend's `costUsd`).
#[allow(clippy::cast_precision_loss)] // token counts stay far below 2^53
pub fn cost_usd(tokens: Tokens, price: &ModelPrice) -> f64 {
    (tokens.input as f64 * price.input_usd
        + tokens.cache_read as f64 * price.cache_read_usd
        + tokens.cache_write as f64 * price.cache_write_usd
        + tokens.output as f64 * price.output_usd)
        / 1_000_000.0
}

/// One booked call, the contract's `AiUsageEntry`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub at: String,
    /// `at` on the business calendar (`YYYY-MM-DD`).
    pub date: String,
    pub task: String,
    pub provider: String,
    pub model: String,
    pub runtime: String,
    pub request_id: Option<i64>,
    pub user_id: Uuid,
    pub team_id: Option<Uuid>,
    pub tokens: Tokens,
    pub cost_usd: f64,
    pub cost_krw: f64,
}

impl Entry {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            at: row.try_get("at_iso")?,
            date: row.try_get("date")?,
            task: row.try_get("task")?,
            provider: row.try_get("provider")?,
            model: row.try_get("model")?,
            runtime: row.try_get("runtime")?,
            request_id: row.try_get("request_id")?,
            user_id: row.try_get("user_id")?,
            team_id: row.try_get("team_id")?,
            tokens: Tokens {
                input: row.try_get("input_tokens")?,
                cache_read: row.try_get("cache_read_tokens")?,
                cache_write: row.try_get("cache_write_tokens")?,
                output: row.try_get("output_tokens")?,
                reasoning: row.try_get("reasoning_tokens")?,
            },
            cost_usd: row.try_get("cost_usd")?,
            cost_krw: row.try_get("cost_krw")?,
        })
    }

    pub fn to_json(&self) -> Value {
        let mut e = Map::new();
        e.insert("id".into(), json!(self.id));
        e.insert("at".into(), json!(self.at));
        e.insert("task".into(), json!(self.task));
        e.insert("provider".into(), json!(self.provider));
        e.insert("model".into(), json!(self.model));
        e.insert("runtime".into(), json!(self.runtime));
        if let Some(request_id) = self.request_id {
            e.insert("requestId".into(), json!(request_id));
        }
        e.insert("userId".into(), json!(self.user_id));
        if let Some(team_id) = self.team_id {
            e.insert("teamId".into(), json!(team_id));
        }
        e.insert("tokens".into(), self.tokens.to_json());
        e.insert("costUsd".into(), money(self.cost_usd));
        e.insert("costKrw".into(), money(self.cost_krw));
        Value::Object(e)
    }
}

/// One AI call to book.
#[derive(Debug, Clone)]
pub struct Call<'a> {
    pub task: &'a str,
    pub selection: &'a Selection,
    /// How the call ran (the contract's `AiRuntime`): `api` for a call this
    /// server made, the provider's coding agent for a runner's run, whatever
    /// the task (`Spend::runtime`).
    pub runtime: &'a str,
    /// `None` for a precheck, which runs before the request exists.
    pub request_id: Option<i64>,
    /// The person whose action started the call.
    pub user_id: Uuid,
    pub tokens: Tokens,
    /// Booked instead of the price table's figure when the provider knows the
    /// cost better; the model must still be in the table.
    pub cost_usd: Option<f64>,
}

fn select(time_zone_param: &str) -> String {
    format!(
        "select id, {} as at_iso, to_char(at at time zone {time_zone_param}, 'YYYY-MM-DD') as date,
                task, provider, model, runtime, request_id, user_id, team_id,
                input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, reasoning_tokens,
                cost_usd, cost_krw
           from ai_usage",
        iso("at")
    )
}

/// Books one call at today's prices against the caller and the team they are
/// in now (the frontend's `bookUsage`). `None`, and nothing booked, when the
/// model has no price.
///
/// # Errors
///
/// Database failures.
pub async fn book(db: &mut PgConnection, time_zone: &str, call: &Call<'_>) -> Result<Option<Entry>, sqlx::Error> {
    let pricing = settings::pricing(&mut *db).await?;
    let Some(price) = pricing.price_of(call.selection) else { return Ok(None) };
    let usd = call.cost_usd.unwrap_or_else(|| cost_usd(call.tokens, price));
    let row = sqlx::query(&format!(
        "with booked as (
           insert into ai_usage (id, task, provider, model, runtime, request_id, user_id, team_id,
                                 input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                                 reasoning_tokens, cost_usd, cost_krw)
           values ($2, $3, $4, $5, $6, $7, $8, (select team_id from users where id = $8),
                   $9, $10, $11, $12, $13, $14, $15)
           returning *
         )
         {} where true",
        select("$1::text").replace("from ai_usage", "from booked")
    ))
    .bind(time_zone)
    .bind(token::generate_with("ai_"))
    .bind(call.task)
    .bind(&call.selection.provider)
    .bind(&call.selection.model)
    .bind(call.runtime)
    .bind(call.request_id)
    .bind(call.user_id)
    .bind(call.tokens.input)
    .bind(call.tokens.cache_read)
    .bind(call.tokens.cache_write)
    .bind(call.tokens.output)
    .bind(call.tokens.reasoning)
    .bind(round_money(usd))
    .bind(round_money(usd * pricing.usd_krw))
    .fetch_one(&mut *db)
    .await?;
    Entry::from_row(&row).map(Some)
}

/// Every call booked against a request, all months, in booking order.
///
/// # Errors
///
/// Database failures.
pub async fn for_request(db: &mut PgConnection, time_zone: &str, request_id: i64) -> Result<Vec<Entry>, sqlx::Error> {
    let rows = sqlx::query(&format!("{} where request_id = $2 order by at, seq", select("$1::text")))
        .bind(time_zone)
        .bind(request_id)
        .fetch_all(db)
        .await?;
    rows.iter().map(Entry::from_row).collect()
}

/// Every call booked in `month` (`YYYY-MM`) on the business calendar, in
/// booking order.
///
/// # Errors
///
/// Database failures.
pub async fn in_month(db: &mut PgConnection, time_zone: &str, month: &str) -> Result<Vec<Entry>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "{} where to_char(at at time zone $1::text, 'YYYY-MM') = $2 order by at, seq",
        select("$1::text")
    ))
    .bind(time_zone)
    .bind(month)
    .fetch_all(db)
    .await?;
    rows.iter().map(Entry::from_row).collect()
}

/// What a request has spent, all months: the per-request limit's measure.
pub fn spent_krw(entries: &[Entry]) -> f64 {
    total(entries.iter().map(|e| e.cost_krw))
}

/// The contract's `RequestAiUsage`.
pub fn request_usage(entries: &[Entry], limits: &BudgetLimits) -> Value {
    json!({
        "spentKrw": money(round_money(spent_krw(entries))),
        "limitKrw": limits.per_request_limit_krw,
        "entries": entries.iter().map(Entry::to_json).collect::<Vec<_>>(),
    })
}

/// Whole won, as the mock's `Math.round` gives it.
#[allow(clippy::cast_possible_truncation)] // sums of booked amounts stay far below i64::MAX
fn won(value: f64) -> i64 {
    value.round() as i64
}

/// The month's spending in whole won.
pub fn month_spent_krw(entries: &[Entry]) -> i64 {
    won(spent_krw(entries))
}

/// The contract's `Budget` for `month`.
pub fn budget_json(month: &str, limits: &BudgetLimits, entries: &[Entry]) -> Value {
    let mut by_model: Vec<(String, String, f64)> = Vec::new();
    for e in entries {
        match by_model.iter_mut().find(|(p, m, _)| *p == e.provider && *m == e.model) {
            Some(row) => row.2 += e.cost_krw,
            None => by_model.push((e.provider.clone(), e.model.clone(), e.cost_krw)),
        }
    }
    let mut by_model: Vec<(String, String, i64)> = by_model.into_iter().map(|(p, m, s)| (p, m, won(s))).collect();
    by_model.sort_by_key(|row| Reverse(row.2));
    json!({
        "month": month,
        "monthlyLimitKrw": limits.monthly_limit_krw,
        "perRequestLimitKrw": limits.per_request_limit_krw,
        "alertThresholdPct": limits.alert_threshold_pct,
        "spentKrw": month_spent_krw(entries),
        "byModel": by_model.iter().map(|(provider, model, spent)| json!({ "provider": provider, "model": model, "spentKrw": spent })).collect::<Vec<_>>(),
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    calls: i64,
    cost_krw: f64,
    tokens: Tokens,
}

/// Entries grouped by `key`, in the order each key first appears.
fn group<K: PartialEq + Clone>(entries: &[Entry], key: impl Fn(&Entry) -> K) -> Vec<(K, Acc)> {
    let mut groups: Vec<(K, Acc)> = Vec::new();
    for e in entries {
        let k = key(e);
        let index = match groups.iter().position(|(g, _)| *g == k) {
            Some(index) => index,
            None => {
                groups.push((k, Acc::default()));
                groups.len() - 1
            }
        };
        let acc = &mut groups[index].1;
        acc.calls += 1;
        acc.cost_krw += e.cost_krw;
        acc.tokens = acc.tokens.plus(e.tokens);
    }
    groups
}

/// Most expensive first; equal costs keep their order (a stable sort, as JavaScript's).
fn by_cost<K>(mut groups: Vec<(K, Acc)>) -> Vec<(K, Acc)> {
    groups.sort_by(|a, b| b.1.cost_krw.total_cmp(&a.1.cost_krw));
    groups
}

/// The contract's `AiUsageReport` for `month`. `projects` maps each request
/// in the month's entries to its project.
pub fn report_json(
    month: &str,
    limits: &BudgetLimits,
    pricing: &Pricing,
    entries: &[Entry],
    projects: &HashMap<i64, Uuid>,
) -> Value {
    let by_provider = group(entries, |e| e.provider.clone());
    let mut by_model: Vec<Value> = group(entries, |e| (e.provider.clone(), e.model.clone()))
        .into_iter()
        .map(|((provider, model), a)| {
            json!({ "provider": provider, "model": model, "calls": a.calls, "costKrw": won(a.cost_krw), "tokens": a.tokens.to_json() })
        })
        .collect();
    by_model.sort_by_key(|row| Reverse(row["costKrw"].as_i64()));
    let by_task = group(entries, |e| e.task.clone());
    let mut daily: Vec<(String, String, i64)> = group(entries, |e| (e.date.clone(), e.provider.clone()))
        .into_iter()
        .map(|((date, provider), a)| (date, provider, won(a.cost_krw)))
        .collect();
    daily.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let requests =
        group(&entries.iter().filter(|e| e.request_id.is_some()).cloned().collect::<Vec<_>>(), |e| e.request_id);
    let by_team = by_cost(group(entries, |e| e.team_id));
    let by_person = by_cost(group(entries, |e| e.user_id));
    // A person's row shows the team of their latest call.
    let mut latest: HashMap<Uuid, &Entry> = HashMap::new();
    for e in entries {
        if latest.get(&e.user_id).is_none_or(|seen| e.at > seen.at) {
            latest.insert(e.user_id, e);
        }
    }

    json!({
        "month": month,
        "totalKrw": won(spent_krw(entries)),
        "totalUsd": money((total(entries.iter().map(|e| e.cost_usd)) * 100.0).round() / 100.0),
        "calls": entries.len(),
        "monthlyLimitKrw": limits.monthly_limit_krw,
        "alertThresholdPct": limits.alert_threshold_pct,
        "usdKrw": money(pricing.usd_krw),
        "pricingAsOf": pricing.as_of,
        "byProvider": PROVIDERS.iter().filter_map(|p| {
            let (_, a) = by_provider.iter().find(|(k, _)| k == p)?;
            Some(json!({ "provider": p, "calls": a.calls, "costKrw": won(a.cost_krw), "tokens": a.tokens.to_json() }))
        }).collect::<Vec<_>>(),
        "byModel": by_model,
        "byTask": TASKS.iter().filter_map(|t| {
            let (_, a) = by_task.iter().find(|(k, _)| k == t)?;
            Some(json!({ "task": t, "calls": a.calls, "costKrw": won(a.cost_krw) }))
        }).collect::<Vec<_>>(),
        "daily": daily.iter().map(|(date, provider, cost)| json!({ "date": date, "provider": provider, "costKrw": cost })).collect::<Vec<_>>(),
        "topRequests": by_cost(requests).into_iter().take(10).filter_map(|(id, a)| {
            let id = id?;
            let project = projects.get(&id)?;
            Some(json!({ "requestId": id, "projectId": project, "costKrw": won(a.cost_krw), "calls": a.calls }))
        }).collect::<Vec<_>>(),
        "byTeam": by_team.iter().map(|(team, a)| {
            let mut row = json!({ "calls": a.calls, "costKrw": won(a.cost_krw) });
            if let Some(team) = team {
                row["teamId"] = json!(team);
            }
            row
        }).collect::<Vec<_>>(),
        "byPerson": by_person.iter().map(|(user, a)| {
            let mut row = json!({ "userId": user, "calls": a.calls, "costKrw": won(a.cost_krw) });
            if let Some(team) = latest.get(user).and_then(|e| e.team_id) {
                row["teamId"] = json!(team);
            }
            row
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::settings::runtime_for;

    fn tokens(input: i64, cache_read: i64, cache_write: i64, output: i64) -> Tokens {
        Tokens { input, cache_read, cache_write, output, reasoning: None }
    }

    fn entry(provider: &str, model: &str, task: &str, krw: f64, request: Option<i64>, user: u128, at: &str) -> Entry {
        Entry {
            id: format!("ai_{at}"),
            at: format!("2026-11-{at}T00:00:00.000Z"),
            date: format!("2026-11-{at}"),
            task: task.into(),
            provider: provider.into(),
            model: model.into(),
            runtime: runtime_for(task, provider).into(),
            request_id: request,
            user_id: Uuid::from_u128(user),
            team_id: (user == 1).then(|| Uuid::from_u128(100)),
            tokens: tokens(10, 0, 0, 5),
            cost_usd: krw / 1400.0,
            cost_krw: krw,
        }
    }

    #[test]
    fn costs_follow_the_price_table() {
        let opus = &Pricing::default().models[0];
        let t = tokens(100_000, 1_000_000, 10_000, 50_000);
        // 0.4 + 0.2 + 0.05 + 1.0
        assert!((cost_usd(t, opus) - 1.65).abs() < 1e-12);
        assert_eq!(round_money(0.004_9), 0.0);
        assert_eq!(round_money(0.005), 0.01);
        assert_eq!(round_money(2310.004), 2310.0);
    }

    #[test]
    fn tokens_add_and_keep_reasoning_only_when_reported() {
        let a = tokens(1, 2, 3, 4);
        assert_eq!(a.plus(a).reasoning, None);
        let b = Tokens { reasoning: Some(2), ..a };
        assert_eq!(a.plus(b), Tokens { input: 2, cache_read: 4, cache_write: 6, output: 8, reasoning: Some(2) });
        assert_eq!(a.to_json(), json!({ "input": 1, "cacheRead": 2, "cacheWrite": 3, "output": 4 }));
    }

    #[test]
    fn budgets_and_reports_add_up_like_the_mock() {
        let entries = vec![
            entry("anthropic", "claude-haiku-5-5", "precheck", 0.4, None, 1, "02"),
            entry("anthropic", "claude-opus-5-5", "report", 1200.3, Some(7), 1, "02"),
            entry("openai", "gpt-6.1-sol", "work", 3000.0, Some(8), 2, "01"),
            entry("anthropic", "claude-opus-5-5", "work", 1200.3, Some(7), 2, "03"),
        ];
        let limits =
            BudgetLimits { monthly_limit_krw: 500_000, per_request_limit_krw: 20_000, alert_threshold_pct: 80 };
        let budget = budget_json("2026-11", &limits, &entries);
        assert_eq!(budget["spentKrw"], 5401);
        assert_eq!(
            budget["byModel"],
            json!([
                { "provider": "openai", "model": "gpt-6.1-sol", "spentKrw": 3000 },
                { "provider": "anthropic", "model": "claude-opus-5-5", "spentKrw": 2401 },
                { "provider": "anthropic", "model": "claude-haiku-5-5", "spentKrw": 0 },
            ])
        );

        let projects: HashMap<i64, Uuid> = [(7, Uuid::from_u128(70)), (8, Uuid::from_u128(80))].into();
        let r = report_json("2026-11", &limits, &Pricing::default(), &entries, &projects);
        assert_eq!((r["totalKrw"].as_i64(), r["calls"].as_i64()), (Some(5401), Some(4)));
        let providers: Vec<&str> =
            r["byProvider"].as_array().unwrap().iter().map(|p| p["provider"].as_str().unwrap()).collect();
        assert_eq!(providers, ["anthropic", "openai"], "in the contract's order");
        assert_eq!(r["byProvider"][0]["tokens"], json!({ "input": 30, "cacheRead": 0, "cacheWrite": 0, "output": 15 }));
        let tasks: Vec<&str> = r["byTask"].as_array().unwrap().iter().map(|t| t["task"].as_str().unwrap()).collect();
        assert_eq!(tasks, ["precheck", "report", "work"]);
        assert_eq!(r["daily"][0], json!({ "date": "2026-11-01", "provider": "openai", "costKrw": 3000 }));
        assert_eq!(r["daily"][1], json!({ "date": "2026-11-02", "provider": "anthropic", "costKrw": 1201 }));
        assert_eq!(
            r["topRequests"],
            json!([
                { "requestId": 8, "projectId": Uuid::from_u128(80), "costKrw": 3000, "calls": 1 },
                { "requestId": 7, "projectId": Uuid::from_u128(70), "costKrw": 2401, "calls": 2 },
            ]),
            "a precheck belongs to no request"
        );
        assert_eq!(r["byTeam"][0], json!({ "calls": 2, "costKrw": 4200 }), "no team: one row without teamId");
        assert_eq!(r["byTeam"][1]["teamId"], json!(Uuid::from_u128(100)));
        assert_eq!(r["byPerson"][0]["userId"], json!(Uuid::from_u128(2)));
        assert_eq!(r["byPerson"][1]["teamId"], json!(Uuid::from_u128(100)));
        assert_eq!(r["usdKrw"], 1400);
        assert_eq!(money(0.21), json!(0.21));
        assert_eq!(money(-0.0), json!(0));
    }

    #[test]
    fn a_requests_usage() {
        let entries = vec![entry("anthropic", "claude-opus-5-5", "report", 0.105, Some(7), 1, "02"); 2];
        let limits = BudgetLimits { per_request_limit_krw: 20_000, ..BudgetLimits::default() };
        let usage = request_usage(&entries, &limits);
        assert_eq!(usage["spentKrw"], 0.21);
        assert_eq!(usage["limitKrw"], 20_000);
        assert_eq!(usage["entries"][0]["requestId"], 7);
        assert!(usage["entries"][0].get("teamId").is_some());
    }
}
