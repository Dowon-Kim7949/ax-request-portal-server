//! The operator's AI settings: the budget, the model each task uses (with
//! per-project overrides) and the price table, as the frontend's
//! `src/domain/ai.ts` and `src/domain/model.ts` define them.
//!
//! Each is one JSON document in `ai_config`; a missing one is the template's
//! default (`src/config/ai.ts`; no budget limits). Writers hold one advisory
//! lock for the transaction, so a check that spans settings cannot race.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::body::{Body, Text, integer, utf16_len};
use crate::config::AiSettings;
use crate::requests::draft::is_iso_date;

/// The contract's `AiProvider`, in its order.
pub const PROVIDERS: [&str; 2] = ["anthropic", "openai"];
/// The contract's `AiTask`, in its order.
pub const TASKS: [&str; 3] = ["precheck", "report", "work"];

/// The contract's `LimitKrw` maximum: ten billion won.
pub const LIMIT_MAX_KRW: i64 = 10_000_000_000;

/// The contract's `AiRuntime` for one model API call.
pub const API_RUNTIME: &str = "api";

/// The provider's coding agent (the contract's `AiRuntime`): what a runner
/// runs, whatever the task.
pub fn agent_runtime(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude_code",
        _ => "codex",
    }
}

/// How a task runs on a provider in the frontend's model (its `runtimeFor`):
/// precheck and report are one model call, work is the provider's coding
/// agent. The usage ledger books how a call actually ran instead
/// (`Spend::runtime`): with a runner, a report is a coding-agent run too.
pub fn runtime_for(task: &str, provider: &str) -> &'static str {
    match task {
        "work" => agent_runtime(provider),
        _ => API_RUNTIME,
    }
}

/// The contract's `BudgetLimits`. `0` means no limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetLimits {
    pub monthly_limit_krw: i64,
    pub per_request_limit_krw: i64,
    pub alert_threshold_pct: i64,
}

impl Default for BudgetLimits {
    /// No limits until an operator sets them; warn at 80 % (the mock's threshold).
    fn default() -> Self {
        Self { monthly_limit_krw: 0, per_request_limit_krw: 0, alert_threshold_pct: 80 }
    }
}

impl BudgetLimits {
    /// Whether a request that has spent `spent_krw` may not start another AI
    /// run (the frontend's `overRequestLimit`).
    pub fn request_over_limit(&self, spent_krw: f64) -> bool {
        #[allow(clippy::cast_precision_loss)] // whole won up to 10^10 are exact in f64
        let limit = self.per_request_limit_krw as f64;
        self.per_request_limit_krw > 0 && spent_krw >= limit
    }
}

/// The contract's `AiSelection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub provider: String,
    pub model: String,
}

impl Selection {
    fn new(provider: &str, model: &str) -> Self {
        Self { provider: provider.to_owned(), model: model.to_owned() }
    }
}

/// The contract's `AiTaskSelections`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSelections {
    pub precheck: Selection,
    pub report: Selection,
    pub work: Selection,
}

impl TaskSelections {
    pub fn get(&self, task: &str) -> &Selection {
        match task {
            "precheck" => &self.precheck,
            "report" => &self.report,
            _ => &self.work,
        }
    }
}

/// The contract's `AiProjectOverride`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOverride {
    pub project_id: String,
    pub task: String,
    pub selection: Selection,
}

/// The model per task and per project (`AiSettingsInput`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selections {
    pub defaults: TaskSelections,
    pub project_overrides: Vec<ProjectOverride>,
}

impl Default for Selections {
    /// The template's `DEFAULT_AI_SELECTIONS`.
    fn default() -> Self {
        Self {
            defaults: TaskSelections {
                precheck: Selection::new("anthropic", "claude-haiku-5-5"),
                report: Selection::new("anthropic", "claude-opus-5-5"),
                work: Selection::new("anthropic", "claude-opus-5-5"),
            },
            project_overrides: Vec::new(),
        }
    }
}

impl Selections {
    /// The model `task` uses for `project_id`: the project's override, else
    /// the default (the frontend's `resolveSelection`).
    pub fn resolve(&self, project_id: &str, task: &str) -> &Selection {
        self.project_overrides
            .iter()
            .find(|o| o.project_id == project_id && o.task == task)
            .map_or_else(|| self.defaults.get(task), |o| &o.selection)
    }

