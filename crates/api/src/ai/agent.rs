//! What a coding agent reports, made into the contract's shapes. The runner
//! (`ax-api runner`) clones the project, runs the agent, and posts its answer;
//! the server decides what that answer may claim.
//!
//! A report backed by the code and the wiki may say `feasible` or
//! `conditional`, under the frontend's rules (`src/domain/report.ts`,
//! `src/mocks/ai.ts`): without wiki evidence it is `undetermined`; wiki trust
//! comes from the review state of the documents read, never from the model;
//! and open questions, weak trust or a rule conflict make it at most
//! `conditional`. A work result is checked field by field before it reaches
//! the request.

use serde_json::{Map, Value, json};

use super::claude::{report_from, screens, texts};
use super::{ReportResult, WorkResult};
use crate::lang::Lang;
use crate::projects::Project;
use crate::requests::store::{StoredRequest, field};

const WIKI_DOCS_MAX: usize = 10;
const CODE_FILES_MAX: usize = 20;
const PLAN_MAX: usize = 10;
const SCOPE_ITEMS_MAX: usize = 5;
const SCOPE_QUESTIONS_MAX: usize = 3;
const SCOPE_OPTIONS: std::ops::RangeInclusive<usize> = 2..=4;

/// The frontend's `deriveWikiTrust`: all documents reviewed is `high`, at
/// least half is `medium`, otherwise (or none) `low`.
pub fn wiki_trust(docs: &[Value]) -> &'static str {
    let verified = docs.iter().filter(|d| d["verified"] == true).count();
    if docs.is_empty() {
        "low"
    } else if verified == docs.len() {
        "high"
    } else if verified * 2 >= docs.len() {
        "medium"
    } else {
        "low"
    }
}

/// The wiki documents the agent read: `{ path, title, verified }`, each path
/// once. `verified` is the runner's reading of the document's review state.
fn wiki_docs(answer: &Value) -> Vec<Value> {
    let mut docs: Vec<Value> = Vec::new();
    for doc in answer["wikiDocs"].as_array().into_iter().flatten() {
        let Some(path) = doc["path"].as_str().map(str::trim).filter(|p| !p.is_empty()) else { continue };
        if docs.iter().any(|d| d["path"] == path) || docs.len() >= WIKI_DOCS_MAX {
            continue;
        }
        let title = doc["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(path);
        docs.push(json!({ "path": path, "title": title, "verified": doc["verified"] == true }));
    }
    docs
}

/// The feasibility report from an agent that read the code and the wiki.
pub fn report(
    answer: &Value,
    request: &StoredRequest,
    project: Option<&Project>,
    lang: Lang,
    model: &str,
) -> ReportResult {
    let mut report = report_from(answer, request, project, lang, model);
    if request.kind == "non_code" {
        return ReportResult { report, scope: None };
    }
    let docs = wiki_docs(answer);
    let trust = wiki_trust(&docs);
    let conflict = report["constraintChecks"].as_array().into_iter().flatten().any(|c| c["conflict"] == true);
    let open = report["openQuestions"].as_array().is_some_and(|q| !q.is_empty());
    let verdict = match answer["verdict"].as_str() {
        _ if docs.is_empty() => "undetermined",
        Some("feasible") if !open && trust != "low" && !conflict => "feasible",
        Some("feasible" | "conditional") => "conditional",
        _ => "undetermined",
    };
    let backed = matches!(verdict, "feasible" | "conditional");
    report["verdict"] = json!(verdict);
    report["evidence"] = json!({ "wikiDocs": docs, "codeFiles": texts(&answer["codeFiles"], CODE_FILES_MAX) });
    report["wikiTrust"] = json!(trust);
    report["plan"] = if backed { json!(texts(&answer["plan"], PLAN_MAX)) } else { json!([]) };
    let scope = if backed { scope(&answer["scope"]) } else { None };
    ReportResult { report, scope }
}

/// The contract's `ScopeProposal` (without `revision`): the agent's reading in
/// the requester's words, with ids the server assigns. `None` without a summary.
fn scope(answer: &Value) -> Option<Value> {
    let summary = answer["summary"].as_str().map(str::trim).filter(|s| !s.is_empty())?;
    let questions: Vec<Value> = answer["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| {
            let text = q["text"].as_str().map(str::trim).filter(|t| !t.is_empty())?;
            let labels = texts(&q["options"], *SCOPE_OPTIONS.end());
            SCOPE_OPTIONS.contains(&labels.len()).then_some((text, labels, q["recommended"].as_u64()))
        })
        .take(SCOPE_QUESTIONS_MAX)
        .enumerate()
        .map(|(i, (text, labels, recommended))| {
            let options: Vec<Value> = labels
                .iter()
                .enumerate()
                .map(|(j, label)| json!({ "id": format!("q{}o{}", i + 1, j + 1), "label": label }))
                .collect();
            let mut question = json!({ "id": format!("q{}", i + 1), "text": text, "options": options });
            if let Some(pick) = recommended.and_then(|r| usize::try_from(r).ok()).filter(|r| *r < labels.len()) {
                question["recommended"] = json!(format!("q{}o{}", i + 1, pick + 1));
            }
            question
        })
        .collect();
    Some(json!({
        "summary": summary,
        "changes": texts(&answer["changes"], SCOPE_ITEMS_MAX),
        "unchanged": texts(&answer["unchanged"], SCOPE_ITEMS_MAX),
        "questions": questions,
    }))
}

