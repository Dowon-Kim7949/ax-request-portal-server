//! The contract's `RequestDraft` / `RequestSubmission`, checked like their zod
//! schemas: field by field (lengths in UTF-16 units, trimmed where zod trims),
//! then, only when every field passed, the `superRefine` rules about
//! checklists and criteria.

use serde_json::Value;

use crate::body::{Body, Text, utf16_len};
use crate::error::ApiError;
use crate::lang::Lang;

pub const TYPES: [&str; 6] = ["new_feature", "change", "bug", "copy_design", "non_code", "checklist"];
pub const CRITERIA_MAX: usize = 5;
pub const CHECKLIST_MAX: usize = 30;

/// A request as written, before the server gives it an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub project_id: String,
    /// `type`
    pub kind: String,
    /// Trimmed
    pub title: String,
    pub target_ids: Vec<String>,
    pub target_other: Option<String>,
    /// Trimmed
    pub current_behavior: String,
    /// Trimmed
    pub desired_behavior: String,
    /// Criterion texts, trimmed
    pub acceptance_criteria: Vec<String>,
    pub checklist_source: Option<String>,
    pub checklist: Option<Vec<ChecklistInput>>,
    pub out_of_scope: Option<String>,
    pub attachment_ids: Vec<String>,
    /// `YYYY-MM-DD`
    pub desired_date: Option<String>,
    pub desired_date_reason: Option<String>,
    /// `RequestSubmission` only
    pub draft_id: Option<String>,
}

/// The contract's `ChecklistItemInput`, trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistInput {
    /// `ref`
    pub reference: String,
    pub clause: Option<String>,
    pub finding: String,
    pub expected: Option<String>,
}

const PROJECT: Text = ("프로젝트를 골라 주세요.", "Choose a project.");
const TYPE: Text = ("요청 유형을 골라 주세요.", "Choose a request type.");
const TITLE: Text = ("제목은 2–120자로 입력해 주세요.", "Write a title of 2 to 120 characters.");
const TARGETS: Text = ("대상 목록을 확인해 주세요.", "Check the targets.");
const MAX_200: Text = ("200자 이하로 입력해 주세요.", "Use 200 characters or fewer.");
const CURRENT: Text = ("현재 동작은 2000자 이하로 입력해 주세요.", "Use 2,000 characters or fewer.");
const DESIRED: Text =
    ("바라는 동작을 2000자 이하로 입력해 주세요.", "Describe what should happen in up to 2,000 characters.");
const CRITERIA: Text = ("완료 기준은 5개까지입니다.", "Up to 5 criteria.");
const CRITERION: Text = ("완료 기준은 1–300자로 입력해 주세요.", "Write each criterion in 1 to 300 characters.");
const MAX_100: Text = ("100자 이하로 입력해 주세요.", "Use 100 characters or fewer.");
const CHECKLIST: Text = ("체크리스트 항목은 30개까지입니다.", "Up to 30 checklist items.");
const CHECKLIST_ITEM: Text = ("체크리스트 항목을 확인해 주세요.", "Check this checklist item.");
const REF: Text = ("지적 번호는 1–60자로 입력해 주세요.", "Use 1 to 60 characters for the number.");
const MAX_60: Text = ("60자 이하로 입력해 주세요.", "Use 60 characters or fewer.");
const FINDING: Text = ("지적 내용은 1–500자로 입력해 주세요.", "Describe the finding in 1 to 500 characters.");
const MAX_300: Text = ("300자 이하로 입력해 주세요.", "Use 300 characters or fewer.");
const MAX_1000: Text = ("1000자 이하로 입력해 주세요.", "Use 1,000 characters or fewer.");
const ATTACHMENTS: Text = ("첨부 목록을 확인해 주세요.", "Check the attachments.");
const DATE: Text = ("날짜 형식을 확인해 주세요. (예: 2026-11-30)", "Use a date like 2026-11-30.");
const DRAFT_ID: Text = ("임시 저장 id를 확인해 주세요.", "Check the draft id.");
const NEED_CHECKLIST: Text = ("체크리스트 항목을 하나 이상 입력해 주세요.", "At least one checklist item");
const NEED_CRITERION: Text = ("완료 기준을 하나 이상 입력해 주세요.", "At least one criterion");
const ONLY_CHECKLIST: Text = ("체크리스트는 체크리스트 요청에서만 씁니다.", "Only for checklist requests");

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

/// A string of at most `max` UTF-16 units, untouched.
fn up_to(max: usize) -> impl Fn(&Value) -> Option<String> {
    move |v| v.as_str().filter(|s| utf16_len(s) <= max).map(str::to_owned)
}

/// A string trimmed, then `min..=max` UTF-16 units long.
fn trimmed_between(min: usize, max: usize) -> impl Fn(&Value) -> Option<String> {
    move |v| {
        let t = v.as_str()?.trim();
        (min..=max).contains(&utf16_len(t)).then(|| t.to_owned())
    }
}

fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array()?.iter().map(|s| s.as_str().map(str::to_owned)).collect()
}

/// `YYYY-MM-DD` naming a real day (zod's `z.iso.date()`).
pub fn is_iso_date(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        b[r].iter().try_fold(0u32, |n, c| c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0')))
    };
    let (Some(y), Some(m), Some(d)) = (digits(0..4), digits(5..7), digits(8..10)) else { return false };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

impl Draft {
    /// Reads a `RequestSubmission` (`with_draft_id`) or a `RequestDraft`.
    ///
    /// # Errors
    ///
    /// `400` (`checkInput`) with every field issue, or else the `superRefine` ones.
    pub fn parse(bytes: &[u8], lang: Lang, with_draft_id: bool) -> Result<Self, ApiError> {
        let mut body = Body::parse(bytes, lang)?;
        let project_id =
            body.required("projectId", PROJECT, |v| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned));
        let kind = body.required("type", TYPE, |v| v.as_str().filter(|t| TYPES.contains(t)).map(str::to_owned));
        let title = body.required("title", TITLE, trimmed_between(2, 120));
        let target_ids = body.required("targetIds", TARGETS, strings);
        let target_other = body.optional("targetOther", MAX_200, up_to(200));
        let current_behavior = body.required("currentBehavior", CURRENT, trimmed_between(0, 2000));
        let desired_behavior = body.required("desiredBehavior", DESIRED, trimmed_between(1, 2000));
        let acceptance_criteria = criteria(&mut body);
        let checklist_source = body.optional("checklistSource", MAX_100, trimmed_between(0, 100));
        let checklist = checklist(&mut body);
        let out_of_scope = body.optional("outOfScope", MAX_1000, up_to(1000));
        let attachment_ids = body.required("attachmentIds", ATTACHMENTS, strings);
        let desired_date =
            body.optional("desiredDate", DATE, |v| v.as_str().filter(|d| is_iso_date(d)).map(str::to_owned));
        let desired_date_reason = body.optional("desiredDateReason", MAX_300, up_to(300));
        let draft_id = if with_draft_id { body.optional("draftId", DRAFT_ID, text) } else { None };

        let (
            Some(project_id),
            Some(kind),
            Some(title),
            Some(target_ids),
            Some(current_behavior),
            Some(desired_behavior),
            Some(acceptance_criteria),
            Some(checklist),
            Some(attachment_ids),
        ) = (
            project_id,
            kind,
            title,
            target_ids,
            current_behavior,
            desired_behavior,
            acceptance_criteria,
            checklist,
            attachment_ids,
        )
        else {
            body.finish()?;
            unreachable!("finish() fails when a required field is missing")
        };
        body.finish()?;

        // zod runs superRefine only once every field passed.
        let mut refine = Body::parse(b"{}", lang)?;
        if kind == "checklist" {
            if checklist.as_ref().is_none_or(Vec::is_empty) {
                refine.issue("checklist", NEED_CHECKLIST);
            }
        } else {
            if acceptance_criteria.is_empty() {
                refine.issue("acceptanceCriteria", NEED_CRITERION);
            }
            if checklist.as_ref().is_some_and(|c| !c.is_empty()) {
                refine.issue("checklist", ONLY_CHECKLIST);
            }
        }
        refine.finish()?;

        Ok(Self {
            project_id,
            kind,
            title,
            target_ids,
            target_other,
            current_behavior,
            desired_behavior,
            acceptance_criteria,
            checklist_source,
            checklist,
            out_of_scope,
            attachment_ids,
            desired_date,
            desired_date_reason,
            draft_id,
        })
    }
}

/// `acceptanceCriteria: [{ text }]`, at most 5; `None` after recording issues.
fn criteria(body: &mut Body) -> Option<Vec<String>> {
    let Some(list) = body.field("acceptanceCriteria").and_then(Value::as_array).cloned() else {
        body.issue("acceptanceCriteria", CRITERIA);
        return None;
    };
    if list.len() > CRITERIA_MAX {
        body.issue("acceptanceCriteria", CRITERIA);
        return None;
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, c) in list.iter().enumerate() {
        match c.get("text").and_then(trimmed_between(1, 300)) {
            Some(t) if c.is_object() => out.push(t),
            _ => body.issue(&format!("acceptanceCriteria.{i}.text"), CRITERION),
        }
    }
    (out.len() == list.len()).then_some(out)
}

