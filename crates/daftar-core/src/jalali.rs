//! The Solar Hijri (Jalali) calendar, Iran's own: a day of it from a
//! Gregorian day and back, its month and weekday names, and Persian digits.
//! The arithmetic is jalaali-js's (Borkowski's leap years, as Iran's
//! calendar sets them), good from 1 to 3177 of the Jalali years.

use chrono::Datelike;

use crate::day::Day;

/// A day of the Jalali calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jalali {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

pub const MONTHS: [&str; 12] = ["فروردین", "اردیبهشت", "خرداد", "تیر", "مرداد", "شهریور", "مهر", "آبان", "آذر", "دی", "بهمن", "اسفند"];

/// From Monday, as chrono counts.
pub const WEEKDAYS: [&str; 7] = ["دوشنبه", "سه‌شنبه", "چهارشنبه", "پنجشنبه", "جمعه", "شنبه", "یکشنبه"];

const BREAKS: [i64; 20] = [-61, 9, 38, 199, 426, 686, 756, 818, 1111, 1181, 1210, 1635, 2060, 2097, 2192, 2262, 2324, 2394, 2456, 3178];

/// Whether [year] is leap, the Gregorian year it starts in, and the day of
/// March of that year its first day falls on.
fn year_of(year: i64) -> Option<(bool, i64, i64)> {
    if year < BREAKS[0] || year >= BREAKS[BREAKS.len() - 1] {
        return None;
    }
    let gregorian = year + 621;
    let mut leap_j = -14;
    let mut previous = BREAKS[0];
    let mut jump = 0;
    for &next in &BREAKS[1..] {
        jump = next - previous;
        if year < next {
            break;
        }
        leap_j += jump / 33 * 8 + jump % 33 / 4;
        previous = next;
    }
    let mut n = year - previous;
    leap_j += n / 33 * 8 + (n % 33 + 3) / 4;
    if jump % 33 == 4 && jump - n == 4 {
        leap_j += 1;
    }
    let leap_g = gregorian / 4 - (gregorian / 100 + 1) * 3 / 4 - 150;
    let march = 20 + leap_j - leap_g;
    if jump - n < 6 {
        n = n - jump + (jump + 4) / 33 * 33;
    }
    let mut leap = ((n + 1) % 33 - 1) % 4;
    if leap == -1 {
        leap = 4;
    }
    Some((leap == 0, gregorian, march))
}

/// The Julian day number of a Gregorian day.
fn julian(year: i64, month: i64, day: i64) -> i64 {
    let d = (year + (month - 8) / 6 + 100100) * 1461 / 4 + (153 * ((month + 9) % 12) + 2) / 5 + day - 34840408;
    d - (year + 100100 + (month - 8) / 6) / 100 * 3 / 4 + 752
}

fn gregorian_year(jdn: i64) -> i64 {
    let mut j = 4 * jdn + 139361631;
    j += (4 * jdn + 183187720) / 146097 * 3 / 4 * 4 - 3908;
    let i = j % 1461 / 4 * 5 + 308;
    let month = i / 153 % 12 + 1;
    j / 1461 - 100100 + (8 - month) / 6
}

impl Jalali {
    pub fn from_day(day: Day) -> Jalali {
        let jdn = julian(i64::from(day.year()), i64::from(day.month()), i64::from(day.day()));
        let gregorian = gregorian_year(jdn);
        let mut year = gregorian - 621;
        let (_, _, march) = year_of(year).unwrap_or((false, gregorian, 20));
        let mut k = jdn - julian(gregorian, 3, march);
        if k >= 0 {
            if k <= 185 {
                return Jalali { year: year as i32, month: (1 + k / 31) as u32, day: (k % 31 + 1) as u32 };
            }
            k -= 186;
        } else {
            year -= 1;
            k += 179;
            if year_of(year).is_some_and(|(leap, _, _)| leap) {
                k += 1;
            }
        }
        Jalali { year: year as i32, month: (7 + k / 30) as u32, day: (k % 30 + 1) as u32 }
    }

