//! `daftar-cli agenda --notify`: the app's meeting alerts (`notifier.rs`)
//! with no app running, for a timer that runs it every minute. Each
//! meeting of the week notes, and each booked `job_next` event, that starts
//! within the lead time is sent once as a desktop notification, with its
//! prep note when it is about a company of the job search, and each rule
//! of `[notifications.rules.<id>]` whose note lacks today's heading at its
//! time (`docs/specs/notification-rules.md`); what was sent is
//! kept in `notified-<hash>.json` in Daftar's state folder, beside the
//! drill's state, so the next run sends it again never. `--install-timer`
//! writes the systemd user units that run it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{Map, Value, json};

use daftar_core::{Day, JobSearch, Meeting, NotificationRule, reading_places};

/// Minutes before a meeting, when neither `--before` nor
/// `notifications.meeting_lead` says: the app's.
pub const LEAD: i64 = 5;

/// How late an alert may still be sent, in minutes, after a computer slept
/// through its time: the app's `take_due` drops it after that.
const LATE: i64 = 10;

/// How near a booked event and a meeting about the same company must start
/// to be one event, in minutes, as `JobSearch::for_meeting` matches them.
const SAME_EVENT: i64 = 15;

/// Something with a start to be told of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Who it is, with its start: an event moved is a new one.
    pub key: String,
    pub start: DateTime<Utc>,
    pub title: String,
    pub overlaps: Option<String>,
    pub prep: Option<String>,
}

/// A notification to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub key: String,
    pub start: DateTime<Utc>,
    pub title: String,
    pub body: String,
}

/// The timed meetings of [meetings] (once each) with the prep note of the
/// company each is about, then the booked events of [jobs] with a time
/// that no such meeting is already, their times read on [zone]'s clock.
pub fn events<Tz: TimeZone>(meetings: &[Meeting], jobs: &JobSearch, zone: &Tz) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    let mut about: Vec<(String, DateTime<Utc>)> = Vec::new();
    for meeting in meetings {
        let Some(start) = meeting.start else { continue };
        let key = format!("meeting.{}.{}", start.timestamp(), meeting.title);
        if out.iter().any(|e| e.key == key) {
            continue;
        }
        let application = jobs.for_calendar_meeting(meeting, zone);
        if let Some(a) = application {
            about.push((a.path.clone(), start));
        }
        let title = format!("{}{}", meeting.title, if meeting.tentative { " (maybe)" } else { "" });
        out.push(Event { key, start, title, overlaps: meeting.overlaps.clone(), prep: application.map(|a| a.prep_or_note().to_string()) });
    }
    for application in &jobs.applications {
        let Some(next) = application.next.filter(|_| application.next_has_time) else { continue };
        let Some(start) = zone.from_local_datetime(&next).earliest().map(|at| at.with_timezone(&Utc)) else { continue };
        if about.iter().any(|(path, at)| *path == application.path && (*at - start).num_minutes().abs() <= SAME_EVENT) {
            continue;
        }
        let title = match &application.next_step {
            Some(step) => format!("{}: {step}", application.company),
            None => application.company.clone(),
        };
        let key = format!("job.{}.{}", start.timestamp(), application.path);
        out.push(Event { key, start, title, overlaps: None, prep: Some(application.prep_or_note().to_string()) });
    }
    out
}

/// The alerts of [events] due at [now] with [lead] minutes' notice and not
/// in [sent]: an event whose start less the lead has come, and came no more
/// than `LATE` minutes ago. The body says when, as the app's does.
pub fn due<Tz: TimeZone>(events: &[Event], sent: &Sent, now: DateTime<Utc>, lead: i64, zone: &Tz) -> Vec<Alert> {
    events
        .iter()
        .filter(|e| {
            let at = e.start - Duration::minutes(lead);
            at <= now && now - at <= Duration::minutes(LATE) && !sent.keys.contains_key(&e.key)
        })
        .map(|e| {
            let time = e.start.with_timezone(zone).naive_local().format("%H:%M");
            // Whole minutes left, rounded up: a run at 09:55:30 for 10:00 says 5.
            let left = (e.start - now).num_seconds().div_euclid(60) + i64::from((e.start - now).num_seconds().rem_euclid(60) > 0);
            let mut body = match left {
                ..=0 => format!("Now, at {time}"),
                1 => format!("In 1 minute, at {time}"),
                n => format!("In {n} minutes, at {time}"),
            };
            if let Some(overlaps) = &e.overlaps {
                body.push_str(&format!(". Overlaps {overlaps}"));
            }
            if let Some(prep) = &e.prep {
                body.push_str(&format!(". Prep: {prep}"));
            }
            Alert { key: e.key.clone(), start: e.start, title: e.title.clone(), body }
        })
        .collect()
}

