//! The Open Knowledge Format (OKF 0.3) as Daftar reads it
//! (`docs/specs/open-knowledge-format.md`): a note whose front matter has a
//! `type` is a concept, and its front matter says where it came from, who
//! confirmed it and when it goes stale. Nothing here refuses a note: one
//! that breaks a rule is still read, opened and searched. The port of
//! `OKF.swift`.

use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};

use crate::yaml::{Yaml, YamlNode};

/// A time key's value: an instant, or what was written when it is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Time {
    At(DateTime<Utc>),
    Malformed(String),
}

impl Time {
    pub fn instant(&self) -> Option<DateTime<Utc>> {
        match self {
            Time::At(at) => Some(*at),
            Time::Malformed(_) => None,
        }
    }
}

/// `status`: absent is stable; a value that is none of the three is kept
/// as written and is not stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Draft,
    Stable,
    Deprecated,
    Other(String),
}

impl Status {
    pub fn name(&self) -> &str {
        match self {
            Status::Draft => "draft",
            Status::Stable => "stable",
            Status::Deprecated => "deprecated",
            Status::Other(raw) => raw,
        }
    }

    /// As a chip says it: "Draft", "Deprecated", or the raw value.
    pub fn title(&self) -> String {
        match self {
            Status::Draft => "Draft".into(),
            Status::Stable => "Stable".into(),
            Status::Deprecated => "Deprecated".into(),
            Status::Other(raw) => raw.clone(),
        }
    }
}

/// Who confirmed a concept (§5.3), from its current events only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Unverified,
    MachineConfirmed,
    HumanReviewed,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Unverified => "unverified",
            Tier::MachineConfirmed => "machine-confirmed",
            Tier::HumanReviewed => "human-reviewed",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Tier::Unverified => "Unverified",
            Tier::MachineConfirmed => "Machine-confirmed",
            Tier::HumanReviewed => "Human-reviewed",
        }
    }
}

/// One `{by, at}`: an actor and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub by: String,
    pub at: Time,
}

/// `generated`, with what it has: either key may be missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    pub by: Option<String>,
    pub at: Option<Time>,
}

/// Why a concept is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    Since(DateTime<Utc>),
    /// `stale_after` is no time: stale, never fresh.
    Malformed(String),
}

impl fmt::Display for Stale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stale::Since(at) => write!(f, "stale since {}", at.format("%Y-%m-%d")),
            Stale::Malformed(_) => write!(f, "stale: bad date"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concept {
    /// `type`: the kind, any text.
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub resource: Option<String>,
    pub tags: Vec<String>,
    pub status: Status,
    pub superseded_by: Option<String>,
    pub stale_after: Option<Time>,
    pub generated: Option<Generated>,
    /// The events with both keys, in the file's order.
    pub verified: Vec<Event>,
    /// The `verified` events left out because one lacks `by` or `at`.
    pub incomplete: usize,
}

impl Concept {
    /// The events that count: at or after `generated.at`, or all of them
    /// when nothing says the text was generated.
    pub fn current(&self) -> Vec<&Event> {
        let since = match &self.generated {
            None => None,
            // Generated at a time nobody can read: no event is known to be after it.
            Some(generated) => match generated.at.as_ref().and_then(Time::instant) {
                Some(at) => Some(at),
                None => return Vec::new(),
            },
        };
        self.verified.iter().filter(|event| event.at.instant().is_some_and(|at| since.is_none_or(|since| at >= since))).collect()
    }

    pub fn tier(&self) -> Tier {
        let current = self.current();
        if current.iter().any(|event| Okf::is_human(&event.by)) {
            Tier::HumanReviewed
        } else if current.is_empty() {
            Tier::Unverified
        } else {
            Tier::MachineConfirmed
        }
    }

    /// The last current event, for a chip's tooltip.
    pub fn last_verified(&self) -> Option<&Event> {
        self.current().into_iter().max_by_key(|event| event.at.instant())
    }

    /// Stale when [now] is at or after `stale_after`, or when that is no time.
    pub fn stale(&self, now: DateTime<Utc>) -> Option<Stale> {
        match self.stale_after.as_ref()? {
            Time::At(at) => (now >= *at).then_some(Stale::Since(*at)),
            Time::Malformed(raw) => Some(Stale::Malformed(raw.clone())),
        }
    }

    pub fn is_deprecated(&self) -> bool {
        self.status == Status::Deprecated
    }

    /// The one line the reader has above a deprecated or stale concept's
    /// text, and nothing above any other. [replacement] is the name of the
    /// note `superseded_by` resolves to, when it does.
    pub fn reader_line(&self, replacement: Option<&str>, now: DateTime<Utc>) -> Option<String> {
        if self.is_deprecated() {
            return Some(match replacement {
                Some(name) => format!("Deprecated. Replaced by {name}."),
                None => "Deprecated.".to_string(),
            });
        }
        match self.stale(now)? {
            Stale::Since(at) => Some(format!("Stale since {}: check it again.", at.format("%Y-%m-%d"))),
            Stale::Malformed(_) => Some("Stale: bad date. Check it again.".to_string()),
        }
    }

    /// `concept: <type>; <tier>; <status>; stale since …`, as the MCP
    /// server says it of a note.
    pub fn summary(&self, now: DateTime<Utc>) -> String {
        let mut parts = vec![format!("concept: {}", self.kind), self.tier().name().to_string(), self.status.name().to_string()];
        if let Some(stale) = self.stale(now) {
            parts.push(stale.to_string());
        }
        parts.join("; ")
    }

    /// The time keys that are no time, by name: `stale_after`,
    /// `generated.at`, `verified[].at`.
    pub fn malformed_times(&self) -> Vec<(&'static str, &str)> {
        let mut found = Vec::new();
        if let Some(Time::Malformed(raw)) = &self.stale_after {
            found.push(("stale_after", raw.as_str()));
        }
        if let Some(Generated { at: Some(Time::Malformed(raw)), .. }) = &self.generated {
            found.push(("generated.at", raw.as_str()));
        }
        for event in &self.verified {
            if let Time::Malformed(raw) = &event.at {
                found.push(("verified[].at", raw.as_str()));
            }
        }
        found
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OkfFailure {
    /// The note has no `type`: it is not a concept.
    NotAConcept,
    /// No actor is set: `okf.actor`, or the clone's `user.email`.
    NoActor,
}

impl fmt::Display for OkfFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OkfFailure::NotAConcept => write!(f, "the note is not a concept: its front matter has no type"),
            OkfFailure::NoActor => write!(f, "set okf.actor in config.toml, or user.email in the vault's git config, to say who verifies"),
        }
    }
}

