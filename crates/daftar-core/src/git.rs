//! The vault's repository, through the system `git`. The person's own git
//! config, credential helper and SSH agent apply, as they do in a terminal;
//! Daftar holds no key and no token.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, FixedOffset, Local};

use crate::{ConfigFile, Settings};

pub struct Git {
    pub root: PathBuf,
    /// Variables every git this runs gets, over the process's own.
    environment: Vec<(String, String)>,
    /// The text of the computer's `config.toml`, in place of the file.
    computer: Option<String>,
    /// `GIT_SSH_COMMAND`, found once: the clone's key is kept (`ssh_command`).
    ssh: OnceLock<String>,
    /// The `-c user.name=… -c user.email=…` of a commit, found once.
    author: OnceLock<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFailure {
    pub arguments: Vec<String>,
    pub status: Option<i32>,
    pub output: String,
}

impl fmt::Display for GitFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "git {}: {}", self.arguments.join(" "), self.output)
    }
}

impl std::error::Error for GitFailure {}

/// One thing `Git::doctor` found: how much it matters, a short code, and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosis {
    pub level: &'static str,
    pub code: &'static str,
    pub message: String,
}

impl Diagnosis {
    fn error(code: &'static str, message: &str) -> Diagnosis {
        Diagnosis { level: "error", code, message: message.to_string() }
    }

    fn warning(code: &'static str, message: &str) -> Diagnosis {
        Diagnosis { level: "warning", code, message: message.to_string() }
    }

    fn info(code: &'static str, message: &str) -> Diagnosis {
        Diagnosis { level: "info", code, message: message.to_string() }
    }
}

/// Why a sync or a pull did not happen: git failed, or the pull's rebase
/// stopped on a conflict and is still open; in a read-only clone, the pull
/// could not fast-forward; or the remote refused the push of a key that
/// cannot write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncFailure {
    Git(GitFailure),
    Stopped(Stopped),
    /// A read-only clone with commits or changes of its own that stand in
    /// the pull's way: the files, which the person mends in a shell.
    ReadOnlyDiverged(Vec<String>),
    /// The push was refused because this clone's key has no write access.
    /// The commits already made stay.
    PushRefusedReadOnly(GitFailure),
}

impl From<GitFailure> for SyncFailure {
    fn from(failure: GitFailure) -> SyncFailure {
        SyncFailure::Git(failure)
    }
}

impl fmt::Display for SyncFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncFailure::Git(failure) => failure.fmt(f),
            SyncFailure::Stopped(stopped) => stopped.fmt(f),
            SyncFailure::ReadOnlyDiverged(files) if files.is_empty() => {
                write!(f, "This read-only vault has commits of its own, so the pull could not go on. Mend it with git in a shell.")
            }
            SyncFailure::ReadOnlyDiverged(files) => write!(
                f,
                "This read-only vault has changes of its own, so the pull could not go on: {}. Mend it with git in a shell.",
                files.join(", ")
            ),
            SyncFailure::PushRefusedReadOnly(_) => {
                write!(f, "The remote refused the push: this clone's key cannot write. Your commits are kept here.")
            }
        }
    }
}

impl std::error::Error for SyncFailure {}

/// A pull whose rebase stopped on a conflict. The rebase stays open until
/// the person resolves it or aborts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub conflicts: Vec<Conflicted>,
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.conflicts.as_slice() {
            [one] => write!(f, "The pull stopped on a conflict in {}.", one.path),
            many => write!(f, "The pull stopped on conflicts in {} files.", many.len()),
        }
    }
}

/// What one side of a rebase did to a conflicted file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Changed,
    Added,
    Deleted,
}

/// A file a rebase stopped on. During a rebase git's "ours" is the upstream
/// and "theirs" is the person's commit being replayed; here they are
/// `remote` and `mine`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflicted {
    pub path: String,
    pub remote: Side,
    pub mine: Side,
}

impl Conflicted {
    pub fn is_markdown(&self) -> bool {
        self.path.to_lowercase().ends_with(".md")
    }

    /// Both sides changed the text, so it has conflict marks to read.
    pub fn has_marks(&self) -> bool {
        self.remote != Side::Deleted && self.mine != Side::Deleted
    }
}

/// The person's choice for a whole file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The text to keep, conflict marks taken out.
    Text(String),
    /// The upstream's file, or its deletion.
    KeepRemote,
    /// The person's file, or their deletion.
    KeepMine,
}

/// A commit that changed a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub hash: String,
    pub date: DateTime<FixedOffset>,
    pub author: String,
    pub subject: String,
    /// The note's path in this commit, which a rename changes.
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub changed: usize,
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Synced {
    /// Local changes were committed.
    pub committed: bool,
    /// The pull brought commits.
    pub pulled: bool,
    /// Commits were pushed.
    pub pushed: bool,
}

impl Git {
    pub fn new(root: impl Into<PathBuf>) -> Git {
        Git { root: root.into(), environment: Vec::new(), computer: None, ssh: OnceLock::new(), author: OnceLock::new() }
    }

    /// With [key] set to [value] for every git this runs, over the
    /// process's own environment: a test's `GIT_CONFIG_GLOBAL`.
    #[must_use]
    pub fn with_environment(mut self, key: &str, value: &str) -> Git {
        self.environment.push((key.to_string(), value.to_string()));
        self
    }

    /// With [text] as the computer's `config.toml`, in place of the file.
    #[must_use]
    pub fn with_computer(mut self, text: &str) -> Git {
        self.computer = Some(text.to_string());
        self
    }

    /// Runs git and returns what it printed. Never asks on a terminal: a sync
    /// that needs a password fails and says so instead of waiting for ever.
    pub fn run(&self, arguments: &[&str]) -> Result<String, GitFailure> {
        let (out, err) = self.exec(arguments, &[])?;
        Ok(format!("{out}{err}").trim().to_string())
    }

    /// What git printed on its standard output, to the last byte: a file's
    /// text, or records split by NUL.
    pub fn run_raw(&self, arguments: &[&str]) -> Result<String, GitFailure> {
        Ok(self.exec(arguments, &[])?.0)
    }