    /// Every selection in use, defaults first (the frontend's `selectionsInUse`).
    pub fn in_use(&self) -> Vec<&Selection> {
        TASKS.iter().map(|t| self.defaults.get(t)).chain(self.project_overrides.iter().map(|o| &o.selection)).collect()
    }
}

/// The contract's `ModelPrice`: USD per million tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrice {
    pub provider: String,
    pub model: String,
    pub label: String,
    pub input_usd: f64,
    pub cache_read_usd: f64,
    pub cache_write_usd: f64,
    pub output_usd: f64,
}

/// The contract's `AiPricing`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pricing {
    pub as_of: String,
    pub usd_krw: f64,
    pub models: Vec<ModelPrice>,
}

impl Default for Pricing {
    /// The template's `DEFAULT_AI_PRICING`, with its placeholder exchange rate.
    fn default() -> Self {
        let price = |provider: &str, model: &str, label: &str, prices: [f64; 4]| ModelPrice {
            provider: provider.to_owned(),
            model: model.to_owned(),
            label: label.to_owned(),
            input_usd: prices[0],
            cache_read_usd: prices[1],
            cache_write_usd: prices[2],
            output_usd: prices[3],
        };
        Self {
            as_of: "2026-11-12".to_owned(),
            usd_krw: 1400.0,
            models: vec![
                price("anthropic", "claude-opus-5-5", "Claude Opus 5.5", [4.0, 0.2, 5.0, 20.0]),
                price("anthropic", "claude-sonnet-5-5", "Claude Sonnet 5.5", [2.0, 0.1, 2.5, 10.0]),
                price("anthropic", "claude-haiku-5-5", "Claude Haiku 5.5", [0.1, 0.01, 0.125, 0.5]),
                price("openai", "gpt-6-astra", "GPT-6 Astra", [10.0, 1.0, 12.5, 50.0]),
                price("openai", "gpt-6.1-sol", "GPT-6.1 Sol", [2.0, 0.1, 2.5, 10.0]),
                price("openai", "gpt-6.1-terra", "GPT-6.1 Terra", [1.0, 0.05, 1.25, 5.0]),
                price("openai", "gpt-6-luna", "GPT-6 Luna", [0.1, 0.01, 0.125, 0.5]),
            ],
        }
    }
}

impl Pricing {
    /// The price of a selection (the frontend's `priceOf`).
    pub fn price_of(&self, selection: &Selection) -> Option<&ModelPrice> {
        self.models.iter().find(|m| m.provider == selection.provider && m.model == selection.model)
    }
}

/// Whether `provider` is connected: a key in the configuration and Claude in
/// use, by the server (`AX_AI_PROVIDER=anthropic`) or by a runner
/// (`AX_RUNNER_TOKEN`). With the stub a key does nothing, so the console must
/// not show it as connected. Only Anthropic can be connected: no other
/// provider is wired.
pub fn connected(ai: &AiSettings, provider: &str) -> bool {
    provider == "anthropic" && ai.anthropic.connected() && (ai.provider == "anthropic" || ai.runner_token.is_some())
}

/// The contract's `ProviderConnection` list, one per provider, in order.
pub fn connections(ai: &AiSettings) -> Vec<Value> {
    PROVIDERS
        .iter()
        .map(|provider| match (connected(ai, provider), ai.anthropic.hint()) {
            (true, Some(hint)) => json!({ "provider": provider, "connected": true, "keyHint": hint }),
            _ => json!({ "provider": provider, "connected": false }),
        })
        .collect()
}

/* ---------- storage ---------- */

/// Holds every AI settings write in this transaction one after another.
///
/// # Errors
///
/// Database failures.
pub async fn lock(tx: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock(hashtext('ai_config'))").execute(tx).await.map(|_| ())
}

async fn load<T: DeserializeOwned + Default>(db: &mut PgConnection, name: &str) -> Result<T, sqlx::Error> {
    let value: Option<Value> =
        sqlx::query_scalar("select value from ai_config where name = $1").bind(name).fetch_optional(db).await?;
    match value {
        None => Ok(T::default()),
        Some(value) => serde_json::from_value(value).map_err(|e| sqlx::Error::Decode(Box::new(e))),
    }
}

