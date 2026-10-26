//! The vault's repository from the window: a button in the sidebar's header
//! with the branch and what is waiting, Sync Now and Pull in its menu, a pull
//! at launch, a sync every `git.sync_interval` minutes, and one shortly after
//! the app writes a note. The Mac's `VaultStore` keeps the same rules.
//!
//! Git runs on a worker thread; the window only hears how it went.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{Git, GitFailure, GitStatus, Synced};

/// After an edit, how long to wait for more before syncing them together.
const AFTER_EDIT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Run {
    Sync,
    Pull,
}

pub struct GitSync {
    pub button: gtk::MenuButton,
    icon: gtk::Image,
    spinner: gtk::Spinner,
    label: gtk::Label,
    root: RefCell<Option<PathBuf>>,
    syncing: Cell<bool>,
    /// Minutes between automatic syncs; 0 is off.
    interval: Cell<u32>,
    every: RefCell<Option<glib::SourceId>>,
    soon: RefCell<Option<glib::SourceId>>,
    notice: Box<dyn Fn(&str)>,
}

impl GitSync {
    pub fn new(notice: impl Fn(&str) + 'static) -> Rc<GitSync> {
        let icon = gtk::Image::from_icon_name("emblem-synchronizing-symbolic");
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let label = gtk::Label::builder().css_classes(["caption", "numeric"]).build();
        let content = gtk::Box::builder().spacing(4).build();
        content.append(&icon);
        content.append(&spinner);
        content.append(&label);
        let menu = gio::Menu::new();
        menu.append(Some("Sync Now"), Some("win.sync"));
        menu.append(Some("Pull"), Some("win.pull"));
        let button = gtk::MenuButton::builder().child(&content).menu_model(&menu).css_classes(["flat"]).visible(false).build();
        Rc::new(GitSync {
            button,
            icon,
            spinner,
            label,
            root: RefCell::new(None),
            syncing: Cell::new(false),
            interval: Cell::new(0),
            every: RefCell::new(None),
            soon: RefCell::new(None),
            notice: Box::new(notice),
        })
    }

