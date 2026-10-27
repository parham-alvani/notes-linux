//! Daftar on the command line: the vault the app shows, for a shell, a
//! script or a launcher (`daftar-cli find … | fzf`). It reads the same
//! config file as the app and the same rules from `daftar-core`.

/// `println!` for what the command prints: a reader that stops early, as
/// `head` or `fzf` does, ends the command quietly instead of a panic.
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::emit(format_args!($($arg)*))
    };
}

mod images;
mod tui;
mod vault_commands;

use std::collections::BTreeSet;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use daftar_core::{
    Candidate, Clock, ConfigFile, DailyNotes, Day, Git, QueryTask, QuickCapture, QuickSwitcher, ReplaceOptions, SearchIndex, Side,
    SyncFailure, TaskLine, TasksQuery, Vault, VaultEdits, VaultIndex, VaultTask,
};

#[derive(Parser)]
#[command(name = "daftar-cli", version, about = "Daftar on the command line: an Obsidian vault in a Git repository")]
struct Cli {
    /// The vault's folder; else the vault the current folder is in, else
    /// `vault` in ~/.config/daftar/config.toml.
    #[arg(long, global = true, env = "DAFTAR_VAULT")]
    vault: Option<PathBuf>,
    /// With none, the TUI on a terminal, else this help.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// The vault in the terminal: the tree, the note, the quick switcher,
    /// search, tasks, backlinks and tags, in Daftar's colors.
    Tui,
    /// Notes and files by name, as the quick switcher ranks them.
    Find {
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// The text of every note, with SQLite's full-text search.
    Search {
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Open tasks, the ones due first.
    Tasks {
        /// Only the ones due or scheduled by this day (YYYY-MM-DD, or `today`).
        #[arg(long)]
        due: Option<String>,
        /// Only the tasks in notes whose path starts with this.
        #[arg(long)]
        path: Option<String>,
        /// Closed tasks too.
        #[arg(long)]
        all: bool,
    },
    /// A Tasks plugin query, as a ` ```tasks ` block in a note runs it:
    /// `daftar-cli query 'not done' 'due before tomorrow' 'group by filename'`.
    Query {
        /// The query's lines; with none, the `tasks` blocks of `--note`, else
        /// the query read from stdin.
        lines: Vec<String>,
        /// The note the query is in, which `{{query.file.path}}` and the
        /// others stand for.
        #[arg(long)]
        note: Option<String>,
    },
    /// Ticks or opens the task on a line of a note, as a click in the app does.
    Toggle {
        note: String,
        /// The line, from 1, as `tasks` prints it.
        line: usize,
    },
    /// The notes that link to a note.
    Backlinks { note: String },
    /// Links that go nowhere.
    Broken,
    /// Every tag, with how many notes have it.
    Tags,
    /// Commits every change, pulls with a rebase and pushes, as obsidian-git
    /// and the Mac app do. Uses your git config, credentials and SSH agent.
    /// A pull that stops on a conflict keeps its rebase open and lists the
    /// files; resolve them in the app, or `--abort`.
    Sync {
        /// The commit message; else `update notes @ <time> on <host>`.
        #[arg(short, long)]
        message: Option<String>,
        /// Ends a sync stopped on a conflict: the branch is as it was before
        /// the pull, and the next sync pulls again.
        #[arg(long, conflicts_with = "message")]
        abort: bool,
    },
    /// The commits that changed a note, newest first, across renames:
    /// the hash, the date, the author and the subject.
    History {
        note: String,
        /// The note's text at this commit instead.
        #[arg(long)]
        show: Option<String>,
        #[arg(short = 'n', long, default_value_t = 300)]
        limit: usize,
    },
    /// Adds a line to the capture note (`[capture]` in the config file:
    /// today's daily note at first), as the app's quick capture does.
    Capture {
        /// The line; with none, read from stdin.
        text: Vec<String>,
        /// As a task, `- [ ] text`.
        #[arg(short, long)]
        task: bool,
    },
    /// The branch, how far it is from its remote, and how many files changed.
    Status,
    /// Renames a tag and the tags under it (`#old/sub`) in every note, in the
    /// text and in the `tags` property; code and links are left alone.
    RenameTag {
        old: String,
        new: String,
        /// Lists the notes that would change, and changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Finds and replaces in the text of every note, as Replace in Vault does:
    /// prints each match's line before and after, then writes them.
    Replace {
        find: String,
        replacement: String,
        /// A regular expression, whose replacement names its groups as `$1`.
        #[arg(short = 'e', long)]
        regex: bool,
        #[arg(short = 'c', long)]
        match_case: bool,
        #[arg(short = 'w', long)]
        whole_word: bool,
        /// Code blocks and inline code too, which are left alone otherwise.
        #[arg(long)]
        in_code: bool,
        /// Only the notes in this folder of the vault.
        #[arg(long)]
        folder: Option<String>,
        /// Prints the matches, and changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Done and cancelled tasks first in each section of a note, as the Task
    /// Sorter plugin sorts them.
    SortTasks {
        note: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Moves a task's due date (else its scheduled, else its start date) on,
    /// as the Tasks plugin's Postpone does: by day, 2days, 3days, week, 2weeks or month.
    Postpone {
        note: String,
        /// The line, from 1, as `tasks` prints it.
        line: usize,
        #[arg(default_value = "day")]
        by: String,
    },
    /// The path of today's periodic note (the Daily notes plugin's folder
    /// and format: a day, a week or a month), made from its template when
    /// it is not there.
    Daily {
        /// Another day than today (YYYY-MM-DD).
        #[arg(long)]
        day: Option<String>,
        /// Prints the path and makes nothing.
        #[arg(long)]
        no_create: bool,
    },
    /// Removes pictures no note names, makes the others JPEGs under 1 MB and
    /// renames them in the notes, as notes-cli does.
    Images {
        /// The folder of pictures, in the vault.
        #[arg(default_value = "uploads")]
        folder: PathBuf,
        /// Shows what would change, and changes nothing.
        #[arg(long)]
        dry_run: bool,
        /// Keeps the pictures that were converted.
        #[arg(long)]
        keep_originals: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("daftar-cli: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let Some(command) = cli.command else {
        if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
            return tui::run(vault(cli.vault)?);
        }
        return Cli::command().print_help().map_err(|e| e.to_string());
    };
    let root = vault(cli.vault)?;
    match command {
        Command::Tui => tui::run(root)?,
        Command::Find { query, limit } => {
            let index = Vault::index(&root, None);
            let candidates: Vec<Candidate> = index
                .files
                .iter()
                .map(|path| {
                    let aliases = index.notes.get(path).map(|n| n.parsed.aliases.clone()).unwrap_or_default();
                    Candidate::new(path, &Vault::title(path), &aliases)
                })
                .collect();
            for path in QuickSwitcher::rank(&query.join(" "), &candidates, &[], limit) {
                out!("{path}");
            }
        }
        Command::Search { query, limit } => {
            let search = SearchIndex::new();
            Vault::index(&root, Some(&search));
            let terminal = std::io::stdout().is_terminal();
            for hit in search.search(&query.join(" "), limit) {
                out!("{}\t{}", hit.path, snippet(&hit.snippet, terminal));
            }
        }
        Command::Tasks { due, path, all } => {
            let index = Vault::index(&root, None);
            let by = match due.as_deref() {
                None => None,
                Some("today") => Some(Day::today().to_string()),
                Some(text) => Some(Day::parse(text).ok_or(format!("{text} is not a day (YYYY-MM-DD)"))?.to_string()),
            };
            let mut tasks: Vec<_> = index
                .tasks()
                .into_iter()
                .filter(|t| all || !TaskLine::is_closed(t.task.status))
                .filter(|t| path.as_deref().is_none_or(|p| t.path.starts_with(p)))
                .filter(|t| by.as_deref().is_none_or(|by| t.task.due.as_deref().or(t.task.scheduled.as_deref()).is_some_and(|d| d <= by)))
                .collect();
            // Due first, the earliest on top; then by note and line.
            tasks.sort_by(|a, b| {
                let day = |t: &daftar_core::VaultTask| t.task.due.clone().or(t.task.scheduled.clone()).unwrap_or_else(|| "9999".into());
                day(a).cmp(&day(b)).then(a.path.cmp(&b.path)).then(a.task.line.cmp(&b.task.line))
            });
            for task in tasks {
                out!("{}", task_line(&task));
            }
        }
        Command::Query { lines, note } => {
            let index = Vault::index(&root, None);
            let file = note.as_deref().map(|n| note_path(&index, n)).transpose()?;
            let sources = if !lines.is_empty() {
                vec![lines.join("\n")]
            } else if let Some(file) = &file {
                let text = fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
                let blocks = query_blocks(&text);
                if blocks.is_empty() {
                    return Err(format!("{file} has no tasks query"));
                }
                blocks
            } else {
                vec![std::io::read_to_string(std::io::stdin()).map_err(|e| e.to_string())?]
            };
            let mut tasks = index.tasks();
            tasks.sort_by(|a, b| (&a.path, a.task.line).cmp(&(&b.path, b.task.line)));
            for source in sources {
                let query = TasksQuery::parse(&source, file.as_deref());
                for line in &query.not_understood {
                    eprintln!("daftar-cli: not understood, so not applied: {line}");
                }
                // A group's name on its own line, `#` once per level, when it changes.
                let mut open: Vec<String> = Vec::new();
                for task in query.run(tasks.clone(), Day::today(), |t| QueryTask::from(t)) {
                    let groups = query.groups(&QueryTask::from(&task));
                    for (level, name) in groups.iter().enumerate() {
                        if open.get(level) == Some(name) {
                            continue;
                        }
                        open.truncate(level);
                        open.push(name.clone());
                        if !name.is_empty() {
                            out!("{} {name}", "#".repeat(level + 1));
                        }
                    }
                    out!("{}", task_line(&task));
                }
            }
        }
        Command::Toggle { note, line } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let at = line.checked_sub(1).ok_or("lines count from 1")?;
            let text = fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            let raw = text.split('\n').nth(at).ok_or(format!("{path} has no line {line}"))?;
            let task = TaskLine::parse(raw.trim_end_matches('\r'), at, "").ok_or(format!("{path}:{line} is not a task"))?;
            VaultEdits::apply(&path, &root, |text| VaultEdits::toggle_task(text, at, task.status, Day::today()))
                .map_err(|e| e.to_string())?;
            let after = fs::read_to_string(root.join(&path)).unwrap_or_default();
            out!("{}", after.split('\n').nth(at).unwrap_or_default());
        }
        Command::Backlinks { note } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            for link in index.backlinks(&path) {
                out!("{}:{}", link.source, link.line + 1);
            }
        }
        Command::Broken => {
            let index = Vault::index(&root, None);
            let mut broken = BTreeSet::new();
            for note in index.notes.values() {
                for link in &note.parsed.links {
                    if !link.target.is_empty() && !link.target.contains("://") && index.resolve(&link.target, &note.path).is_none() {
                        broken.insert((note.path.clone(), link.line + 1, link.target.clone()));
                    }
                }
            }
            for (path, line, target) in broken {
                out!("{path}:{line}\t{target}");
            }
        }
        Command::Tags => {
            let index = Vault::index(&root, None);
            let mut tags: Vec<_> = index.tags().iter().collect();
            tags.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (tag, count) in tags {
                out!("{count}\t#{tag}");
            }
        }
        Command::Sync { message: _, abort: true } => {
            let git = repository(&root)?;
            if !git.rebase_in_progress().map_err(|e| e.to_string())? {
                return Err("no sync is stopped".into());
            }
            git.abort_rebase().map_err(|e| e.to_string())?;
            out!("aborted: your commits are as they were before the pull");
        }
        Command::Sync { message, abort: false } => {
            let git = repository(&root)?;
            let message = message.unwrap_or_else(|| Git::message(chrono::Local::now(), &Git::host()));
            let done = match git.sync(&message) {
                Ok(done) => done,
                Err(SyncFailure::Stopped(stopped)) => {
                    for file in &stopped.conflicts {
                        out!("{}\t{} on the remote, {} here", file.path, side(file.remote), side(file.mine));
                    }
                    return Err(format!("{stopped} Resolve it in Daftar, or run `daftar-cli sync --abort`."));
                }
                Err(failure) => return Err(failure.to_string()),
            };
            let status = git.status().map_err(|e| e.to_string())?;
            let mut steps = Vec::new();
            if done.committed {
                steps.push(format!("committed \"{message}\""));
            }
            if done.pulled {
                steps.push("pulled".to_string());
            }
            if done.pushed {
                steps.push("pushed".to_string());
            }
            if steps.is_empty() {
                steps.push("up to date".to_string());
            }
            out!("{}: {}", status.branch, steps.join(", "));
        }
        Command::History { note, show, limit } => {
            let git = repository(&root)?;
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let history = git.history(&path, limit).map_err(|e| e.to_string())?;
            match show {
                Some(hash) => {
                    // A commit before a rename knows the note by its old path.
                    let revision = history.iter().find(|r| r.hash.starts_with(&hash)).ok_or(format!("no commit {hash} changed {path}"))?;
                    let text = git.content(&revision.path, &revision.hash).map_err(|e| e.to_string())?;
                    print!("{text}");
                }
                None => {
                    for revision in history {
                        let moved = if revision.path == path { String::new() } else { format!("\t{}", revision.path) };
                        out!("{}\t{}\t{}\t{}{moved}", &revision.hash[..12.min(revision.hash.len())], revision.date.format("%Y-%m-%d %H:%M"), revision.author, revision.subject);
                    }
                }
            }
        }
        Command::Capture { text, task } => {
            let typed = if text.is_empty() {
                let mut input = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).map_err(|e| e.to_string())?;
                input
            } else {
                text.join(" ")
            };
            let line = QuickCapture::line(&typed, task).ok_or("nothing to capture")?;
            let config = ConfigFile::new(fs::read_to_string(ConfigFile::default_path()).unwrap_or_default());
            let path = capture(&root, &config, &line)?;
            out!("{path}\t{line}");
        }
        Command::Status => {
            let status = repository(&root)?.status().map_err(|e| e.to_string())?;
            out!("{}\tahead {}\tbehind {}\tchanged {}", status.branch, status.ahead, status.behind, status.changed);
        }
        Command::RenameTag { old, new, dry_run } => {
            let index = Vault::index(&root, None);
            for path in vault_commands::rename_tag(&root, &index, &old, &new, dry_run)? {
                out!("{path}");
            }
        }
        Command::Replace { find, replacement, regex, match_case, whole_word, in_code, folder, dry_run } => {
            let index = Vault::index(&root, None);
            let options = ReplaceOptions { match_case, whole_word, regular_expression: regex, skip_code: !in_code };
            for line in vault_commands::replace(&root, &index, &find, &replacement, options, folder.as_deref(), dry_run)? {
                out!("{line}");
            }
        }
        Command::SortTasks { note, dry_run } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let moved = vault_commands::sort_tasks(&root, &path, dry_run)?;
            out!("{path}\t{}", if moved { "sorted" } else { "in order already" });
        }
        Command::Postpone { note, line, by } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let at = line.checked_sub(1).ok_or("lines count from 1")?;
            out!("{}", vault_commands::postpone(&root, &path, at, &by, Day::today())?);
        }
        Command::Daily { day, no_create } => {
            let index = Vault::index(&root, None);
            let day = match day {
                Some(text) => Day::parse(&text).ok_or(format!("{text} is not a day (YYYY-MM-DD)"))?,
                None => Day::today(),
            };
            out!("{}", vault_commands::daily(&root, &index.files, day, !no_create, Clock::now())?);
        }
        Command::Images { folder, dry_run, keep_originals } => {
            let folder = if folder.is_absolute() { folder } else { root.join(folder) };
            if !folder.is_dir() {
                return Err(format!("{} is not a folder", folder.display()));
            }
            images::run(&root, &folder, &images::Options { dry_run, keep_originals }).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn emit(line: std::fmt::Arguments) {
    use std::io::Write;
    if let Err(error) = writeln!(std::io::stdout().lock(), "{line}") {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("daftar-cli: {error}");
        }
        std::process::exit(if error.kind() == std::io::ErrorKind::BrokenPipe { 0 } else { 1 });
    }
}

/// A search snippet on one line, its matches in bold on a terminal; the
/// index marks them with `\u{1}` and `\u{2}`.
fn snippet(text: &str, terminal: bool) -> String {
    let (open, close) = if terminal { ("\x1b[1m", "\x1b[0m") } else { ("", "") };
    text.replace('\n', " ").replace('\u{1}', open).replace('\u{2}', close)
}

fn side(side: Side) -> &'static str {
    match side {
        Side::Changed => "changed",
        Side::Added => "added",
        Side::Deleted => "deleted",
    }
}

/// Adds [line] to the capture note of [config], made first when it is not
/// there: the daily note from its template, a named note empty. Never
/// outside the vault. The note's vault path.
fn capture(root: &std::path::Path, config: &ConfigFile, line: &str) -> Result<String, String> {
    let now = chrono::Local::now();
    let target = config.string("capture.target").unwrap_or_else(|| "daily".into());
    let mut path = QuickCapture::target_path(root, &target, now);
    // "inbox" is Inbox.md, as the app finds it.
    if !root.join(&path).exists()
        && let Some(found) = Vault::index(root, None).files.iter().find(|file| file.to_lowercase() == path.to_lowercase())
    {
        path = found.clone();
    }
    if path.split('/').any(|part| part.is_empty() || part == "." || part == "..") || path.starts_with('/') {
        return Err(format!("{path} is not a note in the vault"));
    }
    let file = root.join(&path);
    if !file.exists() {
        let daily = target.trim().is_empty() || target.trim().eq_ignore_ascii_case("daily");
        let title = Vault::title(&path);
        let first = if daily { DailyNotes::read(root).first_text(root, &title, now) } else { String::new() };
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).map_err(|e| format!("{path}: {e}"))?;
        }
        fs::write(&file, first).map_err(|e| format!("{path}: {e}"))?;
    }
    let heading = config.string("capture.heading").unwrap_or_default();
    let heading = Some(heading.trim()).filter(|h| !h.is_empty());
    VaultEdits::apply(&path, root, |text| Ok(QuickCapture::append(line, text, heading))).map_err(|e| e.to_string())?;
    Ok(path)
}