/// Stores one setting. Call [`lock`] first in the same transaction.
///
/// # Errors
///
/// Database failures.
pub async fn save<T: Serialize>(tx: &mut PgConnection, name: &str, value: &T) -> Result<(), sqlx::Error> {
    let value = serde_json::to_value(value).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
    sqlx::query(
        "insert into ai_config (name, value) values ($1, $2)
         on conflict (name) do update set value = excluded.value, updated_at = now()",
    )
    .bind(name)
    .bind(value)
    .execute(tx)
    .await
    .map(|_| ())
}

/// # Errors
///
/// Database failures, or a stored document that no longer parses.
pub async fn budget(db: &mut PgConnection) -> Result<BudgetLimits, sqlx::Error> {
    load(db, "budget").await
}

/// # Errors
///
/// As [`budget`].
pub async fn selections(db: &mut PgConnection) -> Result<Selections, sqlx::Error> {
    load(db, "selections").await
}

/// # Errors
///
/// As [`budget`].
pub async fn pricing(db: &mut PgConnection) -> Result<Pricing, sqlx::Error> {
    load(db, "pricing").await
}

/* ---------- bodies ---------- */

const LIMIT: Text = ("0–10,000,000,000 사이의 정수(원)로 입력해 주세요.", "Use whole won from 0 to 10,000,000,000.");
const PCT: Text = ("1–100 사이의 정수로 입력해 주세요.", "Use a whole number from 1 to 100.");
const PER_REQUEST_ABOVE_MONTHLY: Text =
    ("요청당 한도는 월 한도보다 클 수 없습니다.", "The per-request limit cannot be higher than the monthly limit");
const PROVIDER: Text = ("anthropic 또는 openai여야 합니다.", "Must be anthropic or openai.");
const TASK: Text = ("precheck, report, work 중 하나여야 합니다.", "Must be precheck, report or work.");
const NAME: Text = ("값을 입력해 주세요.", "Enter a value.");
const TEXT: Text = ("글자로 입력해 주세요.", "Enter text.");
const OBJECT: Text = ("항목을 확인해 주세요.", "Check this item.");
const LIST: Text = ("목록으로 보내 주세요.", "Send a list.");
const PRICE: Text = ("0 이상의 숫자로 입력해 주세요.", "Use a number of 0 or more.");
const RATE: Text = ("0보다 큰 숫자로 입력해 주세요.", "Use a number above 0.");
const DATE: Text = ("YYYY-MM-DD 형식의 날짜로 입력해 주세요.", "Use a date as YYYY-MM-DD.");
const MODELS: Text = ("가격을 하나 이상 넣어 주세요.", "Add at least one price.");
const UNIQUE: Text = ("공급자와 모델 조합은 한 번씩만 넣어 주세요.", "Each provider/model appears once");
const API_KEY: Text = ("API 키는 8–500자로 입력해 주세요.", "Use an API key of 8 to 500 characters.");

type Issues = Vec<(String, Text)>;

/// `BudgetInput`: three whole numbers; a per-request limit not above a set
/// monthly limit (checked once the fields are valid, at `perRequestLimitKrw`).
pub fn budget_input(body: &mut Body) -> Option<BudgetLimits> {
    let limit = |v: &Value| integer(v).filter(|n| (0..=LIMIT_MAX_KRW).contains(n));
    let monthly = body.required("monthlyLimitKrw", LIMIT, limit);
    let per_request = body.required("perRequestLimitKrw", LIMIT, limit);
    let pct = body.required("alertThresholdPct", PCT, |v| integer(v).filter(|n| (1..=100).contains(n)));
    let limits =
        BudgetLimits { monthly_limit_krw: monthly?, per_request_limit_krw: per_request?, alert_threshold_pct: pct? };
    if limits.monthly_limit_krw > 0 && limits.per_request_limit_krw > limits.monthly_limit_krw {
        body.issue("perRequestLimitKrw", PER_REQUEST_ABOVE_MONTHLY);
        return None;
    }
    Some(limits)
}

