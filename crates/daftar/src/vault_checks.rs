//! What `just audit` and `just check` report, inside the app, and what
//! `just fix` mends (`docs/specs/problems-and-format.md`): `daftar-cli
//! check`, `lint` and `fmt --check` with `--json`, run in the background when
//! the vault opens and a moment after files change; and format on save,
//! `daftar-cli fmt --stdin`. One per window. At most one audit runs at a
//! time: changes while one runs ask for one more after it.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use daftar_core::{FormatMoment, NoteFormat, Problem, ProblemCounts, ProblemFilter, ProblemNote, ProblemTool, Problems, Vault};
use gtk::prelude::*;
use gtk::{gio, glib};

use crate::daftar_cli::{self, Output};
use crate::editor::{Saved, write_if_unchanged};
use crate::store::file_in;

/// How long after the last change of a file the audit runs again.
const SETTLE: Duration = Duration::from_secs(2);
/// Past this many notes, one run over the vault is cheaper than a list.
const MOST_NOTES: usize = 500;
/// How long a format on leaving may take; Quit waits for it no longer.
const FORMAT_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Never run for this vault.
    Idle,
    /// The findings are those of the last audit that finished.
    Done,
    /// No `daftar-cli` beside the app or on `$PATH`.
    Missing,
    /// A `daftar-cli` before 0.23.0, which has no `--json`.
    TooOld,
    Failed(String),
}

/// What the next audit has to read again. `lint` and `fmt` find what is in a
/// note alone, so after a save only the notes saved are linted and checked
/// for formatting, and their findings replace those they had; `check` reads
/// links between notes and always reads the whole vault. A whole-vault `fmt
/// --check` keeps every core busy for a second; the note alone takes 10 ms.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    Nothing,
    Notes(BTreeSet<String>),
    Vault,
}

impl Scope {
    fn adding(self, other: Scope) -> Scope {
        match (self, other) {
            (Scope::Vault, _) | (_, Scope::Vault) => Scope::Vault,
            (Scope::Nothing, other) | (other, Scope::Nothing) => other,
            (Scope::Notes(mut a), Scope::Notes(b)) => {
                a.extend(b);
                Scope::Notes(a)
            }
        }
    }
}

/// Hears the notes the app wrote.
type Wrote = Box<dyn Fn(&[String])>;

/// What the window does for the checks.
pub struct Hooks {
    /// Every open editor saves, so a command reads what was typed.
    pub save_editors: Box<dyn Fn()>,
    pub notice: Box<dyn Fn(&str)>,
    /// The findings or the state changed: the panel, the count and the margins.
    pub changed: Box<dyn Fn()>,
    /// The app wrote these notes: they are not "updated since you read", and
    /// the sync knows.
    pub wrote: Wrote,
    /// `editor.format_on_save`.
    pub format_on_save: Box<dyn Fn() -> bool>,
}

/// A note an editor let go, and the text it had when the editor took it.
struct Left {
    path: String,
    loaded: String,
}

pub struct VaultChecks {
    root: RefCell<Option<PathBuf>>,
    cancel: RefCell<gio::Cancellable>,
    problems: RefCell<Rc<Vec<Problem>>>,
    by_path: RefCell<BTreeMap<String, Vec<Problem>>>,
    state: RefCell<State>,
    running: Cell<bool>,
    again: Cell<bool>,
    /// The panel asked for the audit that starts next: it runs at once, unniced.
    asking: Cell<bool>,
    /// The panel asked for an audit since the vault opened: a `daftar-cli`
    /// missing or too old is said once, as a notice.
    asked: Cell<bool>,
    said: Cell<bool>,
    scope: RefCell<Scope>,
    soon: RefCell<Option<glib::SourceId>>,
    fixing: Cell<bool>,
    /// A `daftar-cli` that said it has no `fmt --stdin`: format on save does
    /// not ask it again at every leave, until the panel checks again.
    old_formatter: RefCell<Option<PathBuf>>,
    /// Notes left, formatted one after the other.
    formats: RefCell<VecDeque<Left>>,
    formatting: Cell<bool>,
    hooks: RefCell<Option<Rc<Hooks>>>,
    /// Where `daftar-cli` is; a test gives its own.
    locate: RefCell<Rc<dyn Fn() -> Option<PathBuf>>>,
    /// How long after the last change of a file the audit runs again.
    settle: Cell<Duration>,
}