    /// Follows the vault at [root]: hidden when it is not a repository;
    /// otherwise a pull when [pull] says so, then the automatic syncs.
    pub fn attach(self: &Rc<Self>, root: PathBuf, pull: bool, interval: u32) {
        self.stop_timers();
        *self.root.borrow_mut() = Some(root.clone());
        self.button.set_visible(false);
        self.interval.set(interval);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let probe = root.clone();
            let Ok(true) = gio::spawn_blocking(move || Git::new(probe).is_repository()).await else { return };
            if this.root.borrow().as_ref() != Some(&root) {
                return;
            }
            this.button.set_visible(true);
            if pull {
                this.run(Run::Pull, false);
            } else {
                this.refresh();
            }
            this.schedule();
        });
    }

    /// Commits, pulls and pushes now, or pulls only. [loud] says how it went
    /// even when nothing went wrong, for a run the person asked for.
    pub fn run(self: &Rc<Self>, kind: Run, loud: bool) {
        let Some(root) = self.root.borrow().clone() else { return };
        if !self.button.is_visible() || self.syncing.replace(true) {
            return;
        }
        self.working(true);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let folder = root.clone();
            let outcome = gio::spawn_blocking(move || {
                let git = Git::new(folder);
                let done = match kind {
                    Run::Sync => git.sync(&Git::message(chrono::Local::now(), &Git::host())).map(Some),
                    Run::Pull => git.pull().map(|_| None),
                };
                (done, git.status().ok())
            })
            .await;
            this.syncing.set(false);
            this.working(false);
            if this.root.borrow().as_ref() != Some(&root) {
                return;
            }
            let Ok((done, status)) = outcome else { return };
            if let Some(status) = status {
                this.show(&status);
            }
            match done {
                Err(failure) => (this.notice)(&describe(&failure)),
                Ok(done) if loud => (this.notice)(&summary(kind, done)),
                Ok(_) => {}
            }
        });
    }

    /// The branch and counts again, after files changed on disk.
    pub fn refresh(self: &Rc<Self>) {
        let Some(root) = self.root.borrow().clone() else { return };
        if !self.button.is_visible() || self.syncing.get() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let folder = root.clone();
            if let Ok(Ok(status)) = gio::spawn_blocking(move || Git::new(folder).status()).await
                && this.root.borrow().as_ref() == Some(&root)
            {
                this.show(&status);
            }
        });
    }

    /// After the app wrote a note: a sync a little later, when automatic sync
    /// is on, so several ticks in a row make one commit.
    pub fn edited(self: &Rc<Self>) {
        if self.interval.get() == 0 {
            return;
        }
        if let Some(id) = self.soon.take() {
            id.remove();
        }
        let this = self.clone();
        let id = glib::timeout_add_local_once(AFTER_EDIT, move || {
            this.soon.take();
            this.run(Run::Sync, false);
        });
        *self.soon.borrow_mut() = Some(id);
    }

    /// A new interval from the settings: the timer starts again with it.
    pub fn reconfigure(self: &Rc<Self>, interval: u32) {
        self.stop_timers();
        self.interval.set(interval);
        if self.button.is_visible() {
            self.schedule();
        }
    }

    fn schedule(self: &Rc<Self>) {
        let minutes = self.interval.get();
        if minutes == 0 {
            return;
        }
        self.run(Run::Sync, false);
        let this = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local(minutes * 60, move || match this.upgrade() {
            Some(this) => {
                this.run(Run::Sync, false);
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        *self.every.borrow_mut() = Some(id);
    }

    fn stop_timers(&self) {
        for slot in [&self.every, &self.soon] {
            if let Some(id) = slot.take() {
                id.remove();
            }
        }
    }

    fn working(&self, on: bool) {
        self.icon.set_visible(!on);
        self.spinner.set_visible(on);
        self.spinner.set_spinning(on);
        self.button.set_sensitive(!on);
    }

    fn show(&self, status: &GitStatus) {
        self.label.set_label(&badge(status));
        self.button.set_tooltip_text(Some(&tooltip(status)));
    }
}

/// `↑2 ↓1 •3`: commits to push, to pull, and files changed; nothing when level.
fn badge(status: &GitStatus) -> String {
    let mut parts = Vec::new();
    if status.ahead > 0 {
        parts.push(format!("↑{}", status.ahead));
    }
    if status.behind > 0 {
        parts.push(format!("↓{}", status.behind));
    }
    if status.changed > 0 {
        parts.push(format!("•{}", status.changed));
    }
    parts.join(" ")
}

fn tooltip(status: &GitStatus) -> String {
    let count = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let mut lines = vec![format!("On {}", status.branch)];
    if status.ahead > 0 {
        lines.push(format!("{} to push", count(status.ahead as usize, "commit", "commits")));
    }
    if status.behind > 0 {
        lines.push(format!("{} to pull", count(status.behind as usize, "commit", "commits")));
    }
    if status.changed > 0 {
        lines.push(format!("{} changed", count(status.changed, "file", "files")));
    }
    if lines.len() == 1 {
        lines.push("Up to date".into());
    }
    lines.join("\n")
}

fn summary(kind: Run, done: Option<Synced>) -> String {
    let Some(done) = done else {
        return if kind == Run::Pull { "Pulled.".into() } else { "Up to date.".into() };
    };
    let steps: Vec<&str> = [(done.committed, "committed"), (done.pulled, "pulled"), (done.pushed, "pushed")]
        .into_iter()
        .filter_map(|(did, word)| did.then_some(word))
        .collect();
    match steps.as_slice() {
        [] => "Up to date.".into(),
        [one] => format!("Synced: {one}."),
        [first @ .., last] => format!("Synced: {} and {last}.", first.join(", ")),
    }
}

/// The failure in a line: git's last word is usually the one that matters.
fn describe(failure: &GitFailure) -> String {
    let last = failure.output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if last.is_empty() { format!("git {} failed.", failure.arguments.join(" ")) } else { format!("Sync failed: {last}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_and_the_summary() {
        let status = GitStatus { branch: "main".into(), ahead: 2, behind: 0, changed: 3 };
        assert_eq!(badge(&status), "↑2 •3");
        assert_eq!(tooltip(&status), "On main\n2 commits to push\n3 files changed");
        assert_eq!(badge(&GitStatus { branch: "main".into(), ..Default::default() }), "");
        let done = Synced { committed: true, pulled: true, pushed: true };
        assert_eq!(summary(Run::Sync, Some(done)), "Synced: committed, pulled and pushed.");
        assert_eq!(summary(Run::Sync, Some(Synced::default())), "Up to date.");
        let failure =
            GitFailure { arguments: vec!["push".into()], status: Some(1), output: "To x\n ! [rejected]\nhint: pull first\n".into() };
        assert_eq!(describe(&failure), "Sync failed: hint: pull first");
    }
}
