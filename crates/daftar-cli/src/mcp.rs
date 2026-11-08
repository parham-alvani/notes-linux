//! `daftar-cli mcp`: the vault as a Model Context Protocol server on stdin
//! and stdout, for Claude Code and other agents. Each message is one line of
//! JSON-RPC 2.0. The tools are the command line's: they read the vault
//! again at each call, and they write only through `VaultEdits` and
//! `QuickCapture`, as the app does, so a task line that moved since it was
//! read is refused rather than changed.

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use daftar_core::properties::{PropertyEdit, PropertySheet, PropertyValue};
use daftar_core::{Card, Drill, DrillState, RandomNote, Rating};

use daftar_core::{
    Bookmarks, Clock, ConfigFile, Day, ExcludedFiles, ParsedTask, QueryTask, QuickCapture, SearchIndex, Sections, TaskLine, TaskStatuses,
    TaskType, TasksQuery, Vault, VaultEdits, VaultIndex, VaultMoves, VaultTask,
};

use crate::{check, vault_commands};

/// The protocol version answered when the client asks for none we know.
const PROTOCOL: &str = "2025-06-18";
const KNOWN: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

const INSTRUCTIONS: &str = "Daftar's tools on the person's notes: an Obsidian vault in a Git repository, \
which holds their work, companies, tasks, plans, journal and life. Read before you answer from memory: \
`search` and `find_notes` find notes, `read_note` reads one (or one section), `note_info` gives its \
properties, outline, links and backlinks, `tasks` and `query_tasks` list tasks (Tasks plugin syntax). \
Before writing, read the vault's rules: `read_note` on `CLAUDE.md` (and the CLAUDE.md of the folder you \
write in). A task is changed by its note, line and text as `tasks` printed them; a line that moved is \
refused, so list the tasks again. `note_context` reads a note with everything that holds its state \
elsewhere (tasks and lines that link to it, its last commits) in one call. To change what a note says, \
`set_property` sets its front matter, `replace_in_note` swaps one exact passage and `replace_section` \
rewrites the text under a heading; each reads the file again and refuses when it changed. `commit` \
commits the files named with a message, then pulls with a rebase and pushes, as the vault's workflow does.";

/// What the server keeps between calls: the full-text index, which takes a
/// second to build on a large vault, and the notes it was built from.
struct Server {
    root: PathBuf,
    show_hidden: bool,
    read_only: bool,
    search: Option<(SearchIndex, VaultIndex)>,
    /// The drill's state file, in place of Daftar's: for the tests.
    drill_state: Option<PathBuf>,
}

