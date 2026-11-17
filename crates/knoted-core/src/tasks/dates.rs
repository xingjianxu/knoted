//! Shanghai calendar deadlines and validation; original DateValue JSON stays intact.
use super::service::{DateValue, DomainError, Task, TaskStatus};
use chrono::{DateTime, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};

/// Validate unique IANA local mappings and the final start/due interval.
/// Date-only starts use local midnight; date-only deadlines use the next
/// calendar day's local midnight. Invalid, skipped or repeated times are errors.
pub fn validate_dates(
    start: Option<&DateValue>,
    due: Option<&DateValue>,
) -> Result<(), DomainError> {
    let start = start.map(|d| date_boundary(d, false)).transpose()?;
    let due = due.map(|d| date_boundary(d, true)).transpose()?;
    if start.zip(due).is_some_and(|(s, d)| s > d) {
        return Err(DomainError::InvalidField("start/due"));
    }
    Ok(())
}

/// Derive overdue at an injected instant (normally `ctx.now()`). No due date or
/// a terminal status always yields false. Invalid legacy dates return an error,
/// including ambiguous/skipped local mappings; no offset or fold is guessed.
pub fn is_overdue(task: &Task, now: DateTime<Utc>) -> Result<bool, DomainError> {
    if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
        return Ok(false);
    }
    match task.due.as_ref() {
        None => Ok(false),
        Some(due) => Ok(now >= date_boundary(due, true)?),
    }
}

