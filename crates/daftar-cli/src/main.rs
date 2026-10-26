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

use std::collections::BTreeSet;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use daftar_core::{Candidate, ConfigFile, Day, Git, QuickSwitcher, SearchIndex, TaskLine, Vault, VaultEdits, VaultIndex};

#[derive(Parser)]
#[command(name = "daftar-cli", version, about = "Daftar on the command line: an Obsidian vault in a Git repository")]
struct Cli {
    /// The vault's folder; else `vault` in ~/.config/daftar/config.toml.
    #[arg(long, global = true, env = "DAFTAR_VAULT")]
    vault: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
    Sync {
        /// The commit message; else `update notes @ <time> on <host>`.
        #[arg(short, long)]
        message: Option<String>,
    },
    /// The branch, how far it is from its remote, and how many files changed.
    Status,
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
    let root = vault(cli.vault)?;
    match cli.command {
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
                let t = &task.task;
                let mut line = format!("{}:{}\t[{}] {}", task.path, t.line + 1, t.status, t.text);
                for (mark, value) in [("⏳", &t.scheduled), ("📅", &t.due), ("🔁", &t.recurring)] {
                    if let Some(value) = value {
                        line.push_str(&format!(" {mark} {value}"));
                    }
                }
                out!("{line}");
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
        Command::Sync { message } => {
            let git = repository(&root)?;
            let message = message.unwrap_or_else(|| Git::message(chrono::Local::now(), &Git::host()));
            let done = git.sync(&message).map_err(|e| e.to_string())?;
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
        Command::Status => {
            let status = repository(&root)?.status().map_err(|e| e.to_string())?;
            out!("{}\tahead {}\tbehind {}\tchanged {}", status.branch, status.ahead, status.behind, status.changed);
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

fn repository(root: &std::path::Path) -> Result<Git, String> {
    let git = Git::new(root);
    if git.is_repository() { Ok(git) } else { Err(format!("{} is not a git repository", root.display())) }
}

/// The vault given, else the app's.
fn vault(given: Option<PathBuf>) -> Result<PathBuf, String> {
    let root = match given {
        Some(root) => root,
        None => {
            let text = fs::read_to_string(ConfigFile::default_path()).unwrap_or_default();
            ConfigFile::new(text).vault().ok_or("no vault: pass --vault, or set `vault` in ~/.config/daftar/config.toml")?
        }
    };
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    Ok(root)
}

/// A note named as a path in the vault, or as a link would name it.
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