fn repository(root: &std::path::Path) -> Result<Git, String> {
    let git = Git::new(root);
    if git.is_repository() { Ok(git) } else { Err(format!("{} is not a git repository", root.display())) }
}

/// The vault given, else the one the current folder is in (the nearest
/// folder up with `.obsidian`, as git finds its repository), else the app's.
fn vault(given: Option<PathBuf>) -> Result<PathBuf, String> {
    let root = match given {
        Some(root) => root,
        None => match std::env::current_dir().ok().as_deref().and_then(vault_around) {
            Some(root) => root,
            None => {
                let text = fs::read_to_string(ConfigFile::default_path()).unwrap_or_default();
                ConfigFile::new(text)
                    .vault()
                    .ok_or("no vault: run it in one, pass --vault, or set `vault` in ~/.config/daftar/config.toml")?
            }
        },
    };
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    Ok(root)
}

/// The vault [folder] is in: itself or the nearest folder up with `.obsidian`.
fn vault_around(folder: &std::path::Path) -> Option<PathBuf> {
    folder.ancestors().find(|f| f.join(".obsidian").is_dir()).map(PathBuf::from)
}

/// A note named as a path in the vault, or as a link would name it.
/// `path:line`, a tab, the status and the text, and the dates that matter.
fn task_line(task: &VaultTask) -> String {
    let t = &task.task;
    let mut line = format!("{}:{}\t[{}] {}", task.path, t.line + 1, t.status, t.text);
    for (mark, value) in [("⏳", &t.scheduled), ("📅", &t.due), ("✅", &t.done), ("🔁", &t.recurring)] {
        if let Some(value) = value {
            line.push_str(&format!(" {mark} {value}"));
        }
    }
    line
}