    fn exec(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Result<(String, String), GitFailure> {
        let mut command = self.command();
        command.env("GIT_SSH_COMMAND", self.ssh.get_or_init(|| self.find_ssh_command()));
        // A commit, and a rebase that makes commits again, is the clone's
        // own author's, else the vault's.
        if arguments.first().is_some_and(|name| COMMITS.contains(name)) {
            command.args(self.author.get_or_init(|| self.find_author()));
        }
        command.args(arguments);
        command.envs(environment.iter().copied());
        let failure = |status, output: String| GitFailure {
            arguments: arguments.iter().map(ToString::to_string).collect(),
            status,
            output: output.trim().to_string(),
        };
        let output = command.output().map_err(|e| failure(None, e.to_string()))?;
        let out = String::from_utf8_lossy(&output.stdout).into_owned();
        let err = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.success() { Ok((out, err)) } else { Err(failure(output.status.code(), out + &err)) }
    }

    /// `git -C root`, never asking on a terminal, with this clone's variables.
    fn command(&self) -> Command {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
        command.envs(self.environment.iter().map(|(key, value)| (key, value)));
        command
    }

    /// What a plain `git` with [arguments] printed, when it succeeded: for
    /// the questions `exec` itself asks, which must not ask it again.
    fn ask(&self, arguments: &[&str]) -> Option<String> {
        let output = self.command().args(arguments).output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn is_repository(&self) -> bool {
        self.ask(&["rev-parse", "--is-inside-work-tree"]).is_some_and(|out| out == "true")
    }

    // The ssh command

    /// The options Daftar adds to `ssh`: it never waits for a password.
    const SSH_OPTIONS: [(&str, &str); 1] = [("BatchMode", "yes")];

    /// The `GIT_SSH_COMMAND` of every git Daftar runs. `GIT_SSH_COMMAND`
    /// in the [environment], when set, is the person's and is used as it
    /// is; else the clone's `core.sshCommand` ([configured]), which holds
    /// its key, with Daftar's options after it; else `ssh` with them. An
    /// option the command names already, in any case, is not added again.
    pub fn ssh_command(configured: Option<&str>, environment: Option<&str>) -> String {
        if let Some(asked) = environment.filter(|command| !command.trim().is_empty()) {
            return asked.to_string();
        }
        let mut command = configured.map(str::trim).filter(|command| !command.is_empty()).unwrap_or("ssh").to_string();
        let named = command.to_lowercase();
        for (option, value) in Self::SSH_OPTIONS {
            if !named.contains(&option.to_lowercase()) {
                command.push_str(&format!(" -o {option}={value}"));
            }
        }
        command
    }

    fn find_ssh_command(&self) -> String {
        let asked = self
            .environment
            .iter()
            .rev()
            .find(|(key, _)| key == "GIT_SSH_COMMAND")
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var("GIT_SSH_COMMAND").ok());
        if asked.as_deref().is_some_and(|command| !command.trim().is_empty()) {
            return Self::ssh_command(None, asked.as_deref());
        }
        Self::ssh_command(self.ask(&["config", "--get", "core.sshCommand"]).as_deref(), None)
    }

    /// `core.sshCommand` for a key file: `ssh -i <key> -o IdentitiesOnly=yes`,
    /// the path quoted for a shell.
    pub fn ssh_command_for_key(key: &Path) -> String {
        format!("ssh -i {} -o IdentitiesOnly=yes", shell_quoted(&key.to_string_lossy()))
    }

    /// The key file of a `core.sshCommand` that `ssh_command_for_key`
    /// wrote; `None` for any other command, which is shown as text.
    pub fn ssh_key(command: &str) -> Option<PathBuf> {
        let rest = command.trim().strip_prefix("ssh -i ")?.strip_suffix(" -o IdentitiesOnly=yes")?;
        let path = match rest.strip_prefix('\'').and_then(|inner| inner.strip_suffix('\'')) {
            Some(inner) => inner.replace("'\\''", "'"),
            None => rest.to_string(),
        };
        (Self::ssh_command_for_key(Path::new(&path)) == command.trim()).then(|| PathBuf::from(path))
    }

    /// A key as git finds it for this clone, its own config first, then
    /// the person's and the system's.
    pub fn effective(&self, key: &str) -> Option<String> {
        self.ask(&["config", "--get", key]).filter(|value| !value.is_empty())
    }

    /// A new key for this clone, `daftar_<vault>_ed25519` in [folder]
    /// (`~/.ssh`), with no passphrase: the file, and its public key to give
    /// the remote. Never over a file that is there.
    pub fn generate_key(&self, folder: &Path, host: &str) -> Result<(PathBuf, String), String> {
        let vault = self.root.file_name().map_or_else(|| "vault".into(), |name| name.to_string_lossy().into_owned());
        // Letters, digits, `-` and `.` stay; every other character is `_`.
        let slug: String = vault.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
        let file = folder.join(format!("daftar_{slug}_ed25519"));
        let public = PathBuf::from(format!("{}.pub", file.display()));
        if file.exists() || public.exists() {
            return Err(format!("{} is there already", file.display()));
        }
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let comment = format!("daftar {vault} on {}", host.split('.').next().unwrap_or(host));
        let output = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", &comment, "-f"])
            .arg(&file)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("ssh-keygen: {e}"))?;
        if !output.status.success() {
            return Err(format!("ssh-keygen: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        let key = std::fs::read_to_string(&public).map_err(|e| format!("{}: {e}", public.display()))?;
        Ok((file, key.trim().to_string()))
    }

    // The clone's own settings

    /// A key of this clone's own config (`git config --local`), when set.
    pub fn local(&self, key: &str) -> Option<String> {
        self.ask(&["config", "--local", "--get", key]).filter(|value| !value.is_empty())
    }

    /// Sets a key of this clone's own config; `None`, or nothing but
    /// spaces, unsets it.
    pub fn set_local(&self, key: &str, value: Option<&str>) -> Result<(), GitFailure> {
        let result = match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => self.run(&["config", "--local", key, value]),
            None if self.local(key).is_none() => return Ok(()),
            None => self.run(&["config", "--local", "--unset-all", key]),
        };
        forget_read_only(&self.root);
        result.map(|_| ())
    }

    /// Whether this clone is read-only: `daftar.readonly` in its own config,
    /// a git boolean. Not set is not; a value that is no boolean is, since
    /// when in doubt Daftar does not write. The global and system configs
    /// are not read, and a folder that is no repository is never read-only.
    pub fn is_read_only(&self) -> bool {
        let output = self.command().args(["config", "--local", "--type=bool", "--get", "daftar.readonly"]).output();
        let Ok(output) = output else { return false };
        match output.status.code() {
            Some(0) => String::from_utf8_lossy(&output.stdout).trim() != "false",
            // Not set.
            Some(1) => false,
            _ => self.is_repository(),
        }
    }

    /// Turns read-only on or off: only `daftar.readonly` changes, and the
    /// changes the clone already has stay as they are.
    pub fn set_read_only(&self, on: bool) -> Result<(), GitFailure> {
        self.set_local("daftar.readonly", on.then_some("true"))
    }

    /// Whether the vault at [root] is read-only, asked before every write:
    /// git is asked once, and again only when the clone's config changed.
    pub fn read_only(root: &Path) -> bool {
        let mut known = KNOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let known = known.get_or_insert_default();
        // A folder that was no repository and has a `.git` now is asked about again at once.
        if let Some(entry) = known.get(root)
            && entry.holds()
            && (entry.config.is_some() || !root.join(".git").exists())
        {
            return entry.read_only;
        }
        let git = Git::new(root);
        let read_only = git.is_read_only();
        let config =
            git.ask(&["rev-parse", "--path-format=absolute", "--git-common-dir"]).map(|folder| PathBuf::from(folder).join("config"));
        let stamp = config.as_deref().and_then(stamp);
        known.insert(root.to_path_buf(), Known { read_only, config, stamp, asked: Instant::now() });
        read_only
    }

    // The address

    /// The remote this clone pulls from and pushes to: the one of the
    /// branch's upstream, else `origin`.
    pub fn remote(&self) -> String {
        self.ask(&["symbolic-ref", "--short", "-q", "HEAD"])
            .and_then(|branch| self.ask(&["config", "--get", &format!("branch.{branch}.remote")]))
            .filter(|name| !name.is_empty() && name != ".")
            .unwrap_or_else(|| "origin".into())
    }

    /// The vault's address, `remote.<name>.url`; `None` with no remote yet.
    pub fn address(&self) -> Option<String> {
        self.ask(&["config", "--get", &format!("remote.{}.url", self.remote())]).filter(|url| !url.is_empty())
    }

    /// Moves the clone to [address], as to a mirror when the first host
    /// cannot be reached: `remote.<name>.url`, or a new `origin` when the
    /// clone has no remote, with the branch's upstream written then as
    /// config (`branch.<b>.remote`, `branch.<b>.merge`). Nothing is fetched
    /// now. An empty address is refused, and the old one stays.
    pub fn set_address(&self, address: &str) -> Result<(), GitFailure> {
        let address = address.trim();
        if address.is_empty() {
            return Err(GitFailure {
                arguments: vec!["remote".into(), "set-url".into()],
                status: None,
                output: "the address is empty".into(),
            });
        }
        let remote = self.remote();
        if self.ask(&["config", "--get", &format!("remote.{remote}.url")]).is_some() {
            self.run(&["remote", "set-url", &remote, address]).map(|_| ())
        } else {
            self.run(&["remote", "add", &remote, address])?;
            // The branch's upstream, as config: the next pull needs no more.
            if let Some(branch) = self.ask(&["symbolic-ref", "--short", "-q", "HEAD"]).filter(|branch| !branch.is_empty()) {
                self.run(&["config", "--local", &format!("branch.{branch}.remote"), &remote])?;
                self.run(&["config", "--local", &format!("branch.{branch}.merge"), &format!("refs/heads/{branch}")])?;
            }
            Ok(())
        }
    }

    // The author

    /// The author of a commit made here, as `-c` options: the clone's own
    /// `user.name` and `user.email` when set, else the vault's `git.name`
    /// and `git.email` ([vault]), else whatever git finds. Each of the two
    /// is chosen on its own.
    pub fn author_options(local: (Option<&str>, Option<&str>), vault: (Option<&str>, Option<&str>)) -> Vec<String> {
        let mut options = Vec::new();
        for (key, local, vault) in [("user.name", local.0, vault.0), ("user.email", local.1, vault.1)] {
            if let Some(value) = vault.filter(|value| !value.trim().is_empty())
                && local.is_none_or(|own| own.trim().is_empty())
            {
                options.extend(["-c".to_string(), format!("{key}={value}")]);
            }
        }
        options
    }

    fn find_author(&self) -> Vec<String> {
        let settings = match &self.computer {
            Some(computer) => Settings::of(&self.root, computer),
            None => Settings::of(&self.root, &std::fs::read_to_string(ConfigFile::default_path()).unwrap_or_default()),
        };
        let (name, email) = settings.author();
        if name.is_none() && email.is_none() {
            return Vec::new();
        }
        let (own_name, own_email) = (self.local("user.name"), self.local("user.email"));
        Self::author_options((own_name.as_deref(), own_email.as_deref()), (name.as_deref(), email.as_deref()))
    }

    /// Pulls with a rebase, keeping local changes aside while it does. A
    /// rebase that stops on a conflict stays open and is `Stopped`.
    pub fn pull(&self) -> Result<(), SyncFailure> {
        self.refuse_open_rebase()?;
        if self.is_read_only() {
            return self.pull_only().map(|_| ());
        }
        self.pull_rebase()
    }

    /// A read-only clone's sync: `git pull --ff-only`, with no commit
    /// before it and no push after. Nothing is stashed, reset or rebased:
    /// a clone with commits or changes of its own in the way is
    /// `ReadOnlyDiverged`, and stays as it is.
    fn pull_only(&self) -> Result<Synced, SyncFailure> {
        let before = self.run(&["rev-parse", "HEAD"])?;
        match self.run(&["pull", "--no-rebase", "--ff-only"]) {
            Ok(_) => Ok(Synced { pulled: self.run(&["rev-parse", "HEAD"])? != before, ..Synced::default() }),
            Err(failure) => {
                let ahead = self.ask(&["rev-list", "@{upstream}..HEAD"]).is_some_and(|commits| !commits.is_empty());
                let in_the_way = failure.output.contains("would be overwritten");
                if !ahead && !in_the_way {
                    return Err(failure.into());
                }
                let mut files: Vec<String> = Vec::new();
                if ahead && let Some(own) = self.ask(&["-c", "core.quotePath=false", "diff", "--name-only", "@{upstream}...HEAD"]) {
                    files.extend(own.lines().filter(|line| !line.is_empty()).map(String::from));
                }
                files.extend(self.changes().unwrap_or_default());
                files.sort();
                files.dedup();
                Err(SyncFailure::ReadOnlyDiverged(files))
            }
        }
    }

    fn pull_rebase(&self) -> Result<(), SyncFailure> {
        match self.run(&["pull", "--rebase", "--autostash"]) {
            Ok(_) => Ok(()),
            Err(_) if self.rebase_in_progress()? => Err(SyncFailure::Stopped(Stopped { conflicts: self.conflicts()? })),
            Err(failure) => Err(failure.into()),
        }
    }

    /// A rebase open on a conflict is never committed or pulled over.
    fn refuse_open_rebase(&self) -> Result<(), SyncFailure> {
        if self.rebase_in_progress()? {
            return Err(SyncFailure::Stopped(Stopped { conflicts: self.conflicts()? }));
        }
        // A merge, cherry-pick or revert left open by git itself, or files
        // still marked as conflicted, would be committed with their marks.
        if let Some(operation) = self.other_operation_in_progress()? {
            return Err(Self::refused(format!("a {operation} is in progress: finish or abort it with git first")));
        }
        if !self.conflicts()?.is_empty() {
            return Err(Self::refused("files have unresolved conflicts: resolve them first".into()));
        }
        Ok(())
    }

    fn refused(output: String) -> SyncFailure {
        SyncFailure::Git(GitFailure { arguments: vec!["sync".into()], status: None, output })
    }

    /// A merge, cherry-pick or revert that git left open, by its name.
    pub fn other_operation_in_progress(&self) -> Result<Option<&'static str>, GitFailure> {
        for (name, operation) in [("MERGE_HEAD", "merge"), ("CHERRY_PICK_HEAD", "cherry-pick"), ("REVERT_HEAD", "revert")] {
            let path = self.run(&["rev-parse", "--path-format=absolute", "--git-path", name])?;
            if Path::new(&path).exists() {
                return Ok(Some(operation));
            }
        }
        Ok(None)
    }

    /// The branch, how far it is from its upstream, and how many files changed.
    pub fn status(&self) -> Result<GitStatus, GitFailure> {
        Ok(Self::parse_status(&self.run(&["status", "--porcelain=v2", "--branch"])?))
    }

    pub fn parse_status(text: &str) -> GitStatus {
        let mut status = GitStatus::default();
        for line in text.lines() {
            if let Some(branch) = line.strip_prefix("# branch.head ") {
                status.branch = branch.to_string();
            } else if let Some(counts) = line.strip_prefix("# branch.ab ") {
                let mut parts = counts.split(' ').map(|p| p.trim_start_matches(['+', '-']).parse::<u32>().unwrap_or(0));
                status.ahead = parts.next().unwrap_or(0);
                status.behind = parts.next().unwrap_or(0);
            } else if !line.starts_with('#') && !line.is_empty() {
                status.changed += 1;
            }
        }
        status
    }

    /// What stands in a sync's way, or will: an operation left open, files in
    /// conflict, no branch or no upstream, commits not yet pushed or pulled,
    /// changes not committed, and untracked files over [large] bytes; and,
    /// as information, the committed files over it.
    pub fn doctor(&self, large: u64) -> Result<Vec<Diagnosis>, GitFailure> {
        let mut found = Vec::new();
        if self.rebase_in_progress()? {
            found.push(Diagnosis::error(
                "rebase-open",
                "a pull's rebase stopped on a conflict: resolve it in the app, or `git rebase --abort`",
            ));
        }
        if let Some(operation) = self.other_operation_in_progress()? {
            found.push(Diagnosis::error("operation-open", &format!("a {operation} is in progress: finish it, or abort it with git")));
        }
        let conflicts = self.conflicts()?;
        if !conflicts.is_empty() {
            let paths: Vec<&str> = conflicts.iter().map(|c| c.path.as_str()).collect();
            found.push(Diagnosis::error("conflicts", &format!("{} files in conflict: {}", paths.len(), paths.join(", "))));
        }
        found.extend(Self::diagnose_status(&self.run(&["status", "--porcelain=v2", "--branch"])?));
        found.extend(self.diagnose_read_only()?);
        let untracked = self.run_raw(&["ls-files", "--others", "--exclude-standard", "-z"])?;
        for path in untracked.split('\0').filter(|p| !p.is_empty()) {
            let size = std::fs::metadata(self.root.join(path)).map_or(0, |m| m.len());
            if size > large {
                found.push(Diagnosis::warning(
                    "large-file",
                    &format!("{path} is {:.1} MB and not committed yet", size as f64 / 1_048_576.0),
                ));
            }
        }
        // Committed already, in every clone: said, largest first, but no warning.
        let tracked = self.run_raw(&["ls-files", "-z"])?;
        let mut large_tracked: Vec<(u64, &str)> = tracked
            .split('\0')
            .filter(|p| !p.is_empty())
            .filter_map(|path| Some((std::fs::metadata(self.root.join(path)).ok()?.len(), path)))
            .filter(|(size, _)| *size > large)
            .collect();
        large_tracked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        for (size, path) in large_tracked {
            found.push(Diagnosis::info("large-tracked", &format!("{path} is {:.1} MB, in every clone", size as f64 / 1_048_576.0)));
        }
        Ok(found)
    }

    /// What `git status --porcelain=v2 --branch` says about the branch.
    pub fn diagnose_status(text: &str) -> Vec<Diagnosis> {
        let mut found = Vec::new();
        let status = Self::parse_status(text);
        if status.branch == "(detached)" {
            found.push(Diagnosis::warning("detached", "HEAD is on no branch: a sync commits nowhere it can push"));
        } else if !text.lines().any(|line| line.starts_with("# branch.upstream ")) {
            found.push(Diagnosis::warning(
                "no-upstream",
                &format!("{} has no upstream: `git branch --set-upstream-to=origin/{}`", status.branch, status.branch),
            ));
        }
        if status.ahead > 0 {
            found.push(Diagnosis::info("ahead", &format!("{} commits not pushed yet", status.ahead)));
        }
        if status.behind > 0 {
            found.push(Diagnosis::info("behind", &format!("{} commits on the remote not pulled yet", status.behind)));
        }
        if status.changed > 0 {
            found.push(Diagnosis::info("changed", &format!("{} files changed and not committed", status.changed)));
        }
        found
    }

    /// Paths with changes not yet committed.
    pub fn changes(&self) -> Result<Vec<String>, GitFailure> {
        Ok(Self::changed_paths(&self.run_raw(&["status", "--porcelain=v1", "-z"])?))
    }

    /// The paths of `git status --porcelain=v1 -z`: `XY path`, each record
    /// whole (never trimmed, as its first column may be a space), and a
    /// rename's or copy's source as a record of its own after it, skipped.
    pub fn changed_paths(output: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut records = output.split('\0');
        while let Some(record) = records.next() {
            let (Some(code), Some(path)) = (record.get(..2), record.get(3..)) else { continue };
            if path.is_empty() {
                continue;
            }
            paths.push(path.to_string());
            if code.contains('R') || code.contains('C') {
                records.next();
            }
        }
        paths
    }

    /// Commits everything, pulls with a rebase, and pushes: the vault's own
    /// workflow, as obsidian-git and the Mac app do it. A rebase open on a
    /// conflict is never committed over: it is `Stopped`, and the person
    /// resolves it first.
    pub fn sync(&self, message: &str) -> Result<Synced, SyncFailure> {
        self.refuse_open_rebase()?;
        // A read-only clone pulls and never writes.
        if self.is_read_only() {
            return self.pull_only();
        }
        let mut done = Synced::default();
        if !self.changes()?.is_empty() {
            self.run(&["add", "-A"])?;
            self.run(&["commit", "-m", message])?;
            done.committed = true;
        }
        let before = self.run(&["rev-parse", "HEAD"])?;
        self.pull_rebase()?;
        done.pulled = self.run(&["rev-list", &format!("{before}..HEAD")])?.lines().any(|l| !l.is_empty());
        done.pushed = self.push_if_ahead().map_err(Self::push_failure)?;
        Ok(done)
    }

    /// A push the remote refused for a key that cannot write: GitHub's
    /// deploy key message, or "Write access to repository not granted".
    pub fn push_refused_read_only(output: &str) -> bool {
        let output = output.to_lowercase();
        output.contains("marked as read only") || output.contains("write access to repository not granted")
    }

    /// A failed push by its name: the one a read-only key was refused, or git's.
    pub fn push_failure(failure: GitFailure) -> SyncFailure {
        if Self::push_refused_read_only(&failure.output) { SyncFailure::PushRefusedReadOnly(failure) } else { SyncFailure::Git(failure) }
    }

    /// Pushes when the branch has commits its upstream has not.
    pub fn push_if_ahead(&self) -> Result<bool, GitFailure> {
        if self.run(&["rev-list", "@{upstream}..HEAD"])?.is_empty() {
            return Ok(false);
        }
        self.run(&["push"])?;
        Ok(true)
    }

    // Conflicts

    /// A rebase is open: `rebase-merge` or `rebase-apply` in the git folder.
    pub fn rebase_in_progress(&self) -> Result<bool, GitFailure> {
        for name in ["rebase-merge", "rebase-apply"] {
            let path = self.run(&["rev-parse", "--path-format=absolute", "--git-path", name])?;
            if Path::new(&path).exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The files with unresolved conflicts.
    pub fn conflicts(&self) -> Result<Vec<Conflicted>, GitFailure> {
        Ok(Self::conflicted(&self.run_raw(&["status", "--porcelain=v2", "-z"])?))
    }

    /// The `u` records of `git status --porcelain=v2 -z`.
    pub fn conflicted(output: &str) -> Vec<Conflicted> {
        output
            .split('\0')
            .filter_map(|record| {
                let record = record.strip_prefix("u ")?;
                // XY sub m1 m2 m3 mW h1 h2 h3 path: the path may hold spaces.
                let fields: Vec<&str> = record.splitn(10, ' ').collect();
                if fields.len() != 10 || fields[0].chars().count() != 2 {
                    return None;
                }
                let side = |letter| match letter {
                    'D' => Side::Deleted,
                    'A' => Side::Added,
                    _ => Side::Changed,
                };
                let mut code = fields[0].chars();
                let (remote, mine) = (side(code.next()?), side(code.next()?));
                Some(Conflicted { path: fields[9].to_string(), remote, mine })
            })
            .collect()
    }

    /// Resolves one file and stages it. A side that deleted the file deletes
    /// it when it is kept.
    pub fn resolve(&self, file: &Conflicted, resolution: &Resolution) -> Result<(), GitFailure> {
        match resolution {
            Resolution::Text(text) => {
                let target = self.root.join(&file.path);
                let mut temporary = target.clone().into_os_string();
                temporary.push(".daftar-resolve");
                std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, &target)).map_err(|e| GitFailure {
                    arguments: vec!["add".into(), file.path.clone()],
                    status: None,
                    output: e.to_string(),
                })?;
                self.run(&["add", "--", &file.path])?;
            }
            // In a rebase `--ours` is the upstream and `--theirs` the replayed commit.
            Resolution::KeepRemote => self.keep(file, file.remote, "--ours")?,
            Resolution::KeepMine => self.keep(file, file.mine, "--theirs")?,
        }
        Ok(())
    }

    fn keep(&self, file: &Conflicted, side: Side, flag: &str) -> Result<(), GitFailure> {
        if side == Side::Deleted {
            self.run(&["rm", "-q", "--ignore-unmatch", "--", &file.path])?;
        } else {
            self.run(&["checkout", flag, "--", &file.path])?;
            self.run(&["add", "--", &file.path])?;
        }
        Ok(())
    }

    /// Goes on with the rebase once every file is resolved. Returns the
    /// conflicts of the next commit it replays, if that one stops too, or
    /// none when the rebase is done. Git is never let to open an editor.
    pub fn continue_rebase(&self) -> Result<Vec<Conflicted>, GitFailure> {
        while self.rebase_in_progress()? {
            let open = self.conflicts()?;
            if !open.is_empty() {
                return Ok(open);
            }
            // A stop on the next commit's conflict is not a failure.
            if let Err(failure) = self.exec(&["rebase", "--continue"], &[("GIT_EDITOR", "true")])
                && (!self.rebase_in_progress()? || self.conflicts()?.is_empty())
            {
                return Err(failure);
            }
        }
        Ok(Vec::new())
    }

    /// Resolves [resolutions], goes on with the rebase and pushes when it is
    /// done. Returns the next commit's conflicts when the rebase stops again.
    pub fn finish_rebase(&self, resolutions: &[(Conflicted, Resolution)]) -> Result<Vec<Conflicted>, GitFailure> {
        for (file, resolution) in resolutions {
            self.resolve(file, resolution)?;
        }
        let open = self.continue_rebase()?;
        if open.is_empty() {
            self.push_if_ahead()?;
        }
        Ok(open)
    }

    /// Ends the open rebase and puts the branch back as it was before the pull.
    pub fn abort_rebase(&self) -> Result<(), GitFailure> {
        self.run(&["rebase", "--abort"]).map(|_| ())
    }

    // History

    /// The commits that changed [path], newest first, across renames.
    pub fn history(&self, path: &str, limit: usize) -> Result<Vec<Revision>, GitFailure> {
        let limit = limit.to_string();
        let format = "--format=%x1e%H%x1f%aI%x1f%an%x1f%s";
        Ok(Self::revisions(&self.run_raw(&[
            "-c",
            "core.quotePath=false",
            "log",
            "--follow",
            "-n",
            &limit,
            format,
            "--name-only",
            "--",
            path,
        ])?))
    }

    /// What `git log --format=%x1e%H%x1f%aI%x1f%an%x1f%s --name-only` printed.
    pub fn revisions(output: &str) -> Vec<Revision> {
        output
            .split('\u{1e}')
            .filter_map(|record| {
                let mut lines = record.split('\n').filter(|l| !l.is_empty());
                let head = lines.next()?;
                let fields: Vec<&str> = head.splitn(4, '\u{1f}').collect();
                if fields.len() != 4 {
                    return None;
                }
                let date = DateTime::parse_from_rfc3339(fields[1]).ok()?;
                let path = lines.next_back().unwrap_or("").to_string();
                Some(Revision { hash: fields[0].into(), date, author: fields[2].into(), subject: fields[3].into(), path })
            })
            .collect()
    }

    /// The text of [path] as commit [hash] has it, to the last byte.
    pub fn content(&self, path: &str, hash: &str) -> Result<String, GitFailure> {
        self.run_raw(&["show", &format!("{hash}:{path}")])
    }

    /// The obsidian-git message this vault's history is written with:
    /// `update notes @ 2026-10-25 09:30:00 on host`.
    pub fn message(at: DateTime<Local>, host: &str) -> String {
        let name = host.split('.').next().unwrap_or(host);
        format!("update notes @ {} on {name}", at.format("%Y-%m-%d %H:%M:%S"))
    }

    /// The files changed here and not committed, which a read-only clone
    /// never commits, as `doctor` says them.
    fn diagnose_read_only(&self) -> Result<Vec<Diagnosis>, GitFailure> {
        let mut found = Vec::new();
        if self.is_read_only() {
            found.push(Diagnosis::info("read-only", "this clone is read-only (daftar.readonly): Daftar pulls and never writes"));
            let changes = self.changes()?;
            if !changes.is_empty() {
                found.push(Diagnosis::info(
                    "read-only-changes",
                    &format!("{} files changed here, which a read-only clone never commits: {}", changes.len(), changes.join(", ")),
                ));
            }
            if !crate::VaultSettings::read(&self.root).agents_in_read_only_clones {
                found.push(Diagnosis::info("agents", "agents: refused in this read-only clone"));
            }
        }
        let computer = match &self.computer {
            Some(computer) => computer.clone(),
            None => std::fs::read_to_string(ConfigFile::default_path()).unwrap_or_default(),
        };
        let settings = Settings::of(&self.root, &computer);
        for key in settings.refused() {
            found.push(Diagnosis::warning(
                "vault-config-refused",
                &format!("{key} in .daftar.toml is not the vault's to set: it is ignored"),
            ));
        }
        for line in crate::notification_rules::DigestFilter::of(settings.file()).not_understood() {
            found.push(Diagnosis::warning(
                "digest-filter-not-understood",
                &format!("notifications.digest_filter has a line the Tasks query language does not read, so it is left out: {line}"),
            ));
        }
        Ok(found)
    }

    /// This machine's name, for the message.
    pub fn host() -> String {
        for variable in ["HOSTNAME", "COMPUTERNAME"] {
            if let Some(name) = std::env::var_os(variable).filter(|v| !v.is_empty()) {
                return name.to_string_lossy().into_owned();
            }
        }
        if let Ok(name) = std::fs::read_to_string("/etc/hostname")
            && !name.trim().is_empty()
        {
            return name.trim().to_string();
        }
        Command::new("hostname")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "localhost".into())
    }
}

