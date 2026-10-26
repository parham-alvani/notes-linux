//! Reads the week notes in `Calendar/` (`2026-W44.md`). They are written by
//! Daftar on the phone, and their format is `Calendar/CLAUDE.md` in the vault.
//! The notes are only read here: they are written over at every change.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc, Weekday};
use regex::Regex;

use crate::day::Day;

/// One meeting, as Daftar on the phone wrote it into a week note. Times are
/// instants (UTC); show them in the local zone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Meeting {
    pub day: Day,
    /// `None` for an all-day event.
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub title: String,
    pub calendars: Vec<String>,
    pub join: Option<String>,
    pub tentative: bool,
    /// The calendar is set to busy only: the time is real, the rest is not here.
    pub busy_only: bool,
    pub overlaps: Option<String>,
}

impl Meeting {
    pub fn id(&self) -> String {
        format!("{} {} {}", self.day, self.start.map_or(0, |s| s.timestamp()), self.title)
    }

    pub fn all_day(&self) -> bool {
        self.start.is_none()
    }
}

/// One `## Monday 26 October` section of a week note.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CalendarDay {
    pub day: Day,
    pub free: Option<String>,
    pub meetings: Vec<Meeting>,
}

pub struct CalendarWeek;

static WEEK_NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Calendar/\d{4}-W\d{2}\.md$").unwrap());
static WEEK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4})-W(\d{2})\.md$").unwrap());
static ZONE_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Times are ([A-Za-z_]+/[A-Za-z_/+-]+|UTC)").unwrap());
static DAY_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^## \w+ (\d{1,2}) (\w+)\s*$").unwrap());
static EVENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- (All day|(\d{2}):(\d{2})–(?:\w{3} )?(\d{2}):(\d{2})) (.*)$").unwrap());
static CALENDARS_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(((?:<[^>]+>(?:, )?)+)\)$").unwrap());
static DETAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{2,}- (\w+): (.*)$").unwrap());
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([\\`*_{}\[\]()#+\-.!|<>~=])").unwrap());

const MONTHS: [&str; 12] =
    ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

/// Where the times of a note are: a named zone, or this computer's.
enum Zone {
    Named(chrono_tz::Tz),
    Local,
}

impl CalendarWeek {
    pub const FOLDER: &'static str = "Calendar";

    pub fn is_week_note(path: &str) -> bool {
        WEEK_NOTE.is_match(path)
    }

    /// The days of one week note. [path] gives the year, which the headings leave out.
    pub fn parse(text: &str, path: &str) -> Vec<CalendarDay> {
        let Some(caps) = WEEK.captures(path) else { return vec![] };
        let (Ok(iso_year), Ok(iso_week)) = (caps[1].parse::<i32>(), caps[2].parse::<u32>()) else { return vec![] };
        let Some(monday) = Self::monday_of(iso_year, iso_week) else { return vec![] };
        let zone = ZONE_LINE.captures(text).and_then(|c| c[1].parse::<chrono_tz::Tz>().ok()).map_or(Zone::Local, Zone::Named);

        let mut days: Vec<CalendarDay> = Vec::new();
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if let Some(heading) = DAY_HEADING.captures(line)
                && let Ok(date) = heading[1].parse::<u32>()
                && let Some(month) = MONTHS.iter().position(|m| *m == heading[2].to_lowercase())
            {
                // The week's Monday fixes the year: a week can start in December
                // and end in January.
                let day = [monday.year(), monday.year() + 1]
                    .into_iter()
                    .filter_map(|year| Day::new(year, month as u32 + 1, date))
                    .min_by_key(|d| monday.days_to(*d).abs());
                if let Some(day) = day {
                    days.push(CalendarDay { day, free: None, meetings: vec![] });
                }
                continue;
            }
            let Some(current) = days.last_mut() else { continue };
            if let Some(free) = line.strip_prefix("Free: ") {
                current.free = Some(free.trim_matches(['.', ' ']).to_string());
                continue;
            }
            if let Some(found) = EVENT.captures(line) {
                let day = current.day;
                let mut rest = found[6].to_string();
                let mut calendars = Vec::new();
                if let Some(suffix) = CALENDARS_SUFFIX.captures(&rest) {
                    calendars = suffix[1].split(',').map(|c| c.trim_matches([' ', '<', '>']).to_string()).collect();
                    let at = suffix.get(0).unwrap().start();
                    rest.truncate(at);
                }
                let tentative = rest.ends_with(" (maybe)");
                if tentative {
                    rest.truncate(rest.len() - " (maybe)".len());
                }
                let mut meeting = Meeting {
                    day,
                    start: None,
                    end: None,
                    title: unescape(&rest),
                    calendars,
                    join: None,
                    tentative,
                    busy_only: rest == "Busy",
                    overlaps: None,
                };
                if let (Some(h1), Some(m1), Some(h2), Some(m2)) = (found.get(2), found.get(3), found.get(4), found.get(5)) {
                    let number = |m: regex::Match| m.as_str().parse::<u32>().unwrap_or(0);
                    if let (Some(start), Some(mut end)) =
                        (time(day, number(h1), number(m1), &zone), time(day, number(h2), number(m2), &zone))
                    {
                        // `23:00–Sat 01:00` ends on a later day.
                        while end <= start {
                            end += chrono::Duration::days(1);
                        }
                        meeting.start = Some(start);
                        meeting.end = Some(end);
                    }
                }
                current.meetings.push(meeting);
                continue;
            }
            if let Some(found) = DETAIL.captures(line)
                && let Some(meeting) = current.meetings.last_mut()
            {
                let value = found[2].to_string();
                match &found[1] {
                    "Join" => meeting.join = Some(value.trim_matches(['<', '>']).to_string()),
                    "Overlaps" => meeting.overlaps = Some(value),
                    _ => {}
                }
            }
        }
        days
    }

    /// The path of the week note that holds [day].
    pub fn path(day: Day) -> String {
        let week = day.date().iso_week();
        format!("{}/{:04}-W{:02}.md", Self::FOLDER, week.year(), week.week())
    }

    pub fn monday_of(iso_year: i32, week: u32) -> Option<Day> {
        NaiveDate::from_isoywd_opt(iso_year, week, Weekday::Mon).map(Day::from)
    }
}

fn time(day: Day, hour: u32, minute: u32, zone: &Zone) -> Option<DateTime<Utc>> {
    let naive = day.date().and_hms_opt(hour, minute, 0)?;
    // A wall time a clock change skips is none; one it repeats is the first.
    match zone {
        Zone::Named(tz) => tz.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc)),
        Zone::Local => Local.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc)),
    }
}

/// The week notes escape markdown in titles so the vault's lint passes.
fn unescape(text: &str) -> String {
    ESCAPE.replace_all(text, "$1").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    const NOTE: &str = "# 2026-W53

Generated by Daftar from the calendars on the phone. Times are Europe/Madrid, and free time is counted from 09:00 to 18:00 every day. Edits to this note are written over.

## Thursday 31 December

Free: 09:00–10:00, 11:00–18:00.

- All day Holiday (<personal@example.com>)
- 10:00–11:00 Planning \\#3 (maybe) (<personal@example.com>, <work@example.com>)
    - Overlaps: 10:30 Standup
    - Join: <https://meet.example.com/abc>
- 23:00–Fri 01:00 Late call (<work@example.com>)

## Friday 1 January

No events.";

    #[test]
    fn reads_days_across_the_year() {
        let days = CalendarWeek::parse(NOTE, "Calendar/2026-W53.md");
        assert_eq!(days.iter().map(|d| d.day.to_string()).collect::<Vec<_>>(), ["2026-12-31", "2027-01-01"]);
        assert_eq!(days[0].free.as_deref(), Some("09:00–10:00, 11:00–18:00"));
        assert!(days[1].meetings.is_empty());
    }

    #[test]
    fn reads_meetings() {
        let meetings = &CalendarWeek::parse(NOTE, "Calendar/2026-W53.md")[0].meetings;
        assert_eq!(meetings.len(), 3);
        assert!(meetings[0].all_day() && meetings[0].title == "Holiday");
        let planning = &meetings[1];
        assert_eq!(planning.title, "Planning #3");
        assert!(planning.tentative);
        assert_eq!(planning.calendars, ["personal@example.com", "work@example.com"]);
        assert_eq!(planning.join.as_deref(), Some("https://meet.example.com/abc"));
        assert_eq!(planning.overlaps.as_deref(), Some("10:30 Standup"));
        let start = planning.start.unwrap().with_timezone(&chrono_tz::Europe::Madrid);
        assert_eq!(start.hour(), 10);
        let late = &meetings[2];
        assert_eq!((late.end.unwrap() - late.start.unwrap()).num_seconds(), 2 * 3600);
    }

    #[test]
    fn week_note_path() {
        assert_eq!(CalendarWeek::path(Day::parse("2026-10-25").unwrap()), "Calendar/2026-W43.md");
        assert_eq!(CalendarWeek::path(Day::parse("2027-01-01").unwrap()), "Calendar/2026-W53.md");
        assert!(CalendarWeek::is_week_note("Calendar/2026-W43.md"));
        assert!(!CalendarWeek::is_week_note("Calendar/CLAUDE.md"));
    }
}