impl VaultChecks {
    pub fn new() -> Rc<VaultChecks> {
        Rc::new(VaultChecks {
            root: RefCell::new(None),
            cancel: RefCell::new(gio::Cancellable::new()),
            problems: RefCell::default(),
            by_path: RefCell::default(),
            state: RefCell::new(State::Idle),
            running: Cell::new(false),
            again: Cell::new(false),
            asking: Cell::new(false),
            asked: Cell::new(false),
            said: Cell::new(false),
            scope: RefCell::new(Scope::Nothing),
            soon: RefCell::new(None),
            fixing: Cell::new(false),
            old_formatter: RefCell::new(None),
            formats: RefCell::default(),
            formatting: Cell::new(false),
            hooks: RefCell::new(None),
            locate: RefCell::new(Rc::new(daftar_cli::locate)),
            settle: Cell::new(SETTLE),
        })
    }

    pub fn set_hooks(&self, hooks: Hooks) {
        *self.hooks.borrow_mut() = Some(Rc::new(hooks));
    }

    fn cli(&self) -> Option<PathBuf> {
        let locate = self.locate.borrow().clone();
        locate()
    }

    fn hooks(&self) -> Option<Rc<Hooks>> {
        self.hooks.borrow().clone()
    }

    fn tell(&self) {
        if let Some(hooks) = self.hooks() {
            (hooks.changed)();
        }
    }

    fn notice(&self, text: &str) {
        if let Some(hooks) = self.hooks() {
            (hooks.notice)(text);
        }
    }

    // What the panel shows

    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// An audit runs, perhaps over the findings of the last one.
    pub fn running(&self) -> bool {
        self.running.get()
    }

    /// A fix runs; the Fix buttons wait for it.
    pub fn fixing(&self) -> bool {
        self.fixing.get()
    }

    pub fn counts(&self) -> ProblemCounts {
        Problems::counts(&self.problems.borrow())
    }

    pub fn grouped(&self, filter: ProblemFilter) -> Vec<ProblemNote> {
        Problems::grouped(&self.problems.borrow(), filter)
    }

    /// The findings of [path] by source line, for the editor's margin.
    pub fn lines(&self, path: &str) -> BTreeMap<usize, Vec<Problem>> {
        self.by_path.borrow().get(path).map_or_default(|found| Problems::lines(found, path))
    }

    /// What the panel or a notice says about a state that has no findings.
    pub fn explanation(state: &State) -> Option<String> {
        match state {
            State::Missing => Some(daftar_cli::MISSING.into()),
            State::TooOld => Some(daftar_cli::UPGRADE.into()),
            State::Failed(why) => Some(format!("The audit failed: {why}")),
            State::Idle | State::Done => None,
        }
    }

    fn show(&self, found: Vec<Problem>) {
        let mut by_path: BTreeMap<String, Vec<Problem>> = BTreeMap::new();
        for problem in &found {
            by_path.entry(problem.path.clone()).or_default().push(problem.clone());
        }
        *self.by_path.borrow_mut() = by_path;
        *self.problems.borrow_mut() = Rc::new(found);
    }

    // The audit

    /// A vault opened: what was found in the last one goes. A vault that
    /// keeps `daftar-cli`'s settings is audited at once, in the background;
    /// any other only once the panel asks.
    pub fn opened(self: &Rc<Self>, root: &Path) {
        if let Some(soon) = self.soon.take() {
            soon.remove();
        }
        self.cancel.replace(gio::Cancellable::new()).cancel();
        self.running.set(false);
        self.again.set(false);
        self.asked.set(false);
        self.said.set(false);
        *self.old_formatter.borrow_mut() = None;
        *self.root.borrow_mut() = Some(root.to_path_buf());
        self.show(Vec::new());
        *self.state.borrow_mut() = State::Idle;
        *self.scope.borrow_mut() = Scope::Vault;
        self.tell();
        if daftar_cli::uses_cli(root) {
            self.schedule(Duration::from_millis(500));
        }
    }