/// The git commands that make commits, which take the author's options.
const COMMITS: [&str; 6] = ["commit", "pull", "rebase", "merge", "cherry-pick", "revert"];

/// What `Git::read_only` found, by the vault's folder.
static KNOWN: Mutex<Option<HashMap<PathBuf, Known>>> = Mutex::new(None);

/// The clone's config was changed here: git is asked again.
fn forget_read_only(root: &Path) {
    if let Some(known) = KNOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
        known.remove(root);
    }
}

/// What `Git::read_only` knows of one vault, and the clone's config as it
/// was when git was asked.
struct Known {
    read_only: bool,
    config: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    asked: Instant,
}

impl Known {
    /// A repository's answer holds until its config changes; a folder that
    /// was no repository is asked about again after two seconds.
    fn holds(&self) -> bool {
        match &self.config {
            Some(config) => stamp(config) == self.stamp,
            None => self.asked.elapsed() < Duration::from_secs(2),
        }
    }
}

fn stamp(config: &Path) -> Option<(SystemTime, u64)> {
    let metadata = std::fs::metadata(config).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

/// [text] as one word of a shell command.
fn shell_quoted(text: &str) -> String {
    let plain = !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@%=:,".contains(c));
    if plain { text.to_string() } else { format!("'{}'", text.replace('\'', "'\\''")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflict_text::{Choice, ConflictText};
    use chrono::TimeZone;
    use std::fs;
    use std::path::Path;

    #[test]
    fn the_message_is_obsidian_gits() {
        let at = Local.with_ymd_and_hms(2026, 10, 25, 9, 30, 0).unwrap();
        assert_eq!(Git::message(at, "x-wing.local"), "update notes @ 2026-10-25 09:30:00 on x-wing");
    }

    #[test]
    fn status_reads_porcelain_v2() {
        let text = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b Notes/Inbox.md\n? new.md\n";
        assert_eq!(Git::parse_status(text), GitStatus { branch: "main".into(), ahead: 2, behind: 1, changed: 2 });
    }

    #[test]
    fn the_branch_is_diagnosed_from_status() {
        let codes = |text: &str| Git::diagnose_status(text).iter().map(|d| d.code).collect::<Vec<_>>();
        assert_eq!(codes("# branch.head main\n# branch.upstream origin/main\n# branch.ab +0 -0\n"), [] as [&str; 0]);
        assert_eq!(codes("# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n? a.md\n"), ["ahead", "behind", "changed"]);
        assert_eq!(codes("# branch.head main\n"), ["no-upstream"]);
        assert_eq!(codes("# branch.head (detached)\n"), ["detached"]);
    }

    #[test]
    fn the_doctor_finds_an_open_merge_and_its_conflict() {
        let (_base, one, two) = machines();
        fs::write(one.root.join("a.md"), "# A\none\n").unwrap();
        one.sync("one").unwrap();
        fs::write(two.root.join("a.md"), "# A\ntwo\n").unwrap();
        git(&two.root, &["commit", "-qam", "two"]);
        let _ = Command::new("git").arg("-C").arg(&two.root).args(["-c", "pull.rebase=false", "pull", "-q"]).output();
        fs::write(two.root.join("big.bin"), vec![0; 2048]).unwrap();
        let codes: Vec<&str> = two.doctor(1024).unwrap().iter().map(|d| d.code).collect();
        assert!(codes.starts_with(&["operation-open", "conflicts"]), "{codes:?}");
        assert!(codes.contains(&"large-file"), "{codes:?}");
        assert!(!codes.contains(&"large-tracked"), "{codes:?}");
        let found = two.doctor(4).unwrap();
        let tracked: Vec<&str> = found.iter().filter(|d| d.code == "large-tracked").map(|d| d.message.as_str()).collect();
        assert!(tracked.iter().any(|m| m.starts_with("a.md is")) && found.iter().all(|d| d.code != "large-tracked" || d.level == "info"));
    }

    #[test]
    fn changed_paths_keep_a_first_letter_that_is_not_ascii() {
        let output = " M یادداشت.md\0R  new name.md\0old name.md\0?? 🎉.md\0";
        assert_eq!(Git::changed_paths(output), ["یادداشت.md", "new name.md", "🎉.md"]);
    }

    #[test]
    fn sync_commits_a_note_whose_name_is_persian() {
        let (_base, one, _two) = machines();
        fs::write(one.root.join("یادداشت.md"), "# یادداشت\n").unwrap();
        one.sync("add").unwrap();
        // Unstaged, so git's first record starts with a space: ` M یادداشت.md`.
        fs::write(one.root.join("یادداشت.md"), "# یادداشت\nتغییر\n").unwrap();
        assert!(one.sync("update notes @ test").unwrap().committed);
    }

    #[test]
    fn sync_refuses_an_open_merge() {
        let (_base, one, two) = machines();
        fs::write(one.root.join("a.md"), "# A\none\n").unwrap();
        one.sync("one").unwrap();
        fs::write(two.root.join("a.md"), "# A\ntwo\n").unwrap();
        git(&two.root, &["commit", "-qam", "two"]);
        let merged = Command::new("git").arg("-C").arg(&two.root).args(["-c", "pull.rebase=false", "pull", "-q"]).output().unwrap();
        assert!(!merged.status.success());
        assert_eq!(two.other_operation_in_progress().unwrap(), Some("merge"));
        assert!(matches!(two.sync("over the merge"), Err(SyncFailure::Git(_))));
        assert!(fs::read_to_string(two.root.join("a.md")).unwrap().contains("<<<<<<<"));
        assert_eq!(subjects(&two)[0], "two");
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success();
        assert!(ok, "git {args:?}");
    }

    /// A remote and two clones of it, as two machines with one vault.
    fn machines() -> (tempfile::TempDir, Git, Git) {
        let base = tempfile::tempdir().unwrap();
        let remote = base.path().join("remote.git");
        git(base.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        for name in ["one", "two"] {
            git(base.path(), &["clone", "-q", remote.to_str().unwrap(), name]);
            let dir = base.path().join(name);
            git(&dir, &["config", "user.email", "test@example.com"]);
            git(&dir, &["config", "user.name", "Test"]);
            git(&dir, &["config", "commit.gpgsign", "false"]);
            // Windows' git writes \r\n into the files it checks out.
            git(&dir, &["config", "core.autocrlf", "false"]);
            git(&dir, &["checkout", "-q", "-b", "main"]);
        }
        let one = Git::new(base.path().join("one"));
        fs::write(one.root.join("a.md"), "# A\n").unwrap();
        git(&one.root, &["add", "-A"]);
        git(&one.root, &["commit", "-qm", "start"]);
        git(&one.root, &["push", "-q", "-u", "origin", "main"]);
        let two = Git::new(base.path().join("two"));
        git(&two.root, &["pull", "-q", "origin", "main"]);
        git(&two.root, &["branch", "-q", "--set-upstream-to=origin/main"]);
        (base, one, two)
    }

    #[test]
    fn sync_commits_pulls_and_pushes() {
        let (_base, one, two) = machines();
        assert!(one.is_repository());
        fs::write(one.root.join("b.md"), "# B\n").unwrap();
        let done = one.sync("update notes @ test").unwrap();
        assert_eq!(done, Synced { committed: true, pulled: false, pushed: true });
        // The other machine, with its own change, gets it and sends its own.
        fs::write(two.root.join("c.md"), "# C\n").unwrap();
        let done = two.sync("update notes @ test").unwrap();
        assert_eq!(done, Synced { committed: true, pulled: true, pushed: true });
        assert!(two.root.join("b.md").exists());
        assert_eq!(one.sync("nothing").unwrap(), Synced { committed: false, pulled: true, pushed: false });
        assert!(one.root.join("c.md").exists());
        assert_eq!(one.sync("nothing").unwrap(), Synced::default());
    }

    fn read(git: &Git, path: &str) -> Option<String> {
        fs::read_to_string(git.root.join(path)).ok()
    }

    fn subjects(git: &Git) -> Vec<String> {
        git.run(&["log", "--format=%s"]).unwrap().lines().map(String::from).collect()
    }

    /// Each file at its first text on both machines, then the second machine's
    /// text synced, and the first's own written but not committed.
    fn diverge(files: &[(&str, &str, &str, &str)]) -> (tempfile::TempDir, Git, Git) {
        let (base, one, two) = machines();
        for (path, first, _, _) in files {
            fs::write(one.root.join(path), first).unwrap();
        }
        one.sync("first").unwrap();
        two.pull().unwrap();
        for (path, _, theirs, _) in files {
            fs::write(two.root.join(path), theirs).unwrap();
        }
        two.sync("phone").unwrap();
        for (path, _, _, mine) in files {
            fs::write(one.root.join(path), mine).unwrap();
        }
        (base, one, two)
    }

    fn stopped(result: Result<Synced, SyncFailure>) -> Stopped {
        match result {
            Err(SyncFailure::Stopped(stopped)) => stopped,
            other => panic!("not stopped: {other:?}"),
        }
    }

    #[test]
    fn a_stopped_pull_is_kept_and_resolved() {
        let (_base, mac, phone) = diverge(&[("N.md", "a\nline\nz\n", "a\nphone line\nz\n", "a\nmac line\nz\n")]);
        let stop = stopped(mac.sync("mac"));
        assert_eq!(stop.conflicts, vec![Conflicted { path: "N.md".into(), remote: Side::Changed, mine: Side::Changed }]);
        assert!(mac.rebase_in_progress().unwrap());
        // Another sync never commits the marks over the open rebase.
        stopped(mac.sync("again"));
        assert!(matches!(mac.pull(), Err(SyncFailure::Stopped(_))));

        let text = read(&mac, "N.md").unwrap();
        let conflicts = ConflictText::conflicts(&text);
        assert_eq!(conflicts.iter().map(|c| c.remote.as_str()).collect::<Vec<_>>(), ["phone line\n"]);
        assert_eq!(conflicts.iter().map(|c| c.mine.as_str()).collect::<Vec<_>>(), ["mac line\n"]);
        let resolved = ConflictText::resolve_all(&ConflictText::parse(&text), Choice::Both);
        let open = mac.finish_rebase(&[(stop.conflicts[0].clone(), Resolution::Text(resolved))]).unwrap();
        assert!(open.is_empty());
        assert!(!mac.rebase_in_progress().unwrap());
        assert_eq!(read(&mac, "N.md").unwrap(), "a\nphone line\nmac line\nz\n");
        assert_eq!(subjects(&mac)[..3], ["mac", "phone", "first"]);
        // The resolution reached the remote.
        phone.pull().unwrap();
        assert_eq!(read(&phone, "N.md").unwrap(), "a\nphone line\nmac line\nz\n");
    }

    #[test]
    fn each_replayed_commit_that_conflicts_stops_again() {
        let (_base, mac, _phone) = diverge(&[("N.md", "x\n", "phone\n", "mac one\n")]);
        mac.run(&["commit", "-qam", "mac one"]).unwrap();
        fs::write(mac.root.join("N.md"), "mac two\n").unwrap();
        let stop = stopped(mac.sync("mac two"));
        // The phone's line wins over "mac one", so "mac two" does not apply either.
        let next = mac.finish_rebase(&[(stop.conflicts[0].clone(), Resolution::KeepRemote)]).unwrap();
        assert_eq!(next.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["N.md"]);
        assert!(mac.rebase_in_progress().unwrap());
        let mine: Vec<String> = ConflictText::conflicts(&read(&mac, "N.md").unwrap()).into_iter().map(|c| c.mine).collect();
        assert_eq!(mine, ["mac two\n"]);
        assert!(mac.finish_rebase(&[(next[0].clone(), Resolution::KeepMine)]).unwrap().is_empty());
        assert_eq!(subjects(&mac)[..3], ["mac two", "phone", "first"]);
        assert_eq!(read(&mac, "N.md").unwrap(), "mac two\n");
    }

    #[test]
    fn keeping_the_remote_everywhere_drops_the_empty_commit() {
        let (_base, mac, _phone) = diverge(&[("N.md", "x\n", "phone\n", "mac\n")]);
        let stop = stopped(mac.sync("mac"));
        assert!(mac.finish_rebase(&[(stop.conflicts[0].clone(), Resolution::KeepRemote)]).unwrap().is_empty());
        assert_eq!(subjects(&mac)[..2], ["phone", "first"]);
        assert_eq!(read(&mac, "N.md").unwrap(), "phone\n");
    }

    #[test]
    fn abort_puts_the_branch_back() {
        let (_base, mac, _phone) = diverge(&[("N.md", "x\n", "phone\n", "mac\n")]);
        stopped(mac.sync("mac"));
        mac.abort_rebase().unwrap();
        assert!(!mac.rebase_in_progress().unwrap());
        assert_eq!(subjects(&mac)[..2], ["mac", "first"]);
        assert_eq!(read(&mac, "N.md").unwrap(), "mac\n");
    }

    #[test]
    fn other_files_keep_one_side_and_a_deletion_is_a_side() {
        let (_base, mac, _phone) =
            diverge(&[("data.json", "{}\n", "{\"a\":1}\n", "{\"b\":2}\n"), ("Gone.md", "old\n", "phone\n", "old\n")]);
        // The mac deletes a note the phone changed.
        fs::remove_file(mac.root.join("Gone.md")).unwrap();
        let mut conflicts = stopped(mac.sync("mac")).conflicts;
        conflicts.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            conflicts,
            vec![
                Conflicted { path: "Gone.md".into(), remote: Side::Changed, mine: Side::Deleted },
                Conflicted { path: "data.json".into(), remote: Side::Changed, mine: Side::Changed },
            ]
        );
        assert!(!conflicts[0].has_marks());
        let resolutions: Vec<_> = conflicts
            .iter()
            .map(|c| (c.clone(), if c.path == "Gone.md" { Resolution::KeepMine } else { Resolution::KeepRemote }))
            .collect();
        assert!(mac.finish_rebase(&resolutions).unwrap().is_empty());
        assert_eq!(read(&mac, "Gone.md"), None);
        assert_eq!(read(&mac, "data.json").unwrap(), "{\"a\":1}\n");
    }

    #[test]
    fn the_status_records_are_read() {
        let output = "1 .M N... 100644 100644 100644 a b x.md\0u UD N... 100644 100644 000000 100644 h1 h2 h3 A note.md\0u AA N... 000000 100644 100644 100644 h1 h2 h3 b.png\0";
        assert_eq!(
            Git::conflicted(output),
            vec![
                Conflicted { path: "A note.md".into(), remote: Side::Changed, mine: Side::Deleted },
                Conflicted { path: "b.png".into(), remote: Side::Added, mine: Side::Added },
            ]
        );
    }

    #[test]
    fn the_ssh_command_keeps_the_clones_key() {
        assert_eq!(Git::ssh_command(None, None), "ssh -o BatchMode=yes");
        assert_eq!(
            Git::ssh_command(Some("ssh -i ~/.ssh/notes_ro -o IdentitiesOnly=yes"), None),
            "ssh -i ~/.ssh/notes_ro -o IdentitiesOnly=yes -o BatchMode=yes"
        );
        // An option the command names already is not added again, in any case.
        assert_eq!(Git::ssh_command(Some("ssh -o batchmode=no"), None), "ssh -o batchmode=no");
        // The person's own variable is used as it is.
        assert_eq!(Git::ssh_command(Some("ssh -i ~/.ssh/notes_ro"), Some("ssh -v")), "ssh -v");
        assert_eq!(Git::ssh_command(None, Some("ssh -v")), "ssh -v");
        // Empty or blank is as not set.
        assert_eq!(Git::ssh_command(Some("ssh -i key"), Some("")), "ssh -i key -o BatchMode=yes");
        assert_eq!(Git::ssh_command(Some("ssh -i key"), Some("  ")), "ssh -i key -o BatchMode=yes");
        assert_eq!(Git::ssh_command(Some(""), None), "ssh -o BatchMode=yes");
        assert_eq!(Git::ssh_command(Some(" \t"), Some(" ")), "ssh -o BatchMode=yes");
    }

    #[test]
    fn the_clones_ssh_command_reaches_git() {
        let (_base, one, _two) = machines();
        assert_eq!(one.find_ssh_command(), "ssh -o BatchMode=yes");
        git(&one.root, &["config", "core.sshCommand", "ssh -i /keys/notes_ro -o IdentitiesOnly=yes"]);
        assert_eq!(Git::new(&one.root).find_ssh_command(), "ssh -i /keys/notes_ro -o IdentitiesOnly=yes -o BatchMode=yes");
        let asked = Git::new(&one.root).with_environment("GIT_SSH_COMMAND", "ssh -v");
        assert_eq!(asked.find_ssh_command(), "ssh -v");
    }

    #[test]
    fn a_key_is_written_as_the_clones_ssh_command() {
        assert_eq!(
            Git::ssh_command_for_key(Path::new("/home/me/.ssh/daftar_notes_ed25519")),
            "ssh -i /home/me/.ssh/daftar_notes_ed25519 -o IdentitiesOnly=yes"
        );
        let spaced = Git::ssh_command_for_key(Path::new("/home/me/My Keys/it's"));
        assert_eq!(spaced, "ssh -i '/home/me/My Keys/it'\\''s' -o IdentitiesOnly=yes");
        assert_eq!(Git::ssh_key(&spaced), Some(PathBuf::from("/home/me/My Keys/it's")));
        assert_eq!(Git::ssh_key("ssh -i /k/a -o IdentitiesOnly=yes"), Some(PathBuf::from("/k/a")));
        // A command the app did not write is shown as text, not as a key.
        assert_eq!(Git::ssh_key("ssh -i /k/a -o IdentitiesOnly=yes -v"), None);
        assert_eq!(Git::ssh_key("ssh -F ~/.ssh/work"), None);
    }

    #[test]
    fn a_key_is_never_generated_over_one_that_is_there() {
        let base = tempfile::tempdir().unwrap();
        let vault = base.path().join("My Notes");
        fs::create_dir_all(&vault).unwrap();
        let keys = base.path().join("ssh");
        let Ok((file, public)) = Git::new(&vault).generate_key(&keys, "x-wing.local") else { return };
        assert_eq!(file, keys.join("daftar_My_Notes_ed25519"));
        assert!(public.starts_with("ssh-ed25519 ") && public.ends_with(" daftar My Notes on x-wing"), "{public}");
        let before = fs::read(&file).unwrap();
        assert!(Git::new(&vault).generate_key(&keys, "x-wing.local").is_err());
        assert_eq!(fs::read(&file).unwrap(), before);
    }

    /// A repository of its own, with no author in its config.
    fn repository() -> (tempfile::TempDir, Git) {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("vault");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        let git = Git::new(root);
        (base, git)
    }

    #[test]
    fn a_clone_is_read_only_by_its_own_config() {
        let (base, vault) = repository();
        assert!(!vault.is_read_only());
        for (value, read_only) in [
            ("true", true),
            ("yes", true),
            ("on", true),
            ("1", true),
            ("false", false),
            ("no", false),
            ("off", false),
            ("0", false),
            ("perhaps", true),
        ] {
            git(&vault.root, &["config", "daftar.readonly", value]);
            assert_eq!(vault.is_read_only(), read_only, "{value}");
            assert_eq!(Git::read_only(&vault.root), read_only, "{value}");
        }
        git(&vault.root, &["config", "--unset", "daftar.readonly"]);
        assert!(!vault.is_read_only());
        // The switch writes the key, and takes it out.
        vault.set_read_only(true).unwrap();
        assert!(vault.is_read_only() && Git::read_only(&vault.root));
        assert_eq!(vault.local("daftar.readonly").as_deref(), Some("true"));
        vault.set_read_only(false).unwrap();
        assert!(!vault.is_read_only() && !Git::read_only(&vault.root));
        assert_eq!(vault.local("daftar.readonly"), None);
        // The same key in the global config is not read.
        let global = base.path().join("global.gitconfig");
        fs::write(&global, "[daftar]\n\treadonly = true\n").unwrap();
        let with_global = Git::new(&vault.root).with_environment("GIT_CONFIG_GLOBAL", global.to_str().unwrap());
        assert_eq!(with_global.ask(&["config", "--get", "daftar.readonly"]).as_deref(), Some("true"));
        assert!(!with_global.is_read_only());
        // A folder that is no repository is not read-only.
        let plain = base.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        let outside = Git::new(&plain).with_environment("GIT_CEILING_DIRECTORIES", base.path().to_str().unwrap());
        assert!(!outside.is_repository() && !outside.is_read_only());
    }

    fn remote_head(base: &Path) -> String {
        let out = Command::new("git").arg("-C").arg(base.join("remote.git")).args(["rev-parse", "main"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn a_read_only_sync_only_pulls() {
        let (base, one, two) = machines();
        two.set_read_only(true).unwrap();
        fs::write(one.root.join("b.md"), "# B\n").unwrap();
        one.sync("from one").unwrap();
        let pushed = remote_head(base.path());
        // A file changed by hand in the read-only clone is never committed.
        fs::write(two.root.join("a.md"), "# A\nby hand\n").unwrap();
        assert_eq!(two.sync("never").unwrap(), Synced { committed: false, pulled: true, pushed: false });
        assert_eq!(read(&two, "b.md").unwrap(), "# B\n");
        assert_eq!(two.run(&["rev-list", "@{u}..HEAD"]).unwrap(), "");
        assert_eq!(remote_head(base.path()), pushed);
        assert_eq!(subjects(&two), ["from one", "start"]);
        assert_eq!(read(&two, "a.md").unwrap(), "# A\nby hand\n");
        assert_eq!(two.changes().unwrap(), ["a.md"]);
        // A read-only clone that is only ahead of its remote has nothing to do, and succeeds.
        let (_ahead_base, _first, ahead) = machines();
        fs::write(ahead.root.join("mine.md"), "# Mine\n").unwrap();
        git(&ahead.root, &["add", "-A"]);
        git(&ahead.root, &["commit", "-qm", "its own"]);
        ahead.set_read_only(true).unwrap();
        assert_eq!(ahead.sync("never").unwrap(), Synced::default());
        assert_eq!(subjects(&ahead), ["its own", "start"]);
        // Again, with nothing new: up to date, and the pull alone is the same.
        assert_eq!(two.sync("never").unwrap(), Synced::default());
        two.pull().unwrap();
        assert_eq!(two.changes().unwrap(), ["a.md"]);
        assert_eq!(remote_head(base.path()), pushed);
    }

    #[test]
    fn a_read_only_sync_does_not_mend_a_diverged_clone() {
        let (_base, one, two) = machines();
        fs::write(two.root.join("mine.md"), "# Mine\n").unwrap();
        git(&two.root, &["add", "-A"]);
        git(&two.root, &["commit", "-qm", "its own"]);
        fs::write(two.root.join("a.md"), "# A\nby hand\n").unwrap();
        two.set_read_only(true).unwrap();
        fs::write(one.root.join("b.md"), "# B\n").unwrap();
        one.sync("another").unwrap();
        let head = two.run(&["rev-parse", "HEAD"]).unwrap();
        assert_eq!(two.sync("never"), Err(SyncFailure::ReadOnlyDiverged(vec!["a.md".into(), "mine.md".into()])));
        assert_eq!(two.run(&["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(two.status().unwrap().branch, "main");
        assert_eq!(subjects(&two), ["its own", "start"]);
        assert_eq!(read(&two, "a.md").unwrap(), "# A\nby hand\n");
        assert_eq!(two.changes().unwrap(), ["a.md"]);
        assert!(!two.rebase_in_progress().unwrap() && two.other_operation_in_progress().unwrap().is_none());
        assert!(!two.root.join("b.md").exists());
        // A change in the way of the pull, with no commit of its own, is named too.
        let (_base, one, two) = machines();
        two.set_read_only(true).unwrap();
        fs::write(two.root.join("a.md"), "# A\nby hand\n").unwrap();
        fs::write(one.root.join("a.md"), "# A\nfrom one\n").unwrap();
        one.sync("another").unwrap();
        assert_eq!(two.sync("never"), Err(SyncFailure::ReadOnlyDiverged(vec!["a.md".into()])));
        assert_eq!(read(&two, "a.md").unwrap(), "# A\nby hand\n");
        assert_eq!(subjects(&two), ["start"]);
    }

    #[test]
    fn a_refused_push_is_named() {
        let deploy =
            "ERROR: The key you are authenticating with has been marked as read only.\nfatal: Could not read from remote repository.";
        let granted = "ERROR: Write access to repository not granted.\nfatal: Could not read from remote repository.";
        let denied = "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.";
        assert!(Git::push_refused_read_only(deploy));
        assert!(Git::push_refused_read_only(granted));
        assert!(!Git::push_refused_read_only(denied));
        let failure = |output: &str| GitFailure { arguments: vec!["push".into()], status: Some(128), output: output.into() };
        assert!(matches!(Git::push_failure(failure(deploy)), SyncFailure::PushRefusedReadOnly(_)));
        assert!(matches!(Git::push_failure(failure(granted)), SyncFailure::PushRefusedReadOnly(_)));
        assert!(matches!(Git::push_failure(failure(denied)), SyncFailure::Git(_)));
    }

    #[test]
    fn the_address_is_the_upstreams_remote() {
        let (base, one, two) = machines();
        let first = base.path().join("remote.git");
        assert_eq!(two.remote(), "origin");
        assert_eq!(two.address().as_deref(), first.to_str());
        // A second remote with the same history: a mirror.
        let mirror = base.path().join("mirror.git");
        git(base.path(), &["clone", "-q", "--bare", first.to_str().unwrap(), mirror.to_str().unwrap()]);
        // An empty or blank address is refused, and the old one stays.
        assert!(two.set_address("").is_err() && two.set_address("  \n").is_err());
        assert_eq!(two.address().as_deref(), first.to_str());
        two.set_address(&format!(" {} ", mirror.display())).unwrap();
        assert_eq!(two.local("remote.origin.url").as_deref(), mirror.to_str());
        assert_eq!(two.address().as_deref(), mirror.to_str());
        // The next sync pulls a commit pushed there.
        fs::write(one.root.join("b.md"), "# B\n").unwrap();
        git(&one.root, &["add", "-A"]);
        git(&one.root, &["commit", "-qm", "to the mirror"]);
        git(&one.root, &["push", "-q", mirror.to_str().unwrap(), "main"]);
        assert!(two.sync("nothing").unwrap().pulled);
        assert_eq!(read(&two, "b.md").unwrap(), "# B\n");
        // With the upstream on a remote named `backup`, that is the one read and written.
        let backup = base.path().join("backup.git");
        git(base.path(), &["clone", "-q", "--bare", first.to_str().unwrap(), backup.to_str().unwrap()]);
        git(&two.root, &["remote", "add", "backup", first.to_str().unwrap()]);
        git(&two.root, &["fetch", "-q", "backup"]);
        git(&two.root, &["branch", "-q", "--set-upstream-to=backup/main"]);
        assert_eq!(two.remote(), "backup");
        assert_eq!(two.address().as_deref(), first.to_str());
        two.set_address(backup.to_str().unwrap()).unwrap();
        assert_eq!(two.local("remote.backup.url").as_deref(), backup.to_str());
        assert_eq!(two.local("remote.origin.url").as_deref(), mirror.to_str());
    }

    #[test]
    fn a_clone_with_no_remote_takes_its_address_and_its_upstream_at_the_next_pull() {
        let (base, one, alone) = machines();
        git(&alone.root, &["remote", "remove", "origin"]);
        fs::write(one.root.join("b.md"), "# B\n").unwrap();
        one.sync("later").unwrap();
        assert_eq!(alone.address(), None);
        alone.set_address(base.path().join("remote.git").to_str().unwrap()).unwrap();
        assert_eq!(alone.local("remote.origin.url").as_deref(), base.path().join("remote.git").to_str());
        // The upstream is written then, as config, and nothing is fetched until the next sync.
        assert_eq!(alone.local("branch.main.remote").as_deref(), Some("origin"));
        assert_eq!(alone.local("branch.main.merge").as_deref(), Some("refs/heads/main"));
        assert!(alone.ask(&["rev-parse", "--verify", "-q", "refs/remotes/origin/main"]).is_none());
        alone.set_read_only(true).unwrap();
        alone.pull().unwrap();
        assert_eq!(read(&alone, "b.md").unwrap(), "# B\n");
        assert_eq!(subjects(&alone), subjects(&one));
        assert_eq!(alone.ask(&["rev-parse", "--abbrev-ref", "@{upstream}"]).as_deref(), Some("origin/main"));
    }

    #[test]
    fn the_author_is_the_clones_then_the_vaults() {
        let (base, vault) = repository();
        let global = base.path().join("global.gitconfig");
        fs::write(&global, "[user]\n\tname = Work\n\temail = work@example.com\n").unwrap();
        let open = |root: &Path| {
            Git::new(root)
                .with_environment("GIT_CONFIG_GLOBAL", global.to_str().unwrap())
                .with_environment("GIT_CONFIG_NOSYSTEM", "1")
                .with_computer("")
        };
        let author = |git: &Git| git.run(&["log", "-1", "--format=%an <%ae> %cn <%ce>"]).unwrap();
        let commit = |text: &str| {
            fs::write(vault.root.join("a.md"), text).unwrap();
            let git = open(&vault.root);
            git.run(&["add", "-A"]).unwrap();
            git.run(&["commit", "-qm", text.trim()]).unwrap();
            author(&git)
        };
        // With nothing else, the computer's.
        assert_eq!(commit("one\n"), "Work <work@example.com> Work <work@example.com>");
        // The vault's, for a clone with none of its own.
        fs::write(vault.root.join(".daftar.toml"), "[git]\nname = \"Notes\"\nemail = \"notes@example.com\"\n").unwrap();
        assert_eq!(commit("two\n"), "Notes <notes@example.com> Notes <notes@example.com>");
        // Each of the two is chosen on its own: the clone's email, the vault's name.
        git(&vault.root, &["config", "user.email", "clone@example.com"]);
        assert_eq!(commit("three\n"), "Notes <clone@example.com> Notes <clone@example.com>");
        // A computer that declines the vault's settings keeps its own author.
        git(&vault.root, &["config", "--unset", "user.email"]);
        fs::write(vault.root.join("a.md"), "four\n").unwrap();
        let declined = open(&vault.root).with_computer("vault_settings = false\n");
        declined.run(&["commit", "-qam", "four"]).unwrap();
        assert_eq!(author(&declined), "Work <work@example.com> Work <work@example.com>");
        assert_eq!(
            Git::author_options((None, Some("clone@example.com")), (Some("Notes"), Some("notes@example.com"))),
            ["-c", "user.name=Notes"]
        );
        assert_eq!(Git::author_options((None, None), (None, None)), [] as [&str; 0]);
    }

    #[test]
    fn the_doctor_says_a_clone_is_read_only_and_what_the_vault_may_not_set() {
        let (_base, _one, two) = machines();
        let two = Git::new(&two.root).with_computer("");
        let codes = |git: &Git| git.doctor(u64::MAX).unwrap().iter().map(|d| d.code).collect::<Vec<_>>();
        assert_eq!(codes(&two), [] as [&str; 0]);
        two.set_read_only(true).unwrap();
        assert_eq!(codes(&two), ["read-only"]);
        fs::write(two.root.join(".daftar.toml"), "[agents]\nread_only_clones = false\n\n[terminal]\nshell = \"/tmp/x\"\n").unwrap();
        let found = two.doctor(u64::MAX).unwrap();
        assert_eq!(
            found.iter().map(|d| d.code).collect::<Vec<_>>(),
            ["changed", "read-only", "read-only-changes", "agents", "vault-config-refused"]
        );
        assert_eq!(found[3].message, "agents: refused in this read-only clone");
        assert!(found[2].message.ends_with(": .daftar.toml") && found[4].message.starts_with("terminal.shell "));
        assert_eq!(found[4].level, "warning");
        // A clone that is not read-only is never touched by the agents key.
        two.set_read_only(false).unwrap();
        assert_eq!(codes(&two), ["changed", "vault-config-refused"]);
    }

    #[test]
    fn the_log_is_read() {
        let output = "\u{1e}abc\u{1f}2026-10-25T09:30:00+02:00\u{1f}Parham\u{1f}update notes @ x\n\nNotes/A.md\n\u{1e}def\u{1f}2026-10-24T08:00:00Z\u{1f}P\u{1f}a | b\u{1f}c\n\nOld/A.md";
        let revisions = Git::revisions(output);
        assert_eq!(revisions.iter().map(|r| r.hash.as_str()).collect::<Vec<_>>(), ["abc", "def"]);
        assert_eq!(revisions.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), ["Notes/A.md", "Old/A.md"]);
        assert_eq!(revisions[1].subject, "a | b\u{1f}c");
        assert_eq!(revisions[0].date.timestamp(), DateTime::parse_from_rfc3339("2026-10-25T07:30:00Z").unwrap().timestamp());
        assert!(Git::revisions("").is_empty());
        assert!(Git::revisions("\u{1e}broken").is_empty());
    }

    #[test]
    fn a_note_history_follows_its_renames() {
        let base = tempfile::tempdir().unwrap();
        let git = Git::new(base.path());
        git.run(&["init", "-q", "-b", "main"]).unwrap();
        git.run(&["config", "user.email", "test@example.com"]).unwrap();
        git.run(&["config", "user.name", "Test"]).unwrap();
        git.run(&["config", "commit.gpgsign", "false"]).unwrap();
        git.run(&["config", "core.autocrlf", "false"]).unwrap();
        fs::create_dir_all(base.path().join("Old")).unwrap();
        fs::write(base.path().join("Old/Nöte.md"), "one\n").unwrap();
        git.run(&["add", "-A"]).unwrap();
        git.run(&["commit", "-qm", "first"]).unwrap();
        fs::write(base.path().join("Old/Nöte.md"), "one\ntwo\n").unwrap();
        git.run(&["commit", "-qam", "second"]).unwrap();
        git.run(&["mv", "Old", "New"]).unwrap();
        git.run(&["commit", "-qm", "move"]).unwrap();

        let history = git.history("New/Nöte.md", 300).unwrap();
        assert_eq!(history.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(), ["move", "second", "first"]);
        assert_eq!(history.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), ["New/Nöte.md", "Old/Nöte.md", "Old/Nöte.md"]);
        assert_eq!(git.content(&history[2].path, &history[2].hash).unwrap(), "one\n");
        assert_eq!(git.content(&history[1].path, &history[1].hash).unwrap(), "one\ntwo\n");
        assert!(git.history("Missing.md", 300).unwrap().is_empty());
    }
}