/// The bodies of a note's ` ```tasks ` blocks, in a callout or a quote too.
fn query_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        let mut line = line.trim_start();
        while let Some(rest) = line.strip_prefix('>') {
            line = rest.strip_prefix(' ').unwrap_or(rest).trim_start_matches(' ');
        }
        let trimmed = line.trim();
        match &mut current {
            None if trimmed == "```tasks" => current = Some(Vec::new()),
            Some(body) if trimmed.starts_with("```") => {
                blocks.push(body.join("\n"));
                current = None;
            }
            Some(body) => body.push(line),
            None => {}
        }
    }
    blocks
}

fn note_path(index: &VaultIndex, note: &str) -> Result<String, String> {
    if index.notes.contains_key(note) {
        return Ok(note.to_string());
    }
    let target = note.strip_suffix(".md").unwrap_or(note);
    index.resolve(target, "").filter(|p| Vault::is_note(p)).ok_or(format!("no note {note}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Mac's `CaptureTests`, through the command line's capture.
    #[test]
    fn a_capture_goes_to_todays_note_made_from_its_template() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("Templates")).unwrap();
        fs::write(root.join(".obsidian/daily-notes.json"), r#"{"format":"YYYY-MM-DD","folder":"Journal","template":"Templates/Day"}"#).unwrap();
        fs::write(root.join("Templates/Day.md"), "# {{title}}\n## Inbox\n\n## Log\n").unwrap();
        fs::write(root.join("Inbox.md"), "# Inbox\n- [ ] Old\n").unwrap();
        let today = Day::today().to_string();

        let config = ConfigFile::new("[capture]\ntarget = \"daily\"\nheading = \"Inbox\"\n".to_string());
        let path = capture(root, &config, "- [ ] Call Sara").unwrap();
        assert_eq!(path, format!("Journal/{today}.md"));
        assert_eq!(fs::read_to_string(root.join(&path)).unwrap(), format!("# {today}\n## Inbox\n- [ ] Call Sara\n\n## Log\n"));
        // The file is read again: a change made on disk meanwhile stays.
        fs::write(root.join(&path), "# Today\n## Inbox\n- [ ] Call Sara\n- [ ] Written elsewhere\n").unwrap();
        let config = ConfigFile::new("[capture]\ntarget = \"daily\"\n".to_string());
        capture(root, &config, "Idea").unwrap();
        assert_eq!(fs::read_to_string(root.join(&path)).unwrap(), "# Today\n## Inbox\n- [ ] Call Sara\n- [ ] Written elsewhere\n\nIdea\n");

        // A named note, found without case; one not there yet is made.
        let config = ConfigFile::new("[capture]\ntarget = \"inbox\"\n".to_string());
        assert_eq!(capture(root, &config, "- [ ] New").unwrap(), "Inbox.md");
        assert_eq!(fs::read_to_string(root.join("Inbox.md")).unwrap(), "# Inbox\n- [ ] Old\n- [ ] New\n");
        let config = ConfigFile::new("[capture]\ntarget = \"Notes/Later.md\"\nheading = \"Ideas\"\n".to_string());
        assert_eq!(capture(root, &config, "x").unwrap(), "Notes/Later.md");
        assert_eq!(fs::read_to_string(root.join("Notes/Later.md")).unwrap(), "## Ideas\nx\n");
        // Never outside the vault.
        let config = ConfigFile::new("[capture]\ntarget = \"../escaped\"\n".to_string());
        assert!(capture(root, &config, "x").is_err());
        assert!(!root.parent().unwrap().join("escaped.md").exists());
    }

    #[test]
    fn a_notes_tasks_blocks_are_its_queries() {
        let text = "# Week\n\n```tasks\nnot done\ndue this week\n```\n\n```js\nx\n```\n```tasks\ndone\n```\n";
        assert_eq!(query_blocks(text), ["not done\ndue this week", "done"]);
        let callout = "> [!todo] Now\n>\n> ```tasks\n> not done\n> group by filename\n> ```\n";
        assert_eq!(query_blocks(callout), ["not done\ngroup by filename"]);
    }

    #[test]
    fn the_vault_is_found_from_a_folder_in_it() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::create_dir_all(root.path().join("Notes/Deep")).unwrap();
        assert_eq!(vault_around(&root.path().join("Notes/Deep")).as_deref(), Some(root.path()));
        assert_eq!(vault_around(root.path()).as_deref(), Some(root.path()));
    }

    #[test]
    fn a_snippet_is_bold_only_on_a_terminal() {
        assert_eq!(snippet("a \u{1}plan\u{2}\nnext", false), "a plan next");
        assert_eq!(snippet("a \u{1}plan\u{2}", true), "a \x1b[1mplan\x1b[0m");
    }

    #[test]
    fn a_note_is_found_by_path_or_by_name() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("Notes")).unwrap();
        fs::write(root.path().join("Notes/Inbox.md"), "- [ ] One\n").unwrap();
        let index = Vault::index(root.path(), None);
        assert_eq!(note_path(&index, "Notes/Inbox.md").unwrap(), "Notes/Inbox.md");
        assert_eq!(note_path(&index, "Inbox").unwrap(), "Notes/Inbox.md");
        assert!(note_path(&index, "Missing").is_err());
    }
}