impl std::error::Error for OkfFailure {}

pub struct Okf;

impl Okf {
    /// The version of OKF an export is written as.
    pub const VERSION: &'static str = "0.3";

    /// The concept [front_matter] (the YAML between the fences) describes,
    /// or none when it has no `type`.
    pub fn concept(front_matter: &str) -> Option<Concept> {
        let node = Yaml::parse(front_matter);
        let text = |key: &str| scalar(node.get(key)?);
        let kind = text("type")?;
        let status = match text("status").as_deref() {
            None | Some("stable") => Status::Stable,
            Some("draft") => Status::Draft,
            Some("deprecated") => Status::Deprecated,
            Some(other) => Status::Other(other.to_string()),
        };
        let generated = node.get("generated").map(flow).filter(|node| !is_absent(node)).map(|node| Generated {
            by: node.get("by").and_then(scalar),
            at: node.get("at").and_then(scalar).map(|raw| Self::read_time(&raw)),
        });
        let events: Vec<YamlNode> = node.get("verified").map(flow).map_or_default(|node| match node {
            YamlNode::List(items) => items.iter().map(flow).collect(),
            YamlNode::Null => Vec::new(),
            one => vec![one],
        });
        let verified: Vec<Event> = events
            .iter()
            .filter_map(|event| {
                Some(Event { by: event.get("by").and_then(scalar)?, at: Self::read_time(&event.get("at").and_then(scalar)?) })
            })
            .collect();
        Some(Concept {
            kind,
            title: text("title"),
            description: text("description"),
            resource: text("resource"),
            tags: node.get("tags").map_or_default(|tags| tags.items().into_iter().filter_map(scalar).collect()),
            status,
            superseded_by: text("superseded_by"),
            stale_after: text("stale_after").map(|raw| Self::read_time(&raw)),
            generated,
            incomplete: events.len() - verified.len(),
            verified,
        })
    }

    /// The concept the note at [path] with the text [note] is. `index.md`
    /// and `log.md` are never concepts, whatever their front matter.
    pub fn concept_of(path: &str, note: &str) -> Option<Concept> {
        if Self::is_reserved(path) {
            return None;
        }
        Self::concept(front_matter(note)?.yaml)
    }

