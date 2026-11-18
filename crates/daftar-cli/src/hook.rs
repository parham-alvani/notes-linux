//! `daftar-cli hook`: the vault's git hooks and what they run. Before a
//! commit, the staged notes are formatted (and staged again), linted and
//! audited strictly; the rest of the vault is audited for errors only, so a
//! warning a note got from the passing days (a quiet job process) never
//! stops a commit of another note, while a broken link anywhere does.
//!
//! After a pull, the notes it brought are audited and linted, their findings
//! printed and nothing stopped, and search by meaning is brought up to them
//! in the background when its model is downloaded. A pull that merges or
//! fast-forwards runs `post-merge`; `git pull --rebase` with commits of your
//! own (as the app's sync) runs `post-rewrite rebase` instead. Either way
//! `ORIG_HEAD` is where the branch was before the pull, so what changed is
//! `ORIG_HEAD..HEAD`; the pairs `post-rewrite` reads on stdin are your own
//! commits replayed, not the ones pulled.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use daftar_core::semantic;
use daftar_core::semantic_model;
use daftar_core::{Git, Vault};

/// The first line after the shebang of a hook this command wrote: a hook
/// without it is the person's, and `install` leaves it alone.
const MARK: &str = "# Installed by `daftar-cli hook install`";

/// The hooks `install` writes, by the names git runs them by.
pub const HOOKS: [&str; 3] = ["pre-commit", "post-merge", "post-rewrite"];

/// What the hook file [name] says: it runs the `daftar-cli` on `$PATH` when
/// git runs the hook, so an upgrade needs no new hook. A hook after a pull
/// stops nothing, so it passes quietly when `daftar-cli` is not there.
pub fn script(name: &str) -> String {
    match name {
        "pre-commit" => format!(
            "#!/bin/sh\n{MARK}: formats, lints and audits the staged notes as CI does.\n\
             # `git commit --no-verify` skips it.\n\
             command -v daftar-cli > /dev/null || {{ echo 'pre-commit: daftar-cli is not on $PATH' >&2; exit 1; }}\n\
             exec daftar-cli hook pre-commit\n"
        ),
        "post-merge" => format!(
            "#!/bin/sh\n{MARK}: audits the notes a pull merged or fast-forwarded to,\n\
             # and brings search by meaning up to them. It never stops anything.\n\
             command -v daftar-cli > /dev/null || exit 0\n\
             exec daftar-cli hook post-merge \"$@\"\n"
        ),
        _ => format!(
            "#!/bin/sh\n{MARK}: after `git pull --rebase`, audits the notes it\n\
             # brought and brings search by meaning up to them. An amend is let pass.\n\
             command -v daftar-cli > /dev/null || {{ cat > /dev/null; exit 0; }}\n\
             exec daftar-cli hook post-rewrite \"$@\"\n"
        ),
    }
}

/// The folder git runs the hooks from: `core.hooksPath` when the repository
/// sets one, else `.git/hooks`.
pub fn hooks_folder(git: &Git, root: &Path) -> Result<PathBuf, String> {
    let folder = git.run(&["rev-parse", "--git-path", "hooks"]).map_err(|e| e.to_string())?;
    let folder = PathBuf::from(folder.trim());
    Ok(if folder.is_absolute() { folder } else { root.join(folder) })
}

/// What `install` did: the hooks written, and those left as a hook of the
/// person's own.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Installed {
    pub written: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

impl Installed {
    /// Why a skipped hook was left, to say after the written ones.
    pub fn refusal(&self) -> Option<String> {
        (!self.skipped.is_empty()).then(|| {
            let files: Vec<String> = self.skipped.iter().map(|f| f.display().to_string()).collect();
            let what = if files.len() == 1 { "is a hook" } else { "are hooks" };
            format!("{} {what} of your own; --force puts this one in its place", files.join(", "))
        })
    }
}