/// Serves until stdin ends.
pub fn run(root: PathBuf, show_hidden: bool, read_only: bool) -> Result<(), String> {
    TaskStatuses::set_current(TaskStatuses::load(&root));
    let mut server = Server { root, show_hidden, read_only, search: None, drill_state: None };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(answer) = server.handle(&line) else { continue };
        writeln!(stdout, "{answer}").and_then(|()| stdout.flush()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl Server {
    /// The answer to one message, or none for a notification.
    fn handle(&mut self, line: &str) -> Option<Value> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => return Some(failure(&Value::Null, -32700, &format!("not JSON: {error}"))),
        };
        let id = message.get("id").cloned()?;
        let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
                let version = if KNOWN.contains(&asked) { asked } else { PROTOCOL };
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "daftar", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools(self.read_only) }),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let (text, error) = match self.call(name, &Arguments(&arguments)) {
                    Ok(text) => (text, false),
                    Err(text) => (text, true),
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": error })
            }
            _ => return Some(failure(&id, -32601, &format!("no method {method}"))),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn call(&mut self, name: &str, args: &Arguments) -> Result<String, String> {
        let root = self.root.clone();
        let root = root.as_path();
        if self.read_only && WRITES.contains(&name) {
            return Err(format!("{name} writes, and this server is read-only"));
        }
        let config = ConfigFile::new(fs::read_to_string(ConfigFile::default_path()).unwrap_or_default());
        let hidden = ExcludedFiles::from_config(root, &config, self.show_hidden);
        let index = Vault::index(root, None);
        let lines = match name {
            "find_notes" => crate::find(&index, &hidden, args.text("query")?, args.count("limit", 20)),
            "search" => self.search_notes(&index, &hidden, args.text("query")?, args.count("limit", 20)),
            "read_note" => return read_note(root, &index, args.text("note")?, args.optional("heading")),
            "note_info" => note_info(&index, &hidden, args.text("note")?)?,
            "list_folder" => list_folder(&index, &hidden, args.optional("folder").unwrap_or_default())?,
            "tags" => tags(&index, &hidden, args.optional("tag")),
            "tasks" => tasks(&index, &hidden, args)?,
            "query_tasks" => query_tasks(&index, &hidden, args.text("query")?, args.optional("note"))?,
            "task_statuses" => TaskStatuses::load(root)
                .all
                .iter()
                .map(|s| format!("[{}]\t{}\t{}\tnext [{}]", s.symbol, s.name, s.kind.type_name(), s.next))
                .collect(),
            "recent_changes" => recent_changes(root, args.count("days", 7))?,
            "note_history" => {
                let path = crate::note_path(&index, args.text("note")?)?;
                let git = crate::repository(root)?;
                git.history(&path, args.count("limit", 30))
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|r| {
                        format!("{}\t{}\t{}\t{}", &r.hash[..12.min(r.hash.len())], r.date.format("%Y-%m-%d %H:%M"), r.author, r.subject)
                    })
                    .collect()
            }
            "daily_note" => {
                let day = args.day("day")?;
                let path = vault_commands::daily(root, &index.files, day, false, Clock::now())?;
                let text = fs::read_to_string(root.join(&path)).unwrap_or_default();
                return Ok(if text.is_empty() { format!("{path}\n\n(not written yet)") } else { format!("{path}\n\n{text}") });
            }
            "job_pipeline" => {
                crate::jobs::pipeline(root, &index, |path| !hidden.contains(path), args.0.get("days").and_then(Value::as_i64))
            }
            "doctor" => {
                let found = crate::repository(root)?.doctor(1_048_576).map_err(|e| e.to_string())?;
                found.iter().map(|d| format!("{}\t{}\t{}", d.level, d.code, d.message)).collect()
            }
            "check_vault" => {
                let vault = check::VaultOptions::read(root, args.optional("profile"))?;
                let memory = args.flag("memory") || vault.memory;
                let options = check::Options {
                    strict: false,
                    no_h1: check::joined(vault.no_h1, args.list("no_h1")),
                    callouts: check::joined(vault.callouts, args.list("callouts")),
                    urls: false,
                    unreachable: false,
                    github_token: None,
                    skip_urls: Vec::new(),
                    archive: false,
                    figures: false,
                    github: false,
                    fix: false,
                    dry_run: false,
                    memory: memory.then(|| crate::memory::Rules {
                        event_properties: check::joined(vault.event_properties, args.list("event_properties")),
                        prep_notes: check::joined(vault.prep_notes, args.list("prep_notes")),
                        active: check::joined(vault.active, args.list("active"))
                            .iter()
                            .filter_map(|a| crate::memory::Rules::active_pair(a))
                            .collect(),
                        quiet_days: args.0.get("quiet_days").and_then(Value::as_i64).or(vault.quiet_days),
                    }),
                };
                let mut found = Vec::new();
                let outcome = check::run(root, |path| hidden.contains(path), &options, |line| found.push(line));
                found.push(format!("{} errors, {} warnings in {} notes", outcome.errors, outcome.warnings, outcome.notes));
                found
            }
            "capture" => {
                let line = QuickCapture::line(args.text("text")?, args.flag("task")).ok_or("nothing to capture")?;
                vec![format!("{}\t{line}", crate::capture(root, &config, &line)?)]
            }
            "append_to_note" => {
                let line = QuickCapture::line(args.text("text")?, args.flag("task")).ok_or("nothing to add")?;
                let note = args.text("note")?;
                let target = crate::note_path(&index, note).unwrap_or_else(|_| note.to_string());
                let heading = args.optional("heading").unwrap_or_default();
                vec![format!("{}\t{line}", QuickCapture::capture(root, &target, heading, &line, Day::today(), Clock::now())?)]
            }
            "create_note" => vec![create_note(root, args.text("path")?, args.text("text")?)?],
            "note_context" => return note_context(root, &index, &hidden, args.text("note")?),
            "move_note" => move_note(root, &index, args.text("note")?, args.text("to")?)?,
            "set_property" => vec![set_property(root, &index, args)?],
            "replace_in_note" => {
                let new = args.0.get("new").and_then(Value::as_str).ok_or("new is needed (empty to delete)")?;
                vec![replace_in_note(root, &index, args.text("note")?, args.text("old")?, new)?]
            }
            "replace_section" => {
                let text = args.0.get("text").and_then(Value::as_str).ok_or("text is needed (empty to clear the section)")?;
                vec![replace_section(root, &index, args.text("note")?, args.text("heading")?, text)?]
            }
            "commit" => {
                commit(root, args.text("message")?, &args.list("paths"), args.0.get("push").and_then(Value::as_bool).is_none_or(|p| p))?
            }
            "toggle_task" => {
                let (path, at, task) = task_at(root, &index, args)?;
                VaultEdits::apply(&path, root, |text| VaultEdits::toggle_task(text, at, task.status, Some(&task.text), Day::today()))
                    .map_err(|e| e.to_string())?;
                let after = fs::read_to_string(root.join(&path)).unwrap_or_default();
                vec![format!("{path}:{}\t{}", at + 1, after.split('\n').nth(at).unwrap_or_default())]
            }
            "set_task_status" => {
                let (path, at, _) = task_at(root, &index, args)?;
                let status = match args.text("status")? {
                    "cancelled" => cancelled_symbol(),
                    symbol => symbol.to_string(),
                };
                vec![vault_commands::set_status(root, &path, at, &status, args.optional("reason"), Day::today())?]
            }
            "edit_task" => {
                let (path, at, _) = task_at(root, &index, args)?;
                for key in ["start", "scheduled", "due"] {
                    if let Some(value) = args.0.get(key).and_then(Value::as_str).map(str::trim)
                        && !value.is_empty()
                        && Day::parse(value).is_none()
                    {
                        return Err(format!("{key} {value} is not a day (YYYY-MM-DD)"));
                    }
                }
                if let Some(value) = args.0.get("priority").and_then(Value::as_str)
                    && priority(value).is_none()
                    && !matches!(value.trim().to_lowercase().as_str(), "" | "none")
                {
                    return Err(format!("{value} is not a priority: highest, high, medium, low, lowest or none"));
                }
                VaultEdits::apply(&path, root, |text| {
                    let raw = text.split('\n').nth(at).unwrap_or_default().trim_end_matches('\r');
                    let before = TaskLine::fields(raw).ok_or(daftar_core::Failure::TaskMoved)?;
                    let mut fields = before.clone();
                    for (key, slot) in [("start", &mut fields.start), ("scheduled", &mut fields.scheduled), ("due", &mut fields.due)] {
                        if let Some(value) = args.0.get(key).and_then(Value::as_str) {
                            *slot = Some(value.trim()).filter(|v| !v.is_empty()).map(String::from);
                        }
                    }
                    if let Some(value) = args.0.get("priority").and_then(Value::as_str) {
                        fields.priority = priority(value);
                    }
                    VaultEdits::edit_task(text, at, &before, &fields)
                })
                .map_err(|e| e.to_string())?;
                let after = fs::read_to_string(root.join(&path)).unwrap_or_default();
                vec![format!("{path}:{}\t{}", at + 1, after.split('\n').nth(at).unwrap_or_default())]
            }
            "postpone_task" => {
                let (path, at, _) = task_at(root, &index, args)?;
                vec![vault_commands::postpone(root, &path, at, args.optional("by").unwrap_or("day"), Day::today())?]
            }
            "random_note" => {
                let candidates = RandomNote::candidates(&index, args.optional("folder"), args.optional("tag"), |p| hidden.contains(p));
                let path = RandomNote::pick(&candidates, RandomNote::number).ok_or("no note to pick")?;
                return read_note(root, &index, &path, None);
            }
            "drill_due" => {
                let deck = Drill::deck(root, &crate::drill::notes(&index, &args.list("notes"), |p| hidden.contains(p))?);
                let state = DrillState::load(&self.drill_file());
                let due = Drill::due(&deck, &state, Day::today(), args.count("new", Drill::NEW_CARDS), args.count("limit", 10));
                due.iter()
                    .map(|card| {
                        let when = state.due(card).map_or_else(|| "new".to_string(), |d| d.to_string());
                        let context = crate::drill::context(card);
                        format!(
                            "id: {}\nat: {}:{}\ndue: {when}\ncontext: {context}\nquestion: {}\n",
                            card.id,
                            card.path,
                            card.line + 1,
                            card.question
                        )
                    })
                    .collect()
            }
            "drill_card" => {
                let card = drill_card(root, &index, args.text("id")?)?;
                vec![format!("{}\n{}:{}\n\n{}", card.id, card.path, card.line + 1, card.answer)]
            }
            "drill_rate" => {
                let card = drill_card(root, &index, args.text("id")?)?;
                let rating = args.text("rating")?;
                let rating = Rating::parse(rating).ok_or_else(|| format!("{rating} is not a rating: again, hard, good or easy (1-4)"))?;
                let file = self.drill_file();
                let mut state = DrillState::load(&file);
                let review = state.rate(&card.id, rating, Day::today());
                state.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
                vec![format!("{}\tnext on {} (in {} days, ease {})", card.id, review.due, review.interval, review.ease)]
            }
            _ => return Err(format!("no tool {name}")),
        };
        Ok(if lines.is_empty() { "(nothing)".to_string() } else { lines.join("\n") })
    }

    fn drill_file(&self) -> PathBuf {
        self.drill_state.clone().unwrap_or_else(|| DrillState::file(&self.root))
    }

    /// The notes whose text matches [query], from an index kept between
    /// calls and brought up to date with the files that changed since.
    fn search_notes(&mut self, index: &VaultIndex, hidden: &ExcludedFiles, query: &str, limit: usize) -> Vec<String> {
        if let Some((search, known)) = &mut self.search {
            let changed = changed_since(known, index);
            if !changed.is_empty() {
                Vault::update_in_place(known, &changed, &self.root, Some(search));
            }
        } else {
            let search = SearchIndex::new();
            let known = Vault::index(&self.root, Some(&search));
            self.search = Some((search, known));
        }
        let Some((search, _)) = &self.search else { return Vec::new() };
        search
            .search(query, limit.saturating_add(100))
            .into_iter()
            .filter(|hit| !hidden.contains(&hit.path))
            .take(limit)
            .map(|hit| format!("{}\t{}", hit.path, crate::snippet(&hit.snippet, false)))
            .collect()
    }
}

/// The tools that write to the vault, which `--read-only` leaves out.
const WRITES: [&str; 13] = [
    "capture",
    "append_to_note",
    "create_note",
    "toggle_task",
    "set_task_status",
    "postpone_task",
    "edit_task",
    "set_property",
    "replace_in_note",
    "replace_section",
    "commit",
    "move_note",
    "drill_rate",
];

/// A priority's emoji from its name or its emoji; `None` for none or a word it does not know.
fn priority(value: &str) -> Option<String> {
    let emoji = match value.trim().to_lowercase().as_str() {
        "highest" | "🔺" => "🔺",
        "high" | "⏫" => "⏫",
        "medium" | "🔼" => "🔼",
        "low" | "🔽" => "🔽",
        "lowest" | "⏬" => "⏬",
        _ => return None,
    };
    Some(emoji.to_string())
}

/// The notes added, gone or changed between [old] and [new].
fn changed_since(old: &VaultIndex, new: &VaultIndex) -> BTreeSet<String> {
    let mut changed: BTreeSet<String> = old.files.symmetric_difference(&new.files).cloned().collect();
    for (path, note) in &new.notes {
        if old.notes.get(path).is_none_or(|known| known.modified != note.modified) {
            changed.insert(path.clone());
        }
    }
    changed
}

/// A note's text, or the section under one of its headings.
fn read_note(root: &Path, index: &VaultIndex, note: &str, heading: Option<&str>) -> Result<String, String> {
    let path = crate::note_path(index, note).or_else(|_| {
        // A file that is not a note, such as a `.base` or a `.canvas`.
        if index.files.contains(note) { Ok(note.to_string()) } else { Err(format!("no note {note}")) }
    })?;
    let text = fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
    let Some(heading) = heading else { return Ok(format!("{path}\n\n{text}")) };
    let headings = Sections::headings(&text);
    let wanted = heading.trim_start_matches('#').trim();
    let at = headings.iter().position(|h| h.text == wanted).ok_or_else(|| {
        let names: Vec<&str> = headings.iter().map(|h| h.text.as_str()).collect();
        format!("{path} has no heading “{wanted}”; it has: {}", names.join(" | "))
    })?;
    let lines: Vec<&str> = text.split('\n').collect();
    let end = Sections::end(at, &headings, lines.len());
    Ok(format!("{path}#{wanted} (lines {}-{end})\n\n{}", headings[at].line + 1, lines[headings[at].line..end].join("\n")))
}

