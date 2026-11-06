//! `check --memory`: the warnings that keep notes used as an agent's memory
//! current, for a vault whose notes open with their state now and keep a
//! dated history below (the `org-notes` skill's shape). A note read on
//! another day must not say "next week"; a booked event that has passed
//! must leave the properties; a paragraph that opens with
//! `**Where it stands (YYYY-MM-DD):**` must be no older than the newest
//! history entry (`**YYYY-MM-DD …**` at the start of a line); and a
//! preparation note's call that has been held gets a `> [!done]` callout.

use std::sync::LazyLock;

use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;

/// What `--memory` checks, from the vault's own conventions.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    /// Properties that hold a booked event's date or time (`job_next`).
    pub event_properties: Vec<String>,
    /// The names of preparation notes (`Interview Preparation`), whose
    /// dated call headings must say when the call was held.
    pub prep_notes: Vec<String>,
}

static STATUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\*\*Where it stands \((\d{4}-\d{2}-\d{2})\):?\*\*").unwrap());
static ENTRY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\*\*(\d{4}-\d{2}-\d{2})\b").unwrap());
static RELATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(today|tomorrow|yesterday|tonight|this (?:week|morning|afternoon|evening)|next week|last week)\b").unwrap()
});
/// What a relative word inside is someone else's: code and quotations.
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"`[^`]*`|"[^"]*"|“[^”]*”"#).unwrap());
static DAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{4}-\d{2}-\d{2})\b").unwrap());
static MEETING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(call|interview|round|screen|meeting|demo)\b").unwrap());
static SETTLED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(held|cancell?ed|outcome)\b").unwrap());

/// The problems of the note at [path] with [text]: its line from 1, the
/// rule and what to do.
pub fn problems(path: &str, text: &str, rules: &Rules, now: NaiveDateTime) -> Vec<(usize, &'static str, String)> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let body = front_matter_end(&lines);
    let mut found = Vec::new();
    past_events(&lines[..body], rules, now, &mut found);
    let status = lines.iter().enumerate().skip(body).find_map(|(i, l)| STATUS.captures(l).map(|c| (i, c[1].to_string())));
    if let Some((at, dated)) = &status {
        let newest = lines.iter().skip(body).filter_map(|l| ENTRY.captures(l)).map(|c| c[1].to_string()).max();
        if let Some(newest) = newest.filter(|n| n > dated) {
            found.push((
                at + 1,
                "memory-stale-status",
                format!("the current state is dated {dated}, but the history has an entry of {newest}: rewrite it"),
            ));
        }
        relative_dates(&lines, body, &mut found);
    }
    let stem = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".md");
    if rules.prep_notes.iter().any(|name| name.trim_end_matches(".md") == stem) {
        held_calls(&lines, body, now.date(), &mut found);
    }
    found
}

/// The line after the front matter, or 0 when there is none.
fn front_matter_end(lines: &[&str]) -> usize {
    if lines.first() != Some(&"---") {
        return 0;
    }
    lines.iter().skip(1).position(|l| *l == "---" || *l == "...").map_or(0, |at| at + 2)
}

fn past_events(front: &[&str], rules: &Rules, now: NaiveDateTime, found: &mut Vec<(usize, &'static str, String)>) {
    for (i, line) in front.iter().enumerate() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim();
        if !rules.event_properties.iter().any(|k| k == key) {
            continue;
        }
        let value = value.trim().trim_matches(['"', '\'']);
        let when = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
            .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
            .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M"))
            .ok()
            .or_else(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok().and_then(|d| d.succ_opt()).and_then(|d| d.and_hms_opt(0, 0, 0)));
        if when.is_some_and(|when| when <= now) {
            found.push((i + 1, "memory-past-event", format!("{key} {value} has passed: remove it, or set the next booked event")));
        }
    }
}

fn relative_dates(lines: &[&str], body: usize, found: &mut Vec<(usize, &'static str, String)>) {
    let mut fence = false;
    for (i, line) in lines.iter().enumerate().skip(body) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence || trimmed.starts_with('>') {
            continue;
        }
        let own = QUOTED.replace_all(line, "");
        if let Some(word) = RELATIVE.find(&own) {
            found.push((i + 1, "memory-relative-date", format!("“{}” in a note read on other days: write the date", word.as_str())));
        }
    }
}

