//! The vault's files, watched, as the app's `watcher.rs` does: vault paths,
//! only ones the index reads, gathered until the vault is quiet, so a `git
//! pull` of a hundred files is one update. The TUI's loop asks with
//! [Watch::poll]; `check --watch` and `lint --watch` wait with [Watch::wait]
//! in [run], which clears the terminal and shows each pass under a header.

use std::collections::BTreeSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use daftar_core::Vault;
use daftar_core::text::slugs;

/// How long the vault stays quiet before a change is reported, and the
/// longest a stream of changes waits.
const QUIET: Duration = Duration::from_millis(300);
const LONGEST: Duration = Duration::from_secs(2);

pub struct Watch {
    // Watches while it lives.
    _watcher: notify::RecommendedWatcher,
    receiver: Receiver<PathBuf>,
    base: PathBuf,
    pub debounce: Debounce,
}

impl Watch {
    pub fn new(root: &Path) -> notify::Result<Watch> {
        let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let (sender, receiver) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event
                && !matches!(event.kind, notify::EventKind::Access(_))
            {
                for path in event.paths {
                    let _ = sender.send(path);
                }
            }
        })?;
        watcher.watch(&base, RecursiveMode::Recursive)?;
        Ok(Watch { _watcher: watcher, receiver, base, debounce: Debounce::new(QUIET, LONGEST) })
    }

    /// The paths that changed, once the vault is quiet.
    pub fn poll(&mut self, now: Instant) -> Option<BTreeSet<String>> {
        while let Ok(path) = self.receiver.try_recv() {
            if let Some(path) = relative(&self.base, &path) {
                self.debounce.add(path, now);
            }
        }
        self.debounce.due(now)
    }

    /// Blocks until paths changed and the vault is quiet again, and gives
    /// them; `None` when the watcher stopped.
    pub fn wait(&mut self) -> Option<BTreeSet<String>> {
        loop {
            // Asleep until something comes, then awake to see the vault quiet.
            let timeout = if self.debounce.waiting() { Duration::from_millis(50) } else { Duration::from_secs(3600) };
            match self.receiver.recv_timeout(timeout) {
                Ok(path) => {
                    if let Some(path) = relative(&self.base, &path) {
                        self.debounce.add(path, Instant::now());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
            if let Some(changed) = self.debounce.due(Instant::now()) {
                return Some(changed);
            }
        }
    }
}

/// What one pass of a watched command found: its summary for the header
/// and its lines.
pub struct Pass {
    pub summary: String,
    pub lines: Vec<String>,
}

/// Runs [pass] on the vault at [root], first with no paths (the whole of
/// what it covers), then with the paths that changed each time the vault
/// is quiet after a change, until Ctrl+C. On a terminal each pass clears
/// the screen first.
pub fn run(root: &Path, mut pass: impl FnMut(&BTreeSet<String>) -> Pass) -> Result<(), String> {
    let mut watch = Watch::new(root).map_err(|e| format!("cannot watch {}: {e}", root.display()))?;
    let clear = std::io::stdout().is_terminal();
    let mut changed = BTreeSet::new();
    loop {
        let shown = pass(&changed);
        if clear {
            print!("\x1b[2J\x1b[H");
        }
        out!("{}", header(&chrono::Local::now().format("%H:%M:%S").to_string(), &shown.summary));
        for line in shown.lines {
            out!("{line}");
        }
        changed = watch.wait().ok_or("the watcher stopped")?;
    }
}

/// The line above a pass: when it ran and what it found.
pub fn header(time: &str, summary: &str) -> String {
    format!("[{time}] {summary} (watching; Ctrl+C stops)")
}

/// Changes gathered until none came for [quiet], or for [longest] at most.
#[derive(Debug)]
pub struct Debounce {
    quiet: Duration,
    longest: Duration,
    pending: BTreeSet<String>,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debounce {
    pub fn new(quiet: Duration, longest: Duration) -> Debounce {
        Debounce { quiet, longest, pending: BTreeSet::new(), first: None, last: None }
    }

    pub fn add(&mut self, path: String, now: Instant) {
        self.pending.insert(path);
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// Whether a change waits, so the loop wakes for it.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn due(&mut self, now: Instant) -> Option<BTreeSet<String>> {
        let (first, last) = (self.first?, self.last?);
        if now.duration_since(last) < self.quiet && now.duration_since(first) < self.longest {
            return None;
        }
        self.first = None;
        self.last = None;
        Some(std::mem::take(&mut self.pending))
    }
}

/// [path] as a vault path, when it is in [base] and the index reads it.
fn relative(base: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(base).ok()?.to_str()?;
    let relative = slugs::normalize(&rest.replace('\\', "/"));
    (!relative.is_empty() && Vault::counts(&relative)).then_some(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn changes_wait_until_the_vault_is_quiet() {
        let start = Instant::now();
        let mut debounce = Debounce::new(ms(300), ms(2000));
        assert_eq!(debounce.due(start), None);
        debounce.add("A.md".into(), start);
        debounce.add("B.md".into(), start + ms(100));
        debounce.add("A.md".into(), start + ms(200));
        assert!(debounce.waiting());
        assert_eq!(debounce.due(start + ms(400)), None);
        let due = debounce.due(start + ms(500)).unwrap();
        assert_eq!(due.into_iter().collect::<Vec<_>>(), ["A.md", "B.md"]);
        assert!(!debounce.waiting());
        assert_eq!(debounce.due(start + ms(900)), None);
    }

    #[test]
    fn a_stream_of_changes_is_reported_after_the_longest_wait() {
        let start = Instant::now();
        let mut debounce = Debounce::new(ms(300), ms(2000));
        let mut reported = Vec::new();
        // A change every 100 ms: never quiet for 300.
        for n in 0..30 {
            debounce.add(format!("{n}.md"), start + ms(n * 100));
            if let Some(due) = debounce.due(start + ms(n * 100)) {
                reported.push((n, due.len()));
            }
        }
        assert_eq!(reported, [(20, 21)]);
        // The rest start a wait of their own.
        assert_eq!(debounce.due(start + ms(2950)), None);
        assert_eq!(debounce.due(start + ms(3200)).map(|d| d.len()), Some(9));
    }

    #[test]
    fn the_header_says_when_and_what() {
        assert_eq!(header("14:02:09", "2 problems in 1 of 3 notes"), "[14:02:09] 2 problems in 1 of 3 notes (watching; Ctrl+C stops)");
    }

    #[test]
    fn a_change_on_disk_comes_out_of_wait() {
        let root = tempfile::tempdir().unwrap();
        let mut watch = Watch::new(root.path()).unwrap();
        std::fs::write(root.path().join("A.md"), "# A\n").unwrap();
        let changed = watch.wait().unwrap();
        assert!(changed.contains("A.md"), "{changed:?}");
    }

    #[test]
    fn only_the_files_the_index_reads_are_reported() {
        let base = Path::new("/vault");
        assert_eq!(relative(base, Path::new("/vault/Notes/Inbox.md")).as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(relative(base, Path::new("/vault/.git/index")), None);
        assert_eq!(relative(base, Path::new("/vault/.obsidian/workspace.json")), None);
        assert_eq!(relative(base, Path::new("/elsewhere/a.md")), None);
        assert_eq!(relative(base, Path::new("/vault")), None);
    }
}
