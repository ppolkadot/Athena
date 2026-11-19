//! The local-midnight deadline the app schedules itself, so date-sensitive hints (the goal
//! contribution plan, closings) refresh on their own. It never depends on wallpaper playback or
//! the platform preference poll.

use std::time::Duration;

use chrono::{DateTime, TimeZone};

/// Longest wait before the date is checked again, so a changed system clock is noticed.
pub const MAX_WAIT: Duration = Duration::from_mins(30);

/// Time from `now` until the first instant of the next local day. A midnight skipped by a DST
/// change resolves to the first valid hour after it; an ambiguous midnight resolves to the earlier
/// instant. Never zero, so the caller always makes progress.
#[must_use]
pub fn until_next_midnight<Tz: TimeZone>(now: &DateTime<Tz>) -> Duration {
    let tz = now.timezone();
    let Some(tomorrow) = now.date_naive().succ_opt() else {
        return MAX_WAIT;
    };
    for hour in 0..6 {
        let Some(naive) = tomorrow.and_hms_opt(hour, 0, 0) else {
            break;
        };
        if let Some(start) = tz.from_local_datetime(&naive).earliest() {
            return (start - now.clone())
                .to_std()
                .unwrap_or(Duration::from_secs(1))
                .max(Duration::from_secs(1));
        }
    }
    MAX_WAIT
}

/// How long to sleep before re-reading the date: until midnight (plus a second of slack), capped so
/// clock changes are noticed.
#[must_use]
pub fn next_date_check<Tz: TimeZone>(now: &DateTime<Tz>) -> Duration {
    (until_next_midnight(now) + Duration::from_secs(1)).min(MAX_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate};

    fn at(hour: u32, minute: u32) -> DateTime<FixedOffset> {
        let offset = FixedOffset::west_opt(3 * 3600).unwrap();
        offset
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 11, 19)
                    .unwrap()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn midnight_is_the_start_of_the_next_local_day() {
        assert_eq!(until_next_midnight(&at(23, 30)), Duration::from_mins(30));
        assert_eq!(until_next_midnight(&at(12, 0)), Duration::from_hours(12));
        assert_eq!(until_next_midnight(&at(0, 0)), Duration::from_hours(24));
    }

    #[test]
    fn the_check_is_capped_so_a_changed_clock_is_noticed() {
        assert_eq!(next_date_check(&at(23, 59)), Duration::from_secs(61));
        assert_eq!(next_date_check(&at(10, 0)), MAX_WAIT);
    }
}
