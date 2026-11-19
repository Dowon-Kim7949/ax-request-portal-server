//! Advice on a request before it is filed, ported from the frontend's
//! `src/domain/precheck.ts` (`precheckDraft`). Rules only, no model: the
//! same words in, the same advice out, in the request's language. Advice
//! never blocks a request.

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use super::draft::Draft;
use crate::lang::Lang;

/// The contract's `PrecheckIssue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Advice {
    /// A RequestDraft field, e.g. `acceptanceCriteria.0`.
    pub field: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
}

/// Korean phrases that leave the reader guessing (`'좀 '` ends in a space).
const VAGUE_KO: [&str; 9] = ["좀 ", "편하게", "깔끔하게", "알아서", "적당히", "보기 좋게", "예쁘게", "등등", "최대한"];
const VAGUE_EN: [&str; 11] = [
    "somehow",
    "nicer",
    "better",
    "user-friendly",
    "user friendly",
    "cleaner",
    "as soon as possible",
    "etc",
    "and so on",
    "intuitive",
    "easier",
];

struct Patterns {
    vague_en: Vec<(&'static str, Regex)>,
    condition_ko: Regex,
    outcome_ko: Regex,
    condition_en: Regex,
    outcome_en: Regex,
}

/// The frontend's regular expressions. JavaScript's `\b` is an ASCII word
/// boundary, hence `(?-u:\b)`.
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let b = r"(?-u:\b)";
        Patterns {
            vague_en: VAGUE_EN
                .iter()
                .map(|p| {
                    let body = regex::escape(p).replace(r"\-", "-").replace(['-', ' '], r"[-\s]");
                    (*p, Regex::new(&format!("{b}{body}{b}")).expect("a valid pattern"))
                })
                .collect(),
            condition_ko: Regex::new(r"(하면|되면|으면|면\s|면,|\S\s?때|경우|후에)").expect("a valid pattern"),
            outcome_ko: Regex::new(r"(된다|보인다|열린다|나온다|사라진다|바뀐다|이동한다|저장된다|표시된다|한다|있다|없다)\.?$")
                .expect("a valid pattern"),
            condition_en: Regex::new(&format!("(?i){b}(when|if|after|once|whenever|on){b}")).expect("a valid pattern"),
            outcome_en: Regex::new(&format!(
                "(?i){b}(is|are|gets?|shows?|appears?|opens?|closes?|displays?|saves?|is saved|redirects?|returns?|becomes?|changes?|sees?|receives?|disappears?|lists?|downloads?){b}"
            ))
            .expect("a valid pattern"),
        }
    })
}

/// The first vague phrase in `text`: Korean first, then English, in list order.
fn vague_phrase(text: &str) -> Option<&'static str> {
    if let Some(ko) = VAGUE_KO.iter().find(|p| text.contains(*p)) {
        return Some(ko.trim());
    }
    let lower = text.to_lowercase();
    patterns().vague_en.iter().find(|(_, re)| re.is_match(&lower)).map(|(p, _)| *p)
}

/// "When X, Y happens", in Korean or English.
fn looks_like_acceptance_criterion(text: &str) -> bool {
    let p = patterns();
    let t = text.trim();
    (p.condition_ko.is_match(t) && p.outcome_ko.is_match(t)) || (p.condition_en.is_match(t) && p.outcome_en.is_match(t))
}

