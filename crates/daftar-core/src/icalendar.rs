//! An iCalendar file (RFC 5545) of events: `daftar-cli ics` writes the
//! vault's dated tasks and booked job events with it. Only what that needs:
//! VEVENTs with a summary, a description and categories, escaped and folded
//! as the RFC asks. No Swift twin yet.

use std::fmt::Write as _;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

/// When an event starts or ends.
///
/// A time is written floating (RFC 5545 3.3.5, form 1: no `Z`, no `TZID`):
/// the vault writes `09:30` and `job_next: 2026-11-10T15:30` on the wall
/// clock with no zone, and a floating time says exactly that. Google
/// Calendar, Apple Calendar, Thunderbird and Outlook all import it at that
/// hour of the calendar's own zone. UTC would bake in the zone of the
/// computer that wrote the file, and a `TZID` would need a VTIMEZONE of its
/// rules, which Google ignores and Outlook needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcsTime {
    /// An all-day event's day (`VALUE=DATE`).
    Day(NaiveDate),
    /// A time on the wall clock, floating.
    Local(NaiveDateTime),
}

/// One VEVENT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsEvent {
    /// Stable across exports, so a calendar that imports the file again
    /// moves the event instead of adding it twice: see `ICalendar::uid`.
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub categories: Vec<String>,
    pub start: IcsTime,
    /// The day after the last for an all-day event, as DTEND is exclusive.
    pub end: IcsTime,
}

pub struct ICalendar;

impl ICalendar {
    /// The longest line, in octets, without its CRLF (RFC 5545 3.1).
    pub const LINE_OCTETS: usize = 75;

    /// The calendar of [events]; [stamp] is when it was made (DTSTAMP).
    pub fn write(name: &str, events: &[IcsEvent], stamp: DateTime<Utc>) -> String {
        let mut lines = vec![
            "BEGIN:VCALENDAR".to_string(),
            "VERSION:2.0".into(),
            "PRODID:-//Daftar//daftar-cli//EN".into(),
            "CALSCALE:GREGORIAN".into(),
            "METHOD:PUBLISH".into(),
            format!("X-WR-CALNAME:{}", Self::escape(name)),
        ];
        let stamp = stamp.format("%Y%m%dT%H%M%SZ").to_string();
        for event in events {
            lines.push("BEGIN:VEVENT".into());
            lines.push(format!("UID:{}", event.uid));
            lines.push(format!("DTSTAMP:{stamp}"));
            lines.push(format!("DTSTART{}", Self::time(event.start)));
            lines.push(format!("DTEND{}", Self::time(event.end)));
            lines.push(format!("SUMMARY:{}", Self::escape(&event.summary)));
            if !event.description.is_empty() {
                lines.push(format!("DESCRIPTION:{}", Self::escape(&event.description)));
            }
            if !event.categories.is_empty() {
                let categories: Vec<String> = event.categories.iter().map(|c| Self::escape(c)).collect();
                lines.push(format!("CATEGORIES:{}", categories.join(",")));
            }
            if matches!(event.start, IcsTime::Day(_)) {
                // An all-day task does not make its day busy.
                lines.push("TRANSP:TRANSPARENT".into());
            }
            lines.push("END:VEVENT".into());
        }
        lines.push("END:VCALENDAR".into());
        let mut text = String::new();
        for line in lines {
            let _ = write!(text, "{}\r\n", Self::fold(&line));
        }
        text
    }