/// `checklist` (optional): `Some(None)` when absent, `None` after recording issues.
fn checklist(body: &mut Body) -> Option<Option<Vec<ChecklistInput>>> {
    let Some(raw) = body.field("checklist").cloned() else { return Some(None) };
    let Some(list) = raw.as_array() else {
        body.issue("checklist", CHECKLIST);
        return None;
    };
    if list.len() > CHECKLIST_MAX {
        body.issue("checklist", CHECKLIST);
        return None;
    }
    let mut out = Vec::with_capacity(list.len());
    let mut ok = true;
    for (i, item) in list.iter().enumerate() {
        if !item.is_object() {
            body.issue(&format!("checklist.{i}"), CHECKLIST_ITEM);
            ok = false;
            continue;
        }
        let optional = |key: &str, max: usize| match item.get(key) {
            None => Some(None),
            Some(v) => trimmed_between(0, max)(v).map(Some),
        };
        let reference = item.get("ref").and_then(trimmed_between(1, 60));
        let clause = optional("clause", 60);
        let finding = item.get("finding").and_then(trimmed_between(1, 500));
        let expected = optional("expected", 300);
        for (field, valid, message) in [
            ("ref", reference.is_some(), REF),
            ("clause", clause.is_some(), MAX_60),
            ("finding", finding.is_some(), FINDING),
            ("expected", expected.is_some(), MAX_300),
        ] {
            if !valid {
                body.issue(&format!("checklist.{i}.{field}"), message);
            }
        }
        match (reference, clause, finding, expected) {
            (Some(reference), Some(clause), Some(finding), Some(expected)) => {
                out.push(ChecklistInput { reference, clause, finding, expected });
            }
            _ => ok = false,
        }
    }
    ok.then_some(Some(out))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn valid() -> Value {
        json!({
            "projectId": "p", "type": "change", "title": "  Search notices ", "targetIds": ["t"],
            "currentBehavior": " hard to find ", "desiredBehavior": "Search by title",
            "acceptanceCriteria": [{ "text": " When I type, it shows " }], "attachmentIds": [],
            "outOfScope": " keep ", "desiredDate": "2026-12-01", "draftId": "d-1"
        })
    }

    fn paths(body: &Value, with_draft_id: bool) -> Vec<String> {
        Draft::parse(body.to_string().as_bytes(), Lang::En, with_draft_id)
            .unwrap_err()
            .body
            .issues
            .into_iter()
            .map(|i| i.path)
            .collect()
    }

    #[test]
    fn reads_and_trims_like_zod() {
        let d = Draft::parse(valid().to_string().as_bytes(), Lang::En, true).unwrap();
        assert_eq!(d.title, "Search notices");
        assert_eq!(d.current_behavior, "hard to find");
        assert_eq!(d.acceptance_criteria, ["When I type, it shows"]);
        assert_eq!(d.out_of_scope.as_deref(), Some(" keep "), "outOfScope is not trimmed");
        assert_eq!(d.draft_id.as_deref(), Some("d-1"));
        assert_eq!(Draft::parse(valid().to_string().as_bytes(), Lang::En, false).unwrap().draft_id, None);
    }

    #[test]
    fn field_issues_come_before_the_refinements() {
        let mut body = valid();
        body["title"] = json!(" x ");
        body["acceptanceCriteria"] = json!([]);
        body["desiredDate"] = json!("2026-02-30");
        body["targetOther"] = Value::Null;
        // The empty criteria list is a refinement, so it waits for the fields.
        assert_eq!(paths(&body, true), ["title", "targetOther", "desiredDate"]);

        let mut body = valid();
        body["acceptanceCriteria"] = json!([]);
        body["checklist"] = json!([{ "ref": "F-1", "finding": "weak" }]);
        assert_eq!(paths(&body, true), ["acceptanceCriteria", "checklist"]);

        let mut body = valid();
        body["type"] = json!("checklist");
        assert_eq!(paths(&body, true), ["checklist"]);
    }

    #[test]
    fn checklist_items() {
        let mut body = valid();
        body["type"] = json!("checklist");
        body["checklist"] = json!([
            { "ref": " F-1 ", "finding": " weak ", "clause": " ", "expected": " 12 chars " },
            { "ref": "", "finding": "x".repeat(501) }
        ]);
        assert_eq!(paths(&body, true), ["checklist.1.ref", "checklist.1.finding"]);
        body["checklist"] = json!([{ "ref": " F-1 ", "finding": " weak ", "clause": " ", "expected": " 12 chars " }]);
        let d = Draft::parse(body.to_string().as_bytes(), Lang::En, true).unwrap();
        let item = &d.checklist.unwrap()[0];
        assert_eq!(
            (item.reference.as_str(), item.clause.as_deref(), item.expected.as_deref()),
            ("F-1", Some(""), Some("12 chars"))
        );
    }

    #[test]
    fn iso_dates() {
        for ok in ["2026-11-30", "2028-02-29", "2000-02-29"] {
            assert!(is_iso_date(ok), "{ok}");
        }
        for bad in ["2026-02-29", "1900-02-29", "2026-13-01", "2026-1-01", "2026-11-31", "20261130", ""] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }
}
