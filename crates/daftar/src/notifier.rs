//! Meeting alerts, the morning task digest and the person's own rules
//! (`docs/specs/notification-rules.md`), as the Mac's `Notifier` sends
//! them, through the desktop's notifications. Unlike the Mac's, which the
//! system holds while the app is closed, these are checked against the
//! clock every few seconds: they ring while Daftar runs. They are planned again whenever the vault changes.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
#[cfg(test)]
use chrono::Utc;
use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use gtk::{gio, glib};

use daftar_core::{Day, DigestFilter, JobKeys, JobSearch, Meeting, NotificationRule, RuleLook};

use crate::store::VaultStore;
use crate::today::{TaskGroups, plain, prep_for};

/// What the `[notifications]` section asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub meetings: bool,
    /// Minutes before a meeting.
    pub lead: i64,
    pub digest: bool,
    pub digest_at: NaiveTime,
    pub rules: Vec<NotificationRule>,
    /// `notifications.digest_filter`: the tasks the digest counts.
    pub digest_filter: DigestFilter,
    /// The job search's stages (`jobs.waiting`, `jobs.closed`).
    pub jobs: JobKeys,
}

/// One notification to send at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub id: String,
    pub at: DateTime<Local>,
    pub title: String,
    pub body: String,
    /// A web link the notification opens: the meeting's.
    pub join: Option<String>,
    /// The note to prepare with, for a meeting with a company of the job
    /// search: the notification opens it.
    pub prep: Option<String>,
    /// The note a rule is about: a click opens it.
    pub note: Option<String>,
}

/// The alerts for [meetings] still ahead of [now], at most 40, each with
/// the note [prep] gives it, the next digest of [groups] when anything
/// is due or late, and each rule's next time when it rings then, as [look]
/// sees the vault: a note that lacks that day's heading, a query that finds
/// open tasks, a note that has not changed for its days.
pub fn plan(
    meetings: &[Meeting],
    prep: impl Fn(&Meeting) -> Option<String>,
    groups: impl Fn(Day) -> TaskGroups,
    look: &RuleLook,
    now: DateTime<Local>,
    wanted: &Wanted,
) -> Vec<Alert> {
    let mut out = Vec::new();
    if wanted.meetings {
        let lead = Duration::minutes(wanted.lead);
        for meeting in meetings.iter().filter(|m| m.start.is_some_and(|s| s.with_timezone(&Local) - lead > now)).take(40) {
            let start = meeting.start.unwrap().with_timezone(&Local);
            let time = start.format("%H:%M");
            let mut body = if wanted.lead > 0 { format!("In {} minutes, at {time}", wanted.lead) } else { format!("Now, at {time}") };
            if let Some(overlaps) = &meeting.overlaps {
                body.push_str(&format!(". Overlaps {overlaps}"));
            }
            let prep = prep(meeting);
            if let Some(prep) = &prep {
                body.push_str(&format!(". Prep: {prep}"));
            }
            out.push(Alert {
                id: format!("meeting.{}.{}", start.timestamp(), meeting.title),
                at: start - lead,
                title: format!("{}{}", meeting.title, if meeting.tentative { " (maybe)" } else { "" }),
                body,
                join: meeting.join.clone().filter(|url| url.starts_with("https://") || url.starts_with("http://")),
                prep,
                note: None,
            });
        }
    }
    if wanted.digest {
        let at = next(wanted.digest_at, now);
        let groups = groups(Day::from(at.date_naive()));
        if !groups.today.is_empty() || !groups.overdue.is_empty() {
            let due = &groups.today;
            let mut lines: Vec<String> = due.iter().take(5).map(|t| format!("• {}", plain(&t.task.text))).collect();
            if due.len() > 5 {
                lines.push(format!("and {} more", due.len() - 5));
            }
            if !groups.overdue.is_empty() {
                lines.push(format!("{} overdue", groups.overdue.len()));
            }
            let title = if due.is_empty() { "Nothing due today".to_string() } else { format!("{} due today", due.len()) };
            out.push(Alert { id: "digest".into(), at, title, body: lines.join("\n"), join: None, prep: None, note: None });
        }
    }
    for rule in &wanted.rules {
        let at = next(rule.at, now);
        let day = Day::from(at.date_naive());
        if let Some(rung) = rule.alert(day, look) {
            out.push(Alert { id: rule.alert_id(), at, title: rung.title, body: rung.body, join: None, prep: None, note: rung.note });
        }
    }
    out
}

