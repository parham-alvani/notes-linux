//! WakaTime's heartbeats, sent with `wakatime-cli` as the editor plugins
//! and the Mac's `WakaTime` send them: a note opened or read, at most every
//! two minutes for the same note, and every save. Off unless
//! `wakatime.enabled` is on; the key is `wakatime-cli`'s own, in
//! `~/.wakatime.cfg`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use daftar_core::{ConfigFile, Vault};

thread_local! {
    static LAST: RefCell<Option<(PathBuf, Instant)>> = const { RefCell::new(None) };
}

/// The `wakatime-cli` to run: [configured], or the one the plugins install
/// in `~/.wakatime` (with its platform's name), or one on `$PATH`.
pub fn cli(configured: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let configured = configured.trim();
    let mut candidates = Vec::new();
    if !configured.is_empty() {
        candidates.push(match configured.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(configured),
        });
    }
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
    candidates.push(home.join(".wakatime/wakatime-cli"));
    candidates.push(home.join(format!(".wakatime/wakatime-cli-linux-{arch}")));
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|folder| folder.join("wakatime-cli")));
    }
    candidates.into_iter().find(|file| is_executable(file))
}

fn is_executable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    file.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The note at [path] of the vault at [root] was read ([write] false) or saved.
pub fn heartbeat(config: &ConfigFile, root: &Path, path: &str, write: bool) {
    if !config.bool("wakatime.enabled").unwrap_or(false) || !Vault::is_note(path) {
        return;
    }
    let Some(cli) = cli(&config.string("wakatime.cli").unwrap_or_default()) else { return };
    let file = root.join(path);
    let now = Instant::now();
    let recent = LAST.with(|last| {
        let last = last.borrow();
        last.as_ref().is_some_and(|(entity, at)| *entity == file && now.duration_since(*at) < Duration::from_secs(120))
    });
    if !write && recent {
        return;
    }
    LAST.with(|last| *last.borrow_mut() = Some((file.clone(), now)));
    let project = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut command = Command::new(cli);
    command.args(arguments(&file, &project, write, std::time::SystemTime::now()));
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // A child that ends is reaped on a thread, never waited for on the window's.
    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || child.wait());
    }
}

fn arguments(file: &Path, project: &str, write: bool, at: std::time::SystemTime) -> Vec<String> {
    let version = env!("CARGO_PKG_VERSION");
    let time = at.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or_default();
    let mut out = vec![
        "--entity".to_string(),
        file.to_string_lossy().into_owned(),
        "--plugin".into(),
        format!("daftar/{version} daftar-wakatime/{version}"),
        "--project".into(),
        project.to_string(),
        "--language".into(),
        "Markdown".into(),
        "--category".into(),
        "writing docs".into(),
        "--time".into(),
        format!("{time:.3}"),
    ];
    if write {
        out.push("--write".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heartbeat_says_what_the_plugins_say() {
        let at = std::time::UNIX_EPOCH + Duration::from_millis(1_761_469_200_500);
        let args = arguments(Path::new("/v/org/Notes/A.md"), "org", true, at);
        assert_eq!(args[..2], ["--entity", "/v/org/Notes/A.md"]);
        assert!(args.contains(&"Markdown".to_string()) && args.contains(&"writing docs".to_string()));
        assert!(args.windows(2).any(|w| w == ["--time", "1761469200.500"]));
        assert_eq!(args.last().unwrap(), "--write");
        assert!(!arguments(Path::new("/v/A.md"), "v", false, at).contains(&"--write".to_string()));
    }

    #[test]
    fn the_configured_cli_is_used_when_it_runs() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("my-wakatime");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        // A file that cannot run is passed over.
        assert_ne!(cli(&file.to_string_lossy()), Some(file.clone()));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(cli(&file.to_string_lossy()), Some(file));
    }
}
