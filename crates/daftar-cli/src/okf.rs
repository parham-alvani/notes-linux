//! `daftar-cli okf`: the vault as a bundle of the Open Knowledge Format
//! (OKF 0.3, `docs/specs/open-knowledge-format.md`). `export` writes a
//! conformant bundle from the vault: each note in scope at its own path
//! with a `type`, its body turned into CommonMark with `/` links, an
//! `index.md` in every folder and a `log.md` from git. `check` says
//! whether a folder is a bundle. The export writes nothing in the vault,
//! so it runs in a read-only clone too.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use regex::{Captures, Regex};

use daftar_core::okf::{Stale, Status, front_matter};
use daftar_core::yaml::{Yaml, YamlNode};
use daftar_core::{CalendarWeek, ConfigFile, ExcludedFiles, Git, Inbox, Okf, Templates, Vault, VaultIndex};

/// What the root `index.md` of a bundle this command wrote says.
const MARK: &str = "x-daftar-export";

/// What a path in a Markdown link may not hold as it is.
const ENCODED: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`').add(b'#').add(b'?').add(b'(').add(b')').add(b'%');

static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\]\n]+?)\]\]").unwrap());
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[([^\]\n]*)\]\(<?([^)>\s]+)>?\)").unwrap());
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap());
static CALLOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*(?:>\s*)+)\[!([^\]]*)\][+-]?[ \t]*(.*)$").unwrap());
static HIGHLIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==([^=\n]+?)==").unwrap());
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%%.*?%%").unwrap());
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+\^[A-Za-z0-9-]+[ \t]*$").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());
static INDEX_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[*+-] \[[^\]]*\]\([^)]*\)").unwrap());
static LOG_DAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^## \d{4}-\d{2}-\d{2}\s*$").unwrap());

/// The colours a highlight may start with, which a bundle has no use for.
const COLOURS: [char; 12] = ['🔴', '🟠', '🟡', '🟢', '🔵', '🟣', '🟥', '🟧', '🟨', '🟩', '🟦', '🟪'];

/// `[okf]` in the vault's `.daftar.toml`, with the command line's on top.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Folders to export, each with all under it; empty is the whole vault.
    pub include: Vec<String>,
    /// Folders and notes never exported, also under an included folder.
    pub exclude: Vec<String>,
    pub title: String,
    pub description: String,
    pub license: String,
    /// The type given to a note that has none, by folder.
    pub types: Vec<(String, String)>,
}

impl Scope {
    pub fn read(root: &Path) -> Scope {
        let file = ConfigFile::new(fs::read_to_string(root.join(".daftar.toml")).unwrap_or_default());
        let text = |key: &str| file.string(&format!("okf.{key}")).unwrap_or_default();
        let types = file
            .keys()
            .into_iter()
            .filter_map(|(key, value)| {
                let folder = key.strip_prefix("okf.types.")?.trim_matches(['"', '\'']).trim_matches('/').to_string();
                match value {
                    Some(daftar_core::Value::String(kind)) if !kind.trim().is_empty() => Some((folder, kind.trim().to_string())),
                    _ => None,
                }
            })
            .collect();
        Scope {
            include: file.strings("okf.include").unwrap_or_default(),
            exclude: file.strings("okf.exclude").unwrap_or_default(),
            title: text("title"),
            description: text("description"),
            license: text("license"),
            types,
        }
    }

    fn holds(&self, path: &str) -> bool {
        let under = |folder: &String| {
            let folder = folder.trim_matches('/');
            folder.is_empty() || path == folder || path == format!("{folder}.md") || path.starts_with(&format!("{folder}/"))
        };
        (self.include.is_empty() || self.include.iter().any(under)) && !self.exclude.iter().any(under)
    }

    /// The type of a note with none: the longest folder of `[okf.types]`
    /// it is under, else `Note`.
    fn kind(&self, path: &str) -> &str {
        self.types
            .iter()
            .filter(|(folder, _)| folder.is_empty() || path.starts_with(&format!("{folder}/")))
            .max_by_key(|(folder, _)| folder.len())
            .map_or("Note", |(_, kind)| kind)
    }
}

pub struct Options {
    pub output: PathBuf,
    /// How many days back `log.md` goes.
    pub log_days: i64,
    /// Lists what it would write and remove, and writes nothing.
    pub dry_run: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub written: Vec<String>,
    pub removed: Vec<String>,
    /// The files that were there with the same bytes, and were left alone.
    pub unchanged: usize,
}

/// A file of the bundle: its text, or a file of the vault copied as it is.
enum Content {
    Text(String),
    Asset(PathBuf),
}

/// Writes the bundle of the vault at [root] into `options.output`.
pub fn export(root: &Path, index: &VaultIndex, hidden: &ExcludedFiles, scope: &Scope, options: &Options) -> Result<Exported, String> {
    let output = &options.output;
    let vault = root.canonicalize().map_err(|e| format!("{}: {e}", root.display()))?;
    let within = |folder: &Path| folder.canonicalize().is_ok_and(|real| real.starts_with(&vault));
    // The nearest folder that is there says where a new output would be.
    let mut there = output.clone();
    while !there.exists() {
        match there.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => there = parent.to_path_buf(),
            _ => {
                there = PathBuf::from(".");
                break;
            }
        }
    }
    if within(&there) {
        return Err(format!("{} is in the vault: an export is written outside it", output.display()));
    }
    let kept = existing_front_matter(output)?;
    let files = bundle(root, index, hidden, scope, options.log_days, kept.as_deref());
    let mut done = Exported::default();
    for (path, content) in &files {
        let file = output.join(path);
        let bytes = match content {
            Content::Text(text) => text.clone().into_bytes(),
            Content::Asset(from) => fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?,
        };
        // A file whose bytes did not change is not written again.
        if fs::read(&file).is_ok_and(|old| old == bytes) {
            done.unchanged += 1;
            continue;
        }
        done.written.push(path.clone());
        if options.dry_run {
            continue;
        }
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    // What an earlier run wrote and this one does not: gone, but for the dot paths.
    for old in walkdir::WalkDir::new(output)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.'))
        .flatten()
        .filter(|entry| entry.file_type().is_file())
    {
        let path = old.path().strip_prefix(output).unwrap_or_else(|_| old.path()).to_string_lossy().replace('\\', "/");
        if files.contains_key(&path) {
            continue;
        }
        done.removed.push(path);
        if !options.dry_run {
            fs::remove_file(old.path()).map_err(|e| format!("{}: {e}", old.path().display()))?;
        }
    }
    if !options.dry_run {
        remove_empty_folders(output);
    }
    done.removed.sort();
    Ok(done)
}

/// The front matter of the root `index.md` of the bundle already in
/// [output], to keep. An output that holds files and is no export of this
/// command is refused.
fn existing_front_matter(output: &Path) -> Result<Option<String>, String> {
    let Ok(entries) = fs::read_dir(output) else { return Ok(None) };
    let holds_files = entries.flatten().any(|entry| !entry.file_name().to_string_lossy().starts_with('.'));
    let index = fs::read_to_string(output.join("index.md")).unwrap_or_default();
    let front = front_matter(&index).map(|front| front.yaml.to_string());
    let ours = front.as_deref().is_some_and(|yaml| Yaml::parse(yaml).get(MARK).and_then(YamlNode::string) == Some("true"));
    if holds_files && !ours {
        return Err(format!(
            "{} holds files and is not a bundle this command wrote (its index.md has no `{MARK}: true`): nothing is written",
            output.display()
        ));
    }
    Ok(front.filter(|_| ours))
}

fn remove_empty_folders(output: &Path) {
    let folders: Vec<PathBuf> = walkdir::WalkDir::new(output)
        .contents_first(true)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.'))
        .flatten()
        .filter(|entry| entry.file_type().is_dir() && entry.depth() > 0)
        .map(walkdir::DirEntry::into_path)
        .collect();
    for folder in folders {
        // Refused while it holds anything, which is what is wanted.
        let _ = fs::remove_dir(folder);
    }
}

/// A note of the vault as the bundle holds it.
struct Note {
    /// Its path in the bundle: its own, but for `index.md` and `log.md`.
    to: String,
    title: String,
    description: Option<String>,
}

/// Every file of the bundle, by its path in it.
fn bundle(
    root: &Path,
    index: &VaultIndex,
    hidden: &ExcludedFiles,
    scope: &Scope,
    log_days: i64,
    kept: Option<&str>,
) -> BTreeMap<String, Content> {
    let config = crate::config(root);
    let inbox = Inbox::path(config.string("scratchpad.note").as_deref());
    let scratch = config.string("scratchpad.folder").unwrap_or_else(|| "Scratchpad".into());
    let templates: BTreeSet<String> = Templates::read(root).templates(index.files.iter()).into_iter().collect();
    let in_scope = |path: &str| {
        Vault::is_note(path)
            && !hidden.contains(path)
            && !path.split('/').any(|part| part.starts_with('.'))
            && !CalendarWeek::is_week_note(path)
            && path != inbox
            && !path.starts_with(&format!("{}/", scratch.trim_matches('/')))
            && !templates.contains(path)
            && scope.holds(path)
    };
    let texts: BTreeMap<&String, String> = index
        .notes
        .keys()
        .filter(|path| in_scope(path))
        .filter_map(|path| Some((path, fs::read_to_string(root.join(path)).ok()?)))
        .collect();
    let notes: BTreeMap<&str, Note> = texts
        .iter()
        .map(|(path, text)| {
            let concept = front_matter(text).and_then(|front| Okf::concept(front.yaml));
            let note = Note {
                to: renamed(path),
                title: concept.as_ref().and_then(|c| c.title.clone()).unwrap_or_else(|| Vault::title(path)),
                description: front_matter(text)
                    .and_then(|front| Yaml::parse(front.yaml).get("description").and_then(YamlNode::string).map(str::to_string))
                    .filter(|text| !text.trim().is_empty()),
            };
            (path.as_str(), note)
        })
        .collect();
    let mut files: BTreeMap<String, Content> = BTreeMap::new();
    let mut assets: BTreeSet<String> = BTreeSet::new();
    for (path, text) in &texts {
        let body = Body { index, notes: &notes, source: path, assets: &mut assets }.note(text, scope.kind(path));
        files.insert(notes[path.as_str()].to.clone(), Content::Text(body));
    }
    for asset in assets {
        if !files.contains_key(&asset) {
            files.insert(asset.clone(), Content::Asset(root.join(&asset)));
        }
    }
    for (folder, text) in indexes(&notes, scope, root, kept) {
        files.insert(if folder.is_empty() { "index.md".into() } else { format!("{folder}/index.md") }, Content::Text(text));
    }
    if let Some(log) = log(root, &notes, log_days) {
        files.insert("log.md".into(), Content::Text(log));
    }
    files
}

/// A note named `index.md` or `log.md` is written as `index-note.md` or
/// `log-note.md`: the bundle keeps those two names for itself.
fn renamed(path: &str) -> String {
    let (folder, name) = path.rsplit_once('/').map_or(("", path), |(folder, name)| (folder, name));
    let name = match name {
        "index.md" => "index-note.md",
        "log.md" => "log-note.md",
        other => other,
    };
    if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") }
}

fn encoded(path: &str) -> String {
    utf8_percent_encode(path, ENCODED).to_string()
}

/// The body of one note, turned into what OKF allows.
struct Body<'a> {
    index: &'a VaultIndex,
    notes: &'a BTreeMap<&'a str, Note>,
    source: &'a str,
    /// The files of the vault the bundle's notes show or link to.
    assets: &'a mut BTreeSet<String>,
}

impl Body<'_> {
    /// [text] as the bundle has it: its front matter as it is, with a
    /// `type` of [kind] when it has none, and its body in CommonMark.
    fn note(&mut self, text: &str, kind: &str) -> String {
        let (head, body) = match front_matter(text) {
            Some(front) => {
                let open = text.split_inclusive('\n').next().unwrap_or_default();
                let rest = &text[open.len() + front.yaml.len()..];
                let fence = rest.split_inclusive('\n').next().unwrap_or_default();
                let after = &rest[fence.len()..];
                let typed = Okf::concept(front.yaml).is_some();
                let eol = if open.ends_with("\r\n") { "\r\n" } else { "\n" };
                let added = if typed { String::new() } else { format!("type: {kind}{eol}") };
                (format!("{open}{added}{}{fence}", front.yaml), after)
            }
            None => (format!("---\ntype: {kind}\n---\n{}", if text.starts_with('\n') || text.is_empty() { "" } else { "\n" }), text),
        };
        let body = self.body(body);
        self.gather(&body);
        format!("{head}{body}")
    }

    fn body(&mut self, body: &str) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut fence: Option<String> = None;
        let mut comment = false;
        for raw in body.split('\n') {
            let (line, eol) = raw.strip_suffix('\r').map_or((raw, ""), |line| (line, "\r"));
            if let Some(open) = &fence {
                if FENCE.captures(line).is_some_and(|c| c[1].starts_with(open.as_str())) {
                    fence = None;
                }
                out.push(raw.to_string());
                continue;
            }
            if let Some(caps) = FENCE.captures(line) {
                fence = Some(caps[1].to_string());
                out.push(raw.to_string());
                continue;
            }
            // A comment over several lines: all of it goes.
            let mut line = line.to_string();
            if comment {
                match line.find("%%") {
                    Some(end) => {
                        line = line[end + 2..].to_string();
                        comment = false;
                    }
                    None => continue,
                }
            }
            line = self.outside_code(&line, |text| COMMENT.replace_all(text, "").into_owned());
            if let Some(start) = line.find("%%") {
                line.truncate(start);
                comment = true;
                if line.trim().is_empty() {
                    continue;
                }
            }
            let line = self.outside_code(&line, |text| self.inline(text));
            let line = match CALLOUT.captures(&line) {
                Some(caps) => {
                    let title = if caps[3].trim().is_empty() { capitalised(caps[2].trim()) } else { caps[3].trim().to_string() };
                    format!("{}**{title}**", &caps[1])
                }
                None => line,
            };
            out.push(format!("{}{eol}", BLOCK_ID.replace(&line, "")));
        }
        out.join("\n")
    }

    /// [line] with [change] made to what is not in a code span.
    fn outside_code(&self, line: &str, change: impl Fn(&str) -> String) -> String {
        let mut out = String::new();
        let mut at = 0;
        for code in INLINE_CODE.find_iter(line) {
            out.push_str(&change(&line[at..code.start()]));
            out.push_str(code.as_str());
            at = code.end();
        }
        out.push_str(&change(&line[at..]));
        out
    }

    fn inline(&self, text: &str) -> String {
        let text = WIKILINK.replace_all(text, |caps: &Captures| self.wikilink(!caps[1].is_empty(), &caps[2]));
        let text = MARKDOWN_LINK.replace_all(&text, |caps: &Captures| self.markdown_link(caps));
        HIGHLIGHT.replace_all(&text, |caps: &Captures| format!("**{}**", caps[1].trim_start_matches(COLOURS).trim())).into_owned()
    }

    /// `[[Note#Heading|text]]` as `[text](/path/to/Note.md#heading)`; a
    /// picture as `![](/uploads/picture.png)`; a note that is not exported,
    /// or a name that is none, as its text alone.
    fn wikilink(&self, embed: bool, inner: &str) -> String {
        let (target, alias) = match inner.split_once("\\|").or_else(|| inner.split_once('|')) {
            Some((target, alias)) => (target, Some(alias.trim())),
            None => (inner, None),
        };
        let (name, anchor) = match target.split_once('#') {
            Some((name, anchor)) => (name.trim(), Some(anchor.trim())),
            None => (target.trim(), None),
        };
        let shown = alias.filter(|alias| !alias.is_empty()).map_or_else(
            || {
                let title = name.rsplit('/').next().unwrap_or(name);
                match anchor.filter(|a| !a.is_empty()) {
                    Some(anchor) if title.is_empty() => anchor.to_string(),
                    Some(anchor) => format!("{title} > {anchor}"),
                    None => title.to_string(),
                }
            },
            str::to_string,
        );
        let Some(found) = self.index.resolve(name, self.source) else { return shown };
        let fragment = anchor.filter(|a| !a.is_empty()).map_or_default(|a| format!("#{}", encoded(&crate::check::slug(a))));
        match self.notes.get(found.as_str()) {
            Some(note) => format!("[{shown}](/{}{fragment})", encoded(&note.to)),
            None if Vault::is_note(&found) => shown,
            None => self.asset(&found, embed, if embed { "" } else { &shown }),
        }
    }

    fn markdown_link(&self, caps: &Captures) -> String {
        let (embed, text, url) = (!caps[1].is_empty(), &caps[2], &caps[3]);
        if SCHEME.is_match(url) || url.starts_with('#') {
            return caps[0].to_string();
        }
        let decoded = percent_decode_str(url).decode_utf8().map_or_else(|_| url.to_string(), std::borrow::Cow::into_owned);
        let (target, anchor) = decoded.split_once('#').map_or((decoded.as_str(), ""), |(target, anchor)| (target, anchor));
        let Some(found) = self.index.resolve(target, self.source) else { return if embed { String::new() } else { text.to_string() } };
        let fragment = if anchor.is_empty() { String::new() } else { format!("#{}", encoded(anchor)) };
        match self.notes.get(found.as_str()) {
            Some(note) => format!("[{text}](/{}{fragment})", encoded(&note.to)),
            None if Vault::is_note(&found) => text.to_string(),
            None => self.asset(&found, embed, text),
        }
    }

    /// A file of the vault that is no note: copied, and shown or linked.
    fn asset(&self, path: &str, embed: bool, text: &str) -> String {
        // `gather` copies it, from the link written here.
        format!("{}[{text}](/{})", if embed { "!" } else { "" }, encoded(path))
    }

    /// The assets [body] links to with a `/` path: files of the vault that
    /// are no notes of the bundle.
    fn gather(&mut self, body: &str) {
        for caps in MARKDOWN_LINK.captures_iter(body) {
            let Some(path) = caps[3].strip_prefix('/') else { continue };
            let path = percent_decode_str(path.split('#').next().unwrap_or(path)).decode_utf8_lossy().into_owned();
            if self.index.files.contains(&path) && !Vault::is_note(&path) {
                self.assets.insert(path);
            }
        }
    }
}

fn capitalised(word: &str) -> String {
    let mut letters = word.chars();
    letters.next().map_or_default(|first| first.to_uppercase().chain(letters).collect())
}

/// The `index.md` of every folder with a note in it or under it: the
/// notes by name, the folder's own first, then the folders.
fn indexes(notes: &BTreeMap<&str, Note>, scope: &Scope, root: &Path, kept: Option<&str>) -> BTreeMap<String, String> {
    let mut folders: BTreeMap<String, (Vec<&Note>, BTreeSet<String>)> = BTreeMap::new();
    folders.entry(String::new()).or_default();
    for note in notes.values() {
        let folder = note.to.rsplit_once('/').map_or("", |(folder, _)| folder).to_string();
        folders.entry(folder.clone()).or_default().0.push(note);
        // Each folder above it lists the one below.
        let mut below = folder;
        while let Some((parent, name)) = below
            .rsplit_once('/')
            .map(|(p, n)| (p.to_string(), n.to_string()))
            .or_else(|| (!below.is_empty()).then(|| (String::new(), below.clone())))
        {
            folders.entry(parent.clone()).or_default().1.insert(name);
            below = parent;
        }
    }
    folders
        .into_iter()
        .map(|(folder, (mut listed, subfolders))| {
            let own = format!("{}.md", folder.rsplit('/').next().unwrap_or_default());
            let name = |note: &Note| note.to.rsplit('/').next().unwrap_or_default().to_string();
            listed.sort_by_key(|note| (name(note) != own, note.title.to_lowercase(), name(note)));
            let mut text = if folder.is_empty() { root_front_matter(scope, root, kept) } else { String::new() };
            if !listed.is_empty() {
                text.push_str("# Notes\n\n");
                for note in listed {
                    let description = note.description.as_ref().map_or_default(|d| format!(" - {}", d.trim()));
                    text.push_str(&format!("* [{}]({}){description}\n", note.title, encoded(&name(note))));
                }
            }
            if !subfolders.is_empty() {
                if !text.is_empty() && !text.ends_with("---\n") {
                    text.push('\n');
                }
                text.push_str("# Folders\n\n");
                for name in subfolders {
                    text.push_str(&format!("* [{name}]({}/)\n", encoded(&name)));
                }
            }
            (folder, text)
        })
        .collect()
}

/// The root `index.md`'s front matter: the one an earlier export has, kept
/// with only `okf_version` set again, else the bundle's own words.
fn root_front_matter(scope: &Scope, root: &Path, kept: Option<&str>) -> String {
    let quoted = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let version = format!("okf_version: {}", quoted(Okf::VERSION));
    if let Some(kept) = kept {
        let mut seen = false;
        let mut lines: Vec<String> = kept
            .lines()
            .map(|line| {
                if Yaml::split_key(line).is_some_and(|(key, _)| key == "okf_version") && !line.starts_with([' ', '\t']) {
                    seen = true;
                    version.clone()
                } else {
                    line.to_string()
                }
            })
            .collect();
        if !seen {
            lines.insert(0, version);
        }
        return format!("---\n{}\n---\n", lines.join("\n"));
    }
    let title = if scope.title.trim().is_empty() {
        root.canonicalize().ok().and_then(|real| real.file_name().map(|name| name.to_string_lossy().into_owned())).unwrap_or_default()
    } else {
        scope.title.trim().to_string()
    };
    let mut lines = vec![version, format!("title: {}", quoted(&title))];
    for (key, value) in [("description", &scope.description), ("license", &scope.license)] {
        if !value.trim().is_empty() {
            lines.push(format!("{key}: {}", quoted(value.trim())));
        }
    }
    lines.push(format!("{MARK}: true"));
    format!("---\n{}\n---\n", lines.join("\n"))
}

/// `log.md` from git: a heading for each day with changes, newest first,
/// and under it each exported note the day's commits added or changed. No
/// repository is no log.
fn log(root: &Path, notes: &BTreeMap<&str, Note>, days: i64) -> Option<String> {
    let git = Git::new(root);
    if !git.is_repository() {
        return None;
    }
    let since = format!("--since={days} days ago");
    let format = "--format=%x1e%ad";
    let out = git
        .run_raw(&[
            "-c",
            "core.quotePath=false",
            "log",
            &since,
            "--date=short",
            format,
            "--name-status",
            "--no-renames",
            "--relative",
            "--",
            ".",
        ])
        .ok()?;
    // By day, each note once: added on that day wins over changed.
    let mut by_day: BTreeMap<String, BTreeMap<&str, bool>> = BTreeMap::new();
    for record in out.split('\u{1e}') {
        let mut lines = record.lines().filter(|line| !line.is_empty());
        let Some(day) = lines.next() else { continue };
        for line in lines {
            let Some((status, path)) = line.split_once('\t') else { continue };
            let Some((path, _)) = notes.get_key_value(path) else { continue };
            if status.starts_with('D') {
                continue;
            }
            let created = by_day.entry(day.trim().to_string()).or_default().entry(path).or_default();
            *created |= status.starts_with('A');
        }
    }
    let mut text = String::from("# Update Log\n");
    for (day, changed) in by_day.iter().rev() {
        text.push_str(&format!("\n## {day}\n\n"));
        for (path, created) in changed {
            let note = &notes[path];
            text.push_str(&format!("* **{}**: [{}](/{})\n", if *created { "Creation" } else { "Update" }, note.title, encoded(&note.to)));
        }
    }
    Some(text)
}

/// What `okf check` found in a folder.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// What makes the folder no conformant bundle (§11.1).
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub files: usize,
}