    /// A TEXT value (RFC 5545 3.3.11): `\`, `;` and `,` escaped, a line
    /// break as `\n`, and no other control character.
    pub fn escape(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => escaped.push_str("\\\\"),
                ';' => escaped.push_str("\\;"),
                ',' => escaped.push_str("\\,"),
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' | '\r' => escaped.push_str("\\n"),
                '\t' => escaped.push(' '),
                c if c.is_control() => {}
                c => escaped.push(c),
            }
        }
        escaped
    }

    /// [line] folded (RFC 5545 3.1): no line longer than `LINE_OCTETS`
    /// octets, each one after the first starting with a space that counts
    /// in its length, and never in the middle of a character.
    pub fn fold(line: &str) -> String {
        let mut folded = String::with_capacity(line.len() + line.len() / 70 * 3);
        let mut length = 0;
        for c in line.chars() {
            if length + c.len_utf8() > Self::LINE_OCTETS {
                folded.push_str("\r\n ");
                length = 1;
            }
            folded.push(c);
            length += c.len_utf8();
        }
        folded
    }

    /// A UID from [parts] (a note's path and a task's text, say): the
    /// same parts give the same UID on every computer and in every
    /// version, as it is FNV-1a, not Rust's hasher, which may change.
    pub fn uid(parts: &[&str]) -> String {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0100_0000_01b3;
        let mut hash = OFFSET;
        for (index, part) in parts.iter().enumerate() {
            // A separator, so ("ab", "c") and ("a", "bc") differ.
            let separator: &[u8] = if index == 0 { &[] } else { &[0] };
            for byte in separator.iter().chain(part.as_bytes()) {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
        format!("{hash:016x}@daftar")
    }

    fn time(time: IcsTime) -> String {
        match time {
            IcsTime::Day(day) => format!(";VALUE=DATE:{}", day.format("%Y%m%d")),
            IcsTime::Local(at) => format!(":{}", at.format("%Y%m%dT%H%M%S")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(ICalendar::escape("Call Sara; bring notes, slides"), "Call Sara\\; bring notes\\, slides");
        assert_eq!(ICalendar::escape("C:\\path"), "C:\\\\path");
        assert_eq!(ICalendar::escape("one\ntwo\r\nthree\rfour"), "one\\ntwo\\nthree\\nfour");
        assert_eq!(ICalendar::escape("a\tb\u{7}c"), "a bc");
        assert_eq!(ICalendar::escape("Colon: and \"quotes\" stay"), "Colon: and \"quotes\" stay");
    }

    #[test]
    fn a_long_line_is_folded_at_75_octets_and_never_inside_a_character() {
        let line = format!("SUMMARY:{}", "a".repeat(200));
        let folded = ICalendar::fold(&line);
        let parts: Vec<&str> = folded.split("\r\n").collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 75);
        assert_eq!(parts[1].len(), 75);
        assert!(parts[1].starts_with(' ') && parts[2].starts_with(' '));
        assert_eq!(parts.iter().enumerate().map(|(i, p)| if i == 0 { *p } else { &p[1..] }).collect::<String>(), line);

        // Persian is two octets a letter and an emoji four: no part is cut.
        let line = format!("SUMMARY:{}", "سلام 📅 ".repeat(20));
        let folded = ICalendar::fold(&line);
        for part in folded.split("\r\n") {
            assert!(part.len() <= ICalendar::LINE_OCTETS, "{part}");
        }
        assert_eq!(folded.replace("\r\n ", ""), line);
        assert_eq!(ICalendar::fold("SHORT:yes"), "SHORT:yes");
        assert_eq!(ICalendar::fold(&"x".repeat(75)), "x".repeat(75));
    }

    #[test]
    fn a_uid_is_stable_and_tells_its_parts_apart() {
        let uid = ICalendar::uid(&["Tasks/Work.md", "Send the report"]);
        assert_eq!(uid, ICalendar::uid(&["Tasks/Work.md", "Send the report"]));
        assert!(uid.ends_with("@daftar") && uid.len() == 16 + "@daftar".len());
        assert_ne!(uid, ICalendar::uid(&["Tasks/Home.md", "Send the report"]));
        assert_ne!(ICalendar::uid(&["ab", "c"]), ICalendar::uid(&["a", "bc"]));
        // FNV-1a's own value: a change of the hash would move every event.
        assert_eq!(ICalendar::uid(&["a"]), "af63dc4c8601ec8c@daftar");
    }

    #[test]
    fn a_calendar_has_its_events_with_crlf_lines() {
        let events = [
            IcsEvent {
                uid: "1@daftar".into(),
                summary: "Pay rent, today".into(),
                description: "Tasks/Home.md".into(),
                categories: vec!["Task".into()],
                start: IcsTime::Day(day("2026-11-16")),
                end: IcsTime::Day(day("2026-11-17")),
            },
            IcsEvent {
                uid: "2@daftar".into(),
                summary: "Acme (screening)".into(),
                description: String::new(),
                categories: vec![],
                start: IcsTime::Local(day("2026-11-17").and_hms_opt(14, 0, 0).unwrap()),
                end: IcsTime::Local(day("2026-11-17").and_hms_opt(15, 0, 0).unwrap()),
            },
        ];
        let stamp = day("2026-11-15").and_hms_opt(8, 30, 0).unwrap().and_utc();
        let text = ICalendar::write("Daftar", &events, stamp);
        assert!(text.ends_with("END:VCALENDAR\r\n"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        let lines: Vec<&str> = text.split("\r\n").collect();
        for expected in [
            "BEGIN:VCALENDAR",
            "VERSION:2.0",
            "DTSTAMP:20261115T083000Z",
            "DTSTART;VALUE=DATE:20261116",
            "DTEND;VALUE=DATE:20261117",
            "SUMMARY:Pay rent\\, today",
            "CATEGORIES:Task",
            "TRANSP:TRANSPARENT",
            "DTSTART:20261117T140000",
            "DTEND:20261117T150000",
        ] {
            assert!(lines.contains(&expected), "{expected} in\n{text}");
        }
        assert_eq!(lines.iter().filter(|l| **l == "BEGIN:VEVENT").count(), 2);
        assert_eq!(lines.iter().filter(|l| l.starts_with("DESCRIPTION:")).count(), 1);
    }
}
