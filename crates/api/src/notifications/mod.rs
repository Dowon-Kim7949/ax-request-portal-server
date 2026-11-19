//! In-app notifications: stored as a message key with parameters, written out
//! in the reader's language when listed (the frontend mock's `notify`,
//! `notifyStatus` and `renderNotification`).

pub mod handlers;

use serde_json::{Map, Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::lang::Lang;
use crate::requests::lifecycle::Status;
use crate::requests::store::{StoredRequest, field};

/// Notifications kept per person; older ones go as new ones arrive.
pub const PER_USER_MAX: i64 = 200;

/// Stores a notification for each recipient (each once, in order), except the
/// person who caused it, and only for those whose settings take it: in-app on,
/// and, with "only what needs me", only notifications that ask for action.
///
/// # Errors
///
/// Database failures.
pub async fn notify(
    tx: &mut Transaction<'_, Postgres>,
    recipients: &[Uuid],
    key: &str,
    params: Value,
    request: Option<i64>,
    action_required: bool,
    except: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    let mut done: Vec<Uuid> = Vec::new();
    for &user in recipients {
        if done.contains(&user) || Some(user) == except {
            continue;
        }
        done.push(user);
        let settings: Option<(bool, bool)> =
            sqlx::query_as("select in_app, only_action_required from notification_settings where user_id = $1")
                .bind(user)
                .fetch_optional(&mut **tx)
                .await?;
        let (in_app, only_action) = settings.unwrap_or((true, false));
        if !in_app || (only_action && !action_required) {
            continue;
        }
        sqlx::query("insert into notifications (user_id, request_id, key, params) values ($1, $2, $3, $4)")
            .bind(user)
            .bind(request)
            .bind(key)
            .bind(&params)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "delete from notifications
              where user_id = $1
                and seq in (select seq from notifications where user_id = $1 order by seq desc offset $2)",
        )
        .bind(user)
        .bind(PER_USER_MAX)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// The notifications a request's new status sends (the mock's `notifyStatus`).
/// `event` is the one that led there; `actor` (if a person) is left out.
///
/// # Errors
///
/// Database failures.
pub async fn notify_status(
    tx: &mut Transaction<'_, Postgres>,
    request: &StoredRequest,
    owners: &[Uuid],
    event: &str,
    actor: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    let devs = request.responsible(owners);
    let requester = [request.requester_id];
    let title = json!({ "title": request.title });
    let d = &request.details;
    let id = Some(request.id);
    let (to, key, params, action): (&[Uuid], &str, Value, bool) = match request.status {
        Status::ScopeCheck => (&requester, "scopeCheck", title, true),
        Status::Review => {
            let key = if event == "approveScope" { "scopeApproved" } else { "reportReady" };
            (&devs, key, title, true)
        }
        Status::InfoNeeded => {
            let questions = field(d, "infoRequest")["questions"]
                .as_array()
                .map(|q| q.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" / "));
            (&requester, "infoNeeded", json!({ "questions": questions.unwrap_or_else(|| request.title.clone()) }), true)
        }
        Status::Reporting if event == "provideInfo" => (&devs, "requesterAnswered", json!({}), false),
        Status::Rejected => {
            let reason = field(d, "rejectionReason").as_str().unwrap_or(&request.title);
            (&requester, "rejected", json!({ "reason": reason }), false)
        }
        Status::InProgress => (&requester, "workStarted", title, false),
        Status::CodeReview => {
            notify(tx, &devs, "workFinished", json!({}), id, true, actor).await?;
            if field(d, "work").get("preview").is_some() {
                notify(tx, &requester, "previewReady", title, id, false, actor).await?;
            }
            return Ok(());
        }
        Status::Verification => {
            let count = field(d, "acceptanceCriteria").as_array().map_or(0, Vec::len);
            (&requester, "pleaseCheck", json!({ "count": count }), true)
        }
        Status::ChangesRequested => (&devs, "issueReported", title, true),
        Status::ReleaseScheduled => (&devs, "scheduleRelease", json!({}), true),
        Status::Done => (&requester, "released", title, false),
        Status::Cancelled => (&devs, "cancelled", title, false),
        Status::Submitted | Status::Reporting => return Ok(()),
    };
    notify(tx, to, key, params, id, action, actor).await
}

/// A notification as the contract's `Notification`, in `lang`.
pub fn render(
    lang: Lang,
    id: Uuid,
    request: Option<i64>,
    key: &str,
    params: &Map<String, Value>,
    created_at: &str,
    read_at: Option<&str>,
) -> Value {
    let (title, body) = text(lang, key, params);
    let mut n = Map::new();
    n.insert("id".into(), json!(id));
    if let Some(request) = request {
        n.insert("requestId".into(), json!(request));
    }
    let title = match request {
        Some(request) => format!("[#{request}] {title}"),
        None => title,
    };
    n.insert("title".into(), json!(title));
    n.insert("body".into(), json!(body));
    n.insert("createdAt".into(), json!(created_at));
    if let Some(read_at) = read_at {
        n.insert("readAt".into(), json!(read_at));
    }
    Value::Object(n)
}

/// `String(params[name])`: a string as is, anything else as JSON writes it.
fn param(params: &Map<String, Value>, name: &str) -> String {
    match params.get(name) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// The mock's `NOTIFICATIONS`, word for word: `(title, body)`.
fn text(lang: Lang, key: &str, params: &Map<String, Value>) -> (String, String) {
    let p = |name: &str| param(params, name);
    let ko = lang == Lang::Ko;
    let (title, body): (&str, String) = match key {
        "reportReady" => {
            (if ko { "구현 확인 보고서가 준비되었습니다" } else { "Feasibility report is ready" }, p("title"))
        }
        "scopeCheck" => (if ko { "진행 범위를 확인해 주세요" } else { "Please confirm the scope" }, p("title")),
        "scopeApproved" => (
            if ko { "요청자가 진행 범위를 확인했습니다" } else { "The requester confirmed the scope" },
            p("title"),
        ),
        "infoNeeded" => (if ko { "추가 정보가 필요합니다" } else { "More information needed" }, p("questions")),
        "requesterAnswered" => (
            if ko { "요청자가 답변했습니다" } else { "The requester answered" },
            (if ko { "보고서를 다시 쓰고 있습니다." } else { "The report is being rewritten." }).into(),
        ),
        "rejected" => (if ko { "진행하지 않기로 했습니다" } else { "This request will not go ahead" }, p("reason")),
        "workStarted" => (if ko { "작업을 시작했습니다" } else { "Work has started" }, p("title")),
        "workFinished" => (
            if ko { "AI 작업이 끝났습니다" } else { "AI work is done" },
            (if ko {
                "변경 내용과 테스트 결과를 검토해 주세요."
            } else {
                "Review the changes and test results."
            })
            .into(),
        ),
        "previewReady" => (
            if ko { "바뀐 화면을 미리 볼 수 있습니다" } else { "You can preview the change" },
            if ko {
                format!("{} \u{2014} 반영 전에 보고 의견을 주시면 먼저 고칠 수 있습니다.", p("title"))
            } else {
                format!("{} \u{2014} tell us what you think before it goes in, and it can be fixed first.", p("title"))
            },
        ),
        "previewGood" => (
            if ko { "요청자가 미리보기를 좋다고 했습니다" } else { "The requester likes the preview" },
            p("title"),
        ),
        "previewChange" => (
            if ko {
                "요청자가 미리보기에서 수정을 요청했습니다"
            } else {
                "The requester asked for a change in the preview"
            },
            p("note"),
        ),
        "pleaseCheck" => (
            if ko { "결과를 확인해 주세요" } else { "Please check the result" },
            if ko {
                format!("완료 기준 {}개를 하나씩 확인해 주시면 됩니다.", p("count"))
            } else {
                format!("Check the {} acceptance criteria one by one.", p("count"))
            },
        ),
        "issueReported" => {
            (if ko { "확인에서 문제가 나왔습니다" } else { "A problem was found in the check" }, p("title"))
        }
        "scheduleRelease" => (
            if ko { "배포 일정을 정해 주세요" } else { "Set a release date" },
            (if ko { "요청자가 결과를 확인했습니다." } else { "The requester has confirmed the result." }).into(),
        ),
        "released" => (if ko { "반영되었습니다" } else { "It is live" }, p("title")),
        "cancelled" => (if ko { "요청이 취소되었습니다" } else { "Request cancelled" }, p("title")),
        "releaseDateSet" => (
            if ko { "배포 예정일이 정해졌습니다" } else { "Release date set" },
            if ko {
                format!("{}에 반영할 예정입니다.", readable_date(lang, &p("date")))
            } else {
                format!("Planned to go live on {}.", readable_date(lang, &p("date")))
            },
        ),
        other => (other, String::new()),
    };
    (title.to_owned(), body)
}

/// A calendar date as people read it, like the mock's `readableDate`:
/// "11월 5일 (목)" / "Thu, Nov 5". Anything but `YYYY-MM-DD` comes back as is;
/// an out-of-range day rolls over, as JavaScript's `Date.UTC` does.
pub fn readable_date(lang: Lang, value: &str) -> String {
    let b = value.as_bytes();
    let well_formed = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if !well_formed {
        return value.to_owned();
    }
    let num = |r: std::ops::Range<usize>| value[r].parse::<i64>().unwrap_or_default();
    let (year, month, day) = (num(0..4), num(5..7), num(8..10));
    // Months roll into years, days into months, as Date.UTC(year, month - 1, day).
    let months = year * 12 + (month - 1);
    let days = days_from_civil(months.div_euclid(12), months.rem_euclid(12) + 1, 1) + day - 1;
    let (_, m, d) = civil_from_days(days);
    let weekday = (days + 4).rem_euclid(7) as usize; // 1970-01-01 was a Thursday
    match lang {
        Lang::Ko => format!("{m}월 {d}일 ({})", ["일", "월", "화", "수", "목", "금", "토"][weekday]),
        Lang::En => {
            let wd = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][weekday];
            let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
                [usize::try_from(m - 1).unwrap_or_default()];
            format!("{wd}, {mon} {d}")
        }
    }
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_dates() {
        assert_eq!(readable_date(Lang::Ko, "2026-11-05"), "11월 5일 (목)");
        assert_eq!(readable_date(Lang::En, "2026-11-05"), "Thu, Nov 5");
        assert_eq!(readable_date(Lang::En, "2028-02-29"), "Tue, Feb 29");
        assert_eq!(readable_date(Lang::Ko, "2027-01-01"), "1월 1일 (금)");
        assert_eq!(readable_date(Lang::En, "2026-02-31"), "Tue, Mar 3", "rolls over like Date.UTC");
        assert_eq!(readable_date(Lang::En, "next week"), "next week");
    }

    #[test]
    fn notifications_read_like_the_mock() {
        let params = |v: Value| v.as_object().unwrap().clone();
        let n = render(Lang::En, Uuid::nil(), Some(7), "pleaseCheck", &params(json!({ "count": 3 })), "t", None);
        assert_eq!(n["title"], "[#7] Please check the result");
        assert_eq!(n["body"], "Check the 3 acceptance criteria one by one.");
        assert!(n.get("readAt").is_none());
        let n = render(
            Lang::Ko,
            Uuid::nil(),
            Some(7),
            "releaseDateSet",
            &params(json!({ "date": "2026-11-05" })),
            "t",
            Some("r"),
        );
        assert_eq!(n["body"], "11월 5일 (목)에 반영할 예정입니다.");
        assert_eq!(n["readAt"], "r");
        let n = render(Lang::Ko, Uuid::nil(), None, "previewReady", &params(json!({ "title": "검색" })), "t", None);
        assert_eq!(n["title"], "바뀐 화면을 미리 볼 수 있습니다");
        assert_eq!(n["body"], "검색 \u{2014} 반영 전에 보고 의견을 주시면 먼저 고칠 수 있습니다.");
    }
}