/// Writes the hooks [only] names (every one of [HOOKS] when empty), each
/// never over one this command did not write unless [force]: a hook of the
/// person's own is skipped and the others are written.
pub fn install(git: &Git, root: &Path, force: bool, only: &[String]) -> Result<Installed, String> {
    let folder = hooks_folder(git, root)?;
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut done = Installed::default();
    for name in HOOKS.into_iter().filter(|name| only.is_empty() || only.iter().any(|o| o == name)) {
        let file = folder.join(name);
        if let Ok(there) = fs::read_to_string(&file)
            && !there.contains(MARK)
            && !force
        {
            done.skipped.push(file);
            continue;
        }
        fs::write(&file, script(name)).map_err(|e| format!("{}: {e}", file.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", file.display()))?;
        }
        done.written.push(file);
    }
    Ok(done)
}

/// Removes the hooks this command wrote, and only those: the removed files,
/// then the hooks of the person's own left where they are.
pub fn uninstall(git: &Git, root: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let folder = hooks_folder(git, root)?;
    let (mut removed, mut left) = (Vec::new(), Vec::new());
    for name in HOOKS {
        let file = folder.join(name);
        let Ok(there) = fs::read_to_string(&file) else { continue };
        if there.contains(MARK) {
            fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            removed.push(file);
        } else {
            left.push(file);
        }
    }
    Ok((removed, left))
}

/// Markdown notes git lists with [arguments] before `-- *.md`, from the
/// vault's folder.
fn notes(git: &Git, arguments: &[&str]) -> Result<Vec<String>, String> {
    let mut words = arguments.to_vec();
    words.extend(["--", "*.md"]);
    let out = git.run_raw(&words).map_err(|e| e.to_string())?;
    Ok(out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
}

/// The staged Markdown notes, from the vault's folder: those added, copied,
/// changed or renamed, and whether a note was deleted or renamed away, which
/// can break a link in a note not staged.
pub fn staged(git: &Git) -> Result<(Vec<String>, bool), String> {
    let names = |filter: &str| notes(git, &["diff", "--cached", "--relative", "--name-only", "-z", &format!("--diff-filter={filter}")]);
    let kept = names("ACMR")?;
    let gone = !names("DR")?.is_empty();
    Ok((kept, gone))
}

/// The notes a pull brought, between where the branch was (`ORIG_HEAD`) and
/// `HEAD`: those there now, then those gone (a rename is both). Nothing when
/// `ORIG_HEAD` is not there.
pub fn pulled(git: &Git) -> Result<(Vec<String>, Vec<String>), String> {
    if git.run(&["rev-parse", "--verify", "--quiet", "ORIG_HEAD^{commit}"]).is_err() {
        return Ok((Vec::new(), Vec::new()));
    }
    let names = |filter: &str| {
        notes(git, &["diff", "--relative", "--name-only", "--no-renames", "-z", &format!("--diff-filter={filter}"), "ORIG_HEAD", "HEAD"])
    };
    Ok((names("ACMT")?, names("D")?))
}

/// The steps of a pre-commit check of [notes]: each a `daftar-cli`
/// command line, and whether its failure stops the commit.
pub fn steps(notes: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if !notes.is_empty() {
        out.push(with(&["fmt"], notes));
        out.push(with(&["lint"], notes));
        out.push(with(&["check", "--strict"], notes));
    }
    // Every note, errors only: a staged change can break a link elsewhere.
    out.push(vec!["check".into()]);
    out
}

/// The steps after a pull brought [notes]: their audit and their lint,
/// whose findings are shown and stop nothing. The rest of the vault is not
/// audited: the commits pulled passed their own pre-commit check of it.
pub fn audit_steps(notes: &[String]) -> Vec<Vec<String>> {
    if notes.is_empty() {
        return Vec::new();
    }
    vec![with(&["check"], notes), with(&["lint"], notes)]
}

/// The command line [words] `--` [notes].
fn with(words: &[&str], notes: &[String]) -> Vec<String> {
    let mut line: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
    line.push("--".into());
    line.extend(notes.iter().cloned());
    line
}

/// The hook's own lines on stderr, in color on a terminal (not with
/// `NO_COLOR`), so a commit or a pull shows each step run; a GUI client gets
/// plain text.
struct Paint {
    on: bool,
    /// The hook, as git names it.
    hook: &'static str,
}

impl Paint {
    fn new(hook: &'static str) -> Self {
        Self { on: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()), hook }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.on { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    /// The hook's name, before each of its lines.
    fn name(&self) -> String {
        self.paint("1;35", &format!("daftar {}", self.hook))
    }

    fn start(&self, what: &str) -> String {
        format!("{} {} {what}", self.name(), self.paint("1;36", "▸"))
    }

    fn passed(&self, what: &str) -> String {
        format!("{} {}", self.name(), self.paint("1;32", &format!("✓ {what}")))
    }

    fn warned(&self, what: &str) -> String {
        format!("{} {}", self.name(), self.paint("1;33", &format!("⚠ {what}")))
    }

    fn failed(&self, what: &str) -> String {
        format!("{} {}", self.name(), self.paint("1;31", &format!("✗ {what}")))
    }
}

/// [count] notes, with what they are: `1 staged note`, `3 pulled notes`.
fn counted(count: usize, kind: &str) -> String {
    if count == 1 { format!("1 {kind} note") } else { format!("{count} {kind} notes") }
}

/// What a step does, in words, of [notes] notes of [kind] (staged, pulled).
fn label(step: &[String], notes: usize, kind: &str) -> String {
    let notes = counted(notes, kind);
    match (step[0].as_str(), step.get(1).map(String::as_str)) {
        ("fmt", _) => format!("formatting {notes}"),
        ("lint", _) => format!("linting {notes}"),
        ("check", Some(_)) => format!("auditing {notes}"),
        _ => "auditing the vault for errors".to_string(),
    }
}

/// Runs the check on what is staged; an error stops the commit.
pub fn pre_commit(root: &Path, git: &Git) -> Result<(), String> {
    let (notes, gone) = staged(git)?;
    if notes.is_empty() && !gone {
        return Ok(());
    }
    let paint = Paint::new("pre-commit");
    let began = std::time::Instant::now();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    for step in steps(&notes) {
        let what = label(&step, notes.len(), "staged");
        eprintln!("{}", paint.start(&what));
        let quiet = step[0] == "fmt";
        let mut command = Command::new(&me);
        command.arg("--vault").arg(root).args(&step).current_dir(root);
        if quiet {
            command.stdout(Stdio::null());
        }
        let status = command.status().map_err(|e| format!("{}: {e}", me.display()))?;
        if !status.success() {
            eprintln!("{}", paint.failed(&format!("{what} failed; `git commit --no-verify` commits anyway")));
            return Err(format!("pre-commit stopped the commit at `daftar-cli {}`", step[..step.len().min(2)].join(" ")));
        }
        if quiet {
            // The formatted notes are what is committed.
            let mut add = vec!["add", "--"];
            add.extend(notes.iter().map(String::as_str));
            git.run(&add).map_err(|e| e.to_string())?;
        }
    }
    eprintln!("{}", paint.passed(&format!("all checks passed in {:.1}s", began.elapsed().as_secs_f64())));
    Ok(())
}

/// `post-merge`: a merge or a fast-forward, [squash] when `git merge
/// --squash` left the change uncommitted, which brought nothing yet.
pub fn post_merge(root: &Path, git: &Git, squash: bool) {
    if !squash {
        after_pull(root, git, "post-merge");
    }
}

/// `post-rewrite`: after a rebase, what it brought; an amend brought
/// nothing. The pairs git writes on stdin are read so it never meets a
/// closed pipe.
pub fn post_rewrite(root: &Path, git: &Git, kind: &str) {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let _ = std::io::copy(&mut stdin.lock(), &mut std::io::sink());
    }
    if kind == "rebase" {
        after_pull(root, git, "post-rewrite");
    }
}

/// What a hook after a pull does: search by meaning brought up to the notes
/// pulled, in the background, then their audit and lint, each finding
/// printed. Nothing it finds or fails at stops anything; with no note
/// pulled it says nothing.
fn after_pull(root: &Path, git: &Git, hook: &'static str) {
    let paint = Paint::new(hook);
    let (kept, gone) = match pulled(git) {
        Ok(found) => found,
        Err(message) => {
            eprintln!("{}", paint.warned(&format!("could not tell what the pull changed: {message}")));
            return;
        }
    };
    if kept.is_empty() && gone.is_empty() {
        return;
    }
    let me = match std::env::current_exe() {
        Ok(me) => me,
        Err(e) => {
            eprintln!("{}", paint.warned(&e.to_string()));
            return;
        }
    };
    if semantic_model::is_downloaded() {
        eprintln!("{}", paint.start(&format!("refreshing search by meaning ({})", counted(kept.len() + gone.len(), "pulled"))));
        match refresh_in_background(&me, root) {
            Ok(Refresh::Started(log)) => {
                eprintln!("{}", paint.passed(&format!("refreshing in the background; its log is {}", log.display())));
            }
            Ok(Refresh::Queued) => eprintln!("{}", paint.passed("a refresh already running takes these notes too")),
            Err(message) => eprintln!("{}", paint.warned(&format!("search by meaning not refreshed: {message}"))),
        }
    }
    for step in audit_steps(&kept) {
        let what = label(&step, kept.len(), "pulled");
        eprintln!("{}", paint.start(&what));
        let ran = Command::new(&me).arg("--vault").arg(root).args(&step).current_dir(root).stdin(Stdio::null()).output();
        let done = match ran {
            Ok(done) => done,
            Err(e) => {
                eprintln!("{}", paint.warned(&format!("{}: {e}", me.display())));
                continue;
            }
        };
        let findings: Vec<&str> = std::str::from_utf8(&done.stdout).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).collect();
        for line in &findings {
            eprintln!("{line}");
        }
        if findings.is_empty() && !done.status.success() {
            eprint!("{}", String::from_utf8_lossy(&done.stderr));
        }
        let verb = if step[0] == "lint" { "linting" } else { "auditing" };
        match findings.len() {
            0 if done.status.success() => eprintln!("{}", paint.passed(&format!("{verb} found nothing"))),
            0 => eprintln!("{}", paint.warned(&format!("{verb} failed"))),
            1 => eprintln!("{}", paint.warned(&format!("{verb} found 1 problem"))),
            n => eprintln!("{}", paint.warned(&format!("{verb} found {n} problems"))),
        }
    }
}

/// What [refresh_in_background] did.
#[derive(Debug, PartialEq, Eq)]
enum Refresh {
    /// Started `hook refresh`, which writes to this log.
    Started(PathBuf),
    /// Asked the refresh already running to go again when it is done.
    Queued,
}

/// The files of a refresh of the vault at [root], beside its index in the
/// cache: the lock a refresh holds, the mark that asks it to go again, and
/// its log.
fn refresh_files(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let index = semantic::index_file(root);
    (index.with_extension("lock"), index.with_extension("again"), index.with_extension("log"))
}

/// A lock older than this was left by a refresh that died: a whole vault
/// is embedded in minutes.
const STALE: Duration = Duration::from_secs(3600);

/// Whether the lock in [lock] is held: there and younger than [STALE].
fn held(lock: &Path) -> bool {
    fs::metadata(lock).and_then(|m| m.modified()).is_ok_and(|at| SystemTime::now().duration_since(at).unwrap_or_default() < STALE)
}

/// Takes the lock in [lock] for this process; false when a refresh holds it.
fn take(lock: &Path) -> bool {
    if held(lock) {
        return false;
    }
    let _ = fs::remove_file(lock);
    fs::OpenOptions::new().write(true).create_new(true).open(lock).is_ok()
}

/// Starts `daftar-cli hook refresh` on its own, its output in the log, so
/// the pull returns at once: loading the model and embedding the notes take
/// seconds, and the app's sync waits for git. It is in a process group of
/// its own, so a Ctrl+C at the terminal that ran the pull does not stop it,
/// and holds none of git's pipes, so nothing reading the pull's output
/// waits for it. When a refresh is running it is asked to go again instead.
fn refresh_in_background(me: &Path, root: &Path) -> Result<Refresh, String> {
    let (lock, again, log) = refresh_files(root);
    if let Some(folder) = log.parent() {
        fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    if held(&lock) {
        fs::write(&again, "").map_err(|e| format!("{}: {e}", again.display()))?;
        return Ok(Refresh::Queued);
    }
    let out = fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(me);
    command.arg("--vault").arg(root).args(["hook", "refresh"]).current_dir(root);
    command.stdin(Stdio::null()).stdout(out).stderr(err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().map_err(|e| format!("{}: {e}", me.display()))?;
    Ok(Refresh::Started(log))
}

/// `hook refresh`: the vault's index of search by meaning brought up to its
/// notes, holding the lock, and again while a pull asked for it meanwhile.
/// Without the model it does nothing.
pub fn refresh(root: &Path) -> Result<(), String> {
    if !semantic_model::is_downloaded() {
        return Ok(());
    }
    let (lock, again, _) = refresh_files(root);
    if let Some(folder) = lock.parent() {
        fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    if !take(&lock) {
        fs::write(&again, "").map_err(|e| format!("{}: {e}", again.display()))?;
        return Ok(());
    }
    let built = loop {
        let _ = fs::remove_file(&again);
        let vault = Vault::index(root, None);
        if let Err(message) = crate::meaning::build(root, &vault, false) {
            break Err(message);
        }
        if !again.exists() {
            break Ok(());
        }
    };
    let _ = fs::remove_file(&lock);
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs git in [root] as a person with a name who does not sign, with
    /// [path] first on `$PATH` when given, so a hook finds a stand-in there.
    fn git_in(root: &Path, words: &[&str], path: Option<&Path>) -> String {
        let mut command = Command::new("git");
        command.arg("-C").arg(root).env("GIT_MERGE_AUTOEDIT", "no");
        command.args(["-c", "user.name=T", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false"]).args(words);
        if let Some(path) = path {
            let mut paths = vec![path.to_path_buf()];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            command.env("PATH", std::env::join_paths(paths).unwrap());
        }
        let done = command.output().unwrap();
        assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
        String::from_utf8_lossy(&done.stdout).into_owned()
    }

    fn git(root: &Path, words: &[&str]) {
        git_in(root, words, None);
    }

    #[test]
    fn the_staged_notes_are_checked_and_the_vault_for_errors() {
        assert_eq!(steps(&[]), [vec!["check".to_string()]]);
        let notes = ["-x.md".to_string()];
        let steps = steps(&notes);
        assert_eq!(steps[0], ["fmt", "--", "-x.md"]);
        assert_eq!(steps[2], ["check", "--strict", "--", "-x.md"]);
        assert_eq!(steps[3], ["check"]);

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        fs::write(root.join("A.md"), "# A\n").unwrap();
        fs::write(root.join("B.md"), "# B\n").unwrap();
        fs::write(root.join("pic.png"), "").unwrap();
        git(root, &["add", "."]);
        let repository = Git::new(root);
        assert_eq!(staged(&repository).unwrap(), (vec!["A.md".to_string(), "B.md".to_string()], false));
        git(root, &["commit", "-qm", "one"]);
        git(root, &["rm", "-q", "B.md"]);
        assert_eq!(staged(&repository).unwrap(), (Vec::new(), true));
    }

    #[test]
    fn each_step_is_named_and_colored_only_on_a_terminal() {
        let notes = ["A.md".to_string()];
        let steps = steps(&notes);
        let labels: Vec<String> = steps.iter().map(|step| label(step, 1, "staged")).collect();
        assert_eq!(
            labels,
            ["formatting 1 staged note", "linting 1 staged note", "auditing 1 staged note", "auditing the vault for errors"]
        );
        assert_eq!(label(&steps[0], 2, "staged"), "formatting 2 staged notes");
        let plain = Paint { on: false, hook: "pre-commit" };
        assert_eq!(plain.start("linting"), "daftar pre-commit ▸ linting");
        assert_eq!(plain.failed("x"), "daftar pre-commit ✗ x");
        let colored = Paint { on: true, hook: "pre-commit" };
        assert_eq!(colored.passed("ok"), "\x1b[1;35mdaftar pre-commit\x1b[0m \x1b[1;32m✓ ok\x1b[0m");
    }

    #[test]
    fn a_pull_audits_and_lints_its_notes_and_nothing_else() {
        assert!(audit_steps(&[]).is_empty());
        let notes = ["A.md".to_string(), "B.md".to_string()];
        let steps = audit_steps(&notes);
        assert_eq!(steps, [vec!["check", "--", "A.md", "B.md"], vec!["lint", "--", "A.md", "B.md"]]);
        let labels: Vec<String> = steps.iter().map(|step| label(step, 2, "pulled")).collect();
        assert_eq!(labels, ["auditing 2 pulled notes", "linting 2 pulled notes"]);
        let plain = Paint { on: false, hook: "post-merge" };
        assert_eq!(plain.start("refreshing search by meaning (12 notes)"), "daftar post-merge ▸ refreshing search by meaning (12 notes)");
        assert_eq!(plain.warned("linting found 1 problem"), "daftar post-merge ⚠ linting found 1 problem");
    }

    #[test]
    fn install_writes_each_hook_and_leaves_a_hook_of_your_own() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        let repository = Git::new(root);
        let hooks = root.join(".git/hooks");
        let done = install(&repository, root, false, &[]).unwrap();
        assert_eq!(done.written, HOOKS.map(|name| hooks.join(name)));
        assert_eq!(done.refusal(), None);
        assert!(fs::read_to_string(hooks.join("pre-commit")).unwrap().contains("exec daftar-cli hook pre-commit\n"));
        assert!(fs::read_to_string(hooks.join("post-merge")).unwrap().contains("exec daftar-cli hook post-merge \"$@\""));
        assert!(fs::read_to_string(hooks.join("post-rewrite")).unwrap().contains("exec daftar-cli hook post-rewrite \"$@\""));
        // Its own hooks are written again; the person's is not, but with
        // --force, and the others are written all the same.
        install(&repository, root, false, &[]).unwrap();
        fs::write(hooks.join("post-merge"), "#!/bin/sh\nmine\n").unwrap();
        let done = install(&repository, root, false, &[]).unwrap();
        assert_eq!(done.written, [hooks.join("pre-commit"), hooks.join("post-rewrite")]);
        assert_eq!(done.skipped, [hooks.join("post-merge")]);
        assert!(done.refusal().unwrap().contains("--force"));
        // Uninstall takes its own and leaves the person's.
        let (removed, left) = uninstall(&repository, root).unwrap();
        assert_eq!(removed, [hooks.join("pre-commit"), hooks.join("post-rewrite")]);
        assert_eq!(left, [hooks.join("post-merge")]);
        assert_eq!(fs::read_to_string(hooks.join("post-merge")).unwrap(), "#!/bin/sh\nmine\n");
        assert!(!hooks.join("pre-commit").exists());
        let done = install(&repository, root, true, &["post-merge".to_string()]).unwrap();
        assert_eq!((done.written, done.skipped), (vec![hooks.join("post-merge")], Vec::new()));
        assert!(!hooks.join("pre-commit").exists());
        // core.hooksPath is where git looks.
        git(root, &["config", "core.hooksPath", ".githooks"]);
        let done = install(&repository, root, false, &["pre-commit".to_string()]).unwrap();
        assert_eq!(done.written, [root.join(".githooks/pre-commit")]);
    }

    /// A pull fast-forwarded, merged or rebased runs one of the hooks, with
    /// `ORIG_HEAD` where the branch was, so [pulled] finds what it brought.
    #[test]
    fn a_pull_runs_its_hook_and_finds_the_notes_it_brought() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        let (remote, theirs, mine, bin) = (top.join("remote.git"), top.join("theirs"), top.join("mine"), top.join("bin"));
        git(top, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        git(top, &["clone", "-q", "remote.git", "theirs"]);
        git(&theirs, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        fs::write(theirs.join("A.md"), "# A\n").unwrap();
        fs::write(theirs.join("Gone.md"), "# Gone\n").unwrap();
        git(&theirs, &["add", "."]);
        git(&theirs, &["commit", "-qm", "one"]);
        git(&theirs, &["push", "-q", "-u", "origin", "main"]);
        git(top, &["clone", "-q", remote.to_str().unwrap(), "mine"]);
        // A stand-in for daftar-cli that writes down how each hook ran it.
        fs::create_dir(&bin).unwrap();
        let ran = top.join("ran");
        let stand_in = bin.join("daftar-cli");
        fs::write(&stand_in, format!("#!/bin/sh\ncat > /dev/null\necho \"$*\" >> '{}'\n", ran.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&stand_in, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let repository = Git::new(&mine);
        install(&repository, &mine, false, &[]).unwrap();
        let push = |file: &str, text: &str| {
            git(&theirs, &["pull", "-q", "--no-rebase"]);
            if text.is_empty() {
                git(&theirs, &["rm", "-q", file]);
            } else {
                fs::write(theirs.join(file), text).unwrap();
                git(&theirs, &["add", "."]);
            }
            git(&theirs, &["commit", "-qm", file]);
            git(&theirs, &["push", "-q"]);
        };
        let commit_mine = |file: &str| {
            fs::write(mine.join(file), "# Mine\n").unwrap();
            git(&mine, &["add", "."]);
            git(&mine, &["commit", "-qm", file]);
        };
        let pull = |how: &str| {
            let _ = fs::remove_file(&ran);
            git_in(&mine, &["pull", "-q", how], Some(&bin));
            fs::read_to_string(&ran).unwrap_or_default()
        };
        let owned = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();

        // `pull --rebase` with nothing of mine fast-forwards: post-merge.
        push("B.md", "# B\n");
        assert_eq!(pull("--rebase"), "hook post-merge 0\n");
        assert_eq!(pulled(&repository).unwrap(), (owned(&["B.md"]), Vec::new()));

        // With a commit of mine, the rebase runs post-rewrite, and what it
        // brought leaves my own note out.
        push("A.md", "# A\n\nMore.\n");
        commit_mine("Mine.md");
        assert_eq!(pull("--rebase"), "hook post-rewrite rebase\n");
        assert_eq!(pulled(&repository).unwrap(), (owned(&["A.md"]), Vec::new()));

        // A merge of two histories runs post-merge; a note deleted is gone.
        push("Gone.md", "");
        commit_mine("Mine 2.md");
        assert_eq!(pull("--no-rebase"), "hook post-merge 0\n");
        assert_eq!(pulled(&repository).unwrap(), (Vec::new(), owned(&["Gone.md"])));
    }

    #[test]
    fn a_refresh_running_holds_the_lock_and_a_stale_one_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("vault.lock");
        assert!(take(&lock));
        assert!(held(&lock));
        assert!(!take(&lock));
        let old = SystemTime::now() - STALE - Duration::from_secs(60);
        fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        assert!(!held(&lock));
        assert!(take(&lock));
    }
}