fn held_calls(lines: &[&str], body: usize, today: NaiveDate, found: &mut Vec<(usize, &'static str, String)>) {
    let mut fence = false;
    let mut headings = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(body) {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        } else if !fence && line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count();
            if line[level..].starts_with(' ') {
                headings.push((i, level));
            }
        }
    }
    for (k, &(at, level)) in headings.iter().enumerate() {
        let heading = lines[at];
        let Some(day) = DAY.captures(heading).and_then(|c| NaiveDate::parse_from_str(&c[1], "%Y-%m-%d").ok()) else { continue };
        if day >= today || !MEETING.is_match(heading) || SETTLED.is_match(heading) {
            continue;
        }
        let end = headings[k + 1..].iter().find(|(_, l)| *l <= level).map_or(lines.len(), |(i, _)| *i);
        if !lines[at..end].iter().any(|l| l.trim_start().to_lowercase().starts_with("> [!done]")) {
            found.push((
                at + 1,
                "memory-prep-held",
                format!("the call of {day} has passed with no `> [!done] Held on` callout: say it was held and where the outcome is"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 6).unwrap().and_hms_opt(12, 0, 0).unwrap()
    }

    fn rules() -> Rules {
        Rules { event_properties: vec!["job_next".into()], prep_notes: vec!["Interview Preparation".into()] }
    }

    fn kinds(path: &str, text: &str) -> Vec<(usize, &'static str)> {
        problems(path, text, &rules(), now()).into_iter().map(|(line, rule, _)| (line, rule)).collect()
    }

    #[test]
    fn a_booked_event_that_has_passed_is_found() {
        let text = "---\njob_stage: interview\njob_next: 2026-11-06T09:30\n---\nText\n";
        assert_eq!(kinds("A.md", text), [(3, "memory-past-event")]);
        assert!(kinds("A.md", "---\njob_next: 2026-11-06T18:30\n---\n").is_empty());
        // A day alone lasts to its end.
        assert!(kinds("A.md", "---\njob_next: 2026-11-06\n---\n").is_empty());
        assert_eq!(kinds("A.md", "---\njob_next: \"2026-11-05\"\n---\n"), [(2, "memory-past-event")]);
        // Other properties and the body are not events.
        assert!(kinds("A.md", "---\njob_next_step: 2026-01-01\n---\njob_next: 2026-01-01\n").is_empty());
    }

    #[test]
    fn a_state_older_than_the_history_is_stale() {
        let stale =
            "**Where it stands (2026-11-02):** Waiting.\n\n## History\n\n**2026-11-01:** Applied.\n\n**2026-11-04 10:00 CET** — Call.\n";
        assert_eq!(kinds("A.md", stale), [(1, "memory-stale-status")]);
        let fresh = stale.replace("(2026-11-02)", "(2026-11-04)");
        assert!(kinds("A.md", &fresh).is_empty());
    }

    #[test]
    fn a_relative_date_in_a_memory_note_is_found_but_not_in_a_quote() {
        let text = "**Where it stands (2026-11-06):** The call is tomorrow.\n\n> He said \"see you next week\".\n\nShe wrote \"talk next week\".\n\n`today` is code.\n\n```text\nyesterday\n```\n";
        assert_eq!(kinds("A.md", text), [(1, "memory-relative-date")]);
        // A note with no state paragraph is not a memory note.
        assert!(kinds("Learning/A.md", "Systems today are fast.\n").is_empty());
    }

    #[test]
    fn a_prep_note_says_when_its_call_was_held() {
        let text = "Prep.\n\n## Round 1 — call, Wed 2026-10-28\n\n- When: 12:30\n\n### Questions\n\n- One\n\n## Round 2 — call, Tue 2026-11-10\n\n- When: 10:00\n\n## Call with Ana — 2026-11-02\n\n> [!done] Held on 2026-11-02\n>\n> Outcome in [[A]].\n\n## Outcome — rejected 2026-11-03\n";
        assert_eq!(kinds("Co/Interview Preparation.md", text), [(3, "memory-prep-held")]);
        assert!(kinds("Co/Notes.md", text).is_empty());
    }
}
