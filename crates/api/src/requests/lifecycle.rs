//! The request lifecycle: statuses, events and who may send them, ported from
//! the frontend's `src/domain/lifecycle.ts`.
//!
//! The table-shaped part (statuses, transitions, guards by event, turns and
//! stages) is checked against `contract/lifecycle.json`, which the frontend
//! generates from the same source and the contract sync pins with
//! `openapi.json`. A change there fails the tests here until the port follows.

use serde::{Deserialize, Serialize};

/// The contract's `RequestStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Submitted,
    Reporting,
    ScopeCheck,
    Review,
    InfoNeeded,
    Rejected,
    InProgress,
    CodeReview,
    Verification,
    ChangesRequested,
    ReleaseScheduled,
    Done,
    Cancelled,
}

/// Who sends an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Party {
    Requester,
    Developer,
    Ai,
    System,
}

/// Whose move a status is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Requester,
    Developer,
    Ai,
    None,
}

/// The seven progress stages; a cancelled request has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Submitted,
    Report,
    Review,
    Work,
    Verification,
    Release,
    Done,
}

/// A lifecycle event (the frontend's `TRANSITIONS` keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Event {
    StartReport,
    ReportReady,
    ScopeReady,
    ApproveScope,
    ReviseScope,
    RequestInfo,
    ProvideInfo,
    Reject,
    RunWork,
    WorkFinished,
    RequestRevision,
    ApproveMerge,
    ReportIssue,
    ConfirmResult,
    MarkReleased,
    Cancel,
}

/// One row of the transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub from: &'static [Status],
    pub to: Status,
    pub by: Party,
}

/// A rule a transition must keep besides the status (`guard_failed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    CriteriaAllPassed,
    CriteriaOneFailed,
    AssumptionsUnchecked,
    ScopeUnanswered,
    ScopeRevisionsUsed,
    ReleaseUnscheduled,
}

/// How often the requester may send the scope back for rewriting.
pub const SCOPE_REVISIONS_MAX: usize = 2;

impl Status {
    pub const ALL: [Self; 13] = [
        Self::Submitted,
        Self::Reporting,
        Self::ScopeCheck,
        Self::Review,
        Self::InfoNeeded,
        Self::Rejected,
        Self::InProgress,
        Self::CodeReview,
        Self::Verification,
        Self::ChangesRequested,
        Self::ReleaseScheduled,
        Self::Done,
        Self::Cancelled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Reporting => "reporting",
            Self::ScopeCheck => "scope_check",
            Self::Review => "review",
            Self::InfoNeeded => "info_needed",
            Self::Rejected => "rejected",
            Self::InProgress => "in_progress",
            Self::CodeReview => "code_review",
            Self::Verification => "verification",
            Self::ChangesRequested => "changes_requested",
            Self::ReleaseScheduled => "release_scheduled",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// Done, rejected or cancelled: nothing happens to the request any more.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Rejected | Self::Cancelled)
    }

    pub const fn turn(self) -> Turn {
        match self {
            Self::Submitted | Self::Reporting | Self::InProgress => Turn::Ai,
            Self::ScopeCheck | Self::InfoNeeded | Self::Verification => Turn::Requester,
            Self::Review | Self::CodeReview | Self::ChangesRequested | Self::ReleaseScheduled => Turn::Developer,
            Self::Rejected | Self::Done | Self::Cancelled => Turn::None,
        }
    }

    pub const fn stage(self) -> Option<Stage> {
        Some(match self {
            Self::Submitted => Stage::Submitted,
            Self::Reporting => Stage::Report,
            Self::ScopeCheck | Self::Review | Self::InfoNeeded | Self::Rejected => Stage::Review,
            Self::InProgress | Self::CodeReview | Self::ChangesRequested => Stage::Work,
            Self::Verification => Stage::Verification,
            Self::ReleaseScheduled => Stage::Release,
            Self::Done => Stage::Done,
            Self::Cancelled => return None,
        })
    }
}