fn selection(value: Option<&Value>, path: &str, issues: &mut Issues) -> Option<Selection> {
    let Some(fields) = value.and_then(Value::as_object) else {
        issues.push((path.to_owned(), OBJECT));
        return None;
    };
    let provider = fields.get("provider").and_then(Value::as_str).filter(|p| PROVIDERS.contains(p));
    if provider.is_none() {
        issues.push((format!("{path}.provider"), PROVIDER));
    }
    let model = fields.get("model").and_then(Value::as_str).filter(|m| !m.is_empty());
    if model.is_none() {
        issues.push((format!("{path}.model"), NAME));
    }
    Some(Selection::new(provider?, model?))
}

/// `AiSettingsInput`: a selection per task and the per-project overrides.
pub fn selections_input(body: &mut Body) -> Option<Selections> {
    let mut issues: Issues = Vec::new();
    let defaults = match body.field("defaults").and_then(Value::as_object) {
        None => {
            issues.push(("defaults".to_owned(), OBJECT));
            None
        }
        Some(d) => {
            let mut pick = |task: &str| selection(d.get(task), &format!("defaults.{task}"), &mut issues);
            let (precheck, report, work) = (pick("precheck"), pick("report"), pick("work"));
            Some((precheck, report, work))
        }
    };
    let overrides = match body.field("projectOverrides").and_then(Value::as_array) {
        None => {
            issues.push(("projectOverrides".to_owned(), LIST));
            None
        }
        Some(items) => {
            let read: Vec<Option<ProjectOverride>> = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let path = format!("projectOverrides.{i}");
                    let Some(fields) = item.as_object() else {
                        issues.push((path, OBJECT));
                        return None;
                    };
                    let project_id = fields.get("projectId").and_then(Value::as_str);
                    if project_id.is_none() {
                        issues.push((format!("{path}.projectId"), TEXT));
                    }
                    let task = fields.get("task").and_then(Value::as_str).filter(|t| TASKS.contains(t));
                    if task.is_none() {
                        issues.push((format!("{path}.task"), TASK));
                    }
                    let selection = selection(fields.get("selection"), &format!("{path}.selection"), &mut issues);
                    Some(ProjectOverride {
                        project_id: project_id?.to_owned(),
                        task: task?.to_owned(),
                        selection: selection?,
                    })
                })
                .collect();
            read.into_iter().collect::<Option<Vec<_>>>()
        }
    };
    let ok = issues.is_empty();
    for (path, message) in issues {
        body.issue(&path, message);
    }
    let (Some((Some(precheck), Some(report), Some(work))), Some(project_overrides)) = (defaults, overrides) else {
        return None;
    };
    ok.then_some(Selections { defaults: TaskSelections { precheck, report, work }, project_overrides })
}

/// `AiPricing`: a checked date, a positive rate and at least one price, each
/// provider/model once (checked once every price is valid, at `models`).
pub fn pricing_input(body: &mut Body) -> Option<Pricing> {
    let as_of = body.required("asOf", DATE, |v| v.as_str().filter(|d| is_iso_date(d)).map(str::to_owned));
    let usd_krw = body.required("usdKrw", RATE, |v| v.as_f64().filter(|r| *r > 0.0));
    let mut issues: Issues = Vec::new();
    let models = match body.field("models").and_then(Value::as_array) {
        Some(items) if !items.is_empty() => {
            let read: Vec<Option<ModelPrice>> = items
                .iter()
                .enumerate()
                .map(|(i, item)| model_price(item, &format!("models.{i}"), &mut issues))
                .collect();
            read.into_iter().collect::<Option<Vec<_>>>()
        }
        _ => {
            issues.push(("models".to_owned(), MODELS));
            None
        }
    };
    for (path, message) in issues {
        body.issue(&path, message);
    }
    let models = models?;
    let mut seen = Vec::new();
    for m in &models {
        let key = (m.provider.as_str(), m.model.as_str());
        if seen.contains(&key) {
            body.issue("models", UNIQUE);
            return None;
        }
        seen.push(key);
    }
    Some(Pricing { as_of: as_of?, usd_krw: usd_krw?, models })
}

