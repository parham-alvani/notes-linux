//! A job search as data: every note with a `job_stage` property is an
//! application (`Companies/Acme/Acme.md`), and its front matter says where
//! it stands. From them: how many are at each stage, how each channel
//! (`job_channel`) did, the booked events ahead (`job_next`) with the
//! company's `Interview Preparation.md` when its folder has one, and which
//! company a meeting of the week notes is with.
//!
//! An application "got past applied" when its stage is neither the waiting
//! one (`applied`) nor a closed one (`rejected`, `withdrawn`), or when it is
//! closed but its folder has a preparation note: a rejection after a call
//! is a reply, one without any is not, and the stage alone cannot tell the
//! two apart.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate, NaiveDateTime};

use crate::vault::{Vault, VaultIndex};

/// The properties an application is read from. [Default] is the `job_` ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobKeys {
    pub stage: String,
    pub applied: String,
    pub roles: String,
    /// The next booked event, a date-time (`2026-11-10T15:30`) or a day.
    pub next: String,
    pub next_step: String,
    pub channel: String,
    /// Stages still waiting for a first answer.
    pub waiting: Vec<String>,
    /// Stages of an application that is over.
    pub closed: Vec<String>,
    /// The name of a company's preparation note, in its folder.
    pub prep_note: String,
}

impl Default for JobKeys {
    fn default() -> JobKeys {
        JobKeys {
            stage: "job_stage".into(),
            applied: "job_applied".into(),
            roles: "job_roles".into(),
            next: "job_next".into(),
            next_step: "job_next_step".into(),
            channel: "job_channel".into(),
            waiting: vec!["applied".into()],
            closed: vec!["rejected".into(), "withdrawn".into(), "declined".into(), "closed".into()],
            prep_note: JobSearch::PREP_NOTE.into(),
        }
    }
}

/// One company's application, from its note's front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub path: String,
    /// The note's name: the company.
    pub company: String,
    pub aliases: Vec<String>,
    /// Lower case, as written otherwise.
    pub stage: String,
    pub applied: Option<String>,
    pub roles: Vec<String>,
    /// The next booked event; a day alone is at midnight, with [next_has_time] false.
    pub next: Option<NaiveDateTime>,
    pub next_has_time: bool,
    pub next_step: Option<String>,
    /// [JobSearch::channel] of `job_channel`.
    pub channel: String,
    /// The preparation note in the same folder, when there is one.
    pub prep: Option<String>,
    /// The stage is neither waiting nor closed, or it closed after a call.
    pub past_applied: bool,
    /// The stage is a closed one.
    pub closed: bool,
}

impl Application {
    /// The note to prepare with: the preparation note, else the company's.
    pub fn prep_or_note(&self) -> &str {
        self.prep.as_deref().unwrap_or(&self.path)
    }
}

/// How one channel did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCount {
    pub channel: String,
    pub applications: usize,
    pub past_applied: usize,
}