fn date_boundary(d: &DateValue, due: bool) -> Result<DateTime<Utc>, DomainError> {
    let invalid = || DomainError::InvalidField("start/due");
    if d.timezone != "Asia/Shanghai"
        || d.date.len() != 10
        || !d.date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Err(invalid());
    }
    let mut date = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").map_err(|_| invalid())?;
    let time = if let Some(s) = &d.time {
        if s.len() != 5
            || !s.bytes().enumerate().all(|(i, b)| {
                if i == 2 {
                    b == b':'
                } else {
                    b.is_ascii_digit()
                }
            })
        {
            return Err(invalid());
        }
        NaiveTime::parse_from_str(s, "%H:%M").map_err(|_| invalid())?
    } else {
        if due {
            date = date.succ_opt().ok_or_else(invalid)?;
        }
        NaiveTime::MIN
    };
    match chrono_tz::Asia::Shanghai.from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(instant) => Ok(instant.with_timezone(&Utc)),
        LocalResult::Ambiguous(_, _) | LocalResult::None => Err(invalid()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Clock, FixedClock};
    use crate::tasks::service::TaskPriority;
    use chrono::Duration;
    use uuid::Uuid;

    fn date(d: &str, time: Option<&str>) -> DateValue {
        DateValue {
            date: d.into(),
            time: time.map(Into::into),
            timezone: "Asia/Shanghai".into(),
        }
    }
    fn task(due: Option<DateValue>, status: TaskStatus) -> Task {
        let now = "2026-11-17T00:00:00Z".parse().unwrap();
        Task {
            id: Uuid::nil(),
            project_id: None,
            parent_id: None,
            title: "test".into(),
            description: None,
            status,
            priority: TaskPriority::None,
            start: None,
            due,
            version: 1,
            archived_at: None,
            deleted_at: None,
            archive_batch_id: None,
            created_at: now,
            updated_at: now,
        }
    }
    // Catches UTC midnight, strict >, fixed-now or fixed-duration date-only deadlines.
    #[test]
    fn t35_1_date_only_midnight_boundary() {
        let d = date("2026-11-17", None);
        let t = task(Some(d.clone()), TaskStatus::Todo);
        let clock = FixedClock::new("2026-11-17T15:59:59.999999Z".parse().unwrap());
        assert!(!is_overdue(&t, clock.now()).unwrap());
        clock.advance(Duration::microseconds(1));
        assert!(is_overdue(&t, clock.now()).unwrap());
        clock.advance(Duration::hours(1));
        assert!(is_overdue(&t, clock.now()).unwrap());
        assert_eq!(t.due, Some(d));
        assert!(serde_json::to_value(&t).unwrap().get("overdue").is_none());
        // PRC spring DST makes the next two local midnights only 23h apart.
        for (day, cutoff) in [
            ("1991-04-13", "1991-04-13T16:00:00Z"),
            ("1991-04-14", "1991-04-14T15:00:00Z"),
        ] {
            let t = task(Some(date(day, None)), TaskStatus::Todo);
            let cutoff: DateTime<Utc> = cutoff.parse().unwrap();
            assert!(!is_overdue(&t, cutoff - Duration::microseconds(1)).unwrap());
            assert!(is_overdue(&t, cutoff).unwrap());
        }
    }
    // Catches date-only handling being applied to an explicitly timed deadline.
    #[test]
    fn t35_exact_time_boundary() {
        let t = task(
            Some(date("2026-11-17", Some("10:30"))),
            TaskStatus::InProgress,
        );
        for (now, want) in [
            ("2026-11-17T02:29:59Z", false),
            ("2026-11-17T02:30:00Z", true),
            ("2026-11-17T02:30:01Z", true),
        ] {
            assert_eq!(is_overdue(&t, now.parse().unwrap()).unwrap(), want);
        }
    }
    // Catches treating a start as a deadline or terminal statuses as overdue.
    #[test]
    fn t35_2_no_due_and_terminal() {
        let now = "2026-11-17T00:00:00Z".parse().unwrap();
        for status in [
            TaskStatus::Todo,
            TaskStatus::InProgress,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            let mut t = task(None, status);
            t.start = Some(date("2020-01-01", None));
            assert!(!is_overdue(&t, now).unwrap());
            t.due = Some(date("2020-01-01", None));
            t.archived_at = Some(now);
            t.deleted_at = Some(now);
            assert_eq!(
                is_overdue(&t, now).unwrap(),
                matches!(status, TaskStatus::Todo | TaskStatus::InProgress)
            );
            if matches!(status, TaskStatus::Done | TaskStatus::Cancelled) {
                t.due = Some(date("bad legacy value", None));
                assert!(!is_overdue(&t, now).unwrap());
            }
        }
    }
    // Catches hardcoded +08, guessing a DST fold/gap, or hiding legacy errors.
    #[test]
    fn t35_real_iana_history_and_invalid_local_time() {
        let d = date("1991-07-01", Some("12:00"));
        assert_eq!(
            date_boundary(&d, true).unwrap(),
            "1991-07-01T03:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        let t = task(Some(d), TaskStatus::Todo);
        assert!(!is_overdue(&t, "1991-07-01T02:59:59Z".parse().unwrap()).unwrap());
        assert!(is_overdue(&t, "1991-07-01T03:00:00Z".parse().unwrap()).unwrap());
        for (d, time) in [("1991-04-14", "02:30"), ("1991-09-15", "01:30")] {
            let d = date(d, Some(time));
            assert!(matches!(
                validate_dates(Some(&d), None),
                Err(DomainError::InvalidField("start/due"))
            ));
            assert!(matches!(
                validate_dates(None, Some(&d)),
                Err(DomainError::InvalidField("start/due"))
            ));
            assert!(
                is_overdue(
                    &task(Some(d), TaskStatus::Todo),
                    "2026-11-17T00:00:00Z".parse().unwrap()
                )
                .is_err()
            );
        }
        let invalid = task(Some(date("2026-02-30", None)), TaskStatus::InProgress);
        assert!(is_overdue(&invalid, "2026-11-17T00:00:00Z".parse().unwrap()).is_err());
    }
    // Existing T34 interval semantics remain after conversion into UTC.
    #[test]
    fn t35_existing_date_order_regression() {
        let midnight = date("2026-11-17", None);
        let next = date("2026-11-18", None);
        let exact = date("2026-11-17", Some("10:30"));
        assert!(validate_dates(Some(&midnight), Some(&midnight)).is_ok());
        assert!(validate_dates(Some(&exact), Some(&exact)).is_ok());
        assert!(validate_dates(Some(&next), Some(&exact)).is_err());
        assert!(validate_dates(Some(&next), Some(&midnight)).is_ok());
    }
}