    /// Whether the note at [path] with the front matter's [properties], as
    /// the index holds them, is a deprecated concept: what quick open and
    /// search dim, with no file to read.
    pub fn is_deprecated(path: &str, properties: &[(String, Vec<String>)]) -> bool {
        let first = |key: &str| properties.iter().find(|(name, _)| name == key).and_then(|(_, values)| values.first()).map(|v| v.trim());
        !Self::is_reserved(path) && first("type").is_some_and(|kind| !kind.is_empty()) && first("status") == Some("deprecated")
    }

    /// `index.md` and `log.md`, which list a folder and its changes.
    pub fn is_reserved(path: &str) -> bool {
        matches!(path.rsplit('/').next(), Some("index.md" | "log.md"))
    }

    /// A time: a date-time with an offset, or a date alone, which is the
    /// start of that day in UTC. Anything else is none.
    pub fn time(text: &str) -> Option<DateTime<Utc>> {
        let text = text.trim();
        if let Ok(at) = DateTime::parse_from_rfc3339(text) {
            return Some(at.with_timezone(&Utc));
        }
        // YAML's own form may part the day and the time with a space.
        if let Some((day, time)) = text.split_once(' ')
            && let Ok(at) = DateTime::parse_from_rfc3339(&format!("{day}T{}", time.trim_start()))
        {
            return Some(at.with_timezone(&Utc));
        }
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().and_then(|day| day.and_hms_opt(0, 0, 0)).map(|at| at.and_utc())
    }

    fn read_time(raw: &str) -> Time {
        Self::time(raw).map_or_else(|| Time::Malformed(raw.to_string()), Time::At)
    }

    /// [at] as an event writes it: UTC, to the second.
    pub fn written(at: DateTime<Utc>) -> String {
        at.to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    /// Only `human:<id>` is a person; `team:`, `process:` and a
    /// `<producer>/<version>` are not.
    pub fn is_human(actor: &str) -> bool {
        actor.strip_prefix("human:").is_some_and(|id| !id.trim().is_empty())
    }

    /// The person who verifies: `okf.actor` of the settings, else the local
    /// part of the clone's `user.email`.
    pub fn actor(setting: Option<&str>, email: Option<&str>) -> Result<String, OkfFailure> {
        let id = setting
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| id.strip_prefix("human:").unwrap_or(id))
            .or_else(|| email.and_then(|email| email.trim().split('@').next()).filter(|id| !id.is_empty()))
            .ok_or(OkfFailure::NoActor)?;
        Ok(format!("human:{id}"))
    }

    /// The actor of an agent's write: `<client name>/<client version>` in
    /// lower case with `-` for spaces; with no client, `daftar-mcp/<version>`.
    pub fn producer(client: Option<(&str, &str)>, version: &str) -> String {
        let clean = |text: &str| text.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
        match client.map(|(name, version)| (clean(name), clean(version))).filter(|(name, _)| !name.is_empty()) {
            Some((name, version)) if version.is_empty() => name,
            Some((name, version)) => format!("{name}/{version}"),
            None => format!("daftar-mcp/{version}"),
        }
    }

    /// Mark as Verified: [note] with `{ by: <actor>, at: <now> }` in its
    /// `verified`, every other line byte for byte. An event of that actor
    /// already there is replaced, not added twice. Refused on a note that
    /// is not a concept.
    pub fn mark_verified(path: &str, note: &str, actor: &str, now: DateTime<Utc>) -> Result<String, OkfFailure> {
        Self::concept_of(path, note).ok_or(OkfFailure::NotAConcept)?;
        let front = front_matter(note).ok_or(OkfFailure::NotAConcept)?;
        let node = Yaml::parse(front.yaml);
        let mut events: Vec<Vec<(String, String)>> = node
            .get("verified")
            .map(flow)
            .map_or_default(|node| match node {
                YamlNode::List(items) => items.iter().map(flow).collect(),
                YamlNode::Null => Vec::new(),
                one => vec![one],
            })
            .iter()
            .map(|event| event.entries().iter().filter_map(|(key, value)| Some((key.clone(), scalar(value)?))).collect())
            .filter(|event: &Vec<(String, String)>| !event.is_empty())
            .collect();
        events.retain(|event| !event.iter().any(|(key, value)| key == "by" && value == actor));
        events.push(vec![("by".into(), actor.to_string()), ("at".into(), Self::written(now))]);
        let mut lines = vec!["verified:".to_string()];
        for event in &events {
            for (at, (key, value)) in event.iter().enumerate() {
                lines.push(format!("{}{key}: {}", if at == 0 { "  - " } else { "    " }, quoted(value)));
            }
        }
        Ok(front.with_key(note, "verified", &lines))
    }