    /// Files changed, by a save, a fix or a pull: the audit runs again once
    /// they settle, when it has run before and could.
    pub fn changed(self: &Rc<Self>, paths: &BTreeSet<String>) {
        let scope = self.scope.replace(Scope::Nothing);
        *self.scope.borrow_mut() = scope.adding(Scope::Notes(paths.clone()));
        if matches!(*self.state.borrow(), State::Done | State::Failed(_)) || self.running.get() {
            self.schedule(self.settle.get());
        }
    }

    /// The panel opened, or Check Again: now, or after the audit that runs.
    pub fn refresh(self: &Rc<Self>) {
        self.asked.set(true);
        *self.old_formatter.borrow_mut() = None;
        *self.scope.borrow_mut() = Scope::Vault;
        self.asking.set(true);
        if let Some(soon) = self.soon.take() {
            soon.remove();
        }
        self.start();
    }

    fn schedule(self: &Rc<Self>, delay: Duration) {
        if let Some(soon) = self.soon.take() {
            soon.remove();
        }
        let this = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(delay, move || {
            if let Some(this) = this.upgrade() {
                this.soon.take();
                this.start();
            }
        });
        *self.soon.borrow_mut() = Some(id);
    }

    fn start(self: &Rc<Self>) {
        if self.running.get() {
            self.again.set(true);
            return;
        }
        let Some(root) = self.root.borrow().clone() else { return };
        self.running.set(true);
        self.tell();
        let (this, cancel) = (self.clone(), self.cancel.borrow().clone());
        glib::spawn_future_local(async move {
            this.audit(&root, &cancel).await;
            if cancel.is_cancelled() {
                return;
            }
            this.running.set(false);
            this.tell();
            if this.again.replace(false) {
                this.start();
            }
        });
    }

    async fn audit(&self, root: &Path, cancel: &gio::Cancellable) {
        let Some(cli) = self.cli() else {
            *self.state.borrow_mut() = State::Missing;
            self.show(Vec::new());
            self.announce();
            return;
        };
        // Only notes saved since an audit of the whole vault are read alone.
        let mut taken = self.scope.replace(Scope::Nothing);
        if *self.state.borrow() != State::Done {
            taken = Scope::Vault;
        }
        let notes: Option<Vec<String>> = match &taken {
            Scope::Notes(paths) if paths.len() <= MOST_NOTES => Some(lintable(paths, root)),
            Scope::Nothing => Some(Vec::new()),
            _ => None,
        };
        // The panel waits for its audit; the one after a save is background work.
        let background = !self.asking.replace(false);
        let mut found = Vec::new();
        let mut finished = true;
        for arguments in daftar_cli::audit_arguments(root, daftar_cli::has_audit_profile(root), notes.as_deref()) {
            let output = match daftar_cli::run(&cli, &arguments, root, None, background, cancel).await {
                Ok(output) => output,
                Err(_) if cancel.is_cancelled() => return,
                Err(error) => {
                    *self.state.borrow_mut() = State::Failed(error);
                    finished = false;
                    break;
                }
            };
            if output.too_old() {
                *self.state.borrow_mut() = State::TooOld;
                self.show(Vec::new());
                self.announce();
                finished = false;
                break;
            }
            let parsed = Problems::parse(&output.stdout);
            // `check` fails when it finds an error: that is a finding, not a failure.
            if !output.succeeded() && parsed.is_empty() && !output.stderr.trim().is_empty() {
                *self.state.borrow_mut() = State::Failed(daftar_cli::failure(&output.stderr));
                finished = false;
                break;
            }
            found.extend(parsed);
        }
        if !finished {
            // A run that did not finish leaves the whole vault to read again.
            let scope = self.scope.replace(Scope::Nothing);
            *self.scope.borrow_mut() = scope.adding(Scope::Vault);
            return;
        }
        if let Some(notes) = &notes {
            // The notes read again lose the findings they had; `check` is new in full.
            let read: BTreeSet<&String> = notes.iter().collect();
            found.extend(self.problems.borrow().iter().filter(|p| p.tool != ProblemTool::Check && !read.contains(&p.path)).cloned());
            // A note named that is gone loses its findings too.
            if let Scope::Notes(paths) = &taken {
                found.retain(|p| p.tool == ProblemTool::Check || !paths.contains(&p.path) || read.contains(&p.path));
            }
        }
        self.show(found);
        *self.state.borrow_mut() = State::Done;
    }