impl ChannelCount {
    /// The share that got past applied, in percent.
    pub fn rate(&self) -> f64 {
        if self.applications == 0 { 0.0 } else { self.past_applied as f64 * 100.0 / self.applications as f64 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobSearch {
    /// By path.
    pub applications: Vec<Application>,
}

/// The order of the stages a search usually has; others come after, by name.
const ORDER: [&str; 9] = ["applied", "screening", "interviewing", "offer", "accepted", "rejected", "withdrawn", "declined", "closed"];

/// How far a meeting's start may be from a company's `job_next` and still
/// be that event: a long block of busy time holds many events, and is none.
const NEAR: i64 = 15;

impl JobSearch {
    pub const PREP_NOTE: &'static str = "Interview Preparation.md";

    /// The applications of [index]'s notes that [keep] keeps.
    pub fn new(index: &VaultIndex, keys: &JobKeys, keep: impl Fn(&str) -> bool) -> JobSearch {
        let mut applications: Vec<Application> = index
            .notes
            .values()
            .filter(|note| keep(&note.path))
            .filter_map(|note| {
                let values = |key: &str| {
                    let key = key.to_lowercase();
                    note.parsed.properties.iter().find(|(k, _)| k.to_lowercase() == key).map(|(_, v)| v.clone())
                };
                let first =
                    |key: &str| values(key).and_then(|v| v.into_iter().next()).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
                let stage = first(&keys.stage)?.to_lowercase();
                let folder = note.path.rsplit_once('/').map_or("", |(folder, _)| folder);
                let prep_path = if folder.is_empty() { keys.prep_note.clone() } else { format!("{folder}/{}", keys.prep_note) };
                let prep = (prep_path != note.path && index.files.contains(&prep_path)).then_some(prep_path);
                let closed = keys.closed.iter().any(|s| s.eq_ignore_ascii_case(&stage));
                let waiting = keys.waiting.iter().any(|s| s.eq_ignore_ascii_case(&stage));
                let past_applied = if closed { prep.is_some() } else { !waiting };
                let next_text = first(&keys.next);
                let next = next_text.as_deref().and_then(date_time);
                Some(Application {
                    path: note.path.clone(),
                    company: Vault::title(&note.path),
                    aliases: note.parsed.aliases.clone(),
                    applied: first(&keys.applied),
                    roles: values(&keys.roles).unwrap_or_default(),
                    next: next.map(|(at, _)| at),
                    next_has_time: next.is_some_and(|(_, timed)| timed),
                    next_step: first(&keys.next_step),
                    channel: Self::channel(first(&keys.channel).as_deref().unwrap_or("")),
                    prep,
                    past_applied,
                    closed,
                    stage,
                })
            })
            .collect();
        applications.sort_by(|a, b| a.path.cmp(&b.path));
        JobSearch { applications }
    }

    /// `Kulturo (Someone, LinkedIn)` as `Kulturo`: the text before ` (`,
    /// trimmed, and `unknown` when there is none.
    pub fn channel(text: &str) -> String {
        let name = text.split_once(" (").map_or(text, |(name, _)| name).trim();
        if name.is_empty() { "unknown".into() } else { name.to_string() }
    }

    /// Each stage with how many applications are at it, in a search's order.
    pub fn stages(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for application in &self.applications {
            *counts.entry(application.stage.as_str()).or_default() += 1;
        }
        let mut stages: Vec<(String, usize)> = counts.into_iter().map(|(stage, n)| (stage.to_string(), n)).collect();
        let rank = |stage: &str| ORDER.iter().position(|s| *s == stage).unwrap_or(ORDER.len());
        stages.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then(a.0.cmp(&b.0)));
        stages
    }

    /// Each channel with its applications and how many got past applied,
    /// the most used first.
    pub fn channels(&self) -> Vec<ChannelCount> {
        let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
        for application in &self.applications {
            let entry = counts.entry(application.channel.as_str()).or_default();
            entry.0 += 1;
            entry.1 += usize::from(application.past_applied);
        }
        let mut channels: Vec<ChannelCount> = counts
            .into_iter()
            .map(|(channel, (applications, past_applied))| ChannelCount { channel: channel.to_string(), applications, past_applied })
            .collect();
        channels
            .sort_by(|a, b| b.applications.cmp(&a.applications).then(b.past_applied.cmp(&a.past_applied)).then(a.channel.cmp(&b.channel)));
        channels
    }

    /// The booked events from [now] on, before [until] when given, the
    /// soonest first. An event of a day alone is ahead all that day.
    pub fn upcoming(&self, now: NaiveDateTime, until: Option<NaiveDateTime>) -> Vec<&Application> {
        let mut ahead: Vec<&Application> = self
            .applications
            .iter()
            .filter(|a| {
                a.next.is_some_and(|next| {
                    let ahead = if a.next_has_time { next >= now } else { next.date() >= now.date() };
                    ahead && until.is_none_or(|until| next < until)
                })
            })
            .collect();
        ahead.sort_by(|a, b| a.next.cmp(&b.next).then(a.path.cmp(&b.path)));
        ahead
    }

    /// The application a meeting from [start] to [end] (local times) called
    /// [title] is about: a going company whose name or alias the title
    /// holds as a word, the longest name first; else the company whose
    /// `job_next` is within the meeting and at most a quarter of an hour
    /// from its start.
    pub fn for_meeting(&self, start: NaiveDateTime, end: NaiveDateTime, title: &str) -> Option<&Application> {
        let title = title.to_lowercase();
        let named = self
            .applications
            .iter()
            .filter(|a| !a.closed || a.next.is_some_and(|next| next.date() == start.date()))
            .filter_map(|a| {
                std::iter::once(&a.company)
                    .chain(&a.aliases)
                    .filter(|name| holds_word(&title, &name.to_lowercase()))
                    .map(|name| name.chars().count())
                    .max()
                    .map(|length| (length, a))
            })
            .max_by(|x, y| x.0.cmp(&y.0).then(y.1.path.cmp(&x.1.path)));
        if let Some((_, application)) = named {
            return Some(application);
        }
        self.applications
            .iter()
            .filter(|a| a.next_has_time)
            .filter_map(|a| {
                let next = a.next?;
                let off = (next - start).num_minutes();
                (next >= start && next < end.max(start + Duration::minutes(1)) && off <= NEAR).then_some((off, a))
            })
            .min_by(|x, y| x.0.cmp(&y.0).then(x.1.path.cmp(&y.1.path)))
            .map(|(_, a)| a)
    }

    /// The line, from 1, of [key] in the front matter of [text]; 1 when it is not there.
    pub fn property_line(text: &str, key: &str) -> usize {
        let mut lines = text.split('\n');
        if lines.next().map(str::trim_end) != Some("---") {
            return 1;
        }
        for (at, line) in lines.enumerate() {
            let line = line.trim_end();
            if line == "---" || line == "..." {
                break;
            }
            if line.split_once(':').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key)) {
                return at + 2;
            }
        }
        1
    }
}

