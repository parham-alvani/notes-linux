//! Notifications the person defines: at a time of the day, a note that has
//! no heading for today rings (`docs/specs/notification-rules.md`); or a
//! query of the tasks that finds open ones, or a note left unchanged for a
//! number of days (`docs/specs/more-vault-settings.md`, section 5). A rule
//! is a `[notifications.rules.<id>]` section of the settings, so a vault's
//! `.daftar.toml` can carry it to every device. The port of
//! `NotificationRule.swift`. The morning digest's filter is here too.

use std::path::Path;

use chrono::{Datelike, NaiveTime, Weekday};

use crate::{Clock, ConfigFile, Day, MomentFormat, NoteParser, QueryTask, TasksQuery, VaultTask};

/// The section every rule is under, with its id after the dot.
pub const SECTION: &str = "notifications.rules";

/// The keys of a rule, as the settings check them.
pub const KEYS: &[&str] = &["note", "heading", "at", "title", "message", "days", "query", "quiet_days"];

/// What a rule looks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleKind {
    /// A moment.js format of the day's heading the note should have.
    Heading(String),
    /// Tasks query lines: the rule rings when they find open tasks.
    Query(Vec<String>),
    /// Days: the rule rings when the note has not changed for that many.
    Quiet(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRule {
    pub id: String,
    /// The note's path in the vault; none for a query.
    pub note: Option<String>,
    pub kind: RuleKind,
    /// The time of the check, on the device's wall clock.
    pub at: NaiveTime,
    /// The days of the week it rings on; None is every day.
    pub days: Option<Vec<Weekday>>,
    pub title: Option<String>,
    pub message: Option<String>,
}

/// What a rule reads to decide whether it rings.
pub struct RuleLook<'a> {
    /// A note's text by its path; empty when it is not there.
    pub read: &'a dyn Fn(&str) -> String,
    /// The vault's tasks.
    pub tasks: &'a dyn Fn() -> Vec<VaultTask>,
    /// The day a note last changed (`last_change`); None when it is not there.
    pub changed: &'a dyn Fn(&str) -> Option<Day>,
}

/// An alert a rule rings with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleAlert {
    pub title: String,
    pub body: String,
    /// The note a click opens; none opens Today.
    pub note: Option<String>,
}

impl NotificationRule {
    /// The rules [file] sets, in the order its sections come; a rule
    /// that lacks a key it needs, or has one that does not fit, is left out.
    pub fn all(file: &ConfigFile) -> Vec<NotificationRule> {
        let mut ids: Vec<String> = Vec::new();
        for (key, _) in file.keys() {
            if let Some(id) = id_of(&key)
                && !ids.iter().any(|known| known == id)
            {
                ids.push(id.to_string());
            }
        }
        ids.into_iter().filter_map(|id| Self::read(file, id)).collect()
    }

    fn read(file: &ConfigFile, id: String) -> Option<NotificationRule> {
        let path = |name: &str| format!("{SECTION}.{id}.{name}");
        let key = |name: &str| file.string(&path(name));
        let at = key("at").filter(|at| crate::settings::is_time(at)).and_then(|at| NaiveTime::parse_from_str(&at, "%H:%M").ok())?;
        // Exactly one of `heading`, `query` and `quiet_days`.
        let written = ["heading", "query", "quiet_days"].iter().filter(|name| file.get(&path(name)).is_some()).count();
        if written != 1 {
            return None;
        }
        let kind = if let Some(heading) = file.get(&path("heading")) {
            let crate::Value::String(heading) = heading else { return None };
            (!heading.trim().is_empty() && one_line(&heading)).then_some(RuleKind::Heading(heading))?
        } else if let Some(query) = file.get(&path("query")) {
            let crate::Value::Strings(lines) = query else { return None };
            (!lines.is_empty() && lines.iter().all(|line| one_line(line))).then_some(RuleKind::Query(lines))?
        } else {
            let days = file.int(&path("quiet_days")).filter(|days| (1..=3650).contains(days))?;
            RuleKind::Quiet(days)
        };
        // A query reads no note; the others need one.
        let note = match kind {
            RuleKind::Query(_) => None,
            _ => Some(key("note").filter(|note| is_note_path(note))?),
        };
        let days = match file.get(&path("days")) {
            None => None,
            Some(crate::Value::Strings(words)) => {
                let days: Option<Vec<Weekday>> = words.iter().map(|word| weekday(word)).collect();
                Some(days.filter(|days| !days.is_empty())?)
            }
            Some(_) => return None,
        };
        let optional = |name: &str| key(name).filter(|text| !text.trim().is_empty() && one_line(text));
        Some(NotificationRule { title: optional("title"), message: optional("message"), id, note, kind, at, days })
    }

