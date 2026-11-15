//! `daftar-cli hook`: the vault's pre-commit check, as CI runs it, and the
//! hook that calls it. The staged notes are formatted (and staged again),
//! linted and audited strictly; the rest of the vault is audited for errors
//! only, so a warning a note got from the passing days (a quiet job process)
//! never stops a commit of another note, while a broken link anywhere does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use daftar_core::Git;

/// The first line after the shebang of a hook this command wrote: a hook
/// without it is the person's, and `install` leaves it alone.
const MARK: &str = "# Installed by `daftar-cli hook install`";

/// What the hook file says: it runs the `daftar-cli` on `$PATH` when the
/// commit is made, so an upgrade needs no new hook.
pub fn script() -> String {
    format!(
        "#!/bin/sh\n{MARK}: formats, lints and audits the staged notes as CI does.\n\
         # `git commit --no-verify` skips it.\n\
         command -v daftar-cli > /dev/null || {{ echo 'pre-commit: daftar-cli is not on $PATH' >&2; exit 1; }}\n\
         exec daftar-cli hook pre-commit\n"
    )
}

/// The hook file git runs before a commit: in `core.hooksPath` when the
/// repository sets one, else in `.git/hooks`.
pub fn hook_file(git: &Git, root: &Path) -> Result<PathBuf, String> {
    let folder = git.run(&["rev-parse", "--git-path", "hooks"]).map_err(|e| e.to_string())?;
    let folder = PathBuf::from(folder.trim());
    Ok(if folder.is_absolute() { folder } else { root.join(folder) }.join("pre-commit"))
}

/// Writes the hook, never over one this command did not write unless [force].
pub fn install(git: &Git, root: &Path, force: bool) -> Result<PathBuf, String> {
    let file = hook_file(git, root)?;
    if let Ok(there) = fs::read_to_string(&file)
        && !there.contains(MARK)
        && !force
    {
        return Err(format!("{} is a hook of your own; --force puts this one in its place", file.display()));
    }
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    fs::write(&file, script()).map_err(|e| format!("{}: {e}", file.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(file)
}

/// The staged Markdown notes, from the vault's folder: those added, copied,
/// changed or renamed, and whether a note was deleted or renamed away, which
/// can break a link in a note not staged.
pub fn staged(git: &Git) -> Result<(Vec<String>, bool), String> {
    let names = |filter: &str| -> Result<Vec<String>, String> {
        let out = git
            .run_raw(&["diff", "--cached", "--relative", "--name-only", "-z", &format!("--diff-filter={filter}"), "--", "*.md"])
            .map_err(|e| e.to_string())?;
        Ok(out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
    };
    let kept = names("ACMR")?;
    let gone = !names("DR")?.is_empty();
    Ok((kept, gone))
}

/// The steps of a pre-commit check of [notes]: each a `daftar-cli`
/// command line, and whether its failure stops the commit.
pub fn steps(notes: &[String]) -> Vec<Vec<String>> {
    let with = |words: &[&str]| -> Vec<String> {
        let mut line: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        line.push("--".into());
        line.extend(notes.iter().cloned());
        line
    };
    let mut out = Vec::new();
    if !notes.is_empty() {
        out.push(with(&["fmt"]));
        out.push(with(&["lint"]));
        out.push(with(&["check", "--strict"]));
    }
    // Every note, errors only: a staged change can break a link elsewhere.
    out.push(vec!["check".into()]);
    out
}

/// Runs the check on what is staged; an error stops the commit.
pub fn pre_commit(root: &Path, git: &Git) -> Result<(), String> {
    let (notes, gone) = staged(git)?;
    if notes.is_empty() && !gone {
        return Ok(());
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    for step in steps(&notes) {
        let quiet = step[0] == "fmt";
        let mut command = Command::new(&me);
        command.arg("--vault").arg(root).args(&step).current_dir(root);
        if quiet {
            command.stdout(std::process::Stdio::null());
        }
        let status = command.status().map_err(|e| format!("{}: {e}", me.display()))?;
        if !status.success() {
            return Err(format!(
                "pre-commit: `daftar-cli {}` failed; `git commit --no-verify` commits anyway",
                step[..step.len().min(2)].join(" ")
            ));
        }
        if quiet {
            // The formatted notes are what is committed.
            let mut add = vec!["add", "--"];
            add.extend(notes.iter().map(String::as_str));
            git.run(&add).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, words: &[&str]) {
        let done = Command::new("git").arg("-C").arg(root).args(words).output().unwrap();
        assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
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
        git(root, &["-c", "user.name=T", "-c", "user.email=t@example.org", "commit", "-qm", "one"]);
        git(root, &["rm", "-q", "B.md"]);
        assert_eq!(staged(&repository).unwrap(), (Vec::new(), true));
    }

    #[test]
    fn install_leaves_a_hook_of_your_own() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        let repository = Git::new(root);
        let file = install(&repository, root, false).unwrap();
        assert_eq!(file, root.join(".git/hooks/pre-commit"));
        assert!(fs::read_to_string(&file).unwrap().contains("exec daftar-cli hook pre-commit"));
        // Its own hook is written again; the person's is not, but with --force.
        install(&repository, root, false).unwrap();
        fs::write(&file, "#!/bin/sh\nmine\n").unwrap();
        assert!(install(&repository, root, false).unwrap_err().contains("--force"));
        install(&repository, root, true).unwrap();
        // core.hooksPath is where git looks.
        git(root, &["config", "core.hooksPath", ".githooks"]);
        assert_eq!(install(&repository, root, false).unwrap(), root.join(".githooks/pre-commit"));
    }
}
