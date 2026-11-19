//! Saved drafts: half-written requests a person keeps on the server to finish
//! later, on this device or another (`listDrafts`, `createDraft`, `getDraft`,
//! `updateDraft`, `deleteDraft`), with the frontend mock API's rules.
//!
//! A draft's `content` is the contract's `DraftContent`: every field of a
//! request optional, empty values allowed, only abuse ceilings checked. It is
//! kept as given (unknown fields dropped, as zod drops them) in one JSONB
//! column.

pub mod handlers;

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::attachments::{Attachment, Files};
use crate::body::{Body, Text, integer, utf16_len};
use crate::requests::draft::TYPES;

/// The contract's `DRAFT_STEP_MAX`: the wizard's last step.
pub const STEP_MAX: i64 = 6;
/// The contract's `DRAFT_TEXT_MAX`, in UTF-16 units.
pub const TEXT_MAX: usize = 10_000;
/// The contract's `DRAFT_LIST_MAX`.
pub const LIST_MAX: usize = 50;

/// Marks a draft id (the contract allows `[A-Za-z0-9_-]{1,100}`, never `local-…`).
pub const ID_PREFIX: &str = "drf_";

const TEXT_KEYS: [&str; 9] = [
    "projectId",
    "targetOther",
    "title",
    "currentBehavior",
    "desiredBehavior",
    "checklistSource",
    "outOfScope",
    "desiredDate",
    "desiredDateReason",
];
const CHECKLIST_KEYS: [&str; 4] = ["ref", "clause", "finding", "expected"];

const STEP: Text = ("단계는 0–6 사이의 정수여야 합니다.", "Use a step from 0 to 6.");
const CONTENT: Text = ("초안 내용을 확인해 주세요.", "Check the draft's content.");
const TEXT: Text = ("10,000자 이하의 글자로 입력해 주세요.", "Use text of 10,000 characters or fewer.");
const LIST: Text = ("50개 이하의 목록으로 보내 주세요.", "Send a list of 50 items or fewer.");
const ITEM: Text = ("항목을 확인해 주세요.", "Check this item.");
const TYPE: Text = ("요청 유형을 골라 주세요.", "Choose a request type.");
const CLIENT_ID: Text = ("clientId는 1–100자여야 합니다.", "clientId must be 1 to 100 characters.");
const BASE_VERSION: Text = ("기준 버전을 확인해 주세요.", "Check the base version.");

/// `step`: an integer from 0 to [`STEP_MAX`].
pub fn step(body: &mut Body) -> Option<i64> {
    body.required("step", STEP, |v| integer(v).filter(|s| (0..=STEP_MAX).contains(s)))
}

/// `clientId` of `createDraft`: absent, or 1–100 characters.
pub fn client_id(body: &mut Body) -> Option<String> {
    body.optional("clientId", CLIENT_ID, |v| {
        v.as_str().filter(|s| (1..=100).contains(&utf16_len(s))).map(str::to_owned)
    })
}

/// `baseVersion` of `updateDraft`: a positive integer.
pub fn base_version(body: &mut Body) -> Option<i64> {
    body.required("baseVersion", BASE_VERSION, |v| integer(v).filter(|n| *n > 0))
}