fn model_price(item: &Value, path: &str, issues: &mut Issues) -> Option<ModelPrice> {
    let Some(fields) = item.as_object() else {
        issues.push((path.to_owned(), OBJECT));
        return None;
    };
    let mut text = |key: &str, rule: &dyn Fn(&str) -> bool, message: Text| {
        let value = fields.get(key).and_then(Value::as_str).filter(|v| rule(v)).map(str::to_owned);
        if value.is_none() {
            issues.push((format!("{path}.{key}"), message));
        }
        value
    };
    let provider = text("provider", &|p| PROVIDERS.contains(&p), PROVIDER);
    let model = text("model", &|m| !m.is_empty(), NAME);
    let label = text("label", &|l| !l.is_empty(), NAME);
    let mut price = |key: &str| {
        let value = fields.get(key).and_then(Value::as_f64).filter(|p| *p >= 0.0);
        if value.is_none() {
            issues.push((format!("{path}.{key}"), PRICE));
        }
        value
    };
    let (input, cache_read, cache_write, output) =
        (price("inputUsd"), price("cacheReadUsd"), price("cacheWriteUsd"), price("outputUsd"));
    Some(ModelPrice {
        provider: provider?,
        model: model?,
        label: label?,
        input_usd: input?,
        cache_read_usd: cache_read?,
        cache_write_usd: cache_write?,
        output_usd: output?,
    })
}

/// `ProviderKeyInput`: an API key of 8–500 characters after trimming.
pub fn api_key_input(body: &mut Body) -> Option<String> {
    body.required("apiKey", API_KEY, |v| {
        v.as_str().map(str::trim).filter(|k| (8..=500).contains(&utf16_len(k))).map(str::to_owned)
    })
}

