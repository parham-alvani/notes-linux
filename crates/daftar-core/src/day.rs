//! A calendar day with no time and no zone, as the vault writes it.

use std::fmt;

use chrono::{Datelike, Local, Months, NaiveDate};

/// `2026-10-25`. Months are added as the Tasks plugin adds them: the 31st
/// of January and one month is the last day of February.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day(NaiveDate);

impl Day {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Day> {
        NaiveDate::from_ymd_opt(year, month, day).map(Day)
    }

    /// A date written `YYYY-MM-DD`, and only that; a day that does not
    /// exist, as the 30th of February, is `None`.
    pub fn parse(text: &str) -> Option<Day> {
        let text = crate::text::trim(text);
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<u32> {
            let part = &text[range];
            part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok()).flatten()
        };
        Day::new(digits(0..4)? as i32, digits(5..7)?, digits(8..10)?)
    }

    /// Today on this computer's clock and zone.
    pub fn today() -> Day {
        Day(Local::now().date_naive())
    }

    pub fn year(self) -> i32 {
        self.0.year()
    }

    pub fn month(self) -> u32 {
        self.0.month()
    }

    pub fn day(self) -> u32 {
        self.0.day()
    }

    pub fn date(self) -> NaiveDate {
        self.0
    }

    pub fn adding_days(self, days: i64) -> Day {
        Day(self.0 + chrono::Duration::days(days))
    }

    pub fn adding_months(self, months: i32) -> Day {
        let date = if months >= 0 {
            self.0.checked_add_months(Months::new(months as u32))
        } else {
            self.0.checked_sub_months(Months::new(months.unsigned_abs()))
        };
        Day(date.expect("a date in range"))
    }

    /// Whole days from this day to [other]; negative when [other] is earlier.
    pub fn days_to(self, other: Day) -> i64 {
        (other.0 - self.0).num_days()
    }
}

impl From<NaiveDate> for Day {
    fn from(date: NaiveDate) -> Day {
        Day(date)
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year(), self.month(), self.day())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_arithmetic() {
        assert_eq!(Day::parse("2026-02-30"), None);
        assert_eq!(Day::parse("2026-1-05"), None);
        assert_eq!(Day::parse("2026-10-25").unwrap().adding_days(7).to_string(), "2026-11-01");
        assert_eq!(Day::parse("2026-01-31").unwrap().adding_months(1).to_string(), "2026-02-28");
        assert_eq!(Day::parse("2026-10-01").unwrap().days_to(Day::parse("2026-10-08").unwrap()), 7);
    }
}
