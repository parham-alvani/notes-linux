//! Running `daftar-cli` for the Problems panel, format on save and Export as
//! PDF (`docs/specs/problems-and-format.md` §1): found beside this binary or
//! on `$PATH`, run in the vault's folder off the main thread, with stdout and
//! stderr read together and stdin written, and ended when its work is
//! cancelled.

use std::path::{Path, PathBuf};

use daftar_core::ProblemTool;
use gtk::prelude::*;
use gtk::{gio, glib};

/// The notice for a `daftar-cli` too old for the Problems panel and format on save.
pub const UPGRADE: &str = "Problems and Format Note need daftar-cli 0.23.0 or later: update the daftar-cli package.";

/// The notice when there is no `daftar-cli` at all.
pub const MISSING: &str =
    "Problems and Format Note run daftar-cli, which is neither beside Daftar nor on $PATH: install the daftar-cli-bin package.";

/// The flags the Problems panel and format on save need, which a
/// `daftar-cli` before 0.23.0 does not know.
const NEW_FLAGS: [&str; 3] = ["--json", "--stdin", "--path"];

/// The `daftar-cli` beside this binary (both packages install the two
/// together), else the one on $PATH.
pub fn locate() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.with_file_name("daftar-cli");
    if beside.is_file() {
        return Some(beside);
    }
    glib::find_program_in_path("daftar-cli")
}

/// What a run printed, and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The exit status; `None` when a signal ended it (a cancel).
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// clap's refusal of the arguments: exit status 2 and an `error:` line.
    pub fn usage_error(&self) -> Option<&str> {
        if self.status != Some(2) {
            return None;
        }
        self.stderr.lines().map(str::trim).find(|line| line.starts_with("error: "))
    }

    /// A `daftar-cli` that does not know one of [NEW_FLAGS]: clap's
    /// "unexpected argument '--json'".
    pub fn too_old(&self) -> bool {
        self.usage_error()
            .is_some_and(|usage| NEW_FLAGS.iter().any(|flag| usage.starts_with(&format!("error: unexpected argument '{flag}'"))))
    }

    /// A `daftar-cli` whose `check` takes no files (before 0.23.0) says so as
    /// "unexpected argument '<file>'".
    pub fn refuses_paths(&self, paths: &[String]) -> bool {
        self.usage_error().is_some_and(|usage| {
            usage.starts_with("error: unexpected argument '--'")
                || paths.iter().any(|path| usage.starts_with(&format!("error: unexpected argument '{path}'")))
        })
    }

    /// Ended with exit status 0.
    pub fn succeeded(&self) -> bool {
        self.status == Some(0)
    }
}

/// What a failed run says: the last line of its stderr, without the
/// command's name; a usage error says its `error:` line.
pub fn failure(errors: &str) -> String {
    let lines: Vec<&str> = errors.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if let Some(usage) = lines.iter().find_map(|line| line.strip_prefix("error: ")) {
        return format!("daftar-cli: {usage}");
    }
    lines.last().map_or_else(|| "daftar-cli failed.".to_string(), |line| line.trim_start_matches("daftar-cli: ").to_string())
}

/// Runs [cli] with [arguments] in the vault's folder [root], [input] on its
/// stdin. A [background] run is niced, so typing never waits for it; a
/// cancel of [cancel] ends the process.
pub async fn run(
    cli: &Path,
    arguments: &[String],
    root: &Path,
    input: Option<String>,
    background: bool,
    cancel: &gio::Cancellable,
) -> Result<Output, String> {
    let mut command: Vec<std::ffi::OsString> = Vec::new();
    if background && let Some(nice) = glib::find_program_in_path("nice") {
        command.extend([nice.into_os_string(), "-n".into(), "10".into()]);
    }
    command.push(cli.as_os_str().to_owned());
    command.extend(arguments.iter().map(Into::into));
    let mut flags = gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE;
    if input.is_some() {
        flags |= gio::SubprocessFlags::STDIN_PIPE;
    }
    let launcher = gio::SubprocessLauncher::new(flags);
    launcher.set_cwd(root);
    let argv: Vec<&std::ffi::OsStr> = command.iter().map(std::ffi::OsString::as_os_str).collect();
    let process = launcher.spawn(&argv).map_err(|error| error.to_string())?;
    let handler = {
        let process = process.clone();
        cancel.connect_cancelled_local(move |_| process.force_exit())
    };
    // stdout and stderr are read together, so a full pipe never stops it;
    // the Rust runtime ignores SIGPIPE, so a command that ends before it
    // read its stdin cannot end the app.
    let said = process.communicate_future(input.map(|text| glib::Bytes::from_owned(text.into_bytes())).as_ref()).await;
    if let Some(handler) = handler {
        cancel.disconnect_cancelled(handler);
    }
    if cancel.is_cancelled() {
        return Err("Cancelled.".into());
    }
    let (stdout, stderr) = said.map_err(|error| error.to_string())?;
    let text = |bytes: Option<glib::Bytes>| bytes.map_or_default(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(Output { status: process.has_exited().then(|| process.exit_status()), stdout: text(stdout), stderr: text(stderr) })
}

/// The vault keeps `daftar-cli`'s settings, so it is checked when it opens.
pub fn uses_cli(root: &Path) -> bool {
    [".daftar.toml", ".markdownlint.jsonc", ".prettierrc"].iter().any(|name| root.join(name).exists())
}

/// The vault's own `[audit]` profile of `.daftar.toml`, which `just audit` uses.
pub fn has_audit_profile(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".daftar.toml")).is_ok_and(|text| text.lines().any(|line| line.trim() == "[audit]"))
}