/// The settings as the contract's `AiSettings`.
pub fn settings_json(selections: &Selections, ai: &AiSettings) -> Value {
    let mut out = match serde_json::to_value(selections) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    out.insert("providers".into(), Value::Array(connections(ai)));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Secret, TaskKeys};
    use crate::lang::Lang;

    fn body(value: &Value) -> Body {
        Body::parse(value.to_string().as_bytes(), Lang::En).unwrap()
    }

    fn issues(mut body: Body, read: impl FnOnce(&mut Body) -> bool) -> Vec<String> {
        assert!(!read(&mut body));
        body.finish().unwrap_err().body.issues.into_iter().map(|i| i.path).collect()
    }

    #[test]
    fn budgets() {
        let ok = json!({ "monthlyLimitKrw": 500_000, "perRequestLimitKrw": 20_000.0, "alertThresholdPct": 80 });
        let mut b = body(&ok);
        assert_eq!(
            budget_input(&mut b),
            Some(BudgetLimits { monthly_limit_krw: 500_000, per_request_limit_krw: 20_000, alert_threshold_pct: 80 })
        );
        let unlimited_month = json!({ "monthlyLimitKrw": 0, "perRequestLimitKrw": 30_000, "alertThresholdPct": 1 });
        assert!(budget_input(&mut body(&unlimited_month)).is_some(), "0 is no limit");

        let above = json!({ "monthlyLimitKrw": 10_000, "perRequestLimitKrw": 20_000, "alertThresholdPct": 80 });
        assert_eq!(issues(body(&above), |b| budget_input(b).is_some()), ["perRequestLimitKrw"]);
        let bad = json!({ "monthlyLimitKrw": -1, "perRequestLimitKrw": 1.5, "alertThresholdPct": 0 });
        assert_eq!(
            issues(body(&bad), |b| budget_input(b).is_some()),
            ["monthlyLimitKrw", "perRequestLimitKrw", "alertThresholdPct"]
        );
        let too_much =
            json!({ "monthlyLimitKrw": LIMIT_MAX_KRW + 1, "perRequestLimitKrw": 0, "alertThresholdPct": 100 });
        assert_eq!(issues(body(&too_much), |b| budget_input(b).is_some()), ["monthlyLimitKrw"]);
    }

    #[test]
    fn the_per_request_guard() {
        let limits = BudgetLimits { per_request_limit_krw: 20_000, ..BudgetLimits::default() };
        assert!(!limits.request_over_limit(19_999.99));
        assert!(limits.request_over_limit(20_000.0));
        assert!(!BudgetLimits::default().request_over_limit(1e9), "0 is no limit");
    }

    #[test]
    fn selections_resolve_overrides_first() {
        let mut s = Selections::default();
        s.project_overrides.push(ProjectOverride {
            project_id: "p1".into(),
            task: "work".into(),
            selection: Selection::new("openai", "gpt-6.1-sol"),
        });
        assert_eq!(s.resolve("p1", "work").model, "gpt-6.1-sol");
        assert_eq!(s.resolve("p1", "report").model, "claude-opus-5-5");
        assert_eq!(s.resolve("p2", "work").model, "claude-opus-5-5");
        let models: Vec<&str> = s.in_use().iter().map(|s| s.model.as_str()).collect();
        assert_eq!(models, ["claude-haiku-5-5", "claude-opus-5-5", "claude-opus-5-5", "gpt-6.1-sol"]);
        assert_eq!(runtime_for("work", "anthropic"), "claude_code");
        assert_eq!(runtime_for("work", "openai"), "codex");
        assert_eq!(runtime_for("report", "anthropic"), "api");
        assert_eq!(agent_runtime("anthropic"), "claude_code");
        assert_eq!(agent_runtime("openai"), "codex");
    }

    #[test]
    fn selection_bodies() {
        let ok = json!({
            "defaults": { "precheck": { "provider": "anthropic", "model": "claude-haiku-5-5" },
                          "report": { "provider": "anthropic", "model": "claude-opus-5-5" },
                          "work": { "provider": "openai", "model": "gpt-6-luna" } },
            "projectOverrides": [{ "projectId": "p1", "task": "report", "selection": { "provider": "anthropic", "model": "x" } }],
        });
        let s = selections_input(&mut body(&ok)).unwrap();
        assert_eq!(s.defaults.work, Selection::new("openai", "gpt-6-luna"));
        assert_eq!(serde_json::to_value(&s).unwrap(), ok, "round-trips in the contract's shape");

        let bad = json!({
            "defaults": { "precheck": { "provider": "google", "model": "" }, "report": 1 },
            "projectOverrides": [{ "projectId": 5, "task": "review", "selection": {} }],
        });
        assert_eq!(
            issues(body(&bad), |b| selections_input(b).is_some()),
            [
                "defaults.precheck.provider",
                "defaults.precheck.model",
                "defaults.report",
                "defaults.work",
                "projectOverrides.0.projectId",
                "projectOverrides.0.task",
                "projectOverrides.0.selection.provider",
                "projectOverrides.0.selection.model"
            ]
        );
    }

    #[test]
    fn pricing_bodies() {
        let default = serde_json::to_value(Pricing::default()).unwrap();
        assert_eq!(pricing_input(&mut body(&default)), Some(Pricing::default()));

        let mut twice = default.clone();
        twice["models"][1] = twice["models"][0].clone();
        assert_eq!(issues(body(&twice), |b| pricing_input(b).is_some()), ["models"]);
        let bad = json!({ "asOf": "2026-02-30", "usdKrw": 0, "models": [{ "provider": "anthropic", "model": "m", "label": "", "inputUsd": -1, "cacheReadUsd": 0, "cacheWriteUsd": 0 }] });
        assert_eq!(
            issues(body(&bad), |b| pricing_input(b).is_some()),
            ["asOf", "usdKrw", "models.0.label", "models.0.inputUsd", "models.0.outputUsd"]
        );
        assert_eq!(
            issues(body(&json!({ "asOf": "2026-11-12", "usdKrw": 1400, "models": [] })), |b| pricing_input(b)
                .is_some()),
            ["models"]
        );
    }

    #[test]
    fn connections_show_only_a_hint() {
        let mut ai = AiSettings::default();
        assert_eq!(
            connections(&ai),
            [
                json!({ "provider": "anthropic", "connected": false }),
                json!({ "provider": "openai", "connected": false })
            ]
        );
        ai.anthropic = TaskKeys { default: Some(Secret::new("sk-ant-secret-ab12")), ..TaskKeys::default() };
        assert!(!connected(&ai, "anthropic"), "the stub uses no key");
        assert_eq!(connections(&ai)[0], json!({ "provider": "anthropic", "connected": false }));
        ai.provider = "anthropic".to_owned();
        assert_eq!(connections(&ai)[0], json!({ "provider": "anthropic", "connected": true, "keyHint": "ab12" }));
        assert!(connected(&ai, "anthropic"));
        assert!(!connected(&ai, "openai"), "only wired providers connect");
        ai.provider = "stub".to_owned();
        ai.runner_token = Some(Secret::new("r".repeat(32)));
        assert!(connected(&ai, "anthropic"), "a runner uses the keys");
    }
}