/// Whether [folder] is a conformant OKF bundle: every front matter parses
/// as a mapping, and `index.md` and `log.md` have their shape; then the
/// warnings of the audit on its concepts.
pub fn check(folder: &Path) -> Result<Checked, String> {
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let mut found = Checked::default();
    let now = chrono::Utc::now();
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.'))
        .flatten()
        .filter(|entry| entry.file_type().is_file() && entry.path().extension().is_some_and(|e| e == "md"))
        .map(walkdir::DirEntry::into_path)
        .collect();
    files.sort();
    for file in files {
        let path = file.strip_prefix(folder).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        let Ok(text) = fs::read_to_string(&file) else {
            found.errors.push(format!("{path}\terror\tokf-read\tthe file is not text"));
            continue;
        };
        found.files += 1;
        let opens = text.lines().next().is_some_and(|line| line.trim() == "---");
        let front = front_matter(&text);
        let name = path.rsplit('/').next().unwrap_or(&path);
        if opens && front.is_none() {
            found.errors.push(format!("{path}:1\terror\tokf-front-matter\tthe front matter is not closed"));
            continue;
        }
        if let Some(front) = &front {
            if !is_mapping(front.yaml) {
                found.errors.push(format!("{path}:1\terror\tokf-front-matter\tthe front matter is not a YAML mapping"));
                continue;
            }
            if name == "log.md" || (name == "index.md" && path != "index.md") {
                found.errors.push(format!("{path}:1\terror\tokf-reserved\t{name} has no front matter here; only the root index.md may"));
            }
        }
        let body = front.as_ref().map_or(text.as_str(), |front| {
            let from = text.find(front.yaml).map_or(0, |at| at + front.yaml.len());
            text[from..].split_once('\n').map_or("", |(_, body)| body)
        });
        match name {
            "index.md" => {
                for (at, line) in body.lines().enumerate().filter(|(_, line)| !line.trim().is_empty()) {
                    if !line.starts_with('#') && !INDEX_ITEM.is_match(line) {
                        found.errors.push(format!(
                            "{path}:{}\terror\tokf-index\tan index holds headings and `* [Title](url) - description` entries",
                            at + 1
                        ));
                        break;
                    }
                }
            }
            "log.md" => {
                for (at, line) in body.lines().enumerate().filter(|(_, line)| line.starts_with("## ")) {
                    if !LOG_DAY.is_match(line) {
                        found.errors.push(format!("{path}:{}\terror\tokf-log\ta log's date heading is `## YYYY-MM-DD`", at + 1));
                    }
                }
            }
            _ => {
                let Some(concept) = Okf::concept_of(&path, &text) else { continue };
                if let Some(Stale::Since(at)) = concept.stale(now) {
                    found.warnings.push(format!("{path}:1\twarning\tokf-stale\tthe concept is stale since {}", at.format("%Y-%m-%d")));
                }
                for (key, raw) in concept.malformed_times() {
                    found.warnings.push(format!("{path}:1\twarning\tokf-time\t{key} is not a time: {raw}"));
                }
                if let Status::Other(raw) = &concept.status {
                    found.warnings.push(format!("{path}:1\twarning\tokf-status\tstatus is none of draft, stable and deprecated: {raw}"));
                }
                if concept.incomplete > 0 || concept.generated.as_ref().is_some_and(|generated| generated.by.is_none()) {
                    found.warnings.push(format!("{path}:1\twarning\tokf-event\tan event lacks by or at"));
                }
            }
        }
    }
    Ok(found)
}

