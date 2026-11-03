//! A month as the day picker draws it, in the Gregorian calendar or in the
//! Jalali one (`docs/specs/calendar-events.md`): its title, the names of its
//! weekdays, and its days in weeks. A Gregorian week starts on Monday, as
//! Spain's does; a Jalali one on Saturday, as Iran's does.

use chrono::Datelike;

use crate::day::Day;
use crate::jalali::{self, Jalali};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Calendar {
    Gregorian,
    Jalali,
}

/// One month of a calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month {
    pub calendar: Calendar,
    pub year: i32,
    pub month: u32,
}

const GREGORIAN_WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const JALALI_WEEKDAYS: [&str; 7] = ["ش", "ی", "د", "س", "چ", "پ", "ج"];

impl Month {
    /// The month [day] is in.
    pub fn of(calendar: Calendar, day: Day) -> Month {
        match calendar {
            Calendar::Gregorian => Month { calendar, year: day.year(), month: day.month() },
            Calendar::Jalali => {
                let jalali = Jalali::from_day(day);
                Month { calendar, year: jalali.year, month: jalali.month }
            }
        }
    }

    /// [count] months on, or back when negative.
    #[must_use]
    pub fn adding(self, count: i32) -> Month {
        let index = self.year * 12 + self.month as i32 - 1 + count;
        Month { calendar: self.calendar, year: index.div_euclid(12), month: index.rem_euclid(12) as u32 + 1 }
    }

    /// `November 2026`, or `آبان ۱۴۰۵`.
    pub fn title(self) -> String {
        match self.calendar {
            Calendar::Gregorian => self.first().map(|d| d.date().format("%B %Y").to_string()).unwrap_or_default(),
            Calendar::Jalali => jalali::persian_digits(&format!("{} {}", jalali::MONTHS[(self.month as usize - 1) % 12], self.year)),
        }
    }

    /// The weekdays' short names, from the first of the week.
    pub fn weekdays(self) -> [&'static str; 7] {
        match self.calendar {
            Calendar::Gregorian => GREGORIAN_WEEKDAYS,
            Calendar::Jalali => JALALI_WEEKDAYS,
        }
    }

    /// The day of the month as the calendar writes it: `5`, or `۵`.
    pub fn number(self, day: Day) -> String {
        match self.calendar {
            Calendar::Gregorian => day.day().to_string(),
            Calendar::Jalali => jalali::persian_digits(&Jalali::from_day(day).day.to_string()),
        }
    }

    fn first(self) -> Option<Day> {
        match self.calendar {
            Calendar::Gregorian => Day::new(self.year, self.month, 1),
            Calendar::Jalali => Jalali { year: self.year, month: self.month, day: 1 }.to_day(),
        }
    }

    fn length(self) -> Option<u32> {
        match self.calendar {
            Calendar::Gregorian => {
                let next = self.adding(1);
                Some(self.first()?.days_to(Day::new(next.year, next.month, 1)?) as u32)
            }
            Calendar::Jalali => Jalali::month_length(self.year, self.month),
        }
    }

    /// The month's days in weeks of seven: `None` before the first and
    /// after the last, to fill its first and last weeks. Empty for a month
    /// the calendar does not have.
    pub fn cells(self) -> Vec<Option<Day>> {
        let (Some(first), Some(length)) = (self.first(), self.length()) else { return Vec::new() };
        let from_monday = first.date().weekday().num_days_from_monday() as usize;
        let lead = match self.calendar {
            Calendar::Gregorian => from_monday,
            // Saturday is 5 days from Monday.
            Calendar::Jalali => (from_monday + 2) % 7,
        };
        let mut cells = vec![None; lead];
        cells.extend((0..i64::from(length)).map(|n| Some(first.adding_days(n))));
        while !cells.len().is_multiple_of(7) {
            cells.push(None);
        }
        cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> Day {
        Day::parse(text).unwrap()
    }

    #[test]
    fn a_gregorian_month_starts_its_weeks_on_monday() {
        let month = Month::of(Calendar::Gregorian, day("2026-11-03"));
        assert_eq!(month.title(), "November 2026");
        let cells = month.cells();
        // 1 November 2026 is a Sunday: six days before it.
        assert_eq!(cells.iter().take_while(|c| c.is_none()).count(), 6);
        assert_eq!(cells[6], Some(day("2026-11-01")));
        assert_eq!(cells.iter().flatten().count(), 30);
        assert_eq!(cells.len(), 42);
        assert_eq!(month.weekdays()[0], "Mo");
    }

    #[test]
    fn a_jalali_month_starts_its_weeks_on_saturday() {
        let month = Month::of(Calendar::Jalali, day("2026-11-03"));
        assert_eq!((month.year, month.month), (1405, 8));
        assert_eq!(month.title(), "آبان ۱۴۰۵");
        let cells = month.cells();
        // 1 Aban 1405 is 2026-10-23, a Friday: six days after Saturday.
        assert_eq!(cells.iter().take_while(|c| c.is_none()).count(), 6);
        assert_eq!(cells[6], Some(day("2026-10-23")));
        assert_eq!(cells.iter().flatten().count(), 30);
        assert_eq!(month.number(day("2026-11-03")), "۱۲");
        assert_eq!(month.weekdays()[0], "ش");
    }

    #[test]
    fn months_step_over_the_year() {
        let esfand = Month { calendar: Calendar::Jalali, year: 1403, month: 12 };
        assert_eq!(esfand.cells().iter().flatten().count(), 30);
        assert_eq!(esfand.adding(1), Month { calendar: Calendar::Jalali, year: 1404, month: 1 });
        assert_eq!(esfand.adding(12).cells().iter().flatten().count(), 29);
        let january = Month { calendar: Calendar::Gregorian, year: 2027, month: 1 };
        assert_eq!(january.adding(-1), Month { calendar: Calendar::Gregorian, year: 2026, month: 12 });
        assert_eq!(Month { calendar: Calendar::Gregorian, year: 2028, month: 2 }.cells().iter().flatten().count(), 29);
    }

    #[test]
    fn a_month_off_the_calendar_has_no_days() {
        assert!(Month { calendar: Calendar::Jalali, year: 4000, month: 1 }.cells().is_empty());
    }
}