    /// Whether the rule rings on [day]'s day of the week.
    pub fn rings_on(&self, day: Day) -> bool {
        self.days.as_ref().is_none_or(|days| days.contains(&day.date().weekday()))
    }

    /// The heading [day] should have; empty for a rule of another kind.
    pub fn expected(&self, day: Day) -> String {
        match &self.kind {
            RuleKind::Heading(heading) => MomentFormat::format(heading, day, Clock::new(0, 0, 0)),
            _ => String::new(),
        }
    }

    /// Whether [text], the note's, has no heading for [day]: none whose
    /// text starts with the expected one.
    pub fn is_missing(&self, text: &str, day: Day) -> bool {
        let expected = self.expected(day);
        !NoteParser::parse(text).headings.iter().any(|heading| heading.text.trim().starts_with(&expected))
    }

    /// The alert's title: the rule's, else the note's name, else `Tasks`.
    pub fn title(&self) -> String {
        self.title.clone().unwrap_or_else(|| match &self.note {
            Some(note) => {
                let name = note.rsplit('/').next().unwrap_or(note);
                name.strip_suffix(".md").unwrap_or(name).to_string()
            }
            None => "Tasks".to_string(),
        })
    }

    /// A heading rule's body on [day]: the rule's message, else what is
    /// missing.
    pub fn body(&self, day: Day) -> String {
        self.message.clone().unwrap_or_else(|| format!("No heading for today yet: {}", self.expected(day)))
    }

    /// The alert the rule rings with at its time on [day], as [look] sees
    /// the vault; None when it does not ring.
    pub fn alert(&self, day: Day, look: &RuleLook) -> Option<RuleAlert> {
        if !self.rings_on(day) {
            return None;
        }
        let body = match &self.kind {
            RuleKind::Heading(_) => {
                let note = self.note.as_deref()?;
                self.is_missing(&(look.read)(note), day).then(|| self.body(day))?
            }
            RuleKind::Query(lines) => {
                let query = TasksQuery::parse(&lines.join("\n"), None);
                let found: Vec<VaultTask> = query
                    .run((look.tasks)(), day, |task: &VaultTask| QueryTask::from(task))
                    .into_iter()
                    .filter(|task| !crate::TaskLine::is_closed(task.task.status))
                    .collect();
                if found.is_empty() {
                    return None;
                }
                let texts: Vec<String> = found.iter().take(3).map(|task| plain(&task.task.text)).collect();
                self.message.clone().unwrap_or_else(|| format!("{} tasks: {}", found.len(), texts.join(", ")))
            }
            RuleKind::Quiet(days) => {
                let note = self.note.as_deref()?;
                let quiet = (look.changed)(note).map(|changed| changed.days_to(day));
                if quiet.is_some_and(|quiet| quiet < *days) {
                    return None;
                }
                self.message.clone().unwrap_or_else(|| match quiet {
                    Some(quiet) => format!("No change for {quiet} days"),
                    None => format!("No such note: {note}"),
                })
            }
        };
        Some(RuleAlert { title: self.title(), body, note: self.note.clone() })
    }

    /// The alert's identifier, the same every day.
    pub fn alert_id(&self) -> String {
        format!("rule.{}", self.id)
    }
}

/// The day the note at [path] last changed in the vault at [root]: its last
/// commit, else the file's modified time when no commit has it (a vault
/// with no git); None when it is not there.
pub fn last_change(root: &Path, path: &str) -> Option<Day> {
    let git = crate::Git::new(root);
    if git.is_repository()
        && let Ok(revisions) = git.history(path, 1)
        && let Some(last) = revisions.first()
    {
        return Some(Day::from(last.date.with_timezone(&chrono::Local).date_naive()));
    }
    let modified = std::fs::metadata(root.join(path)).and_then(|m| m.modified()).ok()?;
    Some(Day::from(chrono::DateTime::<chrono::Local>::from(modified).date_naive()))
}

