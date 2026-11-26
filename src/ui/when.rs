//! Dates as the list and the info table show them.

use chrono::{Datelike as _, Local, NaiveDate, NaiveDateTime, TimeZone as _};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Age {
    Today,
    Yesterday,
    ThisWeek,
    ThisMonth,
    Older,
}

impl Age {
    pub fn of(day: NaiveDate, today: NaiveDate) -> Self {
        let days = (today - day).num_days();
        // A clock set back can leave entries in the future.
        if days <= 0 {
            Self::Today
        } else if days == 1 {
            Self::Yesterday
        } else if day.iso_week() == today.iso_week() {
            Self::ThisWeek
        } else if (day.year(), day.month()) == (today.year(), today.month()) {
            Self::ThisMonth
        } else {
            Self::Older
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::ThisWeek => "This Week",
            Self::ThisMonth => "This Month",
            Self::Older => "Older",
        }
    }
}

/// Milliseconds since the Unix epoch, in local time.
pub fn local(millis: i64) -> NaiveDateTime {
    Local
        .timestamp_millis_opt(millis)
        .earliest()
        .unwrap_or_default()
        .naive_local()
}

pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// For example "Today at 14:32" or "3 Nov 2026 at 09:05".
pub fn describe(then: NaiveDateTime, now: NaiveDateTime) -> String {
    let day = match Age::of(then.date(), now.date()) {
        age @ (Age::Today | Age::Yesterday) => age.title().to_owned(),
        _ => then.format("%-d %b %Y").to_string(),
    };
    format!("{day} at {}", then.format("%H:%M"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn groups_days_relative_to_today() {
        // A Wednesday.
        let today = day(11, 25);
        for (date, age) in [
            (day(11, 26), Age::Today),
            (day(11, 25), Age::Today),
            (day(11, 24), Age::Yesterday),
            (day(11, 23), Age::ThisWeek),
            (day(11, 22), Age::ThisMonth),
            (day(11, 1), Age::ThisMonth),
            (day(10, 31), Age::Older),
        ] {
            assert_eq!(Age::of(date, today), age, "{date}");
        }
        // The week wins over the month at a month boundary.
        assert_eq!(Age::of(day(11, 30), day(12, 2)), Age::ThisWeek);
    }

    #[test]
    fn describes_recent_days_by_name() {
        let at = |date: NaiveDate| date.and_hms_opt(9, 5, 0).unwrap();
        let now = at(day(11, 25));
        assert_eq!(describe(now, now), "Today at 09:05");
        assert_eq!(describe(at(day(11, 24)), now), "Yesterday at 09:05");
        assert_eq!(describe(at(day(11, 3)), now), "3 Nov 2026 at 09:05");
    }
}