/// `content`, checked like the contract's `DraftContent`: each problem is
/// recorded at its path (`content.title`, `content.checklist.2.ref`), and the
/// content comes back with only the contract's fields.
pub fn content(body: &mut Body) -> Option<Map<String, Value>> {
    let Some(Value::Object(raw)) = body.field("content").cloned() else {
        body.issue("content", CONTENT);
        return None;
    };
    let mut issues: Vec<(String, Text)> = Vec::new();
    let mut kept = Map::new();
    let text = |v: &Value| v.as_str().is_some_and(|s| utf16_len(s) <= TEXT_MAX);

    for key in TEXT_KEYS {
        if let Some(value) = raw.get(key) {
            if text(value) {
                kept.insert(key.to_owned(), value.clone());
            } else {
                issues.push((format!("content.{key}"), TEXT));
            }
        }
    }
    if let Some(value) = raw.get("type") {
        if value.as_str().is_some_and(|t| t.is_empty() || TYPES.contains(&t)) {
            kept.insert("type".to_owned(), value.clone());
        } else {
            issues.push(("content.type".to_owned(), TYPE));
        }
    }
    for key in ["targetIds", "attachmentIds"] {
        if let Some(list) = list_of(&raw, key, &mut issues, |item, path, issues| {
            if text(item) {
                Some(item.clone())
            } else {
                issues.push((path.to_owned(), TEXT));
                None
            }
        }) {
            kept.insert(key.to_owned(), list);
        }
    }
    if let Some(list) = list_of(&raw, "acceptanceCriteria", &mut issues, |item, path, issues| {
        object_of(item, &["text"], path, issues, text)
    }) {
        kept.insert("acceptanceCriteria".to_owned(), list);
    }
    if let Some(list) = list_of(&raw, "checklist", &mut issues, |item, path, issues| {
        object_of(item, &CHECKLIST_KEYS, path, issues, text)
    }) {
        kept.insert("checklist".to_owned(), list);
    }

    let ok = issues.is_empty();
    for (path, message) in issues {
        body.issue(&path, message);
    }
    ok.then_some(kept)
}

type Issues = Vec<(String, Text)>;

/// A list field: absent is fine; otherwise an array of at most [`LIST_MAX`]
/// items, each read by `item` (which records its own problems).
fn list_of(
    raw: &Map<String, Value>,
    key: &str,
    issues: &mut Issues,
    item: impl Fn(&Value, &str, &mut Issues) -> Option<Value>,
) -> Option<Value> {
    let value = raw.get(key)?;
    let path = format!("content.{key}");
    let Some(items) = value.as_array().filter(|items| items.len() <= LIST_MAX) else {
        issues.push((path, LIST));
        return None;
    };
    let read: Vec<Option<Value>> =
        items.iter().enumerate().map(|(i, value)| item(value, &format!("{path}.{i}"), issues)).collect();
    read.into_iter().collect::<Option<Vec<_>>>().map(Value::Array)
}

/// An object with exactly the string fields `keys`, each required.
fn object_of(
    item: &Value,
    keys: &[&str],
    path: &str,
    issues: &mut Issues,
    text: impl Fn(&Value) -> bool,
) -> Option<Value> {
    let Some(fields) = item.as_object() else {
        issues.push((path.to_owned(), ITEM));
        return None;
    };
    let mut kept = Map::new();
    let mut ok = true;
    for key in keys {
        match fields.get(*key) {
            Some(value) if text(value) => {
                kept.insert((*key).to_owned(), value.clone());
            }
            _ => {
                issues.push((format!("{path}.{key}"), TEXT));
                ok = false;
            }
        }
    }
    ok.then_some(Value::Object(kept))
}

