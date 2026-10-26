//! The vault's repository, through the system `git`. The person's own git
//! config, credential helper and SSH agent apply, as they do in a terminal;
//! Daftar holds no key and no token.

use std::fmt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use chrono::{DateTime, Local};

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
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root).args(arguments).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {
            command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
        }
        let failure = |status, output: String| GitFailure {
            arguments: arguments.iter().map(|a| a.to_string()).collect(),
            status,
            output: output.trim().to_string(),
        };
        let output = command.output().map_err(|e| failure(None, e.to_string()))?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        if output.status.success() { Ok(text.trim().to_string()) } else { Err(failure(output.status.code(), text)) }
    }

    pub fn is_repository(&self) -> bool {
        self.run(&["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out == "true")
    }

    /// Pulls with a rebase, keeping local changes aside while it does.
    pub fn pull(&self) -> Result<(), GitFailure> {
        self.run(&["pull", "--rebase", "--autostash"]).map(|_| ())
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
    /// workflow, as obsidian-git and the Mac app do it.
    pub fn sync(&self, message: &str) -> Result<Synced, GitFailure> {
        let mut done = Synced::default();
        if !self.changes()?.is_empty() {
            self.run(&["add", "-A"])?;
            self.run(&["commit", "-m", message])?;
            done.committed = true;
        }
        let before = self.run(&["rev-parse", "HEAD"])?;
        self.pull()?;
        done.pulled = self.run(&["rev-list", &format!("{before}..HEAD")])?.lines().any(|l| !l.is_empty());
        if !self.run(&["rev-list", "@{upstream}..HEAD"])?.is_empty() {
            self.run(&["push"])?;
            done.pushed = true;
        }
        Ok(done)
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
}