/// What the index knows of a note: properties, tags, outline, links and backlinks.
fn note_info(index: &VaultIndex, hidden: &ExcludedFiles, note: &str) -> Result<Vec<String>, String> {
    let path = crate::note_path(index, note)?;
    let entry = &index.notes[&path];
    let parsed = &entry.parsed;
    let mut lines = vec![format!("path: {path}")];
    let modified: chrono::DateTime<chrono::Local> = entry.modified.into();
    lines.push(format!("file modified: {}", modified.format("%Y-%m-%d %H:%M")));
    if !parsed.aliases.is_empty() {
        lines.push(format!("aliases: {}", parsed.aliases.join(", ")));
    }
    if !parsed.tags.is_empty() {
        lines.push(format!("tags: {}", parsed.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")));
    }
    if !parsed.properties.is_empty() {
        lines.push("properties:".into());
        lines.extend(parsed.properties.iter().map(|(key, values)| format!("  {key}: {}", values.join(", "))));
    }
    if !parsed.headings.is_empty() {
        lines.push("outline:".into());
        lines.extend(parsed.headings.iter().map(|h| format!("  {}{} (line {})", "  ".repeat(h.level - 1), h.text, h.line + 1)));
    }
    let open = parsed.tasks.iter().filter(|t| !TaskLine::is_closed(t.status)).count();
    lines.push(format!("tasks: {} ({open} open)", parsed.tasks.len()));
    let links: BTreeSet<String> = parsed
        .links
        .iter()
        .filter(|l| !l.target.is_empty())
        .map(|l| match index.resolve(&l.target, &path) {
            Some(target) => target,
            None if l.target.contains("://") => l.target.clone(),
            None => format!("{} (no note yet)", l.target),
        })
        .collect();
    if !links.is_empty() {
        lines.push("links to:".into());
        lines.extend(links.into_iter().map(|l| format!("  {l}")));
    }
    let backlinks: BTreeSet<String> =
        index.backlinks(&path).iter().filter(|b| !hidden.contains(&b.source)).map(|b| b.source.clone()).collect();
    if !backlinks.is_empty() {
        lines.push("linked from:".into());
        lines.extend(backlinks.into_iter().map(|b| format!("  {b}")));
    }
    Ok(lines)
}

/// The folders and files right in [folder], a folder with a slash.
fn list_folder(index: &VaultIndex, hidden: &ExcludedFiles, folder: &str) -> Result<Vec<String>, String> {
    let folder = folder.trim_matches('/');
    let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
    let mut folders = BTreeSet::new();
    let mut files = Vec::new();
    for path in index.files.iter().filter(|p| p.starts_with(&prefix) && !hidden.contains(p)) {
        match path[prefix.len()..].split_once('/') {
            Some((sub, _)) => {
                folders.insert(format!("{prefix}{sub}/"));
            }
            None => files.push(path.clone()),
        }
    }
    if folders.is_empty() && files.is_empty() && !folder.is_empty() {
        return Err(format!("no folder {folder}"));
    }
    Ok(folders.into_iter().chain(files).collect())
}

/// Every tag with how many notes have it, or the notes with [tag] or a tag under it.
fn tags(index: &VaultIndex, hidden: &ExcludedFiles, tag: Option<&str>) -> Vec<String> {
    let Some(tag) = tag.map(|t| t.trim_start_matches('#').to_lowercase()) else {
        let counts = hidden.tags(index);
        let mut tags: Vec<_> = counts.iter().collect();
        tags.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        return tags.into_iter().map(|(tag, count)| format!("{count}\t#{tag}")).collect();
    };
    let mut notes: Vec<String> = index
        .notes
        .values()
        .filter(|n| !hidden.contains(&n.path))
        .filter(|n| {
            n.parsed.tags.iter().any(|t| {
                let t = t.to_lowercase();
                t == tag || t.starts_with(&format!("{tag}/"))
            })
        })
        .map(|n| n.path.clone())
        .collect();
    notes.sort();
    notes
}

fn tasks(index: &VaultIndex, hidden: &ExcludedFiles, args: &Arguments) -> Result<Vec<String>, String> {
    let by = match args.optional("due") {
        None => None,
        Some("today") => Some(Day::today().to_string()),
        Some(text) => Some(Day::parse(text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?.to_string()),
    };
    let all = args.flag("all");
    let path = args.optional("path");
    let mut tasks: Vec<VaultTask> = hidden
        .tasks(index)
        .into_iter()
        .filter(|t| all || !TaskLine::is_closed(t.task.status))
        .filter(|t| path.is_none_or(|p| t.path.starts_with(p)))
        .filter(|t| by.as_deref().is_none_or(|by| t.task.due.as_deref().or(t.task.scheduled.as_deref()).is_some_and(|d| d <= by)))
        .collect();
    let day = |t: &VaultTask| t.task.due.clone().or_else(|| t.task.scheduled.clone()).unwrap_or_else(|| "9999".into());
    tasks.sort_by(|a, b| day(a).cmp(&day(b)).then(a.path.cmp(&b.path)).then(a.task.line.cmp(&b.task.line)));
    Ok(tasks.iter().take(args.count("limit", 200)).map(crate::task_line).collect())
}

fn query_tasks(index: &VaultIndex, hidden: &ExcludedFiles, source: &str, note: Option<&str>) -> Result<Vec<String>, String> {
    let file = note.map(|n| crate::note_path(index, n)).transpose()?;
    let query = TasksQuery::parse(source, file.as_deref());
    let mut lines: Vec<String> = query.not_understood.iter().map(|l| format!("not understood, so not applied: {l}")).collect();
    let mut tasks = hidden.tasks(index);
    tasks.sort_by(|a, b| (&a.path, a.task.line).cmp(&(&b.path, b.task.line)));
    let mut open: Vec<String> = Vec::new();
    for task in query.run(tasks, Day::today(), |t| QueryTask::from(t)) {
        for (level, name) in query.groups(&QueryTask::from(&task)).iter().enumerate() {
            if open.get(level) == Some(name) {
                continue;
            }
            open.truncate(level);
            open.push(name.clone());
            if !name.is_empty() {
                lines.push(format!("{} {name}", "#".repeat(level + 1)));
            }
        }
        lines.push(crate::task_line(&task));
    }
    Ok(lines)
}

/// The commits of the last [days] days, each with the notes it changed.
fn recent_changes(root: &Path, days: usize) -> Result<Vec<String>, String> {
    let git = crate::repository(root)?;
    let since = format!("--since={days}.days");
    let log = git
        .run(&["log", &since, "--name-status", "--format=%x00%h\t%ad\t%s", "--date=format:%Y-%m-%d %H:%M"])
        .map_err(|e| e.to_string())?;
    Ok(log.lines().filter(|l| !l.is_empty()).map(|l| l.strip_prefix('\0').map_or_else(|| format!("  {l}"), str::to_string)).collect())
}

/// A note with what holds its state elsewhere: its text, what the index
/// knows of it, each line of another note that links to it (a task line
/// first), and its last commits.
fn note_context(root: &Path, index: &VaultIndex, hidden: &ExcludedFiles, note: &str) -> Result<String, String> {
    let text = read_note(root, index, note, None)?;
    let path = crate::note_path(index, note)?;
    let mut out = vec![text, "--- what the index knows".to_string()];
    out.extend(note_info(index, hidden, note)?);
    let mut tasks = Vec::new();
    let mut lines = Vec::new();
    let backlinks: Vec<_> = index.backlinks(&path).iter().filter(|b| !hidden.contains(&b.source) && b.source != path).collect();
    for backlink in backlinks {
        let Ok(source) = fs::read_to_string(root.join(&backlink.source)) else { continue };
        let Some(raw) = source.split('\n').nth(backlink.line) else { continue };
        let row = format!("{}:{}\t{}", backlink.source, backlink.line + 1, raw.trim_end_matches('\r').trim());
        if TaskLine::parse(raw.trim_end_matches('\r'), backlink.line, "").is_some() { tasks.push(row) } else { lines.push(row) }
    }
    tasks.dedup();
    lines.dedup();
    out.push("--- tasks that link to it".to_string());
    out.extend(if tasks.is_empty() { vec!["(none)".to_string()] } else { tasks });
    out.push("--- other lines that link to it".to_string());
    out.extend(if lines.is_empty() { vec!["(none)".to_string()] } else { lines });
    out.push("--- its last commits".to_string());
    match crate::repository(root).and_then(|git| git.history(&path, 5).map_err(|e| e.to_string())) {
        Ok(revisions) if !revisions.is_empty() => out.extend(
            revisions
                .into_iter()
                .map(|r| format!("{}\t{}\t{}", &r.hash[..12.min(r.hash.len())], r.date.format("%Y-%m-%d %H:%M"), r.subject)),
        ),
        Ok(_) => out.push("(not committed yet)".to_string()),
        Err(error) => out.push(format!("(no history: {error})")),
    }
    Ok(out.join("\n"))
}

/// Renames or moves a note to [to], a vault path (`.md` is added to a name
/// without an extension), and rewrites every link to it in the vault, as the
/// app's Rename and Move do; the vault's bookmarks follow it.
fn move_note(root: &Path, index: &VaultIndex, note: &str, to: &str) -> Result<Vec<String>, String> {
    let from = crate::note_path(index, note)
        .or_else(|_| if index.files.contains(note) { Ok(note.to_string()) } else { Err(format!("no note {note}")) })?;
    let to = to.trim().trim_start_matches("./");
    let (folder, name) = to.rsplit_once('/').map_or(("", to), |(f, n)| (f, n));
    let to = if folder.is_empty() { VaultMoves::note_name(name) } else { format!("{folder}/{}", VaultMoves::note_name(name)) };
    if to.starts_with('/') || to.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.')) {
        return Err(format!("{to} is not a path in the vault"));
    }
    if VaultEdits::within(root, &from).is_none() || VaultEdits::within(root, &to).is_none() {
        return Err(format!("{to} is outside the vault"));
    }
    let moves = VaultMoves::plan(&from, &to, &index.files).map_err(|e| e.to_string())?;
    // The notes whose links change, by their path after the move.
    let relinked = VaultMoves::relinked(index, &moves, |path| fs::read_to_string(root.join(path)).ok());
    VaultMoves::perform(&from, &to, index, root).map_err(|e| e.to_string())?;
    let mut bookmarks = Bookmarks::load(root);
    if bookmarks.follow(&moves, None) {
        bookmarks.save(root).map_err(|e| format!("moved, but the bookmarks were not saved: {e}"))?;
    }
    let mut out = vec![format!("moved\t{from} → {to}")];
    out.extend(relinked.into_keys().filter(|p| *p != to).map(|p| format!("links rewritten\t{p}")));
    Ok(out)
}

/// Sets, or with `remove` takes away, one property of a note's front
/// matter, keeping every other line as it was written; the front matter is
/// made when the note has none.
fn set_property(root: &Path, index: &VaultIndex, args: &Arguments) -> Result<String, String> {
    let path = crate::note_path(index, args.text("note")?)?;
    let key = args.text("key")?.trim().to_string();
    let value = if args.flag("remove") {
        None
    } else if let Some(items) = args.0.get("value").and_then(Value::as_array) {
        Some(PropertyValue::List(items.iter().filter_map(Value::as_str).map(str::to_string).collect()))
    } else {
        match args.0.get("value").and_then(Value::as_str) {
            Some("") => Some(PropertyValue::Empty),
            Some(text) => Some(PropertyValue::Text(text.to_string())),
            None => return Err("value is needed: a string, a list of strings, or `remove`".into()),
        }
    };
    let mut why = None;
    VaultEdits::apply(&path, root, |text| {
        // A key the note has is set; a new one is added after the others.
        let has = PropertySheet::read(text).sheet().is_some_and(|sheet| sheet.value(&key).is_some());
        let edit = match value.clone() {
            None => PropertyEdit::Remove(key.clone()),
            Some(value) if has => PropertyEdit::Set(key.clone(), value),
            Some(value) => PropertyEdit::Add(key.clone(), value),
        };
        PropertySheet::edit(text, &edit).or_else(|reason| {
            why = Some(reason);
            Ok(text.to_string())
        })
    })
    .map_err(|e| format!("{path}: {e}"))?;
    if let Some(why) = why {
        return Err(format!("{path}: {why}"));
    }
    let text = fs::read_to_string(root.join(&path)).unwrap_or_default();
    let shown = match PropertySheet::read(&text).sheet().and_then(|sheet| sheet.value(&key).cloned()) {
        Some(value) => format!("{key}: {}", value.items().join(", ")),
        None => format!("{key} removed"),
    };
    Ok(format!("{path}\t{shown}"))
}

/// Puts [new] in place of [old], which must be in the note exactly once.
fn replace_in_note(root: &Path, index: &VaultIndex, note: &str, old: &str, new: &str) -> Result<String, String> {
    let path = crate::note_path(index, note)?;
    let mut line = 0;
    VaultEdits::apply(&path, root, |text| {
        let found = text.matches(old).count();
        if found != 1 {
            return Err(daftar_core::Failure::Unwritable(if found == 0 {
                "the old text is not in the note (read it again)".to_string()
            } else {
                format!("the old text is in the note {found} times; give more of it so it is there once")
            }));
        }
        let at = text.find(old).unwrap_or_default();
        line = text[..at].matches('\n').count() + 1;
        Ok(text.replacen(old, new, 1))
    })
    .map_err(|e| format!("{path}: {e}"))?;
    Ok(format!("{path}:{line}\treplaced"))
}

/// Puts [body] in place of a heading's own text: from the line after the
/// heading to the next heading of any level, so the headings under it stay.
fn replace_section(root: &Path, index: &VaultIndex, note: &str, heading: &str, body: &str) -> Result<String, String> {
    let path = crate::note_path(index, note)?;
    let wanted = heading.trim_start_matches('#').trim().to_string();
    let mut place = (0, 0);
    VaultEdits::apply(&path, root, |text| {
        let headings = Sections::headings(text);
        let matching: Vec<usize> = headings.iter().enumerate().filter(|(_, h)| h.text == wanted).map(|(i, _)| i).collect();
        let at = match matching.as_slice() {
            [at] => *at,
            [] => {
                let names: Vec<&str> = headings.iter().map(|h| h.text.as_str()).collect();
                return Err(daftar_core::Failure::Unwritable(format!("no heading “{wanted}”; it has: {}", names.join(" | "))));
            }
            _ => return Err(daftar_core::Failure::Unwritable(format!("{} headings are “{wanted}”", matching.len()))),
        };
        let lines: Vec<&str> = text.split('\n').collect();
        let start = headings[at].line + 1;
        let end = headings.get(at + 1).map_or(lines.len(), |next| next.line);
        let mut middle = vec![""];
        let trimmed = body.trim_matches('\n');
        if !trimmed.is_empty() {
            middle.extend(trimmed.split('\n'));
            middle.push("");
        }
        // The note's last line keeps its newline, and no blank line is left at the end.
        if end == lines.len() {
            while middle.len() > 1 && middle.last() == Some(&"") {
                middle.pop();
            }
            middle.push("");
        }
        place = (start + 1, start + middle.len());
        Ok([&lines[..start], &middle[..], &lines[end..]].concat().join("\n"))
    })
    .map_err(|e| format!("{path}: {e}"))?;
    Ok(format!("{path}#{wanted}\tlines {}-{} written", place.0, place.1))
}

/// Commits [paths] (every change when none is named) with [message], then
/// pulls with a rebase and pushes when [push]; refused while `doctor` finds
/// what would stop a sync.
fn commit(root: &Path, message: &str, paths: &[String], push: bool) -> Result<Vec<String>, String> {
    for path in paths {
        if path.starts_with('/') || path.split('/').any(|part| part == "..") || VaultEdits::within(root, path).is_none() {
            return Err(format!("{path} is not in the vault"));
        }
    }
    let git = crate::repository(root)?;
    let stops: Vec<String> =
        git.doctor(u64::MAX).map_err(|e| e.to_string())?.into_iter().filter(|d| d.level == "error").map(|d| d.message).collect();
    if !stops.is_empty() {
        return Err(format!("not committed: {}", stops.join("; ")));
    }
    let mut add = vec!["add", "-A", "--"];
    add.extend(if paths.is_empty() { vec!["."] } else { paths.iter().map(String::as_str).collect() });
    git.run(&add).map_err(|e| e.to_string())?;
    if git.run(&["diff", "--cached", "--quiet"]).is_ok() {
        return Err("nothing to commit in the files named".into());
    }
    let mut commit = vec!["commit", "-m", message, "--"];
    if !paths.is_empty() {
        commit.extend(paths.iter().map(String::as_str));
    }
    git.run(&commit).map_err(|e| e.to_string())?;
    let mut out = vec![format!("committed\t{}", git.run(&["log", "-1", "--format=%h %s"]).map_err(|e| e.to_string())?)];
    if push {
        git.pull().map_err(|e| format!("committed, but the pull failed: {e}"))?;
        let pushed = git.push_if_ahead().map_err(|e| format!("committed and pulled, but the push failed: {e}"))?;
        out.push(if pushed { "pulled and pushed".into() } else { "pulled; nothing to push".into() });
    }
    Ok(out)
}

/// Makes a note at [path] with [text]; never over a file that is there,
/// never outside the vault.
fn create_note(root: &Path, path: &str, text: &str) -> Result<String, String> {
    let path = path.trim();
    let path = if path.to_lowercase().ends_with(".md") { path.to_string() } else { format!("{path}.md") };
    if path.starts_with('/') || path.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.')) {
        return Err(format!("{path} is not a note in the vault"));
    }
    let file = VaultEdits::within(root, &path).ok_or_else(|| format!("{path} is outside the vault"))?;
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder).map_err(|e| format!("{path}: {e}"))?;
    }
    let mut text = text.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let mut made = fs::OpenOptions::new().write(true).create_new(true).open(&file).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{path} is there already; use append_to_note"),
        _ => format!("{path}: {e}"),
    })?;
    made.write_all(text.as_bytes()).map_err(|e| format!("{path}: {e}"))?;
    Ok(path)
}