/// The next [time] of the day after [now]: today's, else tomorrow's.
fn next(time: NaiveTime, now: DateTime<Local>) -> DateTime<Local> {
    let at = Local.from_local_datetime(&now.date_naive().and_time(time)).earliest().unwrap_or(now);
    if at <= now { at + Duration::days(1) } else { at }
}

/// The alerts planned last, rung by the wall clock: a check every
/// [TICK] seconds sends what is due. One glib timer per alert would wait
/// on the monotonic clock, which stops while the computer sleeps, so a
/// reminder planned before a night's sleep would ring hours late.
pub struct Notifier {
    pending: Rc<RefCell<Vec<Alert>>>,
    tick: RefCell<Option<glib::SourceId>>,
}

/// Seconds between two looks at the clock.
const TICK: u32 = 20;

impl Notifier {
    pub fn new() -> Rc<Notifier> {
        let pending: Rc<RefCell<Vec<Alert>>> = Rc::default();
        let due = pending.clone();
        let tick = glib::timeout_add_seconds_local(TICK, move || {
            let ready = take_due(&mut due.borrow_mut(), Local::now());
            for alert in &ready {
                send(alert);
            }
            glib::ControlFlow::Continue
        });
        Rc::new(Notifier { pending, tick: RefCell::new(Some(tick)) })
    }

    /// Replaces the alerts with the ones the vault calls for now.
    pub fn plan(&self, store: &VaultStore, wanted: &Wanted) {
        let meetings: Vec<Meeting> = store.calendar_days(Day::today(), 3).into_iter().flat_map(|day| day.meetings).collect();
        let tasks = store.index().tasks();
        let jobs = JobSearch::new(&store.index(), &wanted.jobs, |_| true);
        let prep = |meeting: &Meeting| prep_for(&jobs, meeting).map(|(_, path)| path);
        let read = |note: &str| std::fs::read_to_string(store.root.join(note)).unwrap_or_default();
        let all = || tasks.clone();
        let changed = |note: &str| daftar_core::notification_rules::last_change(&store.root, note);
        let look = RuleLook { read: &read, tasks: &all, changed: &changed };
        let groups = |day| TaskGroups::new(wanted.digest_filter.keep(tasks.clone(), day), day, 0);
        let alerts = plan(&meetings, prep, groups, &look, Local::now(), wanted);
        *self.pending.borrow_mut() = alerts;
    }
}

impl Notifier {
    /// No more alerts: the window that planned them closed.
    pub fn clear(&self) {
        self.pending.borrow_mut().clear();
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        // The source holds only the list, never the notifier, and never stops itself.
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
    }
}

/// The alerts of [pending] whose time has come at [now], taken out of it.
/// One more than ten minutes late, after the computer slept through it, is
/// dropped unsent: a meeting that began long ago is no reminder.
pub fn take_due(pending: &mut Vec<Alert>, now: DateTime<Local>) -> Vec<Alert> {
    let (due, ahead): (Vec<Alert>, Vec<Alert>) = std::mem::take(pending).into_iter().partition(|alert| alert.at <= now);
    *pending = ahead;
    due.into_iter().filter(|alert| now - alert.at <= Duration::minutes(10)).collect()
}