    /// Once for the vault, after the panel asked.
    fn announce(&self) {
        if !self.asked.get() || self.said.replace(true) {
            return;
        }
        if let Some(text) = Self::explanation(&self.state()) {
            self.notice(&text);
        }
    }

    // Fixes

    /// The Fix of one finding: its tool's fix of its note.
    pub fn fix(self: &Rc<Self>, problem: &Problem) {
        self.run_fixes(&[problem.tool], vec![problem.path.clone()]);
    }

    /// Fix All of a note, or of the vault with `None`: what `just fix` does.
    pub fn fix_all(self: &Rc<Self>, path: Option<&str>) {
        self.run_fixes(&ProblemTool::ALL, path.map_or_default(|p| vec![p.to_string()]));
    }

    /// The editors save first, so the fix reads what was typed and no editor
    /// writes its older text over it; the notes are read again after, and an
    /// editor with nothing unsaved takes them.
    fn run_fixes(self: &Rc<Self>, tools: &[ProblemTool], paths: Vec<String>) {
        let (Some(root), Some(hooks)) = (self.root.borrow().clone(), self.hooks()) else { return };
        if self.fixing.get() {
            return;
        }
        let Some(cli) = self.cli() else {
            self.notice(daftar_cli::MISSING);
            return;
        };
        self.fixing.set(true);
        self.tell();
        (hooks.save_editors)();
        let (this, tools, cancel) = (self.clone(), tools.to_vec(), self.cancel.borrow().clone());
        glib::spawn_future_local(async move {
            for arguments in daftar_cli::fix_arguments(&root, &tools, &paths) {
                let output = match daftar_cli::run(&cli, &arguments, &root, None, false, &cancel).await {
                    Ok(output) => output,
                    Err(_) if cancel.is_cancelled() => return,
                    Err(error) => {
                        this.notice(&format!("The fix did not run: {error}"));
                        break;
                    }
                };
                if output.too_old() || output.refuses_paths(&paths) {
                    this.notice(daftar_cli::UPGRADE);
                    break;
                }
                if output.usage_error().is_some() {
                    this.notice(&daftar_cli::failure(&output.stderr));
                    break;
                }
            }
            this.fixing.set(false);
            (hooks.wrote)(&paths);
            this.refresh();
        });
    }

    // Format

    /// [text] formatted as the note at [path]: `daftar-cli fmt --stdin`,
    /// which writes nothing. `Err` says why not, `None` in it when nothing
    /// is to be said (a format on leaving is silent).
    async fn formatted(&self, text: String, path: &str, limit: Option<Duration>) -> Result<String, String> {
        let root = self.root.borrow().clone().ok_or("No vault is open.")?;
        let cli = self.cli().ok_or(daftar_cli::MISSING)?;
        let cancel = gio::Cancellable::new();
        let timer = limit.map(|limit| {
            let cancel = cancel.clone();
            glib::timeout_add_local_once(limit, move || cancel.cancel())
        });
        let result = daftar_cli::run(&cli, &daftar_cli::format_arguments(&root, path), &root, Some(text), false, &cancel).await;
        if let Some(timer) = timer
            && !cancel.is_cancelled()
        {
            timer.remove();
        }
        let output: Output = result?;
        if output.too_old() {
            *self.old_formatter.borrow_mut() = Some(cli);
            return Err(daftar_cli::UPGRADE.into());
        }
        if !output.succeeded() {
            return Err(format!("The note was not formatted: {}", daftar_cli::failure(&output.stderr)));
        }
        Ok(output.stdout)
    }

    /// Edit › Format Note on text an editor holds: the formatted text, or
    /// `None` once a notice has said why not.
    pub async fn formatted_for_command(&self, text: String, path: &str) -> Option<String> {
        match self.formatted(text, path, None).await {
            Ok(out) => Some(out),
            Err(why) => {
                self.notice(&why);
                None
            }
        }
    }