/// `notifications.digest_filter`: the Tasks query lines every task the
/// morning digest counts must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DigestFilter {
    pub lines: Vec<String>,
}

impl DigestFilter {
    pub fn of(file: &ConfigFile) -> DigestFilter {
        DigestFilter { lines: file.strings("notifications.digest_filter").unwrap_or_default() }
    }

    /// The tasks of [tasks] every line matches on [today]; a line that
    /// cannot be read is left out.
    pub fn keep(&self, tasks: Vec<VaultTask>, today: Day) -> Vec<VaultTask> {
        if self.lines.is_empty() {
            return tasks;
        }
        let query = TasksQuery::parse(&self.lines.join("\n"), None);
        // Only the filters: the digest keeps its own order and counts every task.
        let kept = query.run(tasks.iter().enumerate(), today, |(_, task)| QueryTask::from(*task));
        let mut at: Vec<usize> = kept.into_iter().map(|(at, _)| at).collect();
        at.sort_unstable();
        at.into_iter().map(|i| tasks[i].clone()).collect()
    }

    /// The lines the digest cannot read, which `doctor` names.
    pub fn not_understood(&self) -> Vec<String> {
        TasksQuery::parse(&self.lines.join("\n"), None).not_understood
    }
}

/// A day of the week, `mon` to `sun`, in any case.
fn weekday(word: &str) -> Option<Weekday> {
    Some(match word.trim().to_lowercase().as_str() {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// A task's text as the digest writes it: a `[[link|label]]` is its label.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let Some(end) = rest[start + 2..].find("]]") else { break };
        out.push_str(&rest[..start]);
        let inner = &rest[start + 2..start + 2 + end];
        out.push_str(inner.split_once('|').map_or(inner, |(_, label)| label));
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    out
}

/// The rule's id in [key], `notifications.rules.<id>.<name>`: letters,
/// digits, `_` and `-`.
pub fn id_of(key: &str) -> Option<&str> {
    let (id, _) = key.strip_prefix(SECTION)?.strip_prefix('.')?.rsplit_once('.')?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then_some(id)
}

fn one_line(text: &str) -> bool {
    !text.chars().any(char::is_control)
}

/// A note's path as written: relative, with no `..`, and not empty. The
/// settings also refuse a link out of the vault.
fn is_note_path(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && one_line(path)
        && !path.starts_with(['/', '~', '\\'])
        && !path.contains(':')
        && !path.split(['/', '\\']).any(|part| part == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIARY: &str = "[notifications.rules.diary]\nnote = \"Diet/Diary.md\"\nheading = \"D MMM YYYY · dddd\"\nat = \"21:00\"\n";

    fn day() -> Day {
        Day::parse("2026-11-25").unwrap()
    }

    fn diary() -> NotificationRule {
        NotificationRule::all(&ConfigFile::new(DIARY)).remove(0)
    }

    #[test]
    fn a_rule_is_read_from_its_section() {
        let rules = NotificationRule::all(&ConfigFile::new(DIARY));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "diary");
        assert_eq!(rules[0].note.as_deref(), Some("Diet/Diary.md"));
        assert_eq!(rules[0].at, NaiveTime::from_hms_opt(21, 0, 0).unwrap());
        assert_eq!(rules[0].title(), "Diary");
        assert_eq!(rules[0].message, None);
        assert_eq!(rules[0].alert_id(), "rule.diary");
    }

    #[test]
    fn a_rule_that_does_not_fit_is_ignored() {
        let good = "[notifications.rules.log]\nnote = \"Log.md\"\nheading = \"YYYY-MM-DD\"\nat = \"08:30\"\n";
        for bad in [
            "[notifications.rules.a]\nnote = \"A.md\"\nheading = \"D\"\n",
            "[notifications.rules.a]\nnote = \"A.md\"\nheading = \"D\"\nat = \"25:00\"\n",
            "[notifications.rules.a]\nnote = \"../x.md\"\nheading = \"D\"\nat = \"21:00\"\n",
            "[notifications.rules.my diary]\nnote = \"A.md\"\nheading = \"D\"\nat = \"21:00\"\n",
        ] {
            let rules = NotificationRule::all(&ConfigFile::new(format!("{bad}\n{good}")));
            assert_eq!(rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["log"], "{bad}");
        }
    }

    #[test]
    fn the_expected_heading_is_the_day_in_the_format() {
        assert_eq!(diary().expected(day()), "25 Nov 2026 · Wednesday");
        let rule = NotificationRule { kind: RuleKind::Heading("[Day] YYYY-MM-DD".into()), ..diary() };
        assert_eq!(rule.expected(day()), "Day 2026-11-25");
    }

    #[test]
    fn todays_heading_is_found() {
        let rule = diary();
        assert!(rule.is_missing("## 24 Nov 2026 · Tuesday\n", day()));
        assert!(!rule.is_missing("### 25 Nov 2026 · Wednesday\n", day()));
        assert!(!rule.is_missing("## 25 Nov 2026 · Wednesday (fasting)\n", day()));
        assert!(rule.is_missing("25 Nov 2026 · Wednesday\n", day()));
        assert!(rule.is_missing("```text\n## 25 Nov 2026 · Wednesday\n```\n", day()));
        assert!(rule.is_missing("", day()));
    }

    #[test]
    fn the_alert_says_what_is_missing() {
        let rule = diary();
        assert_eq!(rule.title(), "Diary");
        assert_eq!(rule.body(day()), "No heading for today yet: 25 Nov 2026 · Wednesday");
        let file = format!("{DIARY}title = \"Food\"\nmessage = \"Log what you ate.\"\n");
        let rule = NotificationRule::all(&ConfigFile::new(file)).remove(0);
        assert_eq!(rule.title(), "Food");
        assert_eq!(rule.body(day()), "Log what you ate.");
    }

    #[test]
    fn a_vault_may_set_a_rule() {
        let settings = crate::Settings::resolved(DIARY, "");
        let rules = NotificationRule::all(settings.file());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].kind, RuleKind::Heading("D MMM YYYY · dddd".into()));
        let from: Vec<(String, &str)> =
            settings.entries().into_iter().filter(|e| e.key.starts_with(SECTION)).map(|e| (e.key, e.from.name())).collect();
        assert_eq!(
            from,
            [
                ("notifications.rules.diary.note".to_string(), "vault"),
                ("notifications.rules.diary.heading".to_string(), "vault"),
                ("notifications.rules.diary.at".to_string(), "vault"),
            ]
        );
        // A path out of the vault is refused, so the rule has no note.
        let out = DIARY.replace("Diet/Diary.md", "../Diary.md");
        assert!(NotificationRule::all(crate::Settings::resolved(&out, "").file()).is_empty());
    }

    fn tasks(lines: &[(&str, &str)]) -> Vec<VaultTask> {
        lines
            .iter()
            .enumerate()
            .map(|(line, (path, text))| VaultTask { path: (*path).into(), task: crate::TaskLine::parse(text, line, "").unwrap() })
            .collect()
    }

    fn look<'a>(
        read: &'a dyn Fn(&str) -> String,
        tasks: &'a dyn Fn() -> Vec<VaultTask>,
        changed: &'a dyn Fn(&str) -> Option<Day>,
    ) -> RuleLook<'a> {
        RuleLook { read, tasks, changed }
    }

    #[test]
    fn a_rule_rings_on_its_days() {
        let rule = NotificationRule::all(&ConfigFile::new(format!("{DIARY}days = [\"mon\", \"Fri\"]\n"))).remove(0);
        assert!(rule.rings_on(Day::parse("2026-11-23").unwrap()));
        assert!(!rule.rings_on(Day::parse("2026-11-24").unwrap()));
        assert!(rule.rings_on(Day::parse("2026-11-27").unwrap()));
        let (read, none, never) = (|_: &str| String::new(), Vec::new, |_: &str| None);
        assert!(rule.alert(Day::parse("2026-11-24").unwrap(), &look(&read, &none, &never)).is_none());
        assert!(rule.alert(Day::parse("2026-11-23").unwrap(), &look(&read, &none, &never)).is_some());
        // No days is every day; an empty list, or a word that is no day, is no rule.
        assert!(diary().rings_on(Day::parse("2026-11-24").unwrap()));
        for days in ["[\"someday\"]", "[]", "\"mon\""] {
            assert!(NotificationRule::all(&ConfigFile::new(format!("{DIARY}days = {days}\n"))).is_empty(), "{days}");
        }
    }

    #[test]
    fn a_query_rule() {
        let file = "[notifications.rules.late]\nquery = [\"due before tomorrow\", \"not done\"]\nat = \"18:00\"\n";
        let rule = NotificationRule::all(&ConfigFile::new(file)).remove(0);
        assert_eq!((rule.note.as_deref(), rule.title()), (None, "Tasks".to_string()));
        let found = || {
            tasks(&[
                ("Tasks/Personal.md", "- [ ] Pay the [[Rent|rent]] 📅 2026-11-25"),
                ("Tasks/Personal.md", "- [ ] Call the bank 📅 2026-11-20"),
                ("Tasks/Personal.md", "- [x] Paid 📅 2026-11-20"),
                ("Tasks/Personal.md", "- [ ] Later 📅 2026-12-20"),
            ])
        };
        let (read, never) = (|_: &str| String::new(), |_: &str| None);
        let alert = rule.alert(day(), &look(&read, &found, &never)).unwrap();
        assert_eq!(alert.body, "2 tasks: Call the bank, Pay the rent");
        assert_eq!(alert.note, None);
        let none = || tasks(&[("Tasks/Personal.md", "- [ ] Later 📅 2026-12-20")]);
        assert!(rule.alert(day(), &look(&read, &none, &never)).is_none());
        // A query and a heading together, or neither, is no rule.
        assert!(NotificationRule::all(&ConfigFile::new(format!("{file}heading = \"D\"\nnote = \"A.md\"\n"))).is_empty());
        assert!(NotificationRule::all(&ConfigFile::new("[notifications.rules.x]\nnote = \"A.md\"\nat = \"18:00\"\n")).is_empty());
    }

    #[test]
    fn a_quiet_rule() {
        let file = "[notifications.rules.plan]\nnote = \"Plan.md\"\nquiet_days = 14\nat = \"10:00\"\n";
        let rule = NotificationRule::all(&ConfigFile::new(file)).remove(0);
        let (read, none) = (|_: &str| String::new(), Vec::new);
        let fifteen = |_: &str| Some(day().adding_days(-15));
        let alert = rule.alert(day(), &look(&read, &none, &fifteen)).unwrap();
        assert_eq!((alert.title.as_str(), alert.body.as_str(), alert.note.as_deref()), ("Plan", "No change for 15 days", Some("Plan.md")));
        let thirteen = |_: &str| Some(day().adding_days(-13));
        assert!(rule.alert(day(), &look(&read, &none, &thirteen)).is_none());
        // A quiet rule needs its note, and a number of days in range.
        assert!(NotificationRule::all(&ConfigFile::new(file.replace("note = \"Plan.md\"\n", ""))).is_empty());
        assert!(NotificationRule::all(&ConfigFile::new(file.replace("14", "0"))).is_empty());
    }

    #[test]
    fn the_last_change_is_the_files_without_git() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Plan.md"), "x").unwrap();
        assert_eq!(last_change(root.path(), "Plan.md"), Some(Day::today()));
        assert_eq!(last_change(root.path(), "Gone.md"), None);
    }

    #[test]
    fn the_digest_is_filtered() {
        let filter = DigestFilter::of(&ConfigFile::new("[notifications]\ndigest_filter = [\"path does not include Elaheh\"]\n"));
        let due = tasks(&[("Tasks/Elaheh Interviews.md", "- [ ] Apply 📅 2026-11-25"), ("Tasks/Personal.md", "- [ ] Pay 📅 2026-11-25")]);
        let kept = filter.keep(due.clone(), day());
        assert_eq!(kept.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["Tasks/Personal.md"]);
        assert!(filter.not_understood().is_empty());
        // No filter keeps every task; a line it cannot read is left out and named.
        assert_eq!(DigestFilter::default().keep(due.clone(), day()), due);
        let odd = DigestFilter { lines: vec!["path does not include Elaheh".into(), "colour is blue".into()] };
        assert_eq!(odd.keep(due, day()).len(), 1);
        assert_eq!(odd.not_understood(), ["colour is blue"]);
    }
}