fn count(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| *n <= 9_007_199_254_740_991)
}

/// The contract's `WorkRun` result from an agent run, and the regression
/// checks it leaves behind (the frontend mock's: one per screen the report
/// says to re-check, pending).
pub fn work(answer: &Value, request: &StoredRequest, project: Option<&Project>, lang: Lang) -> WorkResult {
    let outcome =
        answer["outcome"].as_str().filter(|o| ["success", "tests_failed", "error"].contains(o)).unwrap_or("error");
    let summary = answer["summary"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| lang.pick("AI 작업이 끝났습니다.", "The AI work run has finished.").to_owned(), str::to_owned);
    let mut extra = Map::new();
    let mr = &answer["mergeRequest"];
    let url = mr["url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://"));
    if let (Some(number), Some(url)) = (count(&mr["number"]).filter(|n| *n > 0), url) {
        extra.insert(
            "mergeRequest".into(),
            json!({
                "number": number,
                "url": url,
                "filesChanged": count(&mr["filesChanged"]).unwrap_or(0),
                "additions": count(&mr["additions"]).unwrap_or(0),
                "deletions": count(&mr["deletions"]).unwrap_or(0),
            }),
        );
    }
    let t = &answer["tests"];
    if let (Some(ep), Some(et), Some(cp), Some(ct)) = (
        count(&t["existingPassed"]),
        count(&t["existingTotal"]),
        count(&t["criteriaPassed"]),
        count(&t["criteriaTotal"]),
    ) {
        extra.insert(
            "tests".into(),
            json!({ "existingPassed": ep.min(et), "existingTotal": et, "criteriaPassed": cp.min(ct), "criteriaTotal": ct }),
        );
    }
    let docs = texts(&answer["wikiDocs"], WIKI_DOCS_MAX);
    if !docs.is_empty() {
        // The AI never approves its own wiki edits: a person reviews them.
        extra.insert("wikiChanges".into(), json!({ "docs": docs, "pendingReview": true }));
    }

    let regression = if extra.contains_key("mergeRequest") { regression(request, project) } else { Vec::new() };
    WorkResult { outcome: outcome.to_owned(), summary, extra, regression }
}

/// One pending check per screen the report says to re-check, with the wiki
/// document it came from (or the wiki's index).
fn regression(request: &StoredRequest, project: Option<&Project>) -> Vec<Value> {
    let report = field(&request.details, "report");
    let wiki = project.map_or("docs/llm-wiki", |p| p.wiki_path.as_str());
    let docs = report["evidence"]["wikiDocs"].as_array().cloned().unwrap_or_default();
    let names = screens(project);
    report["impact"]["recheck"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|label| names.is_empty() || names.iter().any(|(_, n)| n == label))
        .enumerate()
        .map(|(i, label)| {
            let source = docs
                .get(i)
                .and_then(|d| d["path"].as_str())
                .map_or_else(|| format!("{wiki}/index.md"), str::to_owned);
            json!({ "id": format!("rg-{}-{}", request.id, i + 1), "label": label, "source": source, "result": "pending" })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_trust_follows_review_state() {
        let doc = |v: bool| json!({ "verified": v });
        assert_eq!(wiki_trust(&[]), "low");
        assert_eq!(wiki_trust(&[doc(true), doc(true)]), "high");
        assert_eq!(wiki_trust(&[doc(true), doc(false)]), "medium");
        assert_eq!(wiki_trust(&[doc(true), doc(false), doc(false)]), "low");
    }

    #[test]
    fn scopes_get_server_ids_and_limits() {
        let answer = json!({
            "summary": " A search box above the list ",
            "changes": ["A search box", "", "A search box"],
            "unchanged": ["The list order"],
            "questions": [
                { "text": "Where?", "options": ["Top", "Side"], "recommended": 1 },
                { "text": "Only one option", "options": ["Yes"] },
                { "text": "Case?", "options": ["Ignore", "Match"], "recommended": 7 },
            ],
        });
        let s = scope(&answer).unwrap();
        assert_eq!(s["summary"], "A search box above the list");
        assert_eq!(s["changes"], json!(["A search box"]));
        assert_eq!(s["questions"][0]["recommended"], "q1o2");
        assert_eq!(s["questions"][1]["id"], "q2", "the one-option question was dropped");
        assert!(s["questions"][1].get("recommended").is_none(), "an out-of-range pick is dropped");
        assert_eq!(scope(&json!({ "summary": "" })), None);
    }
}