    /// Format on save: an editor let go of [path], whose text it took was
    /// [loaded]. Its text is on disk already. The file is formatted, when
    /// the setting is on and it was typed in, after any note left before it.
    /// With no `daftar-cli`, or one too old, nothing happens and nothing is
    /// said. The app stays open for it, so Quit waits.
    pub fn format_on_leave(self: &Rc<Self>, path: String, loaded: String, moment: FormatMoment) {
        let Some(hooks) = self.hooks() else { return };
        if self.root.borrow().is_none() || !NoteFormat::formats(moment, (hooks.format_on_save)(), &path, true) {
            return;
        }
        self.formats.borrow_mut().push_back(Left { path, loaded });
        if self.formatting.replace(true) {
            return;
        }
        let this = self.clone();
        let hold = gio::Application::default().map(|app| app.hold());
        glib::spawn_future_local(async move {
            loop {
                let next = this.formats.borrow_mut().pop_front();
                let Some(left) = next else { break };
                this.format_file(left).await;
            }
            this.formatting.set(false);
            drop(hold);
        });
    }

    async fn format_file(&self, left: Left) {
        let (Some(root), Some(hooks)) = (self.root.borrow().clone(), self.hooks()) else { return };
        let Some(file) = file_in(&left.path, &root) else { return };
        let Ok(text) = std::fs::read_to_string(&file) else { return };
        if !NoteFormat::formats(FormatMoment::Closed, true, &left.path, text != left.loaded) {
            return;
        }
        if self.old_formatter.borrow().is_some() && *self.old_formatter.borrow() == self.cli() {
            return;
        }
        let Ok(out) = self.formatted(text.clone(), &left.path, Some(FORMAT_LIMIT)).await else { return };
        if out == text || out.is_empty() {
            return;
        }
        // Typed again since, or written by another app: not this format's to change.
        (hooks.save_editors)();
        match write_if_unchanged(&file, &text, &out) {
            Saved::Written => (hooks.wrote)(std::slice::from_ref(&left.path)),
            Saved::Failed(error) => self.notice(&format!("{} could not be formatted: {error}", Vault::title(&left.path))),
            Saved::ChangedOnDisk | Saved::Nothing => {}
        }
    }

    /// Format Note in the reader: the file, through the editors' gate.
    pub fn format_file_now(self: &Rc<Self>, path: String) {
        let (Some(root), Some(hooks)) = (self.root.borrow().clone(), self.hooks()) else { return };
        let Some(file) = file_in(&path, &root) else { return };
        let this = self.clone();
        glib::spawn_future_local(async move {
            (hooks.save_editors)();
            let Ok(text) = std::fs::read_to_string(&file) else { return };
            let Some(out) = this.formatted_for_command(text.clone(), &path).await else { return };
            let title = Vault::title(&path);
            if out == text {
                this.notice(&format!("{title} is formatted already."));
                return;
            }
            (hooks.save_editors)();
            match write_if_unchanged(&file, &text, &out) {
                Saved::Written => (hooks.wrote)(std::slice::from_ref(&path)),
                Saved::ChangedOnDisk | Saved::Nothing => {
                    this.notice(&format!("{title} changed while it was formatted. Nothing was written."));
                }
                Saved::Failed(error) => this.notice(&format!("{title} could not be formatted: {error}")),
            }
        });
    }
}

/// The notes of [paths] that `lint` and `fmt` read in a run over the vault:
/// notes that are there. A path that is not there fails the whole command,
/// so a deleted note is left out (and loses its findings).
fn lintable(paths: &BTreeSet<String>, root: &Path) -> Vec<String> {
    paths.iter().filter(|path| Vault::is_note(path) && file_in(path, root).is_some_and(|file| file.is_file())).cloned().collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Instant;

    use super::*;

    /// A stand-in `daftar-cli` in a vault of its own: it writes each call to
    /// `calls`, finds what a vault with a callout, spaces at a line's end and
    /// an unformatted note has, fixes by writing `fixed`, and formats `*  `
    /// as `- `. An old one knows none of the new flags.
    pub(crate) struct Fake {
        pub(crate) vault: tempfile::TempDir,
        /// Holds the stand-in until the test ends.
        _bin: tempfile::TempDir,
        pub(crate) checks: Rc<VaultChecks>,
        pub(crate) notices: Rc<RefCell<Vec<String>>>,
        format_on_save: Rc<Cell<bool>>,
    }

    pub(crate) const NEW: &str = r#"
shift 2
case "$1 $2" in
"check --json")
  echo '{"tool":"check","level":"error","path":"Home.md","line":2,"column":1,"code":"callout-body","message":"the body is joined","fixable":false}'
  echo 'daftar-cli: 1 error' >&2
  exit 1 ;;