/// The alerts of [rules] due at [now] and not in [sent]: a rule whose time
/// came today on [zone]'s clock, no more than `LATE` minutes ago, and
/// whose note, as [read] gives it, lacks today's heading. Each is keyed by
/// its day, so it rings once a day.
pub fn rule_alerts<Tz: TimeZone>(
    rules: &[NotificationRule],
    read: impl Fn(&str) -> String,
    sent: &Sent,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<Alert> {
    let local = now.with_timezone(zone).naive_local();
    let today = Day::from(local.date());
    rules
        .iter()
        .filter_map(|rule| {
            let start = zone.from_local_datetime(&local.date().and_time(rule.at)).earliest()?.with_timezone(&Utc);
            let key = format!("{}.{today}", rule.alert_id());
            let due = start <= now && now - start <= Duration::minutes(LATE) && !sent.keys.contains_key(&key);
            (due && rule.is_missing(&read(&rule.note), today)).then(|| Alert { key, start, title: rule.title(), body: rule.body(today) })
        })
        .collect()
}

/// The alerts sent, by key, with the start of each: the state file's content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sent {
    pub keys: BTreeMap<String, DateTime<Utc>>,
}

impl Sent {
    /// `notified-<hash>.json` in Daftar's state folder, as the drill's.
    pub fn file(root: &Path) -> PathBuf {
        reading_places::state_file(root, "notified")
    }

    /// The state in [file]; empty when there is none or it cannot be read.
    pub fn load(file: &Path) -> Sent {
        fs::read_to_string(file).map_or_default(|data| Sent::from_data(&data))
    }

    pub fn from_data(data: &str) -> Sent {
        let Ok(value) = serde_json::from_str::<Value>(data) else { return Sent::default() };
        let keys = value.get("sent").and_then(Value::as_object).map_or_else(BTreeMap::new, |sent| {
            sent.iter()
                .filter_map(|(key, start)| Some((key.clone(), DateTime::parse_from_rfc3339(start.as_str()?).ok()?.with_timezone(&Utc))))
                .collect()
        });
        Sent { keys }
    }

    pub fn data(&self) -> String {
        let sent: Map<String, Value> = self.keys.iter().map(|(key, start)| (key.clone(), start.to_rfc3339().into())).collect();
        serde_json::to_string_pretty(&json!({ "version": 1, "sent": sent })).unwrap_or_default()
    }

    /// Without the events that began more than an hour before [now]: none
    /// of them can be due again, whatever the lead.
    pub fn pruned(mut self, now: DateTime<Utc>) -> Sent {
        self.keys.retain(|_, start| *start >= now - Duration::hours(1));
        self
    }

    /// Writes the state to [file] through a temporary file and a rename.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let fail = |e: std::io::Error| format!("{}: {e}", file.display());
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).map_err(fail)?;
        }
        let temporary = file.with_extension("json.tmp");
        fs::write(&temporary, self.data()).map_err(fail)?;
        fs::rename(&temporary, file).map_err(fail)
    }
}

/// Sends what is due for the vault at [root] at [now]: the meetings of the
/// week notes from yesterday to tomorrow and the booked events of [jobs],
/// [lead] minutes ahead, and the [rules] at their time, each once. Gives
/// what it sent.
pub fn run<Tz: TimeZone>(
    root: &Path,
    jobs: &JobSearch,
    rules: &[NotificationRule],
    lead: i64,
    zone: &Tz,
    now: DateTime<Utc>,
) -> Result<Vec<Alert>, String> {
    let today = Day::from(now.with_timezone(zone).date_naive());
    let mut meetings = crate::agenda::meetings(root, today);
    meetings.extend(crate::agenda::meetings(root, today.adding_days(1)));
    let events = events(&meetings, jobs, zone);
    let file = Sent::file(root);
    let mut sent = Sent::load(&file).pruned(now);
    let mut alerts = due(&events, &sent, now, lead, zone);
    alerts.extend(rule_alerts(rules, |note| fs::read_to_string(root.join(note)).unwrap_or_default(), &sent, now, zone));
    let mut failure = None;
    for alert in &alerts {
        if let Err(error) = send(alert) {
            failure = Some(error);
            break;
        }
        sent.keys.insert(alert.key.clone(), alert.start);
    }
    sent.save(&file)?;
    failure.map_or(Ok(alerts), Err)
}