/// A draft as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDraft {
    pub id: String,
    pub step: i32,
    pub content: Map<String, Value>,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl StoredDraft {
    /// `content.attachmentIds`, as stored.
    pub fn attachment_ids(&self) -> Vec<String> {
        self.content
            .get("attachmentIds")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// The contract's `SavedDraftSummary`. `attachments` are the caller's
    /// existing uploads among the draft's ids (see [`StoredDraft::attachment_ids`]).
    pub fn summary(&self, attachment_count: usize) -> Value {
        let text = |key: &str| self.content.get(key).and_then(Value::as_str).unwrap_or_default();
        let mut s = Map::new();
        s.insert("id".into(), json!(self.id));
        // JavaScript's trim also takes the byte order mark.
        s.insert("title".into(), json!(text("title").trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')));
        for key in ["projectId", "type"] {
            if !text(key).is_empty() {
                s.insert(key.into(), json!(text(key)));
            }
        }
        s.insert("step".into(), json!(self.step));
        s.insert("attachmentCount".into(), json!(attachment_count));
        s.insert("version".into(), json!(self.version));
        s.insert("createdAt".into(), json!(self.created_at));
        s.insert("updatedAt".into(), json!(self.updated_at));
        Value::Object(s)
    }

    /// The contract's `SavedDraft`, with fresh attachment links.
    pub fn full(&self, attachments: &[Attachment], files: &Files) -> Value {
        let mut draft = self.summary(attachments.len());
        draft["content"] = Value::Object(self.content.clone());
        draft["attachments"] = attachments.iter().map(|a| files.present(&a.to_json())).collect();
        draft
    }
}

/// How many of `ids` are in `existing`, each counted once (the mock's `attachmentsOf`).
pub fn count_existing(ids: &[String], existing: &HashSet<String>) -> usize {
    let mut seen = HashSet::new();
    ids.iter().filter(|id| existing.contains(*id) && seen.insert(id.as_str())).count()
}

#[cfg(test)]
mod tests {
    use crate::lang::Lang;

    use super::*;

    fn parse(body: &Value) -> Result<Map<String, Value>, Vec<String>> {
        let mut b = Body::parse(body.to_string().as_bytes(), Lang::En).unwrap();
        let content = content(&mut b);
        match b.finish() {
            Ok(()) => Ok(content.unwrap()),
            Err(error) => Err(error.body.issues.into_iter().map(|i| i.path).collect()),
        }
    }

    #[test]
    fn anything_half_written_is_kept_as_given() {
        let body = json!({ "content": {
            "title": "  half ", "type": "", "targetIds": [], "acceptanceCriteria": [{ "text": "", "extra": 1 }],
            "checklist": [{ "ref": "", "clause": "", "finding": "", "expected": "" }], "unknown": true,
        }});
        let kept = parse(&body).unwrap();
        assert_eq!(kept["title"], "  half ", "not trimmed");
        assert_eq!(kept["acceptanceCriteria"], json!([{ "text": "" }]), "unknown fields dropped");
        assert!(kept.get("unknown").is_none());
        assert_eq!(parse(&json!({ "content": {} })).unwrap(), Map::new());
    }

    #[test]
    fn only_abuse_ceilings_apply() {
        let long = "x".repeat(TEXT_MAX + 1);
        let body = json!({ "content": {
            "title": long, "type": "epic", "targetIds": vec!["t"; LIST_MAX + 1], "attachmentIds": ["a", 5],
            "acceptanceCriteria": [{ "text": "ok" }, "no"], "checklist": [{ "ref": "r", "clause": "c", "finding": "f" }],
            "desiredDate": null,
        }});
        assert_eq!(
            parse(&body).unwrap_err(),
            [
                "content.title",
                "content.desiredDate",
                "content.type",
                "content.targetIds",
                "content.attachmentIds.1",
                "content.acceptanceCriteria.1",
                "content.checklist.0.expected"
            ]
        );
        assert_eq!(parse(&json!({ "content": [] })).unwrap_err(), ["content"]);
        assert_eq!(parse(&json!({})).unwrap_err(), ["content"]);
        assert!(parse(&json!({ "content": { "title": "가".repeat(TEXT_MAX) } })).is_ok());
    }

    #[test]
    fn summaries() {
        let draft = StoredDraft {
            id: "drf_1".into(),
            step: 2,
            content: json!({ "title": "\u{feff} Search \n", "projectId": "", "type": "bug" })
                .as_object()
                .unwrap()
                .clone(),
            version: 3,
            created_at: "2026-11-19T00:00:00.000Z".into(),
            updated_at: "2026-11-19T01:00:00.000Z".into(),
        };
        let s = draft.summary(1);
        assert_eq!(s["title"], "Search");
        assert!(s.get("projectId").is_none(), "left out until one is picked");
        assert_eq!(s["type"], "bug");
        assert_eq!(
            (s["step"].as_i64(), s["attachmentCount"].as_i64(), s["version"].as_i64()),
            (Some(2), Some(1), Some(3))
        );
    }

    #[test]
    fn existing_attachments_count_once() {
        let existing: HashSet<String> = ["a".to_owned(), "b".to_owned()].into();
        let ids: Vec<String> = ["a", "x", "a", "b"].map(str::to_owned).to_vec();
        assert_eq!(count_existing(&ids, &existing), 2);
    }
}
