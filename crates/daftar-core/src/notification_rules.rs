//! Notifications the person defines: at a time of the day, a note that has
//! no heading for today rings (`docs/specs/notification-rules.md`). A rule
//! is a `[notifications.rules.<id>]` section of the settings, so a vault's
//! `.daftar.toml` can carry it to every device. The port of
//! `NotificationRule.swift`.

use chrono::NaiveTime;

use crate::{Clock, ConfigFile, Day, MomentFormat, NoteParser};

/// The section every rule is under, with its id after the dot.
pub const SECTION: &str = "notifications.rules";

/// The keys of a rule, as the settings check them.
pub const KEYS: &[&str] = &["note", "heading", "at", "title", "message"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRule {
    pub id: String,
    /// The note's path in the vault.
    pub note: String,
    /// A moment.js format of the day's heading.
    pub heading: String,
    /// The time of the check, on the device's wall clock.
    pub at: NaiveTime,
    pub title: Option<String>,
    pub message: Option<String>,
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
        let key = |name: &str| file.string(&format!("{SECTION}.{id}.{name}"));
        let note = key("note").filter(|note| is_note_path(note))?;
        let heading = key("heading").filter(|heading| !heading.trim().is_empty() && one_line(heading))?;
        let at = key("at").filter(|at| crate::settings::is_time(at)).and_then(|at| NaiveTime::parse_from_str(&at, "%H:%M").ok())?;
        let optional = |name: &str| key(name).filter(|text| !text.trim().is_empty() && one_line(text));
        Some(NotificationRule { title: optional("title"), message: optional("message"), id, note, heading, at })
    }

    /// The heading [day] should have.
    pub fn expected(&self, day: Day) -> String {
        MomentFormat::format(&self.heading, day, Clock::new(0, 0, 0))
    }

    /// Whether [text], the note's, has no heading for [day]: none whose
    /// text starts with the expected one.
    pub fn is_missing(&self, text: &str, day: Day) -> bool {
        let expected = self.expected(day);
        !NoteParser::parse(text).headings.iter().any(|heading| heading.text.trim().starts_with(&expected))
    }

    /// The alert's title: the rule's, else the note's name.
    pub fn title(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            let name = self.note.rsplit('/').next().unwrap_or(&self.note);
            name.strip_suffix(".md").unwrap_or(name).to_string()
        })
    }

    /// The alert's body on [day]: the rule's message, else what is missing.
    pub fn body(&self, day: Day) -> String {
        self.message.clone().unwrap_or_else(|| format!("No heading for today yet: {}", self.expected(day)))
    }

    /// The alert's identifier, the same every day.
    pub fn alert_id(&self) -> String {
        format!("rule.{}", self.id)
    }
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
        assert_eq!(rules[0].note, "Diet/Diary.md");
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
        let rule = NotificationRule { heading: "[Day] YYYY-MM-DD".into(), ..diary() };
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
        assert_eq!(rules[0].heading, "D MMM YYYY · dddd");
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
}