/// The task a tool names by `note`, `line` (from 1) and `text`, which must
/// still be on that line: a line that moved since it was read is refused.
fn task_at(root: &Path, index: &VaultIndex, args: &Arguments) -> Result<(String, usize, ParsedTask), String> {
    let path = crate::note_path(index, args.text("note")?)?;
    let line = args.count("line", 0);
    let at = line.checked_sub(1).ok_or("line counts from 1")?;
    let text = fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
    let raw = text.split('\n').nth(at).ok_or_else(|| format!("{path} has no line {line}"))?;
    let task = TaskLine::parse(raw.trim_end_matches('\r'), at, "").ok_or_else(|| format!("{path}:{line} is not a task: {raw}"))?;
    let expected = args.text("text")?;
    if !TaskLine::holds(raw, expected) {
        return Err(format!("{path}:{line} is now “{raw}”, not the task “{expected}”; list the tasks again"));
    }
    Ok((path, at, task))
}

/// The vault's own cancelled status, `-` unless its settings say otherwise.
fn cancelled_symbol() -> String {
    let statuses = TaskStatuses::current();
    statuses.all.iter().find(|s| s.kind == TaskType::Cancelled).map_or('-', |s| s.symbol).to_string()
}

/// The drill card named [id] (`path#Heading#Under it`), read from its note now.
fn drill_card(root: &Path, index: &VaultIndex, id: &str) -> Result<Card, String> {
    index
        .notes
        .keys()
        .filter(|path| id.starts_with(&format!("{path}#")))
        .find_map(|path| Drill::deck(root, std::slice::from_ref(path)).into_iter().find(|card| card.id == id))
        .ok_or_else(|| format!("no card {id}; list them again with drill_due"))
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool call's arguments.
struct Arguments<'a>(&'a Value);