/// The advice for `draft`, in rule order. `today` is the business calendar's
/// date, `YYYY-MM-DD`.
pub fn advise(draft: &Draft, today: &str, lang: Lang) -> Vec<Advice> {
    let mut out = Vec::new();
    let mut push = |field: String, severity: Severity, message: String| out.push(Advice { field, severity, message });

    if draft.kind != "non_code"
        && draft.target_ids.is_empty()
        && draft.target_other.as_deref().is_none_or(|t| t.trim().is_empty())
    {
        push("targetIds".into(), Severity::Warning, text(lang, Msg::Targets));
    }
    if draft.kind == "bug" {
        if draft.current_behavior.split('\n').filter(|l| !l.trim().is_empty()).count() < 2 {
            push("currentBehavior".into(), Severity::Warning, text(lang, Msg::ReproSteps));
        }
    } else if draft.kind != "new_feature" && draft.kind != "checklist" && draft.current_behavior.trim().is_empty() {
        push("currentBehavior".into(), Severity::Info, text(lang, Msg::Current));
    }
    if let Some(w) = vague_phrase(&draft.desired_behavior) {
        push("desiredBehavior".into(), Severity::Warning, text(lang, Msg::VagueDesired(w)));
    }
    for (i, criterion) in draft.acceptance_criteria.iter().enumerate() {
        let field = format!("acceptanceCriteria.{i}");
        if let Some(w) = vague_phrase(criterion) {
            push(field, Severity::Warning, text(lang, Msg::VagueCriterion(w)));
        } else if !looks_like_acceptance_criterion(criterion) {
            push(field, Severity::Info, text(lang, Msg::CriterionForm));
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for (i, item) in draft.checklist.iter().flatten().enumerate() {
        let r = item.reference.trim();
        if seen.contains(&r) {
            push(format!("checklist.{i}.ref"), Severity::Warning, text(lang, Msg::DuplicateRef(r)));
        }
        seen.push(r);
        if item.expected.as_deref().is_none_or(|e| e.trim().is_empty()) {
            push(format!("checklist.{i}.expected"), Severity::Info, text(lang, Msg::Expected));
        }
    }
    if let Some(date) = draft.desired_date.as_deref() {
        if date < today {
            push("desiredDate".into(), Severity::Warning, text(lang, Msg::PastDate));
        }
        if draft.desired_date_reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
            push("desiredDateReason".into(), Severity::Info, text(lang, Msg::DateReason));
        }
    }
    out
}

enum Msg<'a> {
    Targets,
    ReproSteps,
    Current,
    VagueDesired(&'a str),
    VagueCriterion(&'a str),
    CriterionForm,
    PastDate,
    DateReason,
    DuplicateRef(&'a str),
    Expected,
}

/// The frontend's messages, word for word.
fn text(lang: Lang, msg: Msg<'_>) -> String {
    match (msg, lang) {
        (Msg::Targets, Lang::Ko) => "대상 화면이나 기능을 고르면 AI가 관련 코드를 더 정확하게 찾습니다.".into(),
        (Msg::Targets, Lang::En) => "Picking the screens or features helps the AI find the right code.".into(),
        (Msg::ReproSteps, Lang::Ko) => {
            "버그라면 재현 순서를 한 줄에 하나씩 적어 주세요. (예: 1. 로그인 화면 열기 2. ...)".into()
        }
        (Msg::ReproSteps, Lang::En) => {
            "For a bug, list the steps to reproduce it, one per line (e.g. 1. Open the sign-in screen 2. …).".into()
        }
        (Msg::Current, Lang::Ko) => "지금은 어떻게 동작하는지 적어 주면 무엇을 바꿀지 분명해집니다.".into(),
        (Msg::Current, Lang::En) => "Describing what happens today makes it clear what should change.".into(),
        (Msg::VagueDesired(w), Lang::Ko) => {
            format!("\u{2018}{w}\u{2019} 같은 표현은 사람마다 다르게 읽힙니다. 무엇이 어떻게 되면 좋은지 적어 주세요.")
        }
        (Msg::VagueDesired(w), Lang::En) => {
            format!("Words like \"{w}\" mean different things to different people. Say what should happen.")
        }
        (Msg::VagueCriterion(w), Lang::Ko) => {
            format!("\u{2018}{w}\u{2019}는 확인할 수 있는 기준이 아닙니다. 눈으로 확인할 수 있는 결과로 바꿔 주세요.")
        }
        (Msg::VagueCriterion(w), Lang::En) => {
            format!("\"{w}\" cannot be checked. Describe a result you could see on screen.")
        }
        (Msg::CriterionForm, Lang::Ko) => {
            "\u{2018}~하면 ~된다\u{2019} 형식으로 쓰면 맞는지 하나씩 확인할 수 있습니다.".into()
        }
        (Msg::CriterionForm, Lang::En) => {
            "Writing it as \"when X, Y happens\" makes it easy to check one by one.".into()
        }
        (Msg::PastDate, Lang::Ko) => "희망 일정이 이미 지난 날짜입니다.".into(),
        (Msg::PastDate, Lang::En) => "The wanted-by date is already in the past.".into(),
        (Msg::DateReason, Lang::Ko) => "날짜에 이유가 있으면 우선순위를 정할 때 도움이 됩니다. (예: 고객 약속)".into(),
        (Msg::DateReason, Lang::En) => {
            "A reason for the date helps with prioritising (e.g. a customer commitment).".into()
        }
        (Msg::DuplicateRef(r), Lang::Ko) => {
            format!("\u{2018}{r}\u{2019} 번호가 두 번 나옵니다. 같은 지적이면 하나로 합쳐 주세요.")
        }
        (Msg::DuplicateRef(r), Lang::En) => format!("\"{r}\" appears twice. If it is the same finding, merge them."),
        (Msg::Expected, Lang::Ko) => "어떻게 되면 해결된 것인지 적으면, 나중에 항목별로 확인하기 쉬워집니다.".into(),
        (Msg::Expected, Lang::En) => "Saying what counts as fixed makes each item easy to check later.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests::draft::ChecklistInput;

    fn draft(kind: &str) -> Draft {
        Draft {
            project_id: "p".into(),
            kind: kind.into(),
            title: "Search notices".into(),
            target_ids: vec!["t".into()],
            target_other: None,
            current_behavior: "Notices are hard to find".into(),
            desired_behavior: "Search notices by title".into(),
            acceptance_criteria: vec!["When I type a word, notices with it in the title are shown".into()],
            checklist_source: None,
            checklist: None,
            out_of_scope: None,
            attachment_ids: vec![],
            desired_date: None,
            desired_date_reason: None,
            draft_id: None,
        }
    }

    fn fields(d: &Draft) -> Vec<String> {
        advise(d, "2026-11-19", Lang::En).into_iter().map(|a| a.field).collect()
    }

    #[test]
    fn a_clear_draft_gets_no_advice() {
        assert!(fields(&draft("change")).is_empty());
        let mut ko = draft("change");
        ko.acceptance_criteria = vec!["검색어를 입력하면 제목에 그 단어가 있는 공지만 보인다".into()];
        assert!(fields(&ko).is_empty());
    }

    #[test]
    fn every_rule_in_order() {
        let mut d = draft("bug");
        d.target_ids.clear();
        d.current_behavior = "It breaks".into();
        d.desired_behavior = "Make it user friendly".into();
        d.acceptance_criteria = vec!["좀 빠르게".into(), "Fast".into()];
        d.desired_date = Some("2026-11-18".into());
        assert_eq!(
            fields(&d),
            [
                "targetIds",
                "currentBehavior",
                "desiredBehavior",
                "acceptanceCriteria.0",
                "acceptanceCriteria.1",
                "desiredDate",
                "desiredDateReason"
            ]
        );
        let advice = advise(&d, "2026-11-19", Lang::Ko);
        assert_eq!(
            advice[2].message,
            // "user-friendly" comes first in the list and matches "user friendly" too, as in JavaScript.
            "\u{2018}user-friendly\u{2019} 같은 표현은 사람마다 다르게 읽힙니다. 무엇이 어떻게 되면 좋은지 적어 주세요."
        );
        assert_eq!(
            advice[3].message,
            "\u{2018}좀\u{2019}는 확인할 수 있는 기준이 아닙니다. 눈으로 확인할 수 있는 결과로 바꿔 주세요."
        );
        assert_eq!((advice[4].severity, advice[5].severity), (Severity::Info, Severity::Warning));
    }

    #[test]
    fn vague_words_match_like_javascript() {
        assert_eq!(vague_phrase("Make it user-friendly"), Some("user-friendly"));
        assert_eq!(vague_phrase("Make it user friendly"), Some("user-friendly"), "- and space match each other");
        assert_eq!(vague_phrase("as soon as possible"), Some("as soon as possible"));
        assert_eq!(vague_phrase("logs, etc."), Some("etc"));
        assert_eq!(vague_phrase("Betterment plan"), None, "a word boundary after the phrase");
        assert_eq!(vague_phrase("알아서 해 주세요"), Some("알아서"));
        assert_eq!(vague_phrase("한글better"), Some("better"), "ASCII boundaries: Hangul is not a word character");
    }

    #[test]
    fn checklists_and_the_other_types() {
        let mut d = draft("checklist");
        d.current_behavior = String::new();
        d.acceptance_criteria.clear();
        let item = |r: &str, expected: Option<&str>| ChecklistInput {
            reference: r.into(),
            clause: None,
            finding: "Weak passwords".into(),
            expected: expected.map(Into::into),
        };
        d.checklist = Some(vec![item("F-1", Some("12 characters")), item(" F-1 ", None)]);
        assert_eq!(fields(&d), ["checklist.1.ref", "checklist.1.expected"]);

        let mut n = draft("non_code");
        n.target_ids.clear();
        n.current_behavior = " ".into();
        assert_eq!(fields(&n), ["currentBehavior"]);
        let mut f = draft("new_feature");
        f.current_behavior = String::new();
        assert!(fields(&f).is_empty());
    }
}
