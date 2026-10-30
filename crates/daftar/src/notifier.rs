//! Meeting alerts and the morning task digest, as the Mac's `Notifier` sends
//! them, through the desktop's notifications. Unlike the Mac's, which the
//! system holds while the app is closed, these are timers: they ring while
//! Daftar runs. They are planned again whenever the vault changes.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
#[cfg(test)]
use chrono::Utc;
use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use gtk::{gio, glib};

use daftar_core::{Day, Meeting};

use crate::store::VaultStore;
use crate::today::{TaskGroups, plain};

/// What the `[notifications]` section asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub meetings: bool,
    /// Minutes before a meeting.
    pub lead: i64,
    pub digest: bool,
    pub digest_at: NaiveTime,
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
}

/// The alerts for [meetings] still ahead of [now], at most 40, and the next
/// digest of [groups] when anything is due or late.
pub fn plan(meetings: &[Meeting], groups: impl Fn(Day) -> TaskGroups, now: DateTime<Local>, wanted: &Wanted) -> Vec<Alert> {
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
            out.push(Alert {
                id: format!("meeting.{}.{}", start.timestamp(), meeting.title),
                at: start - lead,
                title: format!("{}{}", meeting.title, if meeting.tentative { " (maybe)" } else { "" }),
                body,
                join: meeting.join.clone().filter(|url| url.starts_with("https://") || url.starts_with("http://")),
            });
        }
    }
    if wanted.digest {
        let mut at = Local.from_local_datetime(&now.date_naive().and_time(wanted.digest_at)).earliest().unwrap_or(now);
        if at <= now {
            at += Duration::days(1);
        }
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
            out.push(Alert { id: "digest".into(), at, title, body: lines.join("\n"), join: None });
        }
    }
    out
}

/// The timers of the alerts planned last.
#[derive(Default)]
pub struct Notifier {
    timers: RefCell<Vec<glib::SourceId>>,
}

impl Notifier {
    pub fn new() -> Rc<Notifier> {
        Rc::new(Notifier::default())
    }

    /// Replaces every timer with the ones the vault calls for now.
    pub fn plan(&self, store: &VaultStore, wanted: &Wanted) {
        for id in self.timers.borrow_mut().drain(..) {
            id.remove();
        }
        let meetings: Vec<Meeting> = store.calendar_days(Day::today(), 3).into_iter().flat_map(|day| day.meetings).collect();
        let tasks = store.index().tasks();
        let alerts = plan(&meetings, |day| TaskGroups::new(tasks.clone(), day, 0), Local::now(), wanted);
        for alert in alerts {
            let wait = (alert.at - Local::now()).to_std().unwrap_or_default();
            // A timer far ahead is planned again before then: the vault changes.
            if wait > std::time::Duration::from_hours(36) {
                continue;
            }
            let id = glib::timeout_add_local_once(wait, move || send(&alert));
            self.timers.borrow_mut().push(id);
        }
    }
}

fn send(alert: &Alert) {
    let Some(app) = gio::Application::default() else { return };
    let notification = gio::Notification::new(&alert.title);
    notification.set_body(Some(&alert.body));
    match &alert.join {
        Some(url) => {
            notification.set_default_action_and_target_value("app.open-url", Some(&url.to_variant()));
            notification.add_button_with_target_value("Join", "app.open-url", Some(&url.to_variant()));
        }
        None => notification.set_default_action("app.today"),
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
    use daftar_core::{TaskLine, VaultTask};

    #[test]
    fn meetings_ahead_ring_before_they_start() {
        let now = Local::now();
        let soon = (now + Duration::hours(1)).with_timezone(&Utc);
        let gone = (now - Duration::hours(1)).with_timezone(&Utc);
        let wanted = Wanted { meetings: true, lead: 5, digest: false, digest_at: NaiveTime::from_hms_opt(9, 0, 0).unwrap() };
        let meetings = [
            meeting("Standup", soon, Some("https://meet.example.com/x")),
            meeting("Gone", gone, None),
            meeting("Odd", soon, Some("zoommtg://x")),
        ];
        let alerts = plan(&meetings, |_| TaskGroups::default(), now, &wanted);
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].title, "Standup");
        assert_eq!(alerts[0].at, soon.with_timezone(&Local) - Duration::minutes(5));
        assert!(alerts[0].body.starts_with("In 5 minutes, at "));
        assert_eq!(alerts[0].join.as_deref(), Some("https://meet.example.com/x"));
        // Only web links leave the app.
        assert_eq!(alerts[1].join, None);
    }

    #[test]
    fn the_digest_lists_what_is_due() {
        let now = Local::now();
        let wanted = Wanted { meetings: false, lead: 0, digest: true, digest_at: NaiveTime::from_hms_opt(9, 0, 0).unwrap() };
        let task = |raw: &str| VaultTask { path: "A.md".into(), task: TaskLine::parse(raw, 0, "").unwrap() };
        let groups =
            |_| TaskGroups { today: vec![task("- [ ] Call [[Sam Lee|Sam]]")], overdue: vec![task("- [ ] Late")], ..Default::default() };
        let alerts = plan(&[], groups, now, &wanted);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].title, "1 due today");
        assert_eq!(alerts[0].body, "• Call Sam\n1 overdue");
        assert!(alerts[0].at > now);
        assert!(plan(&[], |_| TaskGroups::default(), now, &wanted).is_empty());
    }
}
