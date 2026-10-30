//! Dates written the way moment.js writes them, which is how Obsidian names
//! a daily note and fills `{{date:…}}` in a template, and the daily notes
//! read as a series of periods. The port of `PeriodicNotes.swift`, itself
//! from the phone's `MomentFormat.kt` and `PeriodicNotes.kt`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

use chrono::{Datelike, Local, NaiveDate, Timelike, Weekday};
use regex::Regex;

use crate::day::Day;
use crate::text::trim;

/// A moment of the day, for the time tokens of a template.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Clock {
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Clock {
    pub fn new(hour: u32, minute: u32, second: u32) -> Clock {
        Clock { hour, minute, second }
    }

    pub fn now() -> Clock {
        let now = Local::now();
        Clock::new(now.hour(), now.minute(), now.second())
    }
}

static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)\[[^\[]*\]|\\.|Mo|MMMM|MMM|MM|M|Do|DDDD|DDD|DD|D|dddd|ddd|dd|do|d|wo|ww|w|Wo|WW|W|Qo|Q|YYYY|YY|gggg|gg|GGGG|GG|HH|H|hh|h|mm|m|ss|s|A|a|e|E|.").unwrap()
});

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
/// Sunday first, as moment numbers them.
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// A week numbering: moment's English one starts on Sunday and its week 1
/// holds the 1st of January; ISO's starts on Monday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Weeks {
    Locale,
    Iso,
}

impl Weeks {
    /// The first day of a week, as moment numbers a weekday.
    fn first_weekday(self) -> u32 {
        match self {
            Weeks::Locale => 0,
            Weeks::Iso => 1,
        }
    }

    /// The week-numbering year and the week of [day].
    fn week(self, day: Day) -> (i32, u32) {
        match self {
            Weeks::Iso => {
                let week = day.date().iso_week();
                (week.year(), week.week())
            }
            Weeks::Locale => {
                // A week that holds the 1st of January is that year's first.
                let sunday = day.adding_days(-i64::from(moment_weekday(day)));
                let year = sunday.adding_days(6).year();
                let first = locale_week_one(year);
                (year, (first.days_to(sunday) / 7 + 1) as u32)
            }
        }
    }

    /// The first day of week [week] of [year].
    fn start(self, year: i32, week: u32) -> Option<Day> {
        match self {
            Weeks::Iso => NaiveDate::from_isoywd_opt(year, week, Weekday::Mon).map(Day::from),
            Weeks::Locale => Some(locale_week_one(year).adding_days(7 * (i64::from(week) - 1))),
        }
    }
}

fn locale_week_one(year: i32) -> Day {
    let january = Day::new(year, 1, 1).expect("a year in range");
    january.adding_days(-i64::from(moment_weekday(january)))
}

/// 0 for Sunday, as moment counts.
fn moment_weekday(day: Day) -> u32 {
    day.date().weekday().num_days_from_sunday()
}

pub struct MomentFormat;

impl MomentFormat {
    /// [format] cut into the tokens moment reads it as, literals included.
    fn tokens(format: &str) -> Vec<String> {
        TOKEN.find_iter(format).map(|m| m.as_str().to_string()).collect()
    }

    pub fn format(format: &str, day: Day, clock: Clock) -> String {
        Self::tokens(format).iter().map(|t| token(t, day, clock)).collect()
    }

    fn literal(token: &str) -> String {
        let count = token.chars().count();
        if count > 1 && token.starts_with('[') {
            return token[1..token.len() - 1].to_string();
        }
        if count == 2 && token.starts_with('\\') {
            return token[1..].to_string();
        }
        token.to_string()
    }
}

/// English ordinals, as moment's default locale writes them.
fn ordinal(n: i64) -> String {
    if (11..=13).contains(&(n % 100)) {
        return format!("{n}th");
    }
    match n % 10 {
        1 => format!("{n}st"),
        2 => format!("{n}nd"),
        3 => format!("{n}rd"),
        _ => format!("{n}th"),
    }
}