impl Event {
    pub const ALL: [Self; 16] = [
        Self::StartReport,
        Self::ReportReady,
        Self::ScopeReady,
        Self::ApproveScope,
        Self::ReviseScope,
        Self::RequestInfo,
        Self::ProvideInfo,
        Self::Reject,
        Self::RunWork,
        Self::WorkFinished,
        Self::RequestRevision,
        Self::ApproveMerge,
        Self::ReportIssue,
        Self::ConfirmResult,
        Self::MarkReleased,
        Self::Cancel,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StartReport => "startReport",
            Self::ReportReady => "reportReady",
            Self::ScopeReady => "scopeReady",
            Self::ApproveScope => "approveScope",
            Self::ReviseScope => "reviseScope",
            Self::RequestInfo => "requestInfo",
            Self::ProvideInfo => "provideInfo",
            Self::Reject => "reject",
            Self::RunWork => "runWork",
            Self::WorkFinished => "workFinished",
            Self::RequestRevision => "requestRevision",
            Self::ApproveMerge => "approveMerge",
            Self::ReportIssue => "reportIssue",
            Self::ConfirmResult => "confirmResult",
            Self::MarkReleased => "markReleased",
            Self::Cancel => "cancel",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == value)
    }

    /// The frontend's `TRANSITIONS`.
    pub const fn rule(self) -> Rule {
        use Status::*;
        let (from, to, by): (&'static [Status], Status, Party) = match self {
            Self::StartReport => (&[Submitted], Reporting, Party::System),
            Self::ReportReady => (&[Reporting], Review, Party::Ai),
            Self::ScopeReady => (&[Reporting], ScopeCheck, Party::Ai),
            Self::ApproveScope => (&[ScopeCheck], Review, Party::Requester),
            Self::ReviseScope => (&[ScopeCheck], Reporting, Party::Requester),
            Self::RequestInfo => (&[Review], InfoNeeded, Party::Developer),
            Self::ProvideInfo => (&[InfoNeeded], Reporting, Party::Requester),
            Self::Reject => (&[Review, InfoNeeded], Rejected, Party::Developer),
            Self::RunWork => (&[Review, ChangesRequested], InProgress, Party::Developer),
            Self::WorkFinished => (&[InProgress], CodeReview, Party::Ai),
            Self::RequestRevision => (&[CodeReview], InProgress, Party::Developer),
            Self::ApproveMerge => (&[CodeReview], Verification, Party::Developer),
            Self::ReportIssue => (&[Verification], ChangesRequested, Party::Requester),
            Self::ConfirmResult => (&[Verification], ReleaseScheduled, Party::Requester),
            Self::MarkReleased => (&[ReleaseScheduled], Done, Party::Developer),
            Self::Cancel => (&[Submitted, Reporting, ScopeCheck, Review, InfoNeeded], Cancelled, Party::Requester),
        };
        Rule { from, to, by }
    }

    /// Sent by a person through `transitionRequest`. The AI's and the system's
    /// events never come from the API, so the AI cannot step past a person.
    pub const fn is_user_event(self) -> bool {
        matches!(self.rule().by, Party::Requester | Party::Developer)
    }

    /// Whether the event may leave `status`.
    pub fn allowed_from(self, status: Status) -> bool {
        self.rule().from.contains(&status)
    }
}

impl Guard {
    pub const ALL: [Self; 6] = [
        Self::CriteriaAllPassed,
        Self::CriteriaOneFailed,
        Self::AssumptionsUnchecked,
        Self::ScopeUnanswered,
        Self::ScopeRevisionsUsed,
        Self::ReleaseUnscheduled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CriteriaAllPassed => "criteriaAllPassed",
            Self::CriteriaOneFailed => "criteriaOneFailed",
            Self::AssumptionsUnchecked => "assumptionsUnchecked",
            Self::ScopeUnanswered => "scopeUnanswered",
            Self::ScopeRevisionsUsed => "scopeRevisionsUsed",
            Self::ReleaseUnscheduled => "releaseUnscheduled",
        }
    }

    /// The event whose transition the guard checks (the frontend's `GUARD_EVENTS`).
    pub const fn event(self) -> Event {
        match self {
            Self::CriteriaAllPassed => Event::ConfirmResult,
            Self::CriteriaOneFailed => Event::ReportIssue,
            Self::AssumptionsUnchecked => Event::RunWork,
            Self::ScopeUnanswered => Event::ApproveScope,
            Self::ScopeRevisionsUsed => Event::ReviseScope,
            Self::ReleaseUnscheduled => Event::MarkReleased,
        }
    }
}