fn send(alert: &Alert) {
    let Some(app) = gio::Application::default() else { return };
    let notification = gio::Notification::new(&alert.title);
    notification.set_body(Some(&alert.body));
    if let Some(note) = &alert.note {
        notification.set_default_action_and_target_value("app.open-note", Some(&note.to_variant()));
    }
    // The prep note first: a click opens it, and Join is a button beside.
    if let Some(prep) = &alert.prep {
        notification.set_default_action_and_target_value("app.open-note", Some(&prep.to_variant()));
        notification.add_button_with_target_value("Prep", "app.open-note", Some(&prep.to_variant()));
    }
    match &alert.join {
        Some(url) => {
            if alert.prep.is_none() {
                notification.set_default_action_and_target_value("app.open-url", Some(&url.to_variant()));
            }
            notification.add_button_with_target_value("Join", "app.open-url", Some(&url.to_variant()));
        }
        None if alert.prep.is_none() && alert.note.is_none() => notification.set_default_action("app.today"),
        None => {}
    }
    app.send_notification(Some(&alert.id), &notification);
}

impl Wanted {
    pub fn from(file: &daftar_core::ConfigFile) -> Wanted {
        Wanted {
            meetings: file.bool("notifications.meetings").unwrap_or(true),
            lead: file.int("notifications.meeting_lead").unwrap_or(5).max(0),
            digest: file.bool("notifications.digest").unwrap_or(true),
            digest_at: file
                .string("notifications.digest_time")
                .and_then(|t| NaiveTime::parse_from_str(&t, "%H:%M").ok())
                .unwrap_or(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
            rules: NotificationRule::all(file),
            digest_filter: DigestFilter::of(file),
            jobs: JobKeys::of(file),
        }
    }
}

/// For the tests: a meeting at [start] UTC.
#[cfg(test)]
fn meeting(title: &str, start: DateTime<Utc>, join: Option<&str>) -> Meeting {
    Meeting {
        day: Day::from(start.date_naive()),
        start: Some(start),
        end: Some(start + Duration::minutes(30)),
        title: title.into(),
        calendars: vec![],
        join: join.map(String::from),
        tentative: false,
        busy_only: false,
        overlaps: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alert_rings_by_the_wall_clock_and_not_long_after() {
        let at = |h: u32, m: u32| Local.with_ymd_and_hms(2026, 11, 6, h, m, 0).unwrap();
        let alert =
            |id: &str, when| Alert { id: id.into(), at: when, title: id.into(), body: String::new(), join: None, prep: None, note: None };
        let mut pending = vec![alert("soon", at(9, 55)), alert("slept", at(2, 0)), alert("later", at(18, 0))];
        let due = take_due(&mut pending, at(9, 56));
        assert_eq!(due.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["soon"]);
        assert_eq!(pending.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["later"]);
        assert!(take_due(&mut pending, at(9, 57)).is_empty());
    }
    use daftar_core::{TaskLine, VaultTask};

    fn empty(_: &str) -> String {
        String::new()
    }

    fn no_tasks() -> Vec<VaultTask> {
        Vec::new()
    }

    fn never(_: &str) -> Option<Day> {
        None
    }

    /// A vault where no rule finds anything to ring for but a missing heading.
    const NOTHING: RuleLook<'static> = RuleLook { read: &empty, tasks: &no_tasks, changed: &never };

    #[test]
    fn meetings_ahead_ring_before_they_start() {
        let now = Local::now();
        let soon = (now + Duration::hours(1)).with_timezone(&Utc);
        let gone = (now - Duration::hours(1)).with_timezone(&Utc);
        let wanted = Wanted {
            meetings: true,
            lead: 5,
            digest: false,
            digest_at: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            rules: vec![],
            digest_filter: DigestFilter::default(),
            jobs: JobKeys::default(),
        };
        let meetings = [
            meeting("Standup", soon, Some("https://meet.example.com/x")),
            meeting("Gone", gone, None),
            meeting("Odd", soon, Some("zoommtg://x")),
        ];
        let alerts = plan(&meetings, |_| None, |_| TaskGroups::default(), &NOTHING, now, &wanted);
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].title, "Standup");
        assert_eq!(alerts[0].at, soon.with_timezone(&Local) - Duration::minutes(5));
        assert!(alerts[0].body.starts_with("In 5 minutes, at "));
        assert_eq!(alerts[0].join.as_deref(), Some("https://meet.example.com/x"));
        // Only web links leave the app.
        assert_eq!(alerts[1].join, None);
    }

