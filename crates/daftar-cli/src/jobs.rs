//! `daftar-cli jobs` and the MCP's `job_pipeline`: the job search of
//! `daftar_core::JobSearch` as plain lines.

use std::fs;
use std::path::Path;

use chrono::{Duration, NaiveDateTime};

use daftar_core::{Application, JobKeys, JobSearch, VaultIndex};

/// `stage\tcount`, a line each.
pub fn stages(search: &JobSearch) -> Vec<String> {
    search.stages().into_iter().map(|(stage, n)| format!("{stage}\t{n}")).collect()
}

/// `channel\tapplications\tpast applied\trate`, a line each, the most used first.
pub fn channels(search: &JobSearch) -> Vec<String> {
    search.channels().into_iter().map(|c| format!("{}\t{}\t{}\t{:.0}%", c.channel, c.applications, c.past_applied, c.rate())).collect()
}

/// The events from [now] on, within [days] when given: `path:line` of the
/// event's property, then the time, the company, its stage, the next step
/// and the preparation note (empty when there is none).
pub fn upcoming(root: &Path, search: &JobSearch, keys: &JobKeys, now: NaiveDateTime, days: Option<i64>) -> Vec<String> {
    let until = days.map(|days| (now.date() + Duration::days(days.max(0) + 1)).and_hms_opt(0, 0, 0).unwrap_or(now));
    search.upcoming(now, until).into_iter().map(|a| event(root, a, keys)).collect()
}

/// The MCP's `job_pipeline`: the stages, the channels and the events ahead,
/// each under its heading, in one answer.
pub fn pipeline(root: &Path, index: &VaultIndex, keep: impl Fn(&str) -> bool, days: Option<i64>) -> Vec<String> {
    let keys = JobKeys::default();
    let search = JobSearch::new(index, &keys, keep);
    let mut lines = vec!["stages (stage, applications):".to_string()];
    lines.extend(stages(&search));
    lines.push(String::new());
    lines.push("channels (channel, applications, past applied, rate):".into());
    lines.extend(channels(&search));
    lines.push(String::new());
    lines.push("upcoming (note:line, when, company, stage, next step, prep note):".into());
    let events = upcoming(root, &search, &keys, chrono::Local::now().naive_local(), days);
    if events.is_empty() {
        lines.push("(none booked)".into());
    }
    lines.extend(events);
    lines
}

fn event(root: &Path, application: &Application, keys: &JobKeys) -> String {
    let text = fs::read_to_string(root.join(&application.path)).unwrap_or_default();
    let line = JobSearch::property_line(&text, &keys.next);
    let when = application.next.map_or_else(String::new, |next| {
        if application.next_has_time { next.format("%Y-%m-%d %H:%M").to_string() } else { next.format("%Y-%m-%d").to_string() }
    });
    format!(
        "{}:{line}\t{when}\t{}\t{}\t{}\t{}",
        application.path,
        application.company,
        application.stage,
        application.next_step.as_deref().unwrap_or(""),
        application.prep.as_deref().unwrap_or(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use daftar_core::Vault;

    #[test]
    fn an_event_prints_where_it_is_written_and_its_prep_note() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("Companies/Acme")).unwrap();
        fs::write(
            root.path().join("Companies/Acme/Acme.md"),
            "---\njob_stage: screening\njob_next: 2026-11-10T15:30\njob_next_step: \"Recruiter call\"\njob_channel: \"Ashby (acme)\"\n---\n",
        )
        .unwrap();
        fs::write(root.path().join("Companies/Acme/Interview Preparation.md"), "# Prep\n").unwrap();
        let keys = JobKeys::default();
        let search = JobSearch::new(&Vault::index(root.path(), None), &keys, |_| true);
        assert_eq!(stages(&search), ["screening\t1"]);
        assert_eq!(channels(&search), ["Ashby\t1\t1\t100%"]);
        let now = NaiveDateTime::parse_from_str("2026-11-08T09:00", "%Y-%m-%dT%H:%M").unwrap();
        assert_eq!(
            upcoming(root.path(), &search, &keys, now, None),
            ["Companies/Acme/Acme.md:3\t2026-11-10 15:30\tAcme\tscreening\tRecruiter call\tCompanies/Acme/Interview Preparation.md"]
        );
        assert!(upcoming(root.path(), &search, &keys, now, Some(1)).is_empty());
        assert_eq!(upcoming(root.path(), &search, &keys, now, Some(2)).len(), 1);
    }
}