impl Party {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requester => "requester",
            Self::Developer => "developer",
            Self::Ai => "ai",
            Self::System => "system",
        }
    }
}

impl Turn {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requester => "requester",
            Self::Developer => "developer",
            Self::Ai => "ai",
            Self::None => "none",
        }
    }
}

impl Stage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Report => "report",
            Self::Review => "review",
            Self::Work => "work",
            Self::Verification => "verification",
            Self::Release => "release",
            Self::Done => "done",
        }
    }
}

/// The table checked against `contract/lifecycle.json`, entry by entry.
#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use super::*;

    fn doc() -> &'static Value {
        ax_contract::lifecycle()
    }

    fn names(values: impl IntoIterator<Item = &'static str>) -> Value {
        Value::Array(values.into_iter().map(Value::from).collect())
    }

    #[test]
    fn statuses_and_terminal_ones() {
        assert_eq!(doc()["statuses"], names(Status::ALL.map(Status::as_str)));
        let mut ours: Vec<&str> = Status::ALL.into_iter().filter(|s| s.is_terminal()).map(Status::as_str).collect();
        let mut theirs: Vec<&str> = doc()["terminal"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
        ours.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(ours, theirs);
        for status in Status::ALL {
            assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
            assert_eq!(Status::parse(status.as_str()), Some(status));
        }
    }

    #[test]
    fn transitions() {
        let mut ours = Map::new();
        for event in Event::ALL {
            let rule = event.rule();
            ours.insert(
                event.as_str().to_owned(),
                json!({
                    "from": names(rule.from.iter().map(|s| s.as_str())),
                    "to": rule.to.as_str(),
                    "by": rule.by.as_str(),
                }),
            );
            assert_eq!(serde_json::to_value(event).unwrap(), event.as_str());
            assert_eq!(Event::parse(event.as_str()), Some(event));
        }
        assert_eq!(doc()["transitions"], Value::Object(ours), "TRANSITIONS drifted from lifecycle.json");
    }

    #[test]
    fn user_events_guards_and_limits() {
        let user = Event::ALL.into_iter().filter(|e| e.is_user_event()).map(Event::as_str);
        let mut ours: Vec<&str> = user.collect();
        let mut theirs: Vec<&str> =
            doc()["userEvents"].as_array().unwrap().iter().map(|e| e.as_str().unwrap()).collect();
        ours.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(ours, theirs);

        let guards: Map<String, Value> =
            Guard::ALL.iter().map(|g| (g.as_str().to_owned(), Value::from(g.event().as_str()))).collect();
        assert_eq!(doc()["guards"], Value::Object(guards));
        assert_eq!(doc()["scopeRevisionsMax"], SCOPE_REVISIONS_MAX);
    }

    #[test]
    fn turns_and_stages() {
        let turns: Map<String, Value> =
            Status::ALL.iter().map(|s| (s.as_str().to_owned(), Value::from(s.turn().as_str()))).collect();
        assert_eq!(doc()["turn"], Value::Object(turns));
        let stages: Map<String, Value> = Status::ALL
            .iter()
            .map(|s| (s.as_str().to_owned(), s.stage().map_or(Value::Null, |st| Value::from(st.as_str()))))
            .collect();
        assert_eq!(doc()["stage"], Value::Object(stages));
    }

    #[test]
    fn the_ai_never_sends_a_persons_event() {
        for event in [Event::StartReport, Event::ReportReady, Event::ScopeReady, Event::WorkFinished] {
            assert!(!event.is_user_event(), "{}", event.as_str());
        }
        assert!(Event::ApproveMerge.is_user_event());
        assert!(Event::RunWork.allowed_from(Status::ChangesRequested));
        assert!(!Event::ApproveMerge.allowed_from(Status::InProgress));
    }
}