"lint --json")
  echo '{"tool":"lint","level":"warning","path":"Home.md","line":3,"column":null,"code":"MD009","message":"spaces at the end","fixable":true}' ;;
"fmt --check")
  # Of the notes named, or of the vault.
  case "$*" in *" -- "*) case "$*" in *Notes/Plan.md*) ;; *) exit 0 ;; esac ;; esac
  echo '{"tool":"fmt","level":"error","path":"Notes/Plan.md","line":0,"column":null,"code":"fmt","message":"not formatted","fixable":true}' ;;
"fmt --stdin")
  sed 's/^\*  /- /' ;;
*)
  echo fixed > fixed ;;
esac
"#;

    const OLD: &str = r#"
for word in "$@"; do
  case "$word" in --json|--stdin|--path=*) echo "error: unexpected argument '${word%%=*}' found" >&2; echo >&2; echo 'Usage: daftar-cli' >&2; exit 2 ;; esac
done
"#;

    impl Fake {
        pub(crate) fn new(body: &str) -> Fake {
            let (vault, bin) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            std::fs::create_dir(vault.path().join("Notes")).unwrap();
            std::fs::write(vault.path().join("Home.md"), "# Home\n").unwrap();
            std::fs::write(vault.path().join("Notes/Plan.md"), "# Plan\n").unwrap();
            std::fs::write(vault.path().join(".daftar.toml"), "[check]\n").unwrap();
            let script = bin.path().join("daftar-cli");
            let log = vault.path().join("calls");
            std::fs::write(&script, format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{body}", log.display())).unwrap();
            std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            let checks = VaultChecks::new();
            *checks.locate.borrow_mut() = Rc::new(move || Some(script.clone()));
            checks.settle.set(Duration::from_millis(10));
            let (notices, format_on_save) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(true)));
            let (said, on, saves) = (notices.clone(), format_on_save.clone(), log);
            checks.set_hooks(Hooks {
                save_editors: Box::new(move || {
                    let mut calls = std::fs::read_to_string(&saves).unwrap_or_default();
                    calls.push_str("save the editors\n");
                    std::fs::write(&saves, calls).unwrap();
                }),
                notice: Box::new(move |text| said.borrow_mut().push(text.to_string())),
                changed: Box::new(|| {}),
                wrote: Box::new(|_| {}),
                format_on_save: Box::new(move || on.get()),
            });
            Fake { vault, _bin: bin, checks, notices, format_on_save }
        }

        /// The calls, without `--vault <root>`.
        pub(crate) fn calls(&self) -> Vec<String> {
            let prefix = format!("--vault {} ", self.vault.path().display());
            std::fs::read_to_string(self.vault.path().join("calls"))
                .unwrap_or_default()
                .lines()
                .map(|line| line.trim_start_matches(&prefix).to_string())
                .collect()
        }

        pub(crate) fn file(&self, path: &str) -> String {
            std::fs::read_to_string(self.vault.path().join(path)).unwrap_or_default()
        }
    }

    /// What a test started ends with it: a timer or a run left on the shared
    /// main loop would wake in the next test's thread.
    impl Drop for Fake {
        fn drop(&mut self) {
            if let Some(soon) = self.checks.soon.take() {
                soon.remove();
            }
            self.checks.cancel.borrow().cancel();
            let context = glib::MainContext::default();
            for _ in 0..100 {
                while context.iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// The main loop runs until [done], for ten seconds at most.
    pub(crate) fn wait(context: &glib::MainContext, done: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            while context.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    /// The default main loop, which GLib's local timeouts and futures use,
    /// owned by one test at a time.
    pub(crate) fn on_a_loop_of_its_own(test: impl FnOnce(&glib::MainContext)) {
        static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = ONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = glib::MainContext::default();
        let _owner = context.acquire().unwrap();
        test(&context);
    }

    pub(crate) fn idle(checks: &VaultChecks) -> bool {
        !checks.running() && !checks.fixing() && !checks.formatting.get() && checks.soon.borrow().is_none()
    }

    #[test]
    fn the_audit_lists_what_the_three_tools_found() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            assert!(wait(context, || fake.checks.state() == State::Done && idle(&fake.checks)));
            assert_eq!(fake.checks.counts(), ProblemCounts { errors: 2, warnings: 1 });
            let notes = fake.checks.grouped(ProblemFilter::All);
            assert_eq!(notes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["Home.md", "Notes/Plan.md"]);
            assert_eq!(fake.calls(), ["check --json", "lint --json", "fmt --check --json"]);
            assert_eq!(fake.checks.lines("Home.md").keys().copied().collect::<Vec<_>>(), [1, 2]);
        });
    }

    /// After a save only the saved notes are linted and checked for
    /// formatting; the other notes keep their findings, and `check` reads the
    /// whole vault. A note that is gone is not named, and loses its findings.
    #[test]
    fn a_save_lints_only_the_notes_saved() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            assert!(wait(context, || fake.checks.state() == State::Done && idle(&fake.checks)));
            std::fs::remove_file(fake.vault.path().join("calls")).unwrap();
            fake.checks.changed(&["Home.md".to_string(), "Gone.md".to_string(), "picture.png".to_string()].into());
            assert!(wait(context, || fake.calls().len() == 3 && idle(&fake.checks)));
            assert_eq!(fake.calls(), ["check --json", "lint --json -- Home.md", "fmt --check --json -- Home.md"]);
            assert_eq!(fake.checks.counts(), ProblemCounts { errors: 2, warnings: 1 });
            std::fs::remove_file(fake.vault.path().join("calls")).unwrap();
            fake.checks.changed(&["Gone.md".to_string()].into());
            assert!(wait(context, || fake.calls().len() == 1 && idle(&fake.checks)));
            assert_eq!(fake.calls(), ["check --json"]);
            assert_eq!(fake.checks.counts(), ProblemCounts { errors: 2, warnings: 1 });
        });
    }

    #[test]
    fn an_old_command_line_says_to_upgrade_once() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(OLD);
            fake.checks.opened(fake.vault.path());
            assert!(wait(context, || fake.checks.state() == State::TooOld && idle(&fake.checks)));
            // In the background, nothing is said: the panel was not opened.
            assert!(fake.notices.borrow().is_empty());
            fake.checks.refresh();
            assert!(wait(context, || fake.notices.borrow().len() == 1 && idle(&fake.checks)));
            assert_eq!(fake.notices.borrow()[0], daftar_cli::UPGRADE);
            // Not asked again at the next change, nor said again.
            let calls = fake.calls().len();
            fake.checks.changed(&["Home.md".to_string()].into());
            assert!(wait(context, || idle(&fake.checks)));
            assert_eq!(fake.calls().len(), calls);
            fake.checks.refresh();
            assert!(wait(context, || fake.calls().len() > calls && idle(&fake.checks)));
            assert_eq!(fake.notices.borrow().len(), 1);
            assert_eq!(VaultChecks::explanation(&fake.checks.state()).as_deref(), Some(daftar_cli::UPGRADE));
        });
    }

    #[test]
    fn no_command_line_is_said() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            *fake.checks.locate.borrow_mut() = Rc::new(|| None);
            fake.checks.opened(fake.vault.path());
            fake.checks.refresh();
            assert!(wait(context, || fake.checks.state() == State::Missing && idle(&fake.checks)));
            assert_eq!(*fake.notices.borrow(), [daftar_cli::MISSING]);
        });
    }

    #[test]
    fn fix_all_of_a_note_runs_the_three_fixes_after_the_editors_save() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            assert!(wait(context, || fake.checks.state() == State::Done && idle(&fake.checks)));
            std::fs::remove_file(fake.vault.path().join("calls")).unwrap();
            fake.checks.fix_all(Some("-x.md"));
            assert!(wait(context, || fake.calls().len() >= 7 && idle(&fake.checks)));
            assert_eq!(fake.calls()[..4], ["save the editors", "lint --fix -- -x.md", "check --fix -- -x.md", "fmt -- -x.md"]);
            // Then the vault is checked again at once.
            assert_eq!(fake.calls()[4..], ["check --json", "lint --json", "fmt --check --json"]);
            assert_eq!(fake.file("fixed"), "fixed\n");
        });
    }

    #[test]
    fn leaving_a_note_typed_in_formats_its_file() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            std::fs::write(fake.vault.path().join("Home.md"), "# Home\n*  item\n").unwrap();
            fake.checks.format_on_leave("Home.md".into(), "# Home\n".into(), FormatMoment::Closed);
            assert!(wait(context, || fake.file("Home.md") == "# Home\n- item\n"));
            assert!(wait(context, || idle(&fake.checks)));
            assert!(fake.calls().contains(&"fmt --stdin --path=Home.md".to_string()));
        });
    }

    #[test]
    fn a_note_only_read_in_the_editor_is_left_as_it_is() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            let unformatted = "# Home\n*  item\n";
            std::fs::write(fake.vault.path().join("Home.md"), unformatted).unwrap();
            // Opened and left without typing.
            fake.checks.format_on_leave("Home.md".into(), unformatted.into(), FormatMoment::Closed);
            // Typed in, but at the autosave, or with the setting off.
            fake.checks.format_on_leave("Home.md".into(), "# Home\n".into(), FormatMoment::Autosave);
            fake.format_on_save.set(false);
            fake.checks.format_on_leave("Home.md".into(), "# Home\n".into(), FormatMoment::Closed);
            assert!(wait(context, || idle(&fake.checks)));
            assert_eq!(fake.file("Home.md"), unformatted);
            assert!(!fake.calls().iter().any(|call| call.contains("--stdin")));
            // A base or a canvas never.
            fake.format_on_save.set(true);
            fake.checks.format_on_leave("Board.canvas".into(), String::new(), FormatMoment::Closed);
            assert!(wait(context, || idle(&fake.checks)));
            assert!(!fake.calls().iter().any(|call| call.contains("--stdin")));
        });
    }

    #[test]
    fn with_an_old_command_line_leaving_says_nothing() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(OLD);
            fake.checks.opened(fake.vault.path());
            assert!(wait(context, || idle(&fake.checks)));
            let unformatted = "# Home\n*  item\n";
            std::fs::write(fake.vault.path().join("Home.md"), unformatted).unwrap();
            fake.checks.format_on_leave("Home.md".into(), "# Home\n".into(), FormatMoment::Closed);
            assert!(wait(context, || idle(&fake.checks)));
            assert_eq!(fake.file("Home.md"), unformatted);
            assert!(fake.notices.borrow().is_empty());
            // And it is not asked again at the next leave.
            let calls = fake.calls().len();
            fake.checks.format_on_leave("Home.md".into(), "# Home\n".into(), FormatMoment::Closed);
            assert!(wait(context, || idle(&fake.checks)));
            assert_eq!(fake.calls().len(), calls);
        });
    }

    /// Format Note in the reader: through the editors' gate, and "formatted
    /// already" the second time.
    #[test]
    fn format_note_in_the_reader_says_when_it_is_formatted_already() {
        on_a_loop_of_its_own(|context| {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            std::fs::write(fake.vault.path().join("Home.md"), "# Home\n*  item\n").unwrap();
            fake.checks.format_file_now("Home.md".into());
            assert!(wait(context, || fake.file("Home.md") == "# Home\n- item\n"));
            assert_eq!(fake.calls().iter().filter(|call| *call == "save the editors").count(), 2);
            fake.checks.format_file_now("Home.md".into());
            assert!(wait(context, || !fake.notices.borrow().is_empty()));
            assert_eq!(*fake.notices.borrow(), ["Home is formatted already."]);
        });
    }
}