    /// An agent changed the text of [note]: `generated: { by, at }` is set
    /// in it, and `verified` stays as history. None for a note that is not
    /// a concept, which gets no front matter.
    pub fn mark_generated(path: &str, note: &str, by: &str, now: DateTime<Utc>) -> Option<String> {
        Self::concept_of(path, note)?;
        let front = front_matter(note)?;
        let lines = ["generated:".to_string(), format!("  by: {}", quoted(by)), format!("  at: {}", quoted(&Self::written(now)))];
        Some(front.with_key(note, "generated", &lines))
    }

    /// The folders that are bundle roots: each has an `index.md` whose
    /// front matter has `okf_version`. [notes] are vault paths with their
    /// front matter's keys.
    pub fn bundle_roots<'a>(notes: impl IntoIterator<Item = (&'a str, bool)>) -> BTreeSet<String> {
        notes
            .into_iter()
            .filter(|(path, versioned)| *versioned && path.rsplit('/').next() == Some("index.md"))
            .map(|(path, _)| path.rsplit_once('/').map_or_default(|(folder, _)| folder.to_string()))
            .collect()
    }

    /// The bundle root of the note at [source]: the nearest folder at or
    /// above it that is one of [roots], else the vault's root (empty).
    pub fn bundle_root<'a>(source: &str, roots: &'a BTreeSet<String>) -> &'a str {
        let mut folder = source.rsplit_once('/').map_or("", |(folder, _)| folder);
        loop {
            if let Some(root) = roots.get(folder) {
                return root;
            }
            match folder.rsplit_once('/') {
                Some((parent, _)) => folder = parent,
                None if folder.is_empty() => return "",
                None => folder = "",
            }
        }
    }

    /// The vault path a Markdown link's [target] names from the note at
    /// [source], as §6 reads it: from the bundle root when it starts with
    /// `/`, else from the note's folder. None when it leaves the vault,
    /// which is then never opened or read.
    pub fn path(target: &str, source: &str, roots: &BTreeSet<String>) -> Option<String> {
        let (base, rest) = match target.strip_prefix('/') {
            Some(rest) => (Self::bundle_root(source, roots), rest),
            None => (source.rsplit_once('/').map_or("", |(folder, _)| folder), target),
        };
        let floor = if target.starts_with('/') { base.split('/').filter(|part| !part.is_empty()).count() } else { 0 };
        let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
        for part in rest.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    // A `/` path never climbs out of its bundle, nor any path out of the vault.
                    if parts.len() <= floor {
                        return None;
                    }
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        (!parts.is_empty()).then(|| parts.join("/"))
    }
}

/// A key's value as text; null, an empty text and an empty list are absent.
fn scalar(node: &YamlNode) -> Option<String> {
    match node {
        YamlNode::Scalar(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        YamlNode::List(items) => items.first().and_then(scalar),
        _ => None,
    }
}

fn is_absent(node: &YamlNode) -> bool {
    match node {
        YamlNode::Null => true,
        YamlNode::Scalar(text) => text.trim().is_empty(),
        YamlNode::List(items) => items.is_empty(),
        YamlNode::Map(_) => false,
    }
}

/// [node], with a mapping written on one line (`{ by: x, at: y }`) read as
/// one: the YAML reader keeps it as text.
fn flow(node: &YamlNode) -> YamlNode {
    let YamlNode::Scalar(text) = node else { return node.clone() };
    let text = text.trim();
    let Some(inner) = text.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')) else { return node.clone() };
    let mut entries = Vec::new();
    for part in split_flow(inner) {
        let Some((key, value)) = Yaml::split_key(part.trim()) else { continue };
        entries.push((key, Yaml::inline(&value)));
    }
    YamlNode::Map(entries)
}

/// The parts of a flow mapping, cut at the commas outside quotes.
fn split_flow(inner: &str) -> Vec<&str> {
    let (mut parts, mut start, mut quote) = (Vec::new(), 0, None);
    for (at, c) in inner.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, ',') => {
                parts.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
}

/// [text] as a YAML scalar in double quotes: an actor has a colon, and a
/// time would else be read as YAML's own.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The front matter of a note: the YAML between the fences, and where its
/// lines are in the note.
pub struct FrontMatter<'a> {
    pub yaml: &'a str,
    /// The byte offset of the first line after the opening fence.
    start: usize,
    /// The byte offset of the closing fence's line.
    end: usize,
}

