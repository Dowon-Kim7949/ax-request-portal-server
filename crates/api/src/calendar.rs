//! Calendar dates (`YYYY-MM-DD`) as plain day arithmetic, with no time zone:
//! the business calendar's "today" comes from PostgreSQL (`AX_TIME_ZONE`),
//! and these work on the dates it gives.

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01: `(year, month, day)`.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `(year, month, day)` of a `YYYY-MM-DD`, without checking the day exists.
pub fn parts(date: &str) -> Option<(i64, i64, i64)> {
    let b = date.as_bytes();
    let digits = b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !digits {
        return None;
    }
    Some((date[0..4].parse().ok()?, date[5..7].parse().ok()?, date[8..10].parse().ok()?))
}

/// `date` plus `n` days (the frontend mock's `addDays`).
pub fn add_days(date: &str, n: i64) -> Option<String> {
    let (y, m, d) = parts(date)?;
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) + n);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// The month before `YYYY-MM`.
pub fn previous_month(month: &str) -> Option<String> {
    let (y, m) = (month.get(0..4)?.parse::<i64>().ok()?, month.get(5..7)?.parse::<i64>().ok()?);
    Some(if m == 1 { format!("{:04}-12", y - 1) } else { format!("{y:04}-{:02}", m - 1) })
}

/// A contract `YearMonth`: `YYYY-MM` with a month 01–12.
pub fn is_year_month(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || c.is_ascii_digit())
        && value[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_arithmetic() {
        assert_eq!(add_days("2026-11-28", 7).as_deref(), Some("2026-12-05"));
        assert_eq!(add_days("2028-02-28", 1).as_deref(), Some("2028-02-29"));
        assert_eq!(add_days("2026-01-01", -1).as_deref(), Some("2025-12-31"));
        assert_eq!(previous_month("2026-01").as_deref(), Some("2025-12"));
        assert_eq!(previous_month("2026-11").as_deref(), Some("2026-10"));
    }

    #[test]
    fn year_months() {
        assert!(is_year_month("2026-11"));
        for bad in ["2026-13", "2026-00", "2026-1", "26-11", "abcd-ef", "2026-11-01"] {
            assert!(!is_year_month(bad), "{bad}");
        }
    }
}