/// `2026-11-10T15:30` (or with seconds, or a space), or a day alone; with
/// whether it had a time.
fn date_time(text: &str) -> Option<(NaiveDateTime, bool)> {
    let text = text.trim().trim_matches(['"', '\'']);
    ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M:%S"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .map(|at| (at, true))
        .or_else(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|at| (at, false)))
}

/// [haystack] holds [word] with no letter or digit right before or after it.
fn holds_word(haystack: &str, word: &str) -> bool {
    if word.trim().is_empty() {
        return false;
    }
    haystack.match_indices(word).any(|(at, _)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoteParser;
    use crate::vault::NoteEntry;

    fn index(notes: &[(&str, &str)], files: &[&str]) -> VaultIndex {
        let mut index = VaultIndex::default();
        for (path, text) in notes {
            index.notes.insert(
                (*path).into(),
                NoteEntry { path: (*path).into(), modified: std::time::SystemTime::now(), parsed: NoteParser::parse(text) },
            );
            index.files.insert((*path).into());
        }
        for file in files {
            index.files.insert((*file).into());
        }
        index.derive();
        index
    }

    fn search() -> JobSearch {
        let index = index(
            &[
                (
                    "Companies/Acme/Acme.md",
                    "---\njob_stage: interviewing\njob_applied: 2026-10-01\njob_roles:\n  - \"Platform Engineer\"\njob_next: 2026-11-10T15:30\njob_next_step: \"Second round\"\njob_channel: \"Kulturo (Someone, LinkedIn)\"\n---\n# Acme",
                ),
                ("Companies/Acme/Interview Preparation.md", "# Prep"),
                ("Companies/Globex/Globex.md", "---\njob_stage: screening\njob_next: 2026-11-09T14:00\njob_channel: LinkedIn\n---\n"),
                ("Companies/Initech/Initech.md", "---\njob_stage: applied\njob_channel: \"LinkedIn\"\n---\n"),
                ("Companies/Hooli/Hooli.md", "---\njob_stage: Rejected\njob_channel: \"Ashby (hooli)\"\n---\n"),
                ("Companies/Hooli/Interview Preparation.md", "# Prep"),
                ("Companies/Umbrella/Umbrella.md", "---\njob_stage: rejected\njob_channel: Ashby\njob_next: 2026-11-01\n---\n"),
                ("Companies/Stark/Stark.md", "---\naliases: [Stark Industries]\njob_stage: applied\n---\n"),
                ("Companies/Wayne/Wayne.md", "---\nindustry: bats\n---\n"),
            ],
            &[],
        );
        JobSearch::new(&index, &JobKeys::default(), |_| true)
    }

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").unwrap()
    }

    #[test]
    fn a_channel_is_the_text_before_its_parenthesis() {
        assert_eq!(JobSearch::channel("Kulturo (Someone, LinkedIn)"), "Kulturo");
        assert_eq!(JobSearch::channel("  Wellfound → Ashby "), "Wellfound → Ashby");
        assert_eq!(JobSearch::channel(""), "unknown");
        assert_eq!(JobSearch::channel(" (nobody)"), "unknown");
    }

    #[test]
    fn every_note_with_a_stage_is_an_application() {
        let search = search();
        let companies: Vec<&str> = search.applications.iter().map(|a| a.company.as_str()).collect();
        assert_eq!(companies, ["Acme", "Globex", "Hooli", "Initech", "Stark", "Umbrella"]);
        let acme = &search.applications[0];
        assert_eq!(acme.roles, ["Platform Engineer"]);
        assert_eq!(acme.applied.as_deref(), Some("2026-10-01"));
        assert_eq!(acme.prep.as_deref(), Some("Companies/Acme/Interview Preparation.md"));
        assert_eq!(acme.channel, "Kulturo");
        assert_eq!(search.applications[1].prep_or_note(), "Companies/Globex/Globex.md");
    }

    #[test]
    fn stages_are_counted_in_a_searchs_order() {
        assert_eq!(
            search().stages(),
            [("applied".to_string(), 2), ("screening".to_string(), 1), ("interviewing".to_string(), 1), ("rejected".to_string(), 2)]
        );
    }

    #[test]
    fn a_closed_application_got_past_applied_only_with_a_prep_note() {
        let search = search();
        let past: Vec<&str> = search.applications.iter().filter(|a| a.past_applied).map(|a| a.company.as_str()).collect();
        assert_eq!(past, ["Acme", "Globex", "Hooli"]);
        let channels = search.channels();
        let row = |name: &str| channels.iter().find(|c| c.channel == name).unwrap().clone();
        assert_eq!(row("LinkedIn"), ChannelCount { channel: "LinkedIn".into(), applications: 2, past_applied: 1 });
        assert_eq!(row("Ashby"), ChannelCount { channel: "Ashby".into(), applications: 2, past_applied: 1 });
        assert_eq!(row("unknown").applications, 1);
        assert!((row("LinkedIn").rate() - 50.0).abs() < 1e-9);
        assert_eq!(channels[0].channel, "Ashby");
    }

    #[test]
    fn upcoming_events_are_the_booked_ones_ahead_soonest_first() {
        let search = search();
        let ahead: Vec<&str> = search.upcoming(at("2026-11-09T09:00"), None).iter().map(|a| a.company.as_str()).collect();
        assert_eq!(ahead, ["Globex", "Acme"]);
        let ahead: Vec<&str> = search.upcoming(at("2026-11-09T15:00"), None).iter().map(|a| a.company.as_str()).collect();
        assert_eq!(ahead, ["Acme"]);
        let until = Some(at("2026-11-10T00:00"));
        assert_eq!(search.upcoming(at("2026-11-09T09:00"), until).len(), 1);
        // A day alone is ahead all that day.
        assert_eq!(search.upcoming(at("2026-11-01T18:00"), until).first().map(|a| a.company.as_str()), Some("Umbrella"));
    }

    #[test]
    fn a_meeting_is_matched_by_its_time_or_the_company_it_names() {
        let search = search();
        let meeting = |start: &str, end: &str, title: &str| search.for_meeting(at(start), at(end), title).map(|a| a.company.clone());
        assert_eq!(meeting("2026-11-10T15:30", "2026-11-10T16:15", "Interview · Google Meet").as_deref(), Some("Acme"));
        assert_eq!(meeting("2026-11-12T10:00", "2026-11-12T11:00", "Interview with ACME - Backend").as_deref(), Some("Acme"));
        assert_eq!(meeting("2026-11-12T10:00", "2026-11-12T11:00", "Call with Stark Industries").as_deref(), Some("Stark"));
        // A word inside another is not the name, and a closed company is not named.
        assert_eq!(meeting("2026-11-12T10:00", "2026-11-12T11:00", "Acmeology"), None);
        assert_eq!(meeting("2026-11-12T10:00", "2026-11-12T11:00", "Coffee at Hooli"), None);
        // A long block of busy time holds the event, but is not it.
        assert_eq!(meeting("2026-11-09T06:30", "2026-11-09T15:30", "Busy"), None);
        assert_eq!(meeting("2026-11-09T13:50", "2026-11-09T15:00", "Call").as_deref(), Some("Globex"));
    }

    #[test]
    fn the_keys_are_parameters() {
        let index = index(&[("Leads/Acme.md", "---\nstate: talking\nvia: Email\n---\n")], &["Leads/Prep.md"]);
        let keys = JobKeys { stage: "state".into(), channel: "via".into(), prep_note: "Prep.md".into(), ..JobKeys::default() };
        let search = JobSearch::new(&index, &keys, |_| true);
        assert_eq!(search.stages(), [("talking".to_string(), 1)]);
        assert_eq!(search.applications[0].prep.as_deref(), Some("Leads/Prep.md"));
        assert_eq!(search.channels()[0].channel, "Email");
        assert!(JobSearch::new(&index, &keys, |path| !path.starts_with("Leads/")).applications.is_empty());
    }

    #[test]
    fn a_propertys_line_is_found_in_the_front_matter() {
        let text = "---\njob_stage: applied\njob_next: 2026-11-10T15:30\n---\njob_next: no\n";
        assert_eq!(JobSearch::property_line(text, "job_next"), 3);
        assert_eq!(JobSearch::property_line(text, "job_band"), 1);
        assert_eq!(JobSearch::property_line("# No front matter", "job_next"), 1);
    }
}