/// The front matter of [note], when it starts with a `---` line that a
/// `---` or `...` line closes.
pub fn front_matter(note: &str) -> Option<FrontMatter<'_>> {
    let first = note.split_inclusive('\n').next()?;
    if first.trim_end_matches(['\n', '\r']).trim_matches([' ', '\t']) != "---" {
        return None;
    }
    let start = first.len();
    let mut at = start;
    for line in note[start..].split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']).trim_matches([' ', '\t']);
        if bare == "---" || bare == "..." {
            return Some(FrontMatter { yaml: &note[start..at], start, end: at });
        }
        at += line.len();
    }
    None
}

impl FrontMatter<'_> {
    /// [note] with the top-level [key] written as [lines], in its place
    /// when the key is there, else after the others; every other line stays
    /// byte for byte.
    fn with_key(&self, note: &str, key: &str, lines: &[String]) -> String {
        let eol = if note[..self.start].ends_with("\r\n") { "\r\n" } else { "\n" };
        let block: String = lines.iter().map(|line| format!("{line}{eol}")).collect();
        let mut at = self.start;
        let mut found: Option<(usize, usize)> = None;
        for line in self.yaml.split_inclusive('\n') {
            let next = at + line.len();
            let bare = line.trim_end_matches(['\n', '\r']);
            match found {
                None => {
                    if Yaml::split_key(bare).is_some_and(|(name, _)| name == key) && !bare.starts_with([' ', '\t']) {
                        found = Some((at, next));
                    }
                }
                // The key's own lines: indented, or the items of a list at its indent.
                Some((from, _)) if bare.starts_with([' ', '\t']) || bare == "-" || bare.starts_with("- ") => found = Some((from, next)),
                Some(_) => break,
            }
            at = next;
        }
        if let Some((from, to)) = found {
            return format!("{}{block}{}", &note[..from], &note[to..]);
        }
        // A last line with no newline of its own gets one first.
        let gap = if self.yaml.is_empty() || self.yaml.ends_with('\n') { "" } else { eol };
        format!("{}{gap}{block}{}", &note[..self.end], &note[self.end..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        Okf::time(text).unwrap()
    }

    #[test]
    fn a_note_with_a_type_is_a_concept() {
        assert_eq!(Okf::concept_of("Metrics/Revenue.md", "---\ntype: Metric\n---\n# Revenue\n").unwrap().kind, "Metric");
        assert_eq!(Okf::concept_of("N.md", "# No front matter\n"), None);
        assert_eq!(Okf::concept_of("N.md", "---\ntitle: N\n---\n"), None);
        assert_eq!(Okf::concept_of("N.md", "---\ntype:\n---\n"), None);
        assert_eq!(Okf::concept_of("N.md", "---\ntype: \"\"\n---\n"), None);
        assert_eq!(Okf::concept_of("N.md", "---\ntype: null\n---\n"), None);
        // `index.md` and `log.md` are never concepts, whatever their front matter.
        assert_eq!(Okf::concept_of("index.md", "---\ntype: Metric\n---\n"), None);
        assert_eq!(Okf::concept_of("Docs/log.md", "---\ntype: Metric\n---\n"), None);
        assert!(Okf::concept_of("Docs/Index.md", "---\ntype: Metric\n---\n").is_some());
        // The other keys, as written.
        let concept =
            Okf::concept("type: Metric\ntitle: Revenue\ndescription: Money in.\nresource: https://example.com/r\ntags: [a, b]\nowner: x\n")
                .unwrap();
        assert_eq!(concept.title.as_deref(), Some("Revenue"));
        assert_eq!(concept.description.as_deref(), Some("Money in."));
        assert_eq!(concept.resource.as_deref(), Some("https://example.com/r"));
        assert_eq!(concept.tags, ["a", "b"]);
        assert_eq!((concept.title.is_none(), Okf::concept("type: Metric\ntitle: \"\"\n").unwrap().title), (false, None));
    }

    #[test]
    fn the_trust_tier_comes_from_current_events() {
        let tier = |front: &str| Okf::concept(&format!("type: Metric\n{front}")).unwrap().tier();
        assert_eq!(tier(""), Tier::Unverified);
        // A bare mapping is a list of one.
        let human = "verified: { by: \"human:parham\", at: \"2026-06-25T09:00:00Z\" }\n";
        assert_eq!(tier(human), Tier::HumanReviewed);
        assert_eq!(tier("verified:\n  by: \"human:parham\"\n  at: \"2026-06-25T09:00:00Z\"\n"), Tier::HumanReviewed);
        // The text was generated after it: the event is history.
        let generated = "generated: { by: claude-code/2.1, at: \"2026-06-26T00:00:00Z\" }\n";
        assert_eq!(tier(&format!("{human}{generated}")), Tier::Unverified);
        let list = "verified:\n  - by: \"human:parham\"\n    at: \"2026-06-25T09:00:00Z\"\n  - by: process:nightly\n    at: \"2026-06-27T03:00:00Z\"\n";
        assert_eq!(tier(&format!("{generated}{list}")), Tier::MachineConfirmed);
        assert_eq!(tier("verified:\n  - by: team:x\n    at: 2026-06-25\n"), Tier::MachineConfirmed);
        // An event with no `at` is left out.
        let concept = Okf::concept("type: Metric\nverified:\n  - by: \"human:parham\"\n  - at: 2026-06-25\n").unwrap();
        assert_eq!((concept.tier(), concept.verified.len(), concept.incomplete), (Tier::Unverified, 0, 2));
        // An event at the very second of `generated.at` is current.
        assert_eq!(
            tier(
                "generated: { by: x/1, at: \"2026-06-26T00:00:00Z\" }\nverified: { by: \"human:p\", at: \"2026-06-26T02:00:00+02:00\" }\n"
            ),
            Tier::HumanReviewed
        );
        let concept = Okf::concept(&format!("type: Metric\n{generated}{list}")).unwrap();
        assert_eq!(concept.last_verified().unwrap().by, "process:nightly");
        assert_eq!(concept.generated.as_ref().unwrap().by.as_deref(), Some("claude-code/2.1"));
        // Only `human:` is a person.
        assert!(Okf::is_human("human:parham") && !Okf::is_human("team:x") && !Okf::is_human("claude-code/2.1") && !Okf::is_human("human:"));
    }

    #[test]
    fn a_time_has_an_offset_or_is_a_day() {
        let instant = at("2026-06-30T14:00:00Z");
        assert_eq!(at("2026-06-30T16:00:00+02:00"), instant);
        let read = |front: &str| Okf::concept(&format!("type: T\n{front}")).unwrap().stale_after.unwrap();
        assert_eq!(read("stale_after: \"2026-06-30T14:00:00Z\"\n"), Time::At(instant));
        assert_eq!(read("stale_after: 2026-06-30T14:00:00Z\n"), Time::At(instant));
        assert_eq!(read("stale_after: 2026-06-30 16:00:00+02:00\n"), Time::At(instant));
        let day = at("2026-06-30T00:00:00Z");
        assert_eq!(read("stale_after: 2026-06-30\n"), Time::At(day));
        assert_eq!(read("stale_after: \"2026-06-30\"\n"), Time::At(day));
        for wrong in ["2026-06-30T14:00:00", "30/06/2026", "soon"] {
            assert_eq!(Okf::time(wrong), None, "{wrong}");
            assert_eq!(read(&format!("stale_after: \"{wrong}\"\n")), Time::Malformed(wrong.into()));
        }
        assert_eq!(Okf::written(at("2026-06-30T16:00:00.25+02:00")), "2026-06-30T14:00:00Z");
    }

    #[test]
    fn a_concept_is_stale_at_its_instant() {
        let now = at("2026-09-23T00:00:00Z");
        let stale = |front: &str| Okf::concept(&format!("type: T\n{front}")).unwrap().stale(now);
        assert_eq!(stale("stale_after: \"2026-09-23T00:00:00Z\"\n"), Some(Stale::Since(now)));
        assert_eq!(stale("stale_after: \"2026-09-23T00:00:01Z\"\n"), None);
        assert_eq!(stale(""), None);
        assert_eq!(stale("stale_after: null\n"), None);
        assert_eq!(stale("stale_after:\n"), None);
        assert_eq!(stale("stale_after: \"\"\n"), None);
        // A malformed time is stale, never fresh, and is shown as malformed.
        assert_eq!(stale("stale_after: soon\n"), Some(Stale::Malformed("soon".into())));
        let concept = Okf::concept("type: T\nstale_after: soon\n").unwrap();
        assert_eq!(concept.malformed_times(), [("stale_after", "soon")]);
        assert_eq!(stale("stale_after: 2026-09-23\n").unwrap().to_string(), "stale since 2026-09-23");
        assert_eq!(concept.stale(now).unwrap().to_string(), "stale: bad date");
        assert_eq!(concept.summary(now), "concept: T; unverified; stable; stale: bad date");
    }

    #[test]
    fn an_unknown_status_is_not_stable() {
        let status = |front: &str| Okf::concept(&format!("type: T\n{front}")).unwrap().status;
        assert_eq!(status(""), Status::Stable);
        assert_eq!(status("status: draft\n"), Status::Draft);
        assert_eq!(status("status: stable\n"), Status::Stable);
        assert_eq!(status("status: deprecated\n"), Status::Deprecated);
        let archived = status("status: archived\n");
        assert_eq!(archived, Status::Other("archived".into()));
        assert!(archived != Status::Stable && archived.name() == "archived" && archived.title() == "archived");
        // A scalar `tags` is one tag, never split at commas.
        assert_eq!(Okf::concept("type: T\ntags: a, b\n").unwrap().tags, ["a, b"]);
        assert_eq!(Okf::concept("type: T\ntags: []\n").unwrap().tags, [] as [&str; 0]);
        assert_eq!(
            Okf::concept("type: T\nstatus: deprecated\nsuperseded_by: /New.md\n").unwrap().superseded_by.as_deref(),
            Some("/New.md")
        );
    }

    #[test]
    fn the_reader_has_one_line_above_a_deprecated_or_stale_concept() {
        let now = at("2026-09-23T00:00:00Z");
        let line = |front: &str, replacement| Okf::concept(&format!("type: T\n{front}")).unwrap().reader_line(replacement, now);
        assert_eq!(line("", None), None);
        assert_eq!(line("status: draft\n", None), None);
        assert_eq!(line("status: deprecated\nstale_after: 2020-01-01\n", Some("New")).as_deref(), Some("Deprecated. Replaced by New."));
        assert_eq!(line("status: deprecated\n", None).as_deref(), Some("Deprecated."));
        assert_eq!(line("stale_after: 2026-09-01\n", None).as_deref(), Some("Stale since 2026-09-01: check it again."));
        assert_eq!(line("stale_after: soon\n", None).as_deref(), Some("Stale: bad date. Check it again."));
        let properties = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| ((*k).to_string(), vec![(*v).to_string()])).collect::<Vec<_>>();
        assert!(Okf::is_deprecated("N.md", &properties(&[("type", "T"), ("status", "deprecated")])));
        assert!(!Okf::is_deprecated("N.md", &properties(&[("status", "deprecated")])));
        assert!(!Okf::is_deprecated("N.md", &properties(&[("type", "T"), ("status", "draft")])));
        assert!(!Okf::is_deprecated("index.md", &properties(&[("type", "T"), ("status", "deprecated")])));
    }

    #[test]
    fn a_markdown_link_resolves_as_okf_says() {
        let roots = BTreeSet::from(["Docs".to_string()]);
        let from = "Docs/metrics/revenue.md";
        assert_eq!(Okf::path("/tables/orders.md", from, &roots).as_deref(), Some("Docs/tables/orders.md"));
        assert_eq!(Okf::path("../tables/orders.md", from, &roots).as_deref(), Some("Docs/tables/orders.md"));
        assert_eq!(Okf::path("./revenue.md", from, &roots).as_deref(), Some("Docs/metrics/revenue.md"));
        assert_eq!(Okf::path("my note.md", from, &roots).as_deref(), Some("Docs/metrics/my note.md"));
        // With no bundle above the note, `/` is the vault's root.
        assert_eq!(Okf::path("/Docs/tables/orders.md", "Other/x.md", &roots).as_deref(), Some("Docs/tables/orders.md"));
        // What leaves the vault, or its bundle, resolves to nothing.
        assert_eq!(Okf::path("../../../etc/passwd", from, &roots), None);
        assert_eq!(Okf::path("/../x.md", from, &roots), None);
        assert_eq!(Okf::path("/../x.md", "Other/x.md", &roots), None);
        assert_eq!(Okf::bundle_root("Docs/a/b/c.md", &roots), "Docs");
        assert_eq!(Okf::bundle_root("Other/x.md", &roots), "");
        let found =
            Okf::bundle_roots([("Docs/index.md", true), ("index.md", false), ("Docs/a/index.md", false), ("Notes/Index of.md", true)]);
        assert_eq!(found, roots);
        assert_eq!(Okf::bundle_roots([("index.md", true)]), BTreeSet::from([String::new()]));
    }

    #[test]
    fn mark_as_verified_writes_one_event() {
        let note = "---\n# Kept as it is.\ntype:   Metric\ntags: [a,  b]\nverified: { by: \"process:nightly\", at: \"2026-06-25T03:00:00Z\" }\nowner: \"x\"\n---\n\n# Revenue\n\nText.\n";
        let now = at("2026-06-26T10:30:15.5+02:00");
        let marked = Okf::mark_verified("Revenue.md", note, "human:parham", now).unwrap();
        assert_eq!(
            marked,
            "---\n# Kept as it is.\ntype:   Metric\ntags: [a,  b]\nverified:\n  - by: \"process:nightly\"\n    at: \"2026-06-25T03:00:00Z\"\n  - by: \"human:parham\"\n    at: \"2026-06-26T08:30:15Z\"\nowner: \"x\"\n---\n\n# Revenue\n\nText.\n"
        );
        let concept = Okf::concept_of("Revenue.md", &marked).unwrap();
        assert_eq!((concept.verified.len(), concept.tier()), (2, Tier::HumanReviewed));
        // Marking again replaces that actor's event.
        let again = Okf::mark_verified("Revenue.md", &marked, "human:parham", at("2026-07-01T00:00:00Z")).unwrap();
        let concept = Okf::concept_of("Revenue.md", &again).unwrap();
        assert_eq!(concept.verified.iter().map(|e| e.by.as_str()).collect::<Vec<_>>(), ["process:nightly", "human:parham"]);
        assert_eq!(concept.verified[1].at, Time::At(at("2026-07-01T00:00:00Z")));
        assert_eq!(again.replace("2026-07-01T00:00:00Z", "2026-06-26T08:30:15Z"), marked);
        // A concept with no event yet gets the key after the others.
        let first = Okf::mark_verified("N.md", "---\ntype: Guide\n---\nText\n", "human:parham", at("2026-07-01T00:00:00Z")).unwrap();
        assert_eq!(first, "---\ntype: Guide\nverified:\n  - by: \"human:parham\"\n    at: \"2026-07-01T00:00:00Z\"\n---\nText\n");
        // A note with no `type` is refused, and so is an index.
        assert_eq!(Okf::mark_verified("N.md", "---\ntitle: N\n---\n", "human:parham", now), Err(OkfFailure::NotAConcept));
        assert_eq!(Okf::mark_verified("N.md", "# N\n", "human:parham", now), Err(OkfFailure::NotAConcept));
        assert_eq!(Okf::mark_verified("index.md", "---\ntype: T\n---\n", "human:parham", now), Err(OkfFailure::NotAConcept));
        // Who verifies: the setting, else the local part of the clone's email.
        assert_eq!(Okf::actor(Some("parham"), Some("x@example.com")).as_deref(), Ok("human:parham"));
        assert_eq!(Okf::actor(Some("human:parham"), None).as_deref(), Ok("human:parham"));
        assert_eq!(Okf::actor(Some(" "), Some("p.alvani@example.com")).as_deref(), Ok("human:p.alvani"));
        assert_eq!(Okf::actor(None, None), Err(OkfFailure::NoActor));
    }

    #[test]
    fn an_agents_write_sets_generated_and_keeps_the_history() {
        let note = "---\ntype: Metric\ngenerated:\n  by: old/1\n  at: 2026-01-01\nverified:\n- by: \"human:parham\"\n  at: \"2026-06-25T09:00:00Z\"\n---\nText\n";
        let now = at("2026-06-26T00:00:00Z");
        let marked = Okf::mark_generated("M.md", note, "claude-code/2.1.0", now).unwrap();
        assert_eq!(
            marked,
            "---\ntype: Metric\ngenerated:\n  by: \"claude-code/2.1.0\"\n  at: \"2026-06-26T00:00:00Z\"\nverified:\n- by: \"human:parham\"\n  at: \"2026-06-25T09:00:00Z\"\n---\nText\n"
        );
        let concept = Okf::concept_of("M.md", &marked).unwrap();
        assert_eq!((concept.tier(), concept.verified.len()), (Tier::Unverified, 1));
        assert_eq!(Okf::mark_generated("N.md", "# N\n", "x/1", now), None);
        assert_eq!(Okf::mark_generated("N.md", "---\ntitle: N\n---\n", "x/1", now), None);
        assert_eq!(Okf::producer(Some(("Claude Code", "2.1.0")), "0.27.0"), "claude-code/2.1.0");
        assert_eq!(Okf::producer(None, "0.27.0"), "daftar-mcp/0.27.0");
        assert_eq!(Okf::producer(Some(("  ", "1")), "0.27.0"), "daftar-mcp/0.27.0");
        // Windows' line endings stay.
        let windows = Okf::mark_generated("M.md", "---\r\ntype: T\r\n---\r\nText\r\n", "x/1", now).unwrap();
        assert_eq!(windows, "---\r\ntype: T\r\ngenerated:\r\n  by: \"x/1\"\r\n  at: \"2026-06-26T00:00:00Z\"\r\n---\r\nText\r\n");
    }
}