    /// The Gregorian day, or `None` for a day the calendar does not have.
    pub fn to_day(self) -> Option<Day> {
        let (leap, gregorian, march) = year_of(i64::from(self.year))?;
        let length = match self.month {
            1..=6 => 31,
            7..=11 => 30,
            12 if leap => 30,
            12 => 29,
            _ => return None,
        };
        if self.day == 0 || self.day > length {
            return None;
        }
        let (month, day) = (i64::from(self.month), i64::from(self.day));
        let first = julian(gregorian, 3, march);
        let jdn = first + (month - 1) * 31 - month / 7 * (month - 7) + day - 1;
        let date = Day::new(1970, 1, 1)?.date() + chrono::Duration::days(jdn - julian(1970, 1, 1));
        Day::new(date.year(), date.month(), date.day())
    }

    pub fn month_name(self) -> &'static str {
        MONTHS[(self.month.clamp(1, 12) - 1) as usize]
    }

    /// `سه‌شنبه ۵ آبان ۱۴۰۵`: the day as it is said in Persian.
    pub fn long(day: Day) -> String {
        let jalali = Jalali::from_day(day);
        let weekday = WEEKDAYS[day.date().weekday().num_days_from_monday() as usize];
        persian_digits(&format!("{weekday} {} {} {}", jalali.day, jalali.month_name(), jalali.year))
    }

    /// A day typed as `1405-08-05`, in Latin or Persian digits, as the
    /// Gregorian day it is; a year before 1700 is a Jalali year.
    pub fn parse(text: &str) -> Option<Day> {
        let latin = latin_digits(text.trim());
        let mut parts = latin.split(['-', '/']);
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        let day: u32 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || year >= 1700 {
            return None;
        }
        Jalali { year, month, day }.to_day()
    }

    /// `1405-08-05`.
    pub fn to_iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// The digits 0-9 as Persian ones.
pub fn persian_digits(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_digit() { char::from_u32('۰' as u32 + (c as u32 - '0' as u32)).unwrap_or(c) } else { c }).collect()
}

/// Persian and Arabic digits as ASCII ones.
pub fn latin_digits(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '۰'..='۹' => char::from(b'0' + (c as u32 - '۰' as u32) as u8),
            '٠'..='٩' => char::from(b'0' + (c as u32 - '٠' as u32) as u8),
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> Day {
        Day::new(y, m, d).unwrap()
    }

    #[test]
    fn nowruz_and_the_months_fall_where_the_calendar_puts_them() {
        assert_eq!(Jalali::from_day(day(2026, 3, 21)), Jalali { year: 1405, month: 1, day: 1 });
        assert_eq!(Jalali::from_day(day(2024, 3, 20)), Jalali { year: 1403, month: 1, day: 1 });
        assert_eq!(Jalali::from_day(day(2026, 10, 27)), Jalali { year: 1405, month: 8, day: 5 });
        assert_eq!(Jalali::from_day(day(2025, 3, 20)), Jalali { year: 1403, month: 12, day: 30 });
        assert_eq!(Jalali::from_day(day(1979, 2, 11)), Jalali { year: 1357, month: 11, day: 22 });
    }

    #[test]
    fn a_day_goes_there_and_back() {
        let mut at = day(1990, 1, 1);
        for _ in 0..20_000 {
            assert_eq!(Jalali::from_day(at).to_day(), Some(at), "{at:?}");
            at = at.adding_days(3);
        }
        assert_eq!(Jalali { year: 1404, month: 12, day: 30 }.to_day(), None);
        assert_eq!(Jalali { year: 1403, month: 12, day: 30 }.to_day(), Some(day(2025, 3, 20)));
    }

    #[test]
    fn it_reads_as_persian() {
        assert_eq!(Jalali::long(day(2026, 10, 27)), "سه‌شنبه ۵ آبان ۱۴۰۵");
        assert_eq!(persian_digits("2026-10"), "۲۰۲۶-۱۰");
        assert_eq!(latin_digits("۱۴۰۵/۰۸/۰۵"), "1405/08/05");
    }

    #[test]
    fn a_jalali_day_typed_is_its_gregorian_day() {
        assert_eq!(Jalali::parse("1405-08-05"), Some(day(2026, 10, 27)));
        assert_eq!(Jalali::parse("۱۴۰۵/۸/۵"), Some(day(2026, 10, 27)));
        assert_eq!(Jalali::parse("2026-10-27"), None);
        assert_eq!(Jalali::parse("1405-13-01"), None);
        assert_eq!(Jalali::from_day(day(2026, 10, 27)).to_iso(), "1405-08-05");
    }
}