fn token(token: &str, day: Day, clock: Clock) -> String {
    let pad = |n: i64, width: usize| format!("{n:0width$}");
    let weekday = moment_weekday(day) as usize;
    let quarter = (i64::from(day.month()) - 1) / 3 + 1;
    let hour12 = i64::from(if clock.hour.is_multiple_of(12) { 12 } else { clock.hour % 12 });
    let (week_year, week) = Weeks::Locale.week(day);
    let (iso_year, iso_week) = Weeks::Iso.week(day);
    let day_of_year = i64::from(day.date().ordinal());
    match token {
        "YYYY" => pad(i64::from(day.year()), 4),
        "YY" => pad((i64::from(day.year()) % 100 + 100) % 100, 2),
        "Q" => quarter.to_string(),
        "Qo" => ordinal(quarter),
        "MMMM" => MONTHS[day.month() as usize - 1].to_string(),
        "MMM" => MONTHS[day.month() as usize - 1][..3].to_string(),
        "MM" => pad(i64::from(day.month()), 2),
        "M" => day.month().to_string(),
        "Mo" => ordinal(i64::from(day.month())),
        "DDDD" => pad(day_of_year, 3),
        "DDD" => day_of_year.to_string(),
        "DD" => pad(i64::from(day.day()), 2),
        "D" => day.day().to_string(),
        "Do" => ordinal(i64::from(day.day())),
        "dddd" => WEEKDAYS[weekday].to_string(),
        "ddd" => WEEKDAYS[weekday][..3].to_string(),
        "dd" => WEEKDAYS[weekday][..2].to_string(),
        "d" | "e" => weekday.to_string(),
        "do" => ordinal(weekday as i64),
        "E" => (if weekday == 0 { 7 } else { weekday }).to_string(),
        "w" => week.to_string(),
        "ww" => pad(i64::from(week), 2),
        "wo" => ordinal(i64::from(week)),
        "gggg" => pad(i64::from(week_year), 4),
        "gg" => pad(i64::from(week_year) % 100, 2),
        "W" => iso_week.to_string(),
        "WW" => pad(i64::from(iso_week), 2),
        "Wo" => ordinal(i64::from(iso_week)),
        "GGGG" => pad(i64::from(iso_year), 4),
        "GG" => pad(i64::from(iso_year) % 100, 2),
        "HH" => pad(i64::from(clock.hour), 2),
        "H" => clock.hour.to_string(),
        "hh" => pad(hour12, 2),
        "h" => hour12.to_string(),
        "mm" => pad(i64::from(clock.minute), 2),
        "m" => clock.minute.to_string(),
        "ss" => pad(i64::from(clock.second), 2),
        "s" => clock.second.to_string(),
        "A" => (if clock.hour < 12 { "AM" } else { "PM" }).to_string(),
        "a" => (if clock.hour < 12 { "am" } else { "pm" }).to_string(),
        _ => MomentFormat::literal(token),
    }
}

/// Where Obsidian's Daily notes plugin keeps a day's note: the two settings
/// of `.obsidian/daily-notes.json` that decide the path, and the template a
/// new note starts from. Blank or missing means the default.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DailyNotes {
    /// Vault-relative, empty for the root.
    pub folder: String,
    /// A moment format, which may itself hold `/` and so subfolders.
    pub format: String,
    /// The template's path without `.md`, empty for none.
    pub template: String,
}

impl Default for DailyNotes {
    fn default() -> DailyNotes {
        DailyNotes::new("", "", "")
    }
}

impl DailyNotes {
    pub const DEFAULT_FORMAT: &str = "YYYY-MM-DD";

    pub fn new(folder: &str, format: &str, template: &str) -> DailyNotes {
        let format = trim(format);
        DailyNotes {
            folder: trim(folder).trim_matches('/').to_string(),
            format: if format.is_empty() { Self::DEFAULT_FORMAT.to_string() } else { format.to_string() },
            template: trim(template).to_string(),
        }
    }

    pub fn from_json(json: &str) -> DailyNotes {
        let root: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let get = |key: &str| root.get(key).and_then(serde_json::Value::as_str).unwrap_or("");
        DailyNotes::new(get("folder"), get("format"), get("template"))
    }