fn base(root: &Path) -> Vec<String> {
    vec!["--vault".into(), root.to_string_lossy().into_owned()]
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

/// The three commands of an audit, as `just check` and `just audit` run
/// them; with [notes], `lint` and `fmt --check` of those notes only, and
/// neither when the list is empty.
pub fn audit_arguments(root: &Path, profile: bool, notes: Option<&[String]>) -> Vec<Vec<String>> {
    let mut check = [base(root), strings(&["check", "--json"])].concat();
    if profile {
        check.extend(strings(&["--profile", "audit"]));
    }
    let with = |words: &[&str]| -> Vec<String> {
        let mut out = [base(root), strings(words)].concat();
        if let Some(notes) = notes {
            // After `--`, a note whose name starts with `-` is a note.
            out.push("--".into());
            out.extend(notes.iter().cloned());
        }
        out
    };
    if notes.is_some_and(<[String]>::is_empty) {
        return vec![check];
    }
    vec![check, with(&["lint", "--json"]), with(&["fmt", "--check", "--json"])]
}

/// The fixes `just fix` runs, in its order: lint's, then the audit's, then
/// the formatting; for [paths], or the whole vault when there are none.
pub fn fix_arguments(root: &Path, tools: &[ProblemTool], paths: &[String]) -> Vec<Vec<String>> {
    let files: Vec<String> = if paths.is_empty() { Vec::new() } else { [vec!["--".to_string()], paths.to_vec()].concat() };
    [(ProblemTool::Lint, &["lint", "--fix"][..]), (ProblemTool::Check, &["check", "--fix"][..]), (ProblemTool::Fmt, &["fmt"][..])]
        .into_iter()
        .filter(|(tool, _)| tools.contains(tool))
        .map(|(_, words)| [base(root), strings(words), files.clone()].concat())
        .collect()
}

/// [path]'s text, given on stdin, formatted as that note: writes nothing.
/// `--path=` keeps a note named `-x.md` a value.
pub fn format_arguments(root: &Path, path: &str) -> Vec<String> {
    [base(root), strings(&["fmt", "--stdin"]), vec![format!("--path={path}")]].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(line: &str) -> Output {
        Output {
            status: Some(2),
            stdout: String::new(),
            stderr: format!("{line}\n\nUsage: daftar-cli check [OPTIONS]\n\nFor more information, try '--help'.\n"),
        }
    }

    /// A `daftar-cli` before 0.23.0 refuses `--json` and `--stdin`: that is
    /// "upgrade", not an error of the vault.
    #[test]
    fn an_old_command_line_is_told_apart() {
        assert!(usage("error: unexpected argument '--json' found").too_old());
        assert!(usage("error: unexpected argument '--stdin' found\n\n  tip: to pass '--stdin' as a value, use '-- --stdin'").too_old());
        assert!(!usage("error: unexpected argument '--bad' found").too_old());
        // `check` that fails on an error of the vault is not a usage error.
        let failed = Output { status: Some(1), stdout: String::new(), stderr: "error: unexpected argument '--json' found\n".into() };
        assert!(!failed.too_old());
        assert!(usage("error: unexpected argument 'Notes/Plan.md' found").refuses_paths(&["Notes/Plan.md".into()]));
        assert!(usage("error: unexpected argument '--' found").refuses_paths(&["A.md".into()]));
        assert!(!usage("error: unexpected argument '--bad' found").refuses_paths(&["A.md".into()]));
        assert_eq!(failure("error: unexpected argument '--bad' found\n\nUsage: x\n"), "daftar-cli: unexpected argument '--bad' found");
        assert_eq!(failure("daftar-cli: no vault at /x\n"), "no vault at /x");
        assert_eq!(failure(""), "daftar-cli failed.");
    }

    #[test]
    fn the_commands_are_those_of_just_check_audit_and_fix() {
        let root = Path::new("/v");
        let audit = audit_arguments(root, true, None);
        assert_eq!(
            audit,
            [
                strings(&["--vault", "/v", "check", "--json", "--profile", "audit"]),
                strings(&["--vault", "/v", "lint", "--json"]),
                strings(&["--vault", "/v", "fmt", "--check", "--json"]),
            ]
        );
        assert_eq!(audit_arguments(root, false, None)[0], strings(&["--vault", "/v", "check", "--json"]));
        // After a save, the saved notes alone are linted; none, and `check` alone runs.
        let notes = ["-x.md".to_string()];
        assert_eq!(audit_arguments(root, false, Some(&notes))[1], strings(&["--vault", "/v", "lint", "--json", "--", "-x.md"]));
        assert_eq!(audit_arguments(root, false, Some(&[])).len(), 1);
        assert_eq!(
            fix_arguments(root, &ProblemTool::ALL, &[]),
            [
                strings(&["--vault", "/v", "lint", "--fix"]),
                strings(&["--vault", "/v", "check", "--fix"]),
                strings(&["--vault", "/v", "fmt"]),
            ]
        );
        assert_eq!(fix_arguments(root, &[ProblemTool::Check], &notes), [strings(&["--vault", "/v", "check", "--fix", "--", "-x.md"])]);
        assert_eq!(format_arguments(root, "-x.md"), strings(&["--vault", "/v", "fmt", "--stdin", "--path=-x.md"]));
        let vault = tempfile::tempdir().unwrap();
        assert!(!uses_cli(vault.path()) && !has_audit_profile(vault.path()));
        std::fs::write(vault.path().join(".daftar.toml"), "[check]\nmemory = true\n\n[audit]\nquiet_days = 30\n").unwrap();
        assert!(uses_cli(vault.path()) && has_audit_profile(vault.path()));
    }
}