impl Arguments<'_> {
    fn optional(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
    }

    fn text(&self, name: &str) -> Result<&str, String> {
        self.optional(name).ok_or_else(|| format!("{name} is needed"))
    }

    fn flag(&self, name: &str) -> bool {
        self.0.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    fn count(&self, name: &str, default: usize) -> usize {
        let value = self.0.get(name);
        value
            .and_then(Value::as_u64)
            .or_else(|| value.and_then(Value::as_str).and_then(|s| s.parse().ok()))
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(default)
    }

    fn list(&self, name: &str) -> Vec<String> {
        self.0
            .get(name)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn day(&self, name: &str) -> Result<Day, String> {
        match self.optional(name) {
            None | Some("today") => Ok(Day::today()),
            Some(text) => Day::parse(text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)")),
        }
    }
}

/// The tools, with their JSON schemas.
fn tools(read_only: bool) -> Vec<Value> {
    let string = |description: &str| json!({ "type": "string", "description": description });
    let integer = |description: &str| json!({ "type": "integer", "description": description });
    let boolean = |description: &str| json!({ "type": "boolean", "description": description });
    let tool = |name: &str, description: &str, properties: Value, required: &[&str], writes: bool| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": { "type": "object", "properties": properties, "required": required },
            "annotations": { "readOnlyHint": !writes, "destructiveHint": false, "idempotentHint": !writes, "openWorldHint": false },
        })
    };
    let note = "A note's vault path (`Companies/Acme/Acme.md`) or its name as a link writes it (`Acme`)";
    let task = |extra: Value| {
        let mut properties = json!({
            "note": string(note),
            "line": integer("The task's line, from 1, as `tasks` printed it"),
            "text": string("The task's text as `tasks` printed it; the call is refused when the line no longer holds it"),
        });
        if let (Some(properties), Some(extra)) = (properties.as_object_mut(), extra.as_object()) {
            properties.extend(extra.clone());
        }
        properties
    };
    let mut all = vec![
        tool(
            "search",
            "Full-text search over every note (SQLite FTS5, Persian too): the best notes first, each with a snippet.",
            json!({ "query": string("Words to find"), "limit": integer("How many notes, 20 at first") }),
            &["query"],
            false,
        ),
        tool(
            "find_notes",
            "Notes and files by name, ranked as the quick switcher ranks them: words in any order, aliases, `comp/acme` for a path.",
            json!({ "query": string("Part of a name or path"), "limit": integer("How many, 20 at first") }),
            &["query"],
            false,
        ),
        tool(
            "read_note",
            "A note's whole text, or only the section under one heading. Also reads `.base` and `.canvas` files by path.",
            json!({ "note": string(note), "heading": string("A heading's text: only its section") }),
            &["note"],
            false,
        ),
        tool(
            "note_info",
            "What the vault knows of a note without its text: properties, aliases, tags, outline with lines, task counts, the notes it links to and the notes linking to it.",
            json!({ "note": string(note) }),
            &["note"],
            false,
        ),
        tool(
            "list_folder",
            "The folders (ending in /) and files right in a folder of the vault; the top of the vault with none.",
            json!({ "folder": string("A folder's vault path, as `Companies`") }),
            &[],
            false,
        ),
        tool(
            "tags",
            "Every tag with how many notes have it, or with `tag` the notes that have that tag or one under it.",
            json!({ "tag": string("A tag, with or without #") }),
            &[],
            false,
        ),
        tool(
            "tasks",
            "Tasks in the Tasks plugin's format, the ones due first: `path:line`, a tab, `[status] text` and their dates (⏳ scheduled, 📅 due, ✅ done, ❌ cancelled, 🔁 repeat, 💬 reason).",
            json!({
                "due": string("Only those due or scheduled by this day, YYYY-MM-DD or `today`"),
                "path": string("Only in notes whose path starts with this, as `Tasks/`"),
                "all": boolean("Done and cancelled tasks too"),
                "limit": integer("How many, 200 at first"),
            }),
            &[],
            false,
        ),
        tool(
            "query_tasks",
            "Runs a Tasks plugin query, one instruction a line, as a ```tasks block does: `not done`, `due before next week`, `path includes Tasks/Work`, `status.name is In Review`, `has reason`, `done after 7 days ago`, `group by filename`, `sort by due`, `limit 50`. Lines it does not understand are listed first.",
            json!({ "query": string("The query's lines"), "note": string("The note the query stands in, for {{query.file.path}}") }),
            &["query"],
            false,
        ),
        tool("task_statuses", "The vault's task statuses: symbol, name, type and the next status.", json!({}), &[], false),
        tool(
            "recent_changes",
            "The vault's git commits of the last days, newest first, each with the files it changed (A added, M changed, D deleted, R renamed).",
            json!({ "days": integer("How many days back, 7 at first") }),
            &[],
            false,
        ),
        tool(
            "note_history",
            "The commits that changed a note, newest first, across renames.",
            json!({ "note": string(note), "limit": integer("How many, 30 at first") }),
            &["note"],
            false,
        ),
        tool(
            "daily_note",
            "The periodic note of a day (the Daily notes plugin's folder and format) and its text, if it was written.",
            json!({ "day": string("YYYY-MM-DD, today at first") }),
            &[],
            false,
        ),
        tool(
            "check_vault",
            "The vault's audit, as `daftar-cli check`: broken links and embeds, missing headings, ambiguous links, broken callouts, unused pictures, and tasks whose status and dates disagree. The vault's own options are read from its `.daftar.toml` (`[check]`, and a `profile` such as `audit` on top); pass more only to add to them.",
            json!({
                "profile": { "type": "string", "description": "A section of the vault's `.daftar.toml` added to its `[check]`, as `audit`" },
                "callouts": { "type": "array", "items": { "type": "string" }, "description": "Callout types the vault adds beyond Obsidian's and Daftar's `figure` and `read`, as `--callout`" },
                "no_h1": { "type": "array", "items": { "type": "string" }, "description": "Folders whose notes must have no H1, as `--no-h1`" },
                "memory": boolean("Also the memory warnings, as `--memory`: stale current-state paragraphs, relative dates, booked events that passed, held calls with no callout"),
                "event_properties": { "type": "array", "items": { "type": "string" }, "description": "With memory: properties holding a booked event's date, as `--event-property`" },
                "prep_notes": { "type": "array", "items": { "type": "string" }, "description": "With memory: names of preparation notes, as `--prep-note`" },
                "active": { "type": "array", "items": { "type": "string" }, "description": "With memory: `key=value` pairs marking a note as still going (`job_stage=applied`), as `--active`" },
                "quiet_days": integer("With memory and active: warn of a going note with nothing dated within this many days, as `--quiet-days`"),
            }),
            &[],
            false,
        ),
        tool(
            "capture",
            "Adds one line to the capture note (today's daily note unless the settings name another), as quick capture does.",
            json!({ "text": string("The line"), "task": boolean("As a task, `- [ ] text`") }),
            &["text"],
            true,
        ),
        tool(
            "append_to_note",
            "Adds one line at the end of a note, or of one heading's section (the heading is made when it is missing); the note is made when it is missing. For a task, put its dates in the text as the Tasks plugin writes them (📅 2026-11-02) and set `task`.",
            json!({
                "note": string(note),
                "text": string("The line"),
                "heading": string("A heading's text, to add under it"),
                "task": boolean("As a task, `- [ ] text`"),
            }),
            &["note", "text"],
            true,
        ),
        tool(
            "create_note",
            "Makes a new note; refused when the file is there. Follow the vault's CLAUDE.md for where notes go and how they are written.",
            json!({ "path": string("The new note's vault path, as `Notes/Plants.md`"), "text": string("Its Markdown") }),
            &["path", "text"],
            true,
        ),
        tool(
            "toggle_task",
            "Ticks an open task, or opens a closed one, as a click does: done dates and the next occurrence of a repeating task are written as the Tasks plugin writes them.",
            task(json!({})),
            &["note", "line", "text"],
            true,
        ),
        tool(
            "set_task_status",
            "Sets a task's status (a symbol from `task_statuses`: ` `, `/`, `x`, `-`, or `cancelled`), with an optional reason kept as `[reason:: …]`.",
            task(json!({ "status": string("One status symbol, or `cancelled`"), "reason": string("Why; empty takes the reason away") })),
            &["note", "line", "text", "status"],
            true,
        ),
        tool(
            "edit_task",
            "Sets a task's start, scheduled or due date (YYYY-MM-DD; an empty string takes it away) and its priority (highest, high, medium, low, lowest or none), as the app's task form does; what is not given stays.",
            task(json!({
                "start": string("🛫, YYYY-MM-DD or empty"),
                "scheduled": string("⏳, YYYY-MM-DD or empty"),
                "due": string("📅, YYYY-MM-DD or empty"),
                "priority": string("highest, high, medium, low, lowest or none"),
            })),
            &["note", "line", "text"],
            true,
        ),
        tool(
            "note_context",
            "A note and everything that holds its state elsewhere, in one call: its text, its properties, outline, links and backlinks, each task line and each other line that links to it, and its last five commits. Read this before changing a note, so its introduction, properties and the task lines about it are changed together.",
            json!({ "note": string(note) }),
            &["note"],
            false,
        ),
        tool(
            "set_property",
            "Sets one front matter property, keeping every other line as written (the front matter is made when there is none), or takes it away with `remove`. Refused when the front matter is one the app's form cannot edit safely.",
            json!({
                "note": string(note),
                "key": string("The property's name"),
                "value": { "description": "A string (empty for `key:` with nothing after it), or a list of strings", "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "string" } }] },
                "remove": boolean("Take the property away"),
            }),
            &["note", "key"],
            true,
        ),
        tool(
            "replace_in_note",
            "Puts `new` in place of `old`, a passage copied exactly from the note as `read_note` gave it, which must be in the note exactly once. Use it to rewrite a sentence or paragraph that stopped being true (the introduction's current state), never to append.",
            json!({ "note": string(note), "old": string("The exact text to replace, found once"), "new": string("Its replacement; empty deletes it") }),
            &["note", "old", "new"],
            true,
        ),
        tool(
            "replace_section",
            "Rewrites the text right under a heading, up to the next heading of any level (the headings under it and their text stay). The heading must be in the note once.",
            json!({ "note": string(note), "heading": string("The heading's text"), "text": string("The section's new Markdown, without the heading; empty clears it") }),
            &["note", "heading", "text"],
            true,
        ),
        tool(
            "move_note",
            "Renames or moves a note (or a base or canvas) to a new vault path and rewrites every link to it in the vault, as the app's Rename and Move do. Refused when a file is there. Commit the moved note and every note it lists together.",
            json!({ "note": string(note), "to": string("The new vault path, as `Companies/Acme/Acme.md`; `.md` is added to a name without one") }),
            &["note", "to"],
            true,
        ),
        tool(
            "commit",
            "Commits the files named (every change when none is named) with the message, then pulls with a rebase and pushes. Refused while an operation, a rebase or a conflict is open (`doctor`). One logical change per commit, its subject naming the subject and the event.",
            json!({
                "message": string("The commit message"),
                "paths": { "type": "array", "items": { "type": "string" }, "description": "Vault paths to commit; all changes when left out" },
                "push": boolean("Pull and push after committing; true at first"),
            }),
            &["message"],
            true,
        ),
        tool(
            "doctor",
            "What stands in a sync's way in the vault's repository: an operation left open, conflicts, no upstream, commits not pushed or pulled, large untracked files.",
            json!({}),
            &[],
            false,
        ),
        tool(
            "job_pipeline",
            "The job search in one call: every note with `job_stage` is an application. Each stage with its count; each channel (`job_channel` up to ` (`) with its applications, how many got past applied and the rate; and the booked events ahead (`job_next`), soonest first, with the company, `job_next_step` and the path of its `Interview Preparation.md`.",
            json!({ "days": integer("Only the events within this many days; all ahead at first") }),
            &[],
            false,
        ),
        tool(
            "postpone_task",
            "Moves a task's due date (else scheduled, else start) on, as the Tasks plugin's Postpone does.",
            task(json!({ "by": string("day, 2days, 3days, week, 2weeks or month; day at first") })),
            &["note", "line", "text"],
            true,
        ),
    ];
    all.extend([
        tool(
            "random_note",
            "A note picked at random, as Obsidian's Open random note, with its text: from the whole vault, a folder or a tag.",
            json!({ "folder": string("Only a note in this folder, as `Learning`"), "tag": string("Only a note with this tag or one under it") }),
            &[],
            false,
        ),
        tool(
            "drill_due",
            "Spaced-repetition cards due today from notes of questions (a heading is a card, its section the answer), the overdue first, then a few new ones: each card's id, `path:line`, due day (or new), context and question, without the answer. Quiz the person with the question, then read the answer with `drill_card` and rate their reply with `drill_rate`.",
            json!({
                "notes": { "type": "array", "items": { "type": "string" }, "description": "Notes or folders; `Learning/Interview-Preparation` when none" },
                "limit": integer("How many cards, 10 at first"),
                "new": integer("New cards at most, 10 at first"),
            }),
            &[],
            false,
        ),
        tool(
            "drill_card",
            "The answer of a drill card: its heading's section as the note has it now.",
            json!({ "id": string("The card's id, as `drill_due` gave it") }),
            &["id"],
            false,
        ),
        tool(
            "drill_rate",
            "Rates how well the person answered a drill card (again: forgotten, hard, good, easy) and schedules it by SM-2; the note is not written, only Daftar's drill state. Answers when it is due next.",
            json!({ "id": string("The card's id, as `drill_due` gave it"), "rating": string("again, hard, good or easy (or 1-4)") }),
            &["id", "rating"],
            true,
        ),
    ]);
    if read_only {
        all.retain(|tool| tool["annotations"]["readOnlyHint"].as_bool().is_none_or(|read| read));
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path();
        fs::create_dir_all(root_path.join(".obsidian")).unwrap();
        fs::create_dir_all(root_path.join("Garden")).unwrap();
        fs::write(
            root_path.join("Garden/Plants.md"),
            "---\ntags: [garden]\n---\n# Plants\n\nSee [[Seeds]].\n\n## Basil\n\nNeeds sun.\n\n## Mint\n\n- [ ] Water the mint 📅 2026-01-02\n",
        )
        .unwrap();
        fs::write(root_path.join("Seeds.md"), "# Seeds\n\nTomato seeds from [[Plants]].\n").unwrap();
        root
    }

    fn call(server: &mut Server, name: &str, arguments: Value) -> (String, bool) {
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": arguments } });
        let answer = server.handle(&request.to_string()).unwrap();
        let result = &answer["result"];
        (result["content"][0]["text"].as_str().unwrap().to_string(), result["isError"].as_bool().unwrap())
    }

    fn server(root: &Path, read_only: bool) -> Server {
        Server { root: root.to_path_buf(), show_hidden: false, read_only, search: None, drill_state: Some(root.join(".drill-state.json")) }
    }

    #[test]
    fn edit_task_sets_dates_and_priority_and_keeps_the_rest() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("T.md"), "# T\n- [ ] Water plants ➕ 2026-11-01 📅 2026-11-06\n").unwrap();
        let mut server = server(root.path(), false);
        let task = |more: Value| {
            let mut args = json!({ "note": "T", "line": 2, "text": "Water plants" });
            args.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
            args
        };
        let (said, error) = call(&mut server, "edit_task", task(json!({ "scheduled": "2026-11-07", "due": "", "priority": "high" })));
        assert!(!error, "{said}");
        assert_eq!(fs::read_to_string(root.path().join("T.md")).unwrap(), "# T\n- [ ] Water plants ⏫ ➕ 2026-11-01 ⏳ 2026-11-07\n");
        assert!(call(&mut server, "edit_task", task(json!({ "due": "2026-13-01" }))).1);
        assert!(call(&mut server, "edit_task", task(json!({ "priority": "urgent" }))).1);
        let moved = json!({ "note": "T", "line": 2, "text": "Water", "due": "2026-12-01" });
        assert!(call(&mut server, "edit_task", moved).1);
        assert!(!fs::read_to_string(root.path().join("T.md")).unwrap().contains("2026-12-01"));
    }

    #[test]
    fn it_answers_initialize_and_lists_its_tools() {
        let root = vault();
        let mut server = server(root.path(), false);
        let answer = server.handle(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap();
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(answer["result"]["serverInfo"]["name"], "daftar");
        // An unknown version gets the one we speak; a notification gets nothing.
        let answer = server.handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#).unwrap();
        assert_eq!(answer["result"]["protocolVersion"], PROTOCOL);
        assert!(server.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let list = server.handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search") && names.contains(&"toggle_task"));
        let unknown = server.handle(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[test]
    fn a_read_only_server_has_no_writing_tool() {
        let root = vault();
        let mut server = server(root.path(), true);
        let list = server.handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"read_note"));
        assert!(WRITES.iter().all(|w| !names.contains(w)));
        let (text, error) = call(&mut server, "capture", json!({ "text": "x" }));
        assert!(error && text.contains("read-only"));
    }

    #[test]
    fn it_reads_a_note_a_section_and_what_links_there() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (text, _) = call(&mut server, "read_note", json!({ "note": "Plants" }));
        assert!(text.starts_with("Garden/Plants.md\n\n---\ntags"));
        let (text, _) = call(&mut server, "read_note", json!({ "note": "Plants", "heading": "Basil" }));
        assert_eq!(text, "Garden/Plants.md#Basil (lines 8-11)\n\n## Basil\n\nNeeds sun.\n");
        let (text, error) = call(&mut server, "read_note", json!({ "note": "Plants", "heading": "Rose" }));
        assert!(error && text.contains("Plants | Basil | Mint"));
        let (text, _) = call(&mut server, "note_info", json!({ "note": "Plants" }));
        assert!(text.contains("tags: #garden"));
        assert!(text.contains("links to:\n  Seeds.md"));
        assert!(text.contains("linked from:\n  Seeds.md"));
        assert!(text.contains("tasks: 1 (1 open)"));
        let (text, _) = call(&mut server, "list_folder", json!({}));
        assert_eq!(text, "Garden/\nSeeds.md");
        let (text, _) = call(&mut server, "tags", json!({ "tag": "#garden" }));
        assert_eq!(text, "Garden/Plants.md");
    }

    #[test]
    fn search_follows_the_notes_as_they_change() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (text, _) = call(&mut server, "search", json!({ "query": "tomato" }));
        assert!(text.starts_with("Seeds.md\t"));
        fs::write(root.path().join("Garden/Tomato.md"), "Tomato plants need stakes.\n").unwrap();
        fs::remove_file(root.path().join("Seeds.md")).unwrap();
        let (text, _) = call(&mut server, "search", json!({ "query": "tomato" }));
        assert!(text.starts_with("Garden/Tomato.md\t"), "{text}");
        assert!(!text.contains("Seeds.md"));
    }

    #[test]
    fn a_task_is_changed_only_while_its_line_holds_it() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (text, _) = call(&mut server, "tasks", json!({}));
        assert_eq!(text, "Garden/Plants.md:14\t[ ] Water the mint 📅 2026-01-02");
        let (text, error) = call(&mut server, "toggle_task", json!({ "note": "Plants", "line": 14, "text": "Feed the cat" }));
        assert!(error && text.contains("list the tasks again"), "{text}");
        let (text, error) = call(&mut server, "toggle_task", json!({ "note": "Plants", "line": 14, "text": "Water the mint" }));
        assert!(!error, "{text}");
        assert!(text.starts_with("Garden/Plants.md:14\t- [x] Water the mint 📅 2026-01-02 ✅ "));
        let (text, error) = call(
            &mut server,
            "set_task_status",
            json!({ "note": "Plants", "line": 14, "text": "Water the mint", "status": "cancelled", "reason": "rained" }),
        );
        assert!(!error, "{text}");
        assert!(text.contains("- [-] Water the mint") && text.contains("[reason:: rained]"), "{text}");
    }

    #[test]
    fn it_adds_to_notes_and_makes_new_ones_but_never_over_one() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (text, error) =
            call(&mut server, "append_to_note", json!({ "note": "Plants", "heading": "Basil", "text": "Pick leaves", "task": true }));
        assert!(!error, "{text}");
        let plants = fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap();
        assert!(plants.contains("Needs sun.\n\n- [ ] Pick leaves\n\n## Mint"), "{plants}");
        let (text, error) = call(&mut server, "create_note", json!({ "path": "Garden/Soil", "text": "# Soil" }));
        assert!(!error && text == "Garden/Soil.md");
        assert_eq!(fs::read_to_string(root.path().join("Garden/Soil.md")).unwrap(), "# Soil\n");
        let (text, error) = call(&mut server, "create_note", json!({ "path": "Garden/Soil.md", "text": "over" }));
        assert!(error && text.contains("there already"));
        let (_, error) = call(&mut server, "create_note", json!({ "path": "../out", "text": "x" }));
        assert!(error);
        let (_, error) = call(&mut server, "create_note", json!({ "path": ".obsidian/app", "text": "x" }));
        assert!(error);
    }
    #[test]
    fn a_note_is_changed_in_place_and_only_where_it_was_read() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (said, error) = call(&mut server, "set_property", json!({ "note": "Plants", "key": "stage", "value": "growing" }));
        assert!(!error, "{said}");
        assert_eq!(said, "Garden/Plants.md\tstage: growing");
        let (said, error) = call(&mut server, "set_property", json!({ "note": "Plants", "key": "beds", "value": ["north", "south"] }));
        assert!(!error, "{said}");
        let plants = fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap();
        assert!(plants.starts_with("---\ntags: [garden]\nstage: growing\nbeds:\n"), "{plants}");
        assert!(!call(&mut server, "set_property", json!({ "note": "Plants", "key": "stage", "remove": true })).1);
        assert!(!fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap().contains("stage"));
        assert!(!call(&mut server, "set_property", json!({ "note": "Seeds", "key": "kind", "value": "tomato" })).1);
        assert!(fs::read_to_string(root.path().join("Seeds.md")).unwrap().starts_with("---\nkind: tomato\n---\n# Seeds"));

        let (said, error) =
            call(&mut server, "replace_in_note", json!({ "note": "Plants", "old": "Needs sun.", "new": "Needs sun and water." }));
        assert!(!error && said == "Garden/Plants.md:13\treplaced", "{said}");
        let (said, error) = call(&mut server, "replace_in_note", json!({ "note": "Plants", "old": "Needs shade.", "new": "x" }));
        assert!(error && said.contains("not in the note"), "{said}");
        let (said, error) = call(&mut server, "replace_in_note", json!({ "note": "Plants", "old": "## ", "new": "### " }));
        assert!(error && said.contains("times"), "{said}");

        let (said, error) =
            call(&mut server, "replace_section", json!({ "note": "Plants", "heading": "Basil", "text": "Sun, water.\n\nPick often." }));
        assert!(!error, "{said}");
        let plants = fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap();
        assert!(plants.contains("## Basil\n\nSun, water.\n\nPick often.\n\n## Mint\n\n- [ ] Water the mint"), "{plants}");
        assert!(!call(&mut server, "replace_section", json!({ "note": "Plants", "heading": "Mint", "text": "Gone." })).1);
        assert!(fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap().ends_with("## Mint\n\nGone.\n"));
        assert!(call(&mut server, "replace_section", json!({ "note": "Plants", "heading": "Rose", "text": "x" })).1);
    }

    #[test]
    fn the_context_of_a_note_has_the_tasks_and_lines_about_it() {
        let root = vault();
        fs::write(root.path().join("Tasks.md"), "# Tasks\n\n- [ ] Sow [[Seeds]] 📅 2026-03-01\n").unwrap();
        let mut server = server(root.path(), false);
        let (said, error) = call(&mut server, "note_context", json!({ "note": "Seeds" }));
        assert!(!error, "{said}");
        assert!(said.starts_with("Seeds.md\n\n# Seeds"), "{said}");
        assert!(said.contains("--- tasks that link to it\nTasks.md:3\t- [ ] Sow [[Seeds]] 📅 2026-03-01\n"), "{said}");
        assert!(said.contains("--- other lines that link to it\nGarden/Plants.md:6\tSee [[Seeds]].\n"), "{said}");
    }

    #[test]
    fn commit_commits_the_files_named_and_nothing_else() {
        let root = vault();
        let git = daftar_core::Git::new(root.path());
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "T"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "start"],
        ] {
            git.run(&args).unwrap();
        }
        fs::write(root.path().join("Seeds.md"), "# Seeds\n\nChanged.\n").unwrap();
        fs::write(root.path().join("Other.md"), "Not this one.\n").unwrap();
        let mut server = server(root.path(), false);
        let (said, error) = call(&mut server, "commit", json!({ "message": "seeds: changed", "paths": ["Seeds.md"], "push": false }));
        assert!(!error, "{said}");
        assert!(said.starts_with("committed\t") && said.ends_with("seeds: changed"), "{said}");
        assert_eq!(git.run(&["show", "--name-only", "--format=", "HEAD"]).unwrap(), "Seeds.md");
        assert!(git.run(&["status", "--porcelain"]).unwrap().contains("Other.md"));
        assert!(call(&mut server, "commit", json!({ "message": "x", "paths": ["Seeds.md"], "push": false })).1);
        assert!(call(&mut server, "commit", json!({ "message": "x", "paths": ["../out.md"], "push": false })).1);
    }
    #[test]
    fn a_note_moves_with_every_link_to_it() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (said, error) = call(&mut server, "move_note", json!({ "note": "Seeds", "to": "Garden/Seed Bank" }));
        assert!(!error, "{said}");
        assert!(said.starts_with("moved\tSeeds.md → Garden/Seed Bank.md"), "{said}");
        assert!(said.contains("links rewritten\tGarden/Plants.md"), "{said}");
        assert!(root.path().join("Garden/Seed Bank.md").exists() && !root.path().join("Seeds.md").exists());
        let plants = fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap();
        assert!(plants.contains("[[Seed Bank]]") || plants.contains("[[Garden/Seed Bank]]"), "{plants}");
        assert!(call(&mut server, "move_note", json!({ "note": "Plants", "to": "Garden/Seed Bank.md" })).1);
        assert!(call(&mut server, "move_note", json!({ "note": "Plants", "to": "../Out" })).1);
    }

    #[test]
    fn a_drill_asks_without_the_answer_shows_it_and_rates_it() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (said, error) = call(&mut server, "drill_due", json!({ "notes": ["Garden"], "limit": 2 }));
        assert!(!error, "{said}");
        assert!(
            said.starts_with("id: Garden/Plants.md#Plants\nat: Garden/Plants.md:4\ndue: new\ncontext: Plants\nquestion: Plants"),
            "{said}"
        );
        assert!(said.contains("id: Garden/Plants.md#Plants#Basil\n"), "{said}");
        assert!(!said.contains("Needs sun"), "{said}");
        let (said, error) = call(&mut server, "drill_card", json!({ "id": "Garden/Plants.md#Plants#Basil" }));
        assert!(!error, "{said}");
        assert!(said.ends_with("\n\nNeeds sun."), "{said}");
        let (said, error) = call(&mut server, "drill_rate", json!({ "id": "Garden/Plants.md#Plants#Basil", "rating": "good" }));
        assert!(!error, "{said}");
        assert!(said.contains(&format!("next on {}", Day::today().adding_days(1))), "{said}");
        let (said, _) = call(&mut server, "drill_due", json!({ "notes": ["Garden"] }));
        assert!(!said.contains("#Basil"), "{said}");
        assert!(call(&mut server, "drill_rate", json!({ "id": "Garden/Plants.md#Plants#Basil", "rating": "meh" })).1);
        assert!(call(&mut server, "drill_card", json!({ "id": "Garden/Plants.md#Rose" })).1);
        assert!(fs::read_to_string(root.path().join("Garden/Plants.md")).unwrap().contains("Needs sun."));
        let read_only: Vec<Value> = tools(true);
        assert!(read_only.iter().all(|t| t["name"] != "drill_rate"));
    }

    #[test]
    fn a_random_note_comes_with_its_text() {
        let root = vault();
        let mut server = server(root.path(), false);
        let (said, error) = call(&mut server, "random_note", json!({ "tag": "garden" }));
        assert!(!error, "{said}");
        assert!(said.starts_with("Garden/Plants.md\n\n---\ntags: [garden]"), "{said}");
        let (said, _) = call(&mut server, "random_note", json!({}));
        assert!(said.starts_with("Garden/Plants.md\n") || said.starts_with("Seeds.md\n"), "{said}");
        assert!(call(&mut server, "random_note", json!({ "folder": "Nowhere" })).1);
    }

    #[test]
    fn the_job_pipeline_has_stages_channels_and_prep_notes() {
        let root = vault();
        fs::create_dir_all(root.path().join("Companies/Acme")).unwrap();
        fs::write(
            root.path().join("Companies/Acme/Acme.md"),
            "---\njob_stage: interviewing\njob_next: 2999-01-04T10:00\njob_next_step: Final round\njob_channel: \"Ashby (acme)\"\n---\n",
        )
        .unwrap();
        fs::write(root.path().join("Companies/Acme/Interview Preparation.md"), "# Prep\n").unwrap();
        fs::write(root.path().join("Seeds.md"), "---\njob_stage: applied\n---\n").unwrap();
        let mut server = server(root.path(), true);
        let (text, error) = call(&mut server, "job_pipeline", json!({}));
        assert!(!error, "{text}");
        assert!(text.contains("applied\t1\ninterviewing\t1"), "{text}");
        assert!(text.contains("Ashby\t1\t1\t100%") && text.contains("unknown\t1\t0\t0%"), "{text}");
        assert!(
            text.contains(
                "Companies/Acme/Acme.md:3\t2999-01-04 10:00\tAcme\tinterviewing\tFinal round\tCompanies/Acme/Interview Preparation.md"
            ),
            "{text}"
        );
        let (text, _) = call(&mut server, "job_pipeline", json!({ "days": 3 }));
        assert!(text.contains("(none booked)"), "{text}");
    }
}