    /// The settings of the vault at [root].
    pub fn read(root: &Path) -> DailyNotes {
        std::fs::read_to_string(root.join(".obsidian/daily-notes.json")).map(|t| DailyNotes::from_json(&t)).unwrap_or_default()
    }
}

/// How much time one note of a daily-notes format covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Period {
    /// The command that opens the current period's note.
    pub fn current(self) -> &'static str {
        match self {
            Period::Day => "Today's Note",
            Period::Week => "This Week's Note",
            Period::Month => "This Month's Note",
            Period::Quarter => "This Quarter's Note",
            Period::Year => "This Year's Note",
        }
    }

    /// The word the app shows: "Today's note" is a week note in a weekly vault.
    pub fn noun(self) -> &'static str {
        match self {
            Period::Day => "Day",
            Period::Week => "Week",
            Period::Month => "Month",
            Period::Quarter => "Quarter",
            Period::Year => "Year",
        }
    }
}

const DAY_TOKENS: [&str; 12] = ["D", "DD", "Do", "DDD", "DDDD", "d", "dd", "ddd", "dddd", "do", "e", "E"];
const WEEK_TOKENS: [&str; 6] = ["w", "ww", "wo", "W", "WW", "Wo"];
const ISO_TOKENS: [&str; 5] = ["W", "WW", "Wo", "GGGG", "GG"];
const MONTH_TOKENS: [&str; 5] = ["M", "MM", "Mo", "MMM", "MMMM"];

/// The daily notes read as a series of periods: a format with no day in it
/// (`YYYY-[W]ww`, `YYYY-MM`) names one note for a whole week or month. A
/// period is known by the day it starts on. A name is read loosely, then
/// accepted only when a day of its period formats to it again, so
/// `2026-02-30` is an ordinary note.
#[derive(Debug, Clone)]
pub struct PeriodicNotes {
    pub config: DailyNotes,
    pub period: Period,
    weeks: Weeks,
    /// The token each capture group stands for, in order.
    groups: Vec<String>,
    pattern: Regex,
}

impl PeriodicNotes {
    pub fn new(config: DailyNotes) -> PeriodicNotes {
        let tokens = MomentFormat::tokens(&config.format);
        let has = |set: &[&str]| tokens.iter().any(|t| set.contains(&t.as_str()));
        let period = if has(&DAY_TOKENS) {
            Period::Day
        } else if has(&WEEK_TOKENS) {
            Period::Week
        } else if has(&MONTH_TOKENS) {
            Period::Month
        } else if has(&["Q", "Qo"]) {
            Period::Quarter
        } else {
            Period::Year
        };
        let weeks = if has(&ISO_TOKENS) { Weeks::Iso } else { Weeks::Locale };
        let mut groups = Vec::new();
        let mut pattern = String::from("^");
        for token in &tokens {
            match regex_for(token) {
                Some(part) => {
                    groups.push(token.clone());
                    pattern.push_str(&part);
                }
                None => pattern.push_str(&regex::escape(&MomentFormat::literal(token))),
            }
        }
        pattern.push('$');
        let pattern = Regex::new(&pattern).unwrap_or_else(|_| Regex::new("^$^").unwrap());
        PeriodicNotes { config, period, weeks, groups, pattern }
    }

    /// The first day of the period [day] falls in.
    pub fn start(&self, day: Day) -> Day {
        match self.period {
            Period::Day => day,
            Period::Week => day.adding_days(-i64::from((moment_weekday(day) + 7 - self.weeks.first_weekday()) % 7)),
            Period::Month => Day::new(day.year(), day.month(), 1).unwrap(),
            Period::Quarter => Day::new(day.year(), (day.month() - 1) / 3 * 3 + 1, 1).unwrap(),
            Period::Year => Day::new(day.year(), 1, 1).unwrap(),
        }
    }

