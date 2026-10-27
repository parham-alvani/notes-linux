//! The vault's repository, through the system `git`. The person's own git
//! config, credential helper and SSH agent apply, as they do in a terminal;
//! Daftar holds no key and no token.

use std::fmt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use chrono::{DateTime, FixedOffset, Local};

pub struct Git {
    pub root: PathBuf,
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

/// Why a sync or a pull did not happen: git failed, or the pull's rebase
/// stopped on a conflict and is still open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncFailure {
    Git(GitFailure),
    Stopped(Stopped),
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
        Git { root: root.into() }
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
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root).args(arguments).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {
            command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        command.envs(environment.iter().copied());
        let failure = |status, output: String| GitFailure {
            arguments: arguments.iter().map(|a| a.to_string()).collect(),
            status,
            output: output.trim().to_string(),
        };
        let output = command.output().map_err(|e| failure(None, e.to_string()))?;
        let out = String::from_utf8_lossy(&output.stdout).into_owned();
        let err = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.success() { Ok((out, err)) } else { Err(failure(output.status.code(), out + &err)) }
    }

    pub fn is_repository(&self) -> bool {
        self.run(&["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out == "true")
    }

    /// Pulls with a rebase, keeping local changes aside while it does. A
    /// rebase that stops on a conflict stays open and is `Stopped`.
    pub fn pull(&self) -> Result<(), SyncFailure> {
        self.refuse_open_rebase()?;
        self.pull_rebase()
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
        Ok(())
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

    /// Paths with changes not yet committed.
    pub fn changes(&self) -> Result<Vec<String>, GitFailure> {
        let out = self.run(&["status", "--porcelain=v1", "-z"])?;
        Ok(out.split('\0').filter(|e| e.len() > 3).map(|e| e[3..].to_string()).collect())
    }

    /// Commits everything, pulls with a rebase, and pushes: the vault's own
    /// workflow, as obsidian-git and the Mac app do it. A rebase open on a
    /// conflict is never committed over: it is `Stopped`, and the person
    /// resolves it first.
    pub fn sync(&self, message: &str) -> Result<Synced, SyncFailure> {
        self.refuse_open_rebase()?;
        let mut done = Synced::default();
        if !self.changes()?.is_empty() {
            self.run(&["add", "-A"])?;
            self.run(&["commit", "-m", message])?;
            done.committed = true;
        }
        let before = self.run(&["rev-parse", "HEAD"])?;
        self.pull_rebase()?;
        done.pulled = self.run(&["rev-list", &format!("{before}..HEAD")])?.lines().any(|l| !l.is_empty());
        done.pushed = self.push_if_ahead()?;
        Ok(done)
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
            if std::path::Path::new(&path).exists() {
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
                std::fs::write(&temporary, text).and_then(|_| std::fs::rename(&temporary, &target)).map_err(|e| GitFailure {
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