    #[test]
    fn a_meeting_with_a_company_names_its_prep_note() {
        let now = Local::now();
        let soon = (now + Duration::hours(1)).with_timezone(&Utc);
        let wanted = Wanted {
            meetings: true,
            lead: 5,
            digest: false,
            digest_at: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            rules: vec![],
            digest_filter: DigestFilter::default(),
            jobs: JobKeys::default(),
        };
        let index = {
            let mut index = daftar_core::VaultIndex::default();
            let path = "Companies/Acme/Acme.md";
            let parsed = daftar_core::NoteParser::parse("---\njob_stage: screening\n---\n");
            index.notes.insert(path.into(), daftar_core::NoteEntry { path: path.into(), modified: std::time::SystemTime::now(), parsed });
            index.files.insert(path.into());
            index.files.insert("Companies/Acme/Interview Preparation.md".into());
            index.derive();
            index
        };
        let jobs = JobSearch::new(&index, &JobKeys::default(), |_| true);
        let meetings = [meeting("Interview with Acme", soon, None), meeting("Standup", soon, None)];
        let alerts = plan(&meetings, |m| prep_for(&jobs, m).map(|(_, path)| path), |_| TaskGroups::default(), &NOTHING, now, &wanted);
        assert_eq!(alerts[0].prep.as_deref(), Some("Companies/Acme/Interview Preparation.md"));
        assert!(alerts[0].body.ends_with(". Prep: Companies/Acme/Interview Preparation.md"), "{}", alerts[0].body);
        assert_eq!(alerts[1].prep, None);
    }

    #[test]
    fn the_digest_lists_what_is_due() {
        let now = Local::now();
        let wanted = Wanted {
            meetings: false,
            lead: 0,
            digest: true,
            digest_at: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            rules: vec![],
            digest_filter: DigestFilter::default(),
            jobs: JobKeys::default(),
        };
        let task = |raw: &str| VaultTask { path: "A.md".into(), task: TaskLine::parse(raw, 0, "").unwrap() };
        let groups =
            |_| TaskGroups { today: vec![task("- [ ] Call [[Sam Lee|Sam]]")], overdue: vec![task("- [ ] Late")], ..Default::default() };
        let alerts = plan(&[], |_| None, groups, &NOTHING, now, &wanted);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].title, "1 due today");
        assert_eq!(alerts[0].body, "• Call Sam\n1 overdue");
        assert!(alerts[0].at > now);
        assert!(plan(&[], |_| None, |_| TaskGroups::default(), &NOTHING, now, &wanted).is_empty());
    }

    #[test]
    fn a_rule_rings_at_its_time_when_the_note_lacks_the_day() {
        let file = daftar_core::ConfigFile::new(
            "[notifications]\nmeetings = false\ndigest = false\n\n[notifications.rules.diary]\nnote = \"Diet/Diary.md\"\nheading = \"YYYY-MM-DD\"\nat = \"21:00\"\n",
        );
        let wanted = Wanted::from(&file);
        let at = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap();
        let alerts = plan(&[], |_| None, |_| TaskGroups::default(), &NOTHING, at(25, 20), &wanted);
        assert_eq!(alerts.len(), 1);
        assert_eq!((alerts[0].id.as_str(), alerts[0].at), ("rule.diary", at(25, 21)));
        assert_eq!(alerts[0].title, "Diary");
        assert_eq!(alerts[0].body, "No heading for today yet: 2026-11-25");
        assert_eq!(alerts[0].note.as_deref(), Some("Diet/Diary.md"));
        // Written before its time: nothing rings.
        let today = |_: &str| "## 2026-11-25\n".to_string();
        let written = RuleLook { read: &today, tasks: &no_tasks, changed: &never };
        assert!(plan(&[], |_| None, |_| TaskGroups::default(), &written, at(25, 20), &wanted).is_empty());
        // After its time, it is tomorrow's, for tomorrow's heading.
        let alerts = plan(&[], |_| None, |_| TaskGroups::default(), &written, at(25, 22), &wanted);
        assert_eq!((alerts[0].at, alerts[0].body.as_str()), (at(26, 21), "No heading for today yet: 2026-11-26"));
    }
}