/// Sends [alert]: `notify-send` on Linux, `osascript` on macOS, else a
/// line on stdout.
fn send(alert: &Alert) -> Result<(), String> {
    let status = if cfg!(target_os = "linux") {
        Command::new("notify-send")
            .args(["--app-name=Daftar", "--icon=me.parham1995.notes", "--", &alert.title, &alert.body])
            .status()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "notify-send is not on $PATH: install libnotify".to_string(),
                _ => format!("notify-send: {e}"),
            })?
    } else if cfg!(target_os = "macos") {
        Command::new("osascript").args(["-e", &apple_script(alert)]).status().map_err(|e| format!("osascript: {e}"))?
    } else {
        out!("{}\t{}", alert.title, alert.body);
        return Ok(());
    };
    if status.success() { Ok(()) } else { Err(format!("the notification of {} was not sent ({status})", alert.title)) }
}

/// The AppleScript that shows [alert] in Notification Center.
fn apple_script(alert: &Alert) -> String {
    format!("display notification {} with title {}", apple_string(&alert.body), apple_string(&alert.title))
}

/// [text] as an AppleScript string literal.
fn apple_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The first line of a unit `--install-timer` wrote: a unit without it is
/// the person's, and it is left alone.
const MARK: &str = "# Written by `daftar-cli agenda --install-timer`";

/// The units' name, `.service` and `.timer`.
pub const UNIT: &str = "daftar-agenda-notify";

/// The systemd user units' folder: `$XDG_CONFIG_HOME/systemd/user`, else
/// `~/.config/systemd/user`.
pub fn units_folder() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(".config"))
        .join("systemd/user")
}

/// The command the service runs: the `daftar-cli` on `$PATH` as it is
/// written there (a package manager's link, which an upgrade keeps), else
/// this binary, since a user service's `$PATH` may not have
/// `~/.local/bin` or `~/.cargo/bin`; with the vault, and [before] when given.
pub fn command(root: &Path, before: Option<i64>) -> Vec<String> {
    let binary = std::env::var_os("PATH")
        .and_then(|path| std::env::split_paths(&path).map(|folder| folder.join("daftar-cli")).find(|file| file.is_file()))
        .filter(|file| file.is_absolute())
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("daftar-cli"));
    let mut command = vec![binary.to_string_lossy().into_owned(), "--vault".into(), root.to_string_lossy().into_owned()];
    command.extend(["agenda".into(), "--notify".into()]);
    if let Some(before) = before {
        command.extend(["--before".into(), before.to_string()]);
    }
    command
}

/// The `.service` and `.timer` that run [command] every minute.
pub fn units(command: &[String]) -> (String, String) {
    let exec: Vec<String> = command.iter().map(|arg| unit_word(arg)).collect();
    let service = format!(
        "{MARK}\n[Unit]\nDescription=Daftar's meeting notifications\n\n\
         [Service]\nType=oneshot\nExecStart={}\n",
        exec.join(" ")
    );
    let timer = format!(
        "{MARK}\n[Unit]\nDescription=Daftar's meeting notifications, every minute\n\n\
         [Timer]\nOnCalendar=*:*:00\nAccuracySec=1s\nPersistent=false\n\n\
         [Install]\nWantedBy=timers.target\n"
    );
    (service, timer)
}

/// [arg] as a word of `ExecStart`: `%` and `$` doubled, as systemd expands
/// them, and quoted when it has a space, a quote or a backslash.
fn unit_word(arg: &str) -> String {
    let plain = arg.replace('%', "%%").replace('$', "$$");
    if plain.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';')) {
        format!("\"{}\"", plain.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        plain
    }
}

