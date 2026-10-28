//! The vault's files, watched with inotify (the Mac app's `FileWatcher` uses
//! FSEvents). Reports vault paths, and only ones the app reads: a `git` run
//! or a `node_modules` install says nothing. Events are gathered for a
//! moment, so a `git pull` of a hundred files is one report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use daftar_core::text::slugs;
use daftar_core::{TaskStatuses, Vault};
use gtk::glib;
use notify::{RecursiveMode, Watcher};

pub struct FileWatcher {
    // Watches while it lives.
    _watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    /// Calls [handler] on the main thread with the vault paths that changed.
    pub fn new(root: &Path, handler: impl Fn(BTreeSet<String>) + 'static) -> notify::Result<FileWatcher> {
        let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let (sender, receiver) = async_channel::unbounded::<PathBuf>();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event
                && !matches!(event.kind, notify::EventKind::Access(_))
            {
                for path in event.paths {
                    let _ = sender.send_blocking(path);
                }
            }
        })?;
        watcher.watch(&base, RecursiveMode::Recursive)?;
        glib::spawn_future_local(async move {
            while let Ok(first) = receiver.recv().await {
                glib::timeout_future(Duration::from_millis(200)).await;
                let mut changed = BTreeSet::new();
                for path in std::iter::once(first).chain(std::iter::from_fn(|| receiver.try_recv().ok())) {
                    if let Some(relative) = relative(&base, &path) {
                        changed.insert(relative);
                    }
                }
                if !changed.is_empty() {
                    handler(changed);
                }
            }
        });
        Ok(FileWatcher { _watcher: watcher })
    }
}

/// [path] as a vault path, when it is in [base] and the app reads it: a
/// note or a file, or the Tasks plugin's settings, which name the statuses.
fn relative(base: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(base).ok()?.to_str()?;
    let relative = slugs::normalize(rest);
    (!relative.is_empty() && (Vault::counts(&relative) || relative == TaskStatuses::PATH)).then_some(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_files_the_app_reads_are_reported() {
        let base = Path::new("/vault");
        assert_eq!(relative(base, Path::new("/vault/Notes/Inbox.md")).as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(relative(base, Path::new("/vault/.git/index")), None);
        let settings = "/vault/.obsidian/plugins/obsidian-tasks-plugin/data.json";
        assert_eq!(relative(base, Path::new(settings)).as_deref(), Some(TaskStatuses::PATH));
        assert_eq!(relative(base, Path::new("/vault/web/node_modules/a.js")), None);
        assert_eq!(relative(base, Path::new("/vault/Notes/.Inbox.md.daftar-12")), None);
        assert_eq!(relative(base, Path::new("/elsewhere/a.md")), None);
        assert_eq!(relative(base, Path::new("/vault")), None);
    }
}