    /// The start of the period [by] periods after the one starting on [start].
    pub fn shift(&self, start: Day, by: i32) -> Day {
        match self.period {
            Period::Day => start.adding_days(i64::from(by)),
            Period::Week => start.adding_days(7 * i64::from(by)),
            Period::Month => start.adding_months(by),
            Period::Quarter => start.adding_months(3 * by),
            Period::Year => start.adding_months(12 * by),
        }
    }

    /// The period [path] is the note of, by its start; `None` when it is not a daily note.
    pub fn period_of(&self, path: &str) -> Option<Day> {
        let name = self.name_of(path)?;
        let captures = self.pattern.captures(&name)?;
        let fields: BTreeMap<&str, &str> =
            self.groups.iter().enumerate().filter_map(|(i, token)| captures.get(i + 1).map(|m| (token.as_str(), m.as_str()))).collect();
        let start = self.start(self.candidate(&fields)?);
        self.days(start).into_iter().any(|d| self.format(d) == name).then_some(start)
    }

    /// The note of the period starting on [start]. Where its days disagree
    /// (the week over New Year under `YYYY-[W]ww`), the name that reads back
    /// as this period.
    pub fn path_of(&self, start: Day) -> String {
        let mut names: Vec<String> = Vec::new();
        for day in self.days(start) {
            let name = self.format(day);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let own = names.iter().find(|n| self.period_of(&self.path_named(n)) == Some(start)).unwrap_or(&names[0]);
        self.path_named(own)
    }

    /// The note of the period [day] falls in.
    pub fn path_for(&self, day: Day) -> String {
        self.path_of(self.start(day))
    }

    pub fn previous(&self, path: &str) -> Option<String> {
        self.period_of(path).map(|s| self.path_of(self.shift(s, -1)))
    }

    pub fn next(&self, path: &str) -> Option<String> {
        self.period_of(path).map(|s| self.path_of(self.shift(s, 1)))
    }

    /// Of [paths], the daily note nearest to [from] in the direction of
    /// [step]: a journal kept in fits and starts has gaps.
    pub fn nearest<'a>(&self, paths: impl IntoIterator<Item = &'a String>, from: &str, step: i32) -> Option<String> {
        let here = self.period_of(from)?;
        let periods = paths.into_iter().filter_map(|p| self.period_of(p).map(|d| (d, p)));
        if step < 0 {
            periods.filter(|(d, _)| *d < here).max().map(|(_, p)| p.clone())
        } else {
            periods.filter(|(d, _)| *d > here).min().map(|(_, p)| p.clone())
        }
    }

    /// The notes of [paths] for the same time of year as [from] in earlier
    /// years, as years back and path, nearest first. Found by the calendar
    /// day, not by the number in the name. Nothing for a yearly note.
    pub fn years_ago(&self, paths: &BTreeSet<String>, from: &str) -> Vec<(i32, String)> {
        let Some(here) = self.period_of(from).filter(|_| self.period != Period::Year) else { return vec![] };
        let Some(earliest) = paths.iter().filter_map(|p| self.period_of(p)).min().filter(|e| e.year() < here.year()) else {
            return vec![];
        };
        (1..=here.year() - earliest.year())
            .filter_map(|years| {
                let path = self.path_of(self.start(here.adding_months(-12 * years)));
                (paths.contains(&path) && path != from).then_some((years, path))
            })
            .collect()
    }

    fn days(&self, start: Day) -> Vec<Day> {
        let end = self.shift(start, 1);
        (0..start.days_to(end)).map(|i| start.adding_days(i)).collect()
    }

    fn format(&self, day: Day) -> String {
        MomentFormat::format(&self.config.format, day, Clock::default()).trim_matches('/').to_string()
    }

    fn path_named(&self, name: &str) -> String {
        if self.config.folder.is_empty() { format!("{name}.md") } else { format!("{}/{name}.md", self.config.folder) }
    }

    fn name_of(&self, path: &str) -> Option<String> {
        let inside = path.strip_suffix(".md")?;
        if self.config.folder.is_empty() {
            return Some(inside.to_string());
        }
        inside.strip_prefix(&format!("{}/", self.config.folder)).map(String::from)
    }