/// Writes the units of [command] in [folder], never over a unit of the
/// same name this command did not write unless [force]. Gives the files.
pub fn install_timer(folder: &Path, command: &[String], force: bool) -> Result<[PathBuf; 2], String> {
    let (service, timer) = units(command);
    let files = [folder.join(format!("{UNIT}.service")), folder.join(format!("{UNIT}.timer"))];
    for file in &files {
        if let Ok(there) = fs::read_to_string(file)
            && !there.contains(MARK)
            && !force
        {
            return Err(format!("{} is a unit of your own; --force puts this one in its place", file.display()));
        }
    }
    fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    for (file, text) in files.iter().zip([service, timer]) {
        fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use daftar_core::{JobKeys, Vault};

    fn at(text: &str) -> DateTime<Utc> {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").unwrap().and_utc()
    }

    /// A made-up vault on Sunday 15 November 2026, its times UTC: a standup,
    /// an interview with Acme that Acme's booked event is too, a booked call
    /// with Initech no meeting is, and a meeting on Monday.
    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "Calendar/2026-W46.md",
            "# Week 46\n\nTimes are UTC.\n\n## Sunday 15 November\n\n- All day Team offsite\n- 09:00–09:15 Standup\n- 14:00–15:00 Interview with Acme\n",
        );
        write("Calendar/2026-W47.md", "# Week 47\n\nTimes are UTC.\n\n## Monday 16 November\n\n- 00:10–00:40 Night sync\n");
        write("Companies/Acme/Acme.md", "---\njob_stage: interviewing\njob_next: 2026-11-15T14:00\n---\n# Acme\n");
        write("Companies/Acme/Interview Preparation.md", "# Prep\n");
        write(
            "Companies/Initech/Initech.md",
            "---\njob_stage: screening\njob_next: 2026-11-15T16:30\njob_next_step: \"Recruiter call\"\n---\n# Initech\n",
        );
        root
    }

    fn events_of(root: &Path) -> Vec<Event> {
        let index = Vault::index(root, None);
        let jobs = JobSearch::new(&index, &JobKeys::default(), |_| true);
        let today = Day::parse("2026-11-15").unwrap();
        let mut meetings = crate::agenda::meetings(root, today);
        meetings.extend(crate::agenda::meetings(root, today.adding_days(1)));
        events(&meetings, &jobs, &Utc)
    }

    #[test]
    fn a_meeting_and_its_booked_event_are_one_event_with_the_prep_note() {
        let root = vault();
        let events = events_of(root.path());
        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Standup", "Interview with Acme", "Night sync", "Initech: Recruiter call"]);
        assert_eq!(events[1].prep.as_deref(), Some("Companies/Acme/Interview Preparation.md"));
        // With no preparation note, the company's own note.
        assert_eq!(events[3].prep.as_deref(), Some("Companies/Initech/Initech.md"));
        assert_eq!(events[3].key, format!("job.{}.Companies/Initech/Initech.md", at("2026-11-15T16:30:00").timestamp()));
    }

    #[test]
    fn what_is_due_is_what_starts_within_the_lead_and_was_not_sent() {
        let root = vault();
        let events = events_of(root.path());
        let none = Sent::default();
        let titles = |alerts: Vec<Alert>| alerts.into_iter().map(|a| a.title).collect::<Vec<_>>();
        assert!(due(&events, &none, at("2026-11-15T13:54:00"), 5, &Utc).is_empty());
        let alerts = due(&events, &none, at("2026-11-15T13:55:30"), 5, &Utc);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].title, "Interview with Acme");
        assert_eq!(alerts[0].body, "In 5 minutes, at 14:00. Prep: Companies/Acme/Interview Preparation.md");
        assert_eq!(due(&events, &none, at("2026-11-15T13:59:00"), 5, &Utc)[0].body.split('.').next(), Some("In 1 minute, at 14:00"));
        assert_eq!(due(&events, &none, at("2026-11-15T14:03:00"), 5, &Utc)[0].body.split('.').next(), Some("Now, at 14:00"));
        // Slept through it: more than ten minutes after its time it is let go.
        assert!(due(&events, &none, at("2026-11-15T14:06:00"), 5, &Utc).is_empty());
        // Sent once, never again.
        let sent = Sent { keys: [(events[1].key.clone(), events[1].start)].into() };
        assert!(due(&events, &sent, at("2026-11-15T13:57:00"), 5, &Utc).is_empty());
        // A booked call, and a meeting past midnight from the next week's note.
        assert_eq!(titles(due(&events, &none, at("2026-11-15T16:20:00"), 10, &Utc)), ["Initech: Recruiter call"]);
        assert_eq!(titles(due(&events, &none, at("2026-11-16T00:05:00"), 5, &Utc)), ["Night sync"]);
        // A lead of nothing: at the start.
        assert_eq!(due(&events, &none, at("2026-11-15T09:00:00"), 0, &Utc)[0].body, "Now, at 09:00");
    }

    #[test]
    fn a_rule_rings_once_at_its_time_when_the_note_lacks_today() {
        let file = daftar_core::ConfigFile::new(
            "[notifications.rules.diary]\nnote = \"Diet/Diary.md\"\nheading = \"D MMM YYYY · dddd\"\nat = \"21:00\"\n",
        );
        let rules = NotificationRule::all(&file);
        let none = |_: &str| String::new();
        let alerts = rule_alerts(&rules, none, &Sent::default(), at("2026-11-25T21:00:30"), &Utc);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].key, "rule.diary.2026-11-25");
        assert_eq!((alerts[0].title.as_str(), alerts[0].body.as_str()), ("Diary", "No heading for today yet: 25 Nov 2026 · Wednesday"));
        // Before its time, too late, sent already, or written: nothing.
        assert!(rule_alerts(&rules, none, &Sent::default(), at("2026-11-25T20:59:00"), &Utc).is_empty());
        assert!(rule_alerts(&rules, none, &Sent::default(), at("2026-11-25T21:11:00"), &Utc).is_empty());
        let sent = Sent { keys: [(alerts[0].key.clone(), alerts[0].start)].into() };
        assert!(rule_alerts(&rules, none, &sent, at("2026-11-25T21:01:00"), &Utc).is_empty());
        let written = |_: &str| "### 25 Nov 2026 · Wednesday\n".to_string();
        assert!(rule_alerts(&rules, written, &Sent::default(), at("2026-11-25T21:00:30"), &Utc).is_empty());
    }

    #[test]
    fn the_state_keeps_what_was_sent_until_an_hour_after_it_began() {
        let sent = Sent { keys: [("old".to_string(), at("2026-11-15T09:00:00")), ("new".to_string(), at("2026-11-15T14:00:00"))].into() };
        let read = Sent::from_data(&sent.data());
        assert_eq!(read, sent);
        assert_eq!(read.clone().pruned(at("2026-11-15T09:59:00")).keys.len(), 2);
        assert_eq!(read.pruned(at("2026-11-15T10:01:00")).keys.into_keys().collect::<Vec<_>>(), ["new"]);
        assert_eq!(Sent::from_data("not json"), Sent::default());

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state/notified.json");
        sent.save(&file).unwrap();
        assert_eq!(Sent::load(&file), sent);
        assert_eq!(Sent::load(&dir.path().join("none.json")), Sent::default());
    }

    #[test]
    fn apple_script_strings_are_escaped() {
        let alert =
            Alert { key: String::new(), start: at("2026-11-15T14:00:00"), title: r#"Say "hi" \ bye"#.into(), body: "In 5 minutes".into() };
        assert_eq!(apple_script(&alert), r#"display notification "In 5 minutes" with title "Say \"hi\" \\ bye""#);
    }

    #[test]
    fn the_units_run_the_command_every_minute() {
        let command =
            ["/usr/bin/daftar-cli", "--vault", "/home/someone/My Notes", "agenda", "--notify", "--before", "10"].map(String::from);
        let (service, timer) = units(&command);
        assert_eq!(
            service,
            "# Written by `daftar-cli agenda --install-timer`\n[Unit]\nDescription=Daftar's meeting notifications\n\n\
             [Service]\nType=oneshot\nExecStart=/usr/bin/daftar-cli --vault \"/home/someone/My Notes\" agenda --notify --before 10\n"
        );
        assert!(timer.contains("[Timer]\nOnCalendar=*:*:00\nAccuracySec=1s\nPersistent=false\n"), "{timer}");
        assert!(timer.ends_with("[Install]\nWantedBy=timers.target\n"), "{timer}");
        assert_eq!(unit_word("100%$x"), "100%%$$x");
        assert_eq!(unit_word(r#"a "b"\c"#), r#""a \"b\"\\c""#);
    }

    #[test]
    fn the_units_are_never_written_over_ones_of_your_own() {
        let dir = tempfile::tempdir().unwrap();
        let command = ["daftar-cli".to_string()];
        let files = install_timer(dir.path(), &command, false).unwrap();
        assert!(fs::read_to_string(&files[1]).unwrap().contains("OnCalendar"));
        // Its own are written again.
        install_timer(dir.path(), &command, false).unwrap();
        fs::write(&files[0], "[Service]\nExecStart=mine\n").unwrap();
        assert!(install_timer(dir.path(), &command, false).unwrap_err().contains("of your own"));
        assert_eq!(fs::read_to_string(&files[0]).unwrap(), "[Service]\nExecStart=mine\n");
        install_timer(dir.path(), &command, true).unwrap();
        assert!(fs::read_to_string(&files[0]).unwrap().contains(MARK));
    }
}