/// Whether [yaml] is a mapping: every line at the left edge that is not
/// blank or a comment is a `key: value`.
fn is_mapping(yaml: &str) -> bool {
    yaml.lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#') && !line.starts_with([' ', '\t']))
        .all(|line| !line.starts_with("- ") && line != "-" && Yaml::split_key(line).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let base = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let file = base.path().join("vault").join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        base
    }

    fn files_of(folder: &Path) -> BTreeMap<String, Vec<u8>> {
        walkdir::WalkDir::new(folder)
            .into_iter()
            .flatten()
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| (entry.path().strip_prefix(folder).unwrap().to_string_lossy().replace('\\', "/"), fs::read(entry.path()).unwrap()))
            .collect()
    }

    fn run(root: &Path, output: &Path, dry_run: bool) -> Result<Exported, String> {
        let index = Vault::index(root, None);
        let hidden = ExcludedFiles::from_config(root, &ConfigFile::new(""), false);
        export(root, &index, &hidden, &Scope::read(root), &Options { output: output.to_path_buf(), log_days: 90, dry_run })
    }

    const A: &str = "# A\n\nSee [[B|the other]], [[B#Second part]] and [[Secret]], and [[Nowhere]].\n\n![[p.png]]\n\n> [!warning] Mind\n>\n> The step.\n\n> [!tip]\n>\n> A tip.\n\nIt is ==🔴hot== and ==plain==. %%gone%% Kept. ^block-1\n\n%%\ngone too\n%%\n\n```md\n[[B]] ==stays== %%stays%%\n```\n\nCode `[[B]]` and $x = 1$ and #tag.\n\n- [ ] A task 📅 2026-10-05\n";
    const B: &str = "---\ntype: Guide\ndescription: Second.\ntags: [a,  b]\n---\n# B\n\n## Second part\n\nBack to [A](A.md) and [web](https://example.com).\n";
    const CONFIG: &str = "[okf]\ninclude = [\"Learning\"]\ndescription = \"A test bundle.\"\n\n[okf.types]\n\"Learning\" = \"Reference\"\n\"Learning/Sub\" = \"Deep\"\n";

    fn fixture() -> tempfile::TempDir {
        vault(&[
            ("Learning/A.md", A),
            ("Learning/B.md", B),
            ("Learning/Sub/Sub.md", "# Sub\n"),
            ("Learning/index.md", "A note that is named as an index.\n"),
            ("Identity/Secret.md", "# Secret\n\nA number.\n"),
            ("CLAUDE.md", "# Rules\n"),
            ("Learning/CLAUDE.md", "# Rules\n"),
            ("uploads/p.png", "not a picture"),
            ("uploads/unused.png", "nor this"),
            (".daftar.toml", CONFIG),
        ])
    }

    #[test]
    fn an_export_is_a_conformant_bundle() {
        let base = fixture();
        let (root, output) = (base.path().join("vault"), base.path().join("bundle"));
        let before = files_of(&root);
        let done = run(&root, &output, false).unwrap();
        let bundle = files_of(&output);
        let text = |path: &str| String::from_utf8(bundle[path].clone()).unwrap();
        assert_eq!(
            bundle.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "Learning/A.md",
                "Learning/B.md",
                "Learning/Sub/Sub.md",
                "Learning/Sub/index.md",
                "Learning/index-note.md",
                "Learning/index.md",
                "index.md",
                "uploads/p.png"
            ]
        );
        assert_eq!(done.written.len(), 8);
        // A note with no front matter gets a block with its type alone, by its folder.
        let a = text("Learning/A.md");
        assert!(a.starts_with("---\ntype: Reference\n---\n\n# A\n"), "{a}");
        assert!(text("Learning/Sub/Sub.md").starts_with("---\ntype: Deep\n---\n"));
        assert!(
            a.contains("See [the other](/Learning/B.md), [B > Second part](/Learning/B.md#second-part) and Secret, and Nowhere."),
            "{a}"
        );
        assert!(a.contains("![](/uploads/p.png)"));
        assert!(a.contains("> **Mind**\n>\n> The step.") && a.contains("> **Tip**\n>\n> A tip."));
        assert!(a.contains("It is **hot** and **plain**.  Kept.\n"), "{a}");
        assert!(!a.contains("gone") && !a.contains("^block-1"));
        // Code, math, tags and tasks stay as they are.
        assert!(a.contains("```md\n[[B]] ==stays== %%stays%%\n```") && a.contains("Code `[[B]]` and $x = 1$ and #tag."));
        assert!(a.contains("- [ ] A task 📅 2026-10-05"));
        // A note's own front matter is byte for byte; its links become `/` paths.
        let b = text("Learning/B.md");
        assert!(b.starts_with("---\ntype: Guide\ndescription: Second.\ntags: [a,  b]\n---\n"));
        assert!(b.contains("Back to [A](/Learning/A.md) and [web](https://example.com)."));
        assert_eq!(bundle["uploads/p.png"], b"not a picture");
        // The indexes: the notes by name with their descriptions, then the folders.
        assert_eq!(
            text("Learning/index.md"),
            "# Notes\n\n* [A](A.md)\n* [B](B.md) - Second.\n* [index](index-note.md)\n\n# Folders\n\n* [Sub](Sub/)\n"
        );
        assert_eq!(text("Learning/Sub/index.md"), "# Notes\n\n* [Sub](Sub.md)\n");
        assert_eq!(
            text("index.md"),
            "---\nokf_version: \"0.3\"\ntitle: \"vault\"\ndescription: \"A test bundle.\"\nx-daftar-export: true\n---\n# Folders\n\n* [Learning](Learning/)\n"
        );
        // No Identity, no CLAUDE.md, no picture nobody shows, no log with no repository.
        assert!(
            bundle
                .keys()
                .all(|path| !path.contains("Identity") && !path.contains("CLAUDE") && !path.contains("unused") && path != "log.md")
        );
        // The output is a bundle, and the vault is as it was.
        let checked = check(&output).unwrap();
        assert_eq!((checked.errors, checked.warnings, checked.files), (vec![], vec![], 7));
        assert_eq!(files_of(&root), before);
    }

    #[test]
    fn an_export_only_writes_where_it_may() {
        let base = fixture();
        let (root, output) = (base.path().join("vault"), base.path().join("bundle"));
        // An output inside the vault is refused.
        assert!(run(&root, &root.join("export"), false).unwrap_err().contains("is in the vault"));
        assert!(run(&root, &root.join("Learning/out/deep"), false).unwrap_err().contains("is in the vault"));
        assert!(!root.join("export").exists());
        // A folder with files that is not an export is refused and untouched.
        let other = base.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("mine.txt"), "mine").unwrap();
        fs::write(other.join("index.md"), "---\ntitle: Mine\n---\n").unwrap();
        let untouched = files_of(&other);
        assert!(run(&root, &other, false).unwrap_err().contains("is not a bundle this command wrote"));
        assert_eq!(files_of(&other), untouched);
        // A dry run lists and writes nothing.
        let planned = run(&root, &output, true).unwrap();
        assert_eq!(planned.written.len(), 8);
        assert!(!output.exists());
        run(&root, &output, false).unwrap();
        // The person's own words in the root index, and a repository in the output, are kept.
        let index = fs::read_to_string(output.join("index.md")).unwrap();
        fs::write(output.join("index.md"), index.replace("title: \"vault\"", "title: \"Mine\"\nowner: me").replace("\"0.3\"", "\"0.2\""))
            .unwrap();
        fs::create_dir_all(output.join(".git")).unwrap();
        fs::write(output.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(output.join("stray.md"), "left by hand\n").unwrap();
        // A second export after `B.md` is removed from the vault removes it from the bundle.
        fs::remove_file(root.join("Learning/B.md")).unwrap();
        let again = run(&root, &output, false).unwrap();
        assert_eq!(again.removed, ["Learning/B.md", "stray.md"]);
        assert_eq!(again.written, ["Learning/A.md", "Learning/index.md", "index.md"]);
        assert!(!output.join("Learning/B.md").exists() && output.join(".git/HEAD").exists());
        let index = fs::read_to_string(output.join("index.md")).unwrap();
        assert!(
            index.starts_with(
                "---\nokf_version: \"0.3\"\ntitle: \"Mine\"\nowner: me\ndescription: \"A test bundle.\"\nx-daftar-export: true\n---\n"
            ),
            "{index}"
        );
        // A link to the note that is gone is its text alone now.
        assert!(fs::read_to_string(output.join("Learning/A.md")).unwrap().contains("See the other, B > Second part and Secret"));
        // A second export with nothing changed writes no file: the change times stay.
        let times = |folder: &Path| -> BTreeMap<String, std::time::SystemTime> {
            files_of(folder).keys().map(|path| (path.clone(), fs::metadata(folder.join(path)).unwrap().modified().unwrap())).collect()
        };
        let before = times(&output);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let third = run(&root, &output, false).unwrap();
        assert_eq!((third.written.len(), third.removed.len(), third.unchanged), (0, 0, 7));
        assert_eq!(times(&output), before);
    }

    #[test]
    fn the_log_is_the_days_of_git() {
        let base = fixture();
        let (root, output) = (base.path().join("vault"), base.path().join("bundle"));
        let git = Git::new(&root);
        for arguments in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"],
            &["config", "user.name", "Test"],
            &["config", "commit.gpgsign", "false"],
            &["add", "-A"],
            &["commit", "-qm", "start"],
        ] {
            git.run(arguments).unwrap();
        }
        fs::write(root.join("Learning/B.md"), B.replace("Second.", "Second, changed.")).unwrap();
        git.run(&["commit", "-qam", "change"]).unwrap();
        // A read-only clone is exported all the same: nothing is written in it.
        git.set_read_only(true).unwrap();
        run(&root, &output, false).unwrap();
        let log = fs::read_to_string(output.join("log.md")).unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d");
        assert_eq!(
            log,
            format!(
                "# Update Log\n\n## {today}\n\n* **Creation**: [A](/Learning/A.md)\n* **Creation**: [B](/Learning/B.md)\n* **Creation**: [Sub](/Learning/Sub/Sub.md)\n* **Creation**: [index](/Learning/index-note.md)\n"
            )
        );
        assert!(check(&output).unwrap().errors.is_empty());
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), "");
    }

    #[test]
    fn a_folder_that_breaks_the_format_is_named() {
        let base = vault(&[
            ("index.md", "---\nokf_version: \"0.3\"\n---\n# Notes\n\n* [A](a.md) - One.\n"),
            ("a.md", "---\ntype: Metric\nstale_after: 2020-01-01\nstatus: archived\n---\nText\n"),
            ("list.md", "---\n- one\n- two\n---\nText\n"),
            ("open.md", "---\ntype: Metric\nText with no end\n"),
            ("sub/index.md", "---\ntitle: No\n---\nProse that is no entry.\n"),
            ("log.md", "# Log\n\n## 22 May\n* **Update**: x\n"),
            ("plain.md", "No front matter is no concept, and is fine.\n"),
        ]);
        let found = check(&base.path().join("vault")).unwrap();
        let codes = |lines: &[String]| lines.iter().map(|line| line.split('\t').collect::<Vec<_>>()[..3].join(" ")).collect::<Vec<_>>();
        assert_eq!(
            codes(&found.errors),
            [
                "list.md:1 error okf-front-matter",
                "log.md:3 error okf-log",
                "open.md:1 error okf-front-matter",
                "sub/index.md:1 error okf-reserved",
                "sub/index.md:1 error okf-index",
            ]
        );
        assert_eq!(codes(&found.warnings), ["a.md:1 warning okf-stale", "a.md:1 warning okf-status"]);
        assert!(check(&base.path().join("nowhere")).is_err());
    }
}