    /// A day inside the period the fields describe, when they describe one.
    fn candidate(&self, fields: &BTreeMap<&str, &str>) -> Option<Day> {
        let int = |keys: &[&str]| keys.iter().find_map(|k| fields.get(k)).and_then(|t| number(t));
        let century = |yy: i64| yy + if yy > 68 { 1900 } else { 2000 };
        let year = int(&["YYYY"]).or_else(|| int(&["YY"]).map(century));
        let week_year = match self.weeks {
            Weeks::Iso => int(&["GGGG"]).or_else(|| int(&["GG"]).map(century)),
            Weeks::Locale => int(&["gggg"]).or_else(|| int(&["gg"]).map(century)),
        }
        .or(year);
        let month = int(&["MMMM", "MMM", "MM", "M", "Mo"]);
        let day_of_month = int(&["DD", "D", "Do"]);
        let day_of_year = int(&["DDDD", "DDD"]);
        let week = int(&["ww", "w", "wo", "WW", "W", "Wo"]);
        // Sunday first (0) as moment numbers a weekday, ISO's 7 for Sunday from E.
        let weekday = int(&["dddd", "ddd", "dd", "d", "do", "e"]).or_else(|| int(&["E"]).map(|e| e % 7));
        let quarter = int(&["Q", "Qo"]);
        let valid = |y: i64, m: i64, d: i64| Day::new(y as i32, u32::try_from(m).ok()?, u32::try_from(d).ok()?);
        if let (Some(year), Some(month), Some(day)) = (year, month, day_of_month) {
            return valid(year, month, day);
        }
        if let (Some(year), Some(ordinal)) = (year, day_of_year) {
            if !(1..=366).contains(&ordinal) {
                return None;
            }
            let day = valid(year, 1, 1)?.adding_days(ordinal - 1);
            return (i64::from(day.year()) == year).then_some(day);
        }
        if let (Some(week_year), Some(week)) = (week_year, week) {
            if !(1..=53).contains(&week) {
                return None;
            }
            let start = self.weeks.start(i32::try_from(week_year).ok()?, week as u32)?;
            if let Some(weekday) = weekday {
                return Some(start.adding_days((weekday - i64::from(moment_weekday(start)) + 7) % 7));
            }
            return (self.period != Period::Day).then_some(start);
        }
        if self.period == Period::Day {
            return None;
        }
        if let (Some(year), Some(month)) = (year, month) {
            return valid(year, month, 1);
        }
        if let (Some(year), Some(quarter)) = (year, quarter) {
            return if (1..=4).contains(&quarter) { valid(year, (quarter - 1) * 3 + 1, 1) } else { None };
        }
        if let Some(year) = year
            && self.period == Period::Year
        {
            return valid(year, 1, 1);
        }
        None
    }
}

/// Numbers as written, and names as the number they stand for.
fn number(text: &str) -> Option<i64> {
    if let Ok(n) = text.parse() {
        return Some(n);
    }
    if let Ok(n) = text.trim_matches(char::is_alphabetic).parse() {
        return Some(n);
    }
    if let Some(month) = MONTHS.iter().position(|m| *m == text || &m[..3] == text) {
        return Some(month as i64 + 1);
    }
    WEEKDAYS.iter().position(|d| *d == text || &d[..3] == text || &d[..2] == text).map(|d| d as i64)
}

fn names(values: impl IntoIterator<Item = String>) -> String {
    let mut values: Vec<String> = values.into_iter().collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.chars().count()));
    format!("({})", values.iter().map(|v| regex::escape(v)).collect::<Vec<_>>().join("|"))
}

fn regex_for(token: &str) -> Option<String> {
    let fixed = match token {
        "YYYY" | "gggg" | "GGGG" => r"(\d{4})",
        "YY" | "gg" | "GG" | "MM" | "DD" | "ww" | "WW" => r"(\d{2})",
        "M" | "D" | "w" | "W" => r"(\d{1,2})",
        "Mo" | "Do" | "wo" | "Wo" => r"(\d{1,2}(?:st|nd|rd|th))",
        "Q" | "d" | "e" | "E" => r"(\d)",
        "Qo" | "do" => r"(\d(?:st|nd|rd|th))",
        "DDDD" => r"(\d{3})",
        "DDD" => r"(\d{1,3})",
        "MMMM" => return Some(names(MONTHS.iter().map(ToString::to_string))),
        "MMM" => return Some(names(MONTHS.iter().map(|m| m[..3].to_string()))),
        "dddd" => return Some(names(WEEKDAYS.iter().map(ToString::to_string))),
        "ddd" => return Some(names(WEEKDAYS.iter().map(|d| d[..3].to_string()))),
        "dd" => return Some(names(WEEKDAYS.iter().map(|d| d[..2].to_string()))),
        _ => return None,
    };
    Some(fixed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(format: &str, folder: &str) -> PeriodicNotes {
        PeriodicNotes::new(DailyNotes::new(folder, format, ""))
    }

    fn weekly() -> PeriodicNotes {
        notes("YYYY-[W]ww", "")
    }

    fn day(y: i32, m: u32, d: u32) -> Day {
        Day::new(y, m, d).unwrap()
    }

    #[expect(clippy::unnecessary_wraps, reason = "the fields it fills are options")]
    fn s(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    #[test]
    fn the_period_follows_from_the_smallest_thing_written() {
        assert_eq!(notes("YYYY-MM-DD", "").period, Period::Day);
        assert_eq!(notes("YYYY/MM/DDDD", "").period, Period::Day);
        assert_eq!(notes("gggg-[W]ww-ddd", "").period, Period::Day);
        assert_eq!(notes("dddd, MMMM Do YYYY", "").period, Period::Day);
        assert_eq!(notes("YYYY-[W]ww", "").period, Period::Week);
        assert_eq!(notes("GGGG-[W]WW", "").period, Period::Week);
        assert_eq!(notes("YYYY-MM", "").period, Period::Month);
        assert_eq!(notes("MMMM YYYY", "").period, Period::Month);
        assert_eq!(notes("YYYY-[Q]Q", "").period, Period::Quarter);
        assert_eq!(notes("YYYY", "").period, Period::Year);
        assert_eq!(notes("[Daily] YYYY-MM", "").period, Period::Month);
    }

    #[test]
    fn a_daily_note_is_the_day_it_names() {
        let daily = notes("YYYY-MM-DD", "");
        assert_eq!(daily.period_of("2026-03-04.md"), Some(day(2026, 3, 4)));
        assert_eq!(daily.previous("2026-03-01.md"), s("2026-02-28.md"));
        assert_eq!(daily.next("2026-12-31.md"), s("2027-01-01.md"));
    }

    #[test]
    fn what_moment_would_not_write_is_an_ordinary_note() {
        let daily = notes("YYYY-MM-DD", "Journal");
        assert_eq!(daily.period_of("Journal/2026-03-04.md"), Some(day(2026, 3, 4)));
        for path in [
            "2026-03-04.md",
            "Other/2026-03-04.md",
            "Journal/2026-02-30.md",
            "Journal/2026-13-01.md",
            "Journal/2026-3-4.md",
            "Journal/2026-03-04 notes.md",
            "Journal/2026-03-04.pdf",
            "Journal/Plan.md",
        ] {
            assert_eq!(daily.period_of(path), None, "{path}");
        }
        assert_eq!(daily.previous("Journal/Plan.md"), None);
    }

    #[test]
    fn names_weekdays_and_folders_are_read_back() {
        let daily = notes("YYYY/MM-MMMM/YYYY-MM-DD dddd", "Journal/");
        let path = "Journal/2026/03-March/2026-03-04 Wednesday.md";
        assert_eq!(daily.period_of(path), Some(day(2026, 3, 4)));
        assert_eq!(daily.next(path), s("Journal/2026/03-March/2026-03-05 Thursday.md"));
        assert_eq!(daily.previous("Journal/2026/03-March/2026-03-01 Sunday.md"), s("Journal/2026/02-February/2026-02-28 Saturday.md"));
        assert_eq!(daily.period_of("Journal/2026/03-March/2026-03-04 Friday.md"), None);
        assert_eq!(notes("dddd, MMMM Do YYYY", "").period_of("Wednesday, March 4th 2026.md"), Some(day(2026, 3, 4)));
    }

    #[test]
    fn a_weekly_note_starts_on_sunday() {
        assert_eq!(MomentFormat::format("YYYY-[W]ww", day(2026, 9, 24), Clock::default()), "2026-W39");
        assert_eq!(weekly().period_of("2026-W39.md"), Some(day(2026, 9, 20)));
        assert_eq!(weekly().previous("2026-W39.md"), s("2026-W38.md"));
        assert_eq!(weekly().next("2026-W39.md"), s("2026-W40.md"));
        assert_eq!(weekly().period_of("2026-W60.md"), None);
        assert_eq!(weekly().period_of("2026-W00.md"), None);
    }

    /// `2026-W01` is both 28 Dec 2025 – 3 Jan 2026 and 27–31 Dec 2026, and is read as the first.
    #[test]
    fn the_week_over_new_year_is_named_as_moment_names_it() {
        let name = |d: Day| MomentFormat::format("YYYY-[W]ww", d, Clock::default());
        assert_eq!(name(day(2026, 12, 26)), "2026-W52");
        assert_eq!(name(day(2026, 12, 27)), "2026-W01");
        assert_eq!(name(day(2026, 12, 31)), "2026-W01");
        assert_eq!(name(day(2027, 1, 1)), "2027-W01");
        assert_eq!(name(day(2026, 1, 1)), "2026-W01");
        let weekly = weekly();
        assert_eq!(weekly.period_of("2026-W01.md"), Some(day(2025, 12, 28)));
        assert_eq!(weekly.period_of("2027-W01.md"), Some(day(2026, 12, 27)));
        assert_eq!(weekly.period_of("2026-W52.md"), Some(day(2026, 12, 20)));
        assert_eq!(weekly.next("2026-W52.md"), s("2027-W01.md"));
        assert_eq!(weekly.previous("2027-W01.md"), s("2026-W52.md"));
        assert_eq!(weekly.next("2027-W01.md"), s("2027-W02.md"));
        assert_eq!(weekly.previous("2026-W01.md"), s("2025-W52.md"));
    }

    #[test]
    fn iso_weeks_start_on_monday() {
        let iso = notes("GGGG-[W]WW", "");
        assert_eq!(iso.period_of("2026-W53.md"), Some(day(2026, 12, 28)));
        assert_eq!(iso.next("2026-W53.md"), s("2027-W01.md"));
        assert_eq!(iso.period_of("2027-W01.md"), Some(day(2027, 1, 4)));
        assert_eq!(iso.previous("2027-W01.md"), s("2026-W53.md"));
        assert_eq!(iso.period_of("2027-W53.md"), None);
    }

    #[test]
    fn months_quarters_and_years_step_by_their_length() {
        let monthly = notes("YYYY-MM", "Months");
        assert_eq!(monthly.period_of("Months/2026-12.md"), Some(day(2026, 12, 1)));
        assert_eq!(monthly.next("Months/2026-12.md"), s("Months/2027-01.md"));
        assert_eq!(monthly.previous("Months/2026-01.md"), s("Months/2025-12.md"));
        assert_eq!(notes("MMMM YYYY", "").next("December 2026.md"), s("January 2027.md"));
        let quarterly = notes("YYYY-[Q]Q", "");
        assert_eq!(quarterly.period_of("2026-Q3.md"), Some(day(2026, 7, 1)));
        assert_eq!(quarterly.next("2026-Q4.md"), s("2027-Q1.md"));
        assert_eq!(quarterly.period_of("2026-Q5.md"), None);
        assert_eq!(notes("YYYY", "").previous("2026.md"), s("2025.md"));
        assert_eq!(notes("YYYY", "").period_of("Plan.md"), None);
    }

    #[test]
    fn the_period_a_day_falls_in() {
        let thursday = day(2026, 9, 24);
        assert_eq!(weekly().start(thursday), day(2026, 9, 20));
        assert_eq!(notes("GGGG-[W]WW", "").start(thursday), day(2026, 9, 21));
        assert_eq!(notes("YYYY-MM", "").start(thursday), day(2026, 9, 1));
        assert_eq!(notes("YYYY-[Q]Q", "").start(thursday), day(2026, 7, 1));
        assert_eq!(notes("YYYY", "").start(thursday), day(2026, 1, 1));
        assert_eq!(weekly().path_for(thursday), "2026-W39.md");
    }

    #[test]
    fn the_nearest_note_skips_the_gaps() {
        let weekly = weekly();
        let paths: Vec<String> =
            ["2026-W30.md", "2026-W35.md", "2026-W39.md", "2026-W44.md", "Plan.md", "2026-W99.md"].map(String::from).to_vec();
        assert_eq!(weekly.nearest(&paths, "2026-W39.md", -1), s("2026-W35.md"));
        assert_eq!(weekly.nearest(&paths, "2026-W39.md", 1), s("2026-W44.md"));
        assert_eq!(weekly.nearest(&paths, "2026-W30.md", -1), None);
        assert_eq!(weekly.nearest(&paths, "2026-W44.md", 1), None);
        let boundary: Vec<String> = ["2026-W01.md", "2026-W52.md", "2027-W01.md", "2027-W02.md"].map(String::from).to_vec();
        assert_eq!(weekly.nearest(&boundary, "2026-W52.md", 1), s("2027-W01.md"));
        assert_eq!(weekly.nearest(&boundary, "2027-W01.md", -1), s("2026-W52.md"));
        assert_eq!(weekly.nearest(&paths, "Plan.md", 1), None);
    }

    #[test]
    fn the_same_week_in_earlier_years() {
        let set = |paths: &[&str]| paths.iter().map(ToString::to_string).collect::<BTreeSet<String>>();
        let paths = set(&["2026-W39.md", "2025-W38.md", "2025-W39.md", "2024-W38.md", "2022-W39.md", "Plan.md"]);
        let found = weekly().years_ago(&paths, "2026-W39.md");
        assert_eq!(found.iter().map(|f| f.0).collect::<Vec<_>>(), [1, 2, 4]);
        assert_eq!(found.iter().map(|f| f.1.as_str()).collect::<Vec<_>>(), ["2025-W38.md", "2024-W38.md", "2022-W39.md"]);
        assert!(weekly().years_ago(&paths, "Plan.md").is_empty());
        let daily = notes("YYYY-MM-DD", "Journal");
        let journal = set(&["Journal/2026-09-27.md", "Journal/2025-09-27.md", "Journal/2025-09-26.md"]);
        assert_eq!(daily.years_ago(&journal, "Journal/2026-09-27.md"), [(1, "Journal/2025-09-27.md".to_string())]);
        assert!(notes("YYYY", "").years_ago(&set(&["2025.md", "2026.md"]), "2026.md").is_empty());
    }

    #[test]
    fn the_settings_file_is_read() {
        let config = DailyNotes::from_json(r#"{"format":"YYYY-[W]ww","folder":"/Journal/","template":"Templates/Week"}"#);
        assert_eq!(config.format, "YYYY-[W]ww");
        assert_eq!(config.folder, "Journal");
        assert_eq!(config.template, "Templates/Week");
        assert_eq!(DailyNotes::from_json(r#"{"format":""}"#).format, "YYYY-MM-DD");
        assert_eq!(DailyNotes::from_json("nonsense"), DailyNotes::default());
    }

    #[test]
    fn time_tokens_for_templates() {
        let clock = Clock::new(14, 5, 9);
        assert_eq!(MomentFormat::format("YYYY-MM-DD HH:mm:ss", day(2026, 10, 26), clock), "2026-10-26 14:05:09");
        assert_eq!(MomentFormat::format("h:mm A, dddd Do MMMM", day(2026, 10, 26), clock), "2:05 PM, Monday 26th October");
    }
}
