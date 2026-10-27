//! The note's history in the vault's repository, as the Mac's `HistorySheet`
//! and obsidian-git's history view show it: each commit that changed the
//! note, newest first, across renames, and the lines it added and removed.
//! An older text can be restored; the restore is an edit like any other,
//! and the next sync commits it, so nothing in the history is lost.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{DiffKind, DiffLine, Git, LineDiff, Revision, Vault};

/// The working copy, when it differs from the last commit.
const WORKING: &str = "working";

/// Writes [text] as the note at [path]; why not, when it cannot.
type Restore = Box<dyn Fn(&str, &str) -> Result<(), String>>;

pub struct Hooks {
    /// The note's text written back, with the vault's file rule.
    pub restore: Restore,
    pub notice: Box<dyn Fn(&str)>,
}

struct State {
    root: PathBuf,
    path: String,
    hooks: Hooks,
    dialog: adw::Dialog,
    list: gtk::ListBox,
    detail: gtk::Box,
    count: gtk::Label,
    whole: gtk::ToggleButton,
    restore: gtk::Button,
    revisions: RefCell<Vec<Revision>>,
    /// The text of each commit, by its hash, once read.
    texts: RefCell<HashMap<String, String>>,
    uncommitted: RefCell<bool>,
    selection: RefCell<Option<String>>,
}

pub fn present(parent: &impl IsA<gtk::Widget>, root: PathBuf, path: &str, hooks: Hooks) {
    let dialog = adw::Dialog::builder().title("History").content_width(920).content_height(620).build();
    let list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
    let detail = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).vexpand(true).build();
    let count = gtk::Label::builder().css_classes(["caption", "dim-label"]).build();
    let changes = gtk::ToggleButton::builder().label("Changes").active(true).build();
    let whole = gtk::ToggleButton::builder().label("Whole Note").group(&changes).build();
    let restore = gtk::Button::builder().label("Restore This Version…").css_classes(["pill"]).sensitive(false).build();
    let state = Rc::new(State {
        root,
        path: path.to_string(),
        hooks,
        dialog: dialog.clone(),
        list: list.clone(),
        detail: detail.clone(),
        count: count.clone(),
        whole: whole.clone(),
        restore: restore.clone(),
        revisions: RefCell::new(Vec::new()),
        texts: RefCell::new(HashMap::new()),
        uncommitted: RefCell::new(false),
        selection: RefCell::new(None),
    });

    let header = gtk::Box::builder().spacing(12).build();
    header.append(
        &gtk::Image::builder()
            .icon_name("document-open-recent-symbolic")
            .pixel_size(20)
            .css_classes(["daftar-sheet-icon", "history"])
            .build(),
    );
    let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    words.append(&gtk::Label::builder().label("History").xalign(0.0).css_classes(["title-3"]).build());
    words.append(
        &gtk::Label::builder()
            .label(Vault::title(path))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build(),
    );
    header.append(&words);
    header.append(&count);

    let side = gtk::ScrolledWindow::builder().child(&list).width_request(280).hscrollbar_policy(gtk::PolicyType::Never).build();
    let body = gtk::Box::builder().spacing(14).vexpand(true).build();
    body.append(&side);
    body.append(&detail);

    let toggle = gtk::Box::builder().css_classes(["linked"]).build();
    toggle.append(&changes);
    toggle.append(&whole);
    let done = gtk::Button::builder().label("Done").css_classes(["suggested-action", "pill"]).build();
    let bar = gtk::Box::builder().spacing(8).build();
    bar.append(&toggle);
    bar.append(&gtk::Box::builder().hexpand(true).build());
    bar.append(&restore);
    bar.append(&done);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(22)
        .margin_bottom(22)
        .margin_start(22)
        .margin_end(22)
        .build();
    content.append(&header);
    content.append(&body);
    content.append(&bar);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
    view.set_content(Some(&content));
    dialog.set_child(Some(&view));

    {
        let state = state.clone();
        list.connect_row_selected(move |_, row| {
            let Some(id) = row.map(|row| row.widget_name().to_string()) else { return };
            if state.selection.borrow().as_deref() != Some(id.as_str()) {
                *state.selection.borrow_mut() = Some(id);
                state.load_selection();
            }
        });
    }
    {
        let state = state.clone();
        whole.connect_toggled(move |_| state.draw_detail());
    }
    {
        let dialog = dialog.clone();
        done.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let state = state.clone();
        restore.connect_clicked(move |_| state.confirm_restore());
    }
    state.load();
    dialog.present(Some(parent));
}

impl State {
    /// The commits, read away from the window, then the newest selected.
    fn load(self: &Rc<Self>) {
        self.detail.append(&gtk::Spinner::builder().spinning(true).vexpand(true).build());
        let this = self.clone();
        let (root, path) = (self.root.clone(), self.path.clone());
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || {
                let git = Git::new(&root);
                if !git.is_repository() {
                    return None;
                }
                let revisions = git.history(&path, 300).unwrap_or_default();
                let last = revisions.first().and_then(|r| git.content(&r.path, &r.hash).ok());
                let now = std::fs::read_to_string(root.join(&path)).ok();
                Some((revisions, last, now))
            })
            .await;
            let Ok(read) = read else { return };
            let Some((revisions, last, now)) = read else {
                this.empty("Not a Repository", "The vault is not a Git repository, so it keeps no history.");
                return;
            };
            if let (Some(first), Some(last)) = (revisions.first(), last) {
                *this.uncommitted.borrow_mut() = now.as_ref().is_some_and(|now| *now != last);
                this.texts.borrow_mut().insert(first.hash.clone(), last);
            }
            this.count.set_label(&match revisions.len() {
                0 => String::new(),
                1 => "1 commit".into(),
                n => format!("{n} commits"),
            });
            let uncommitted = *this.uncommitted.borrow();
            *this.selection.borrow_mut() = if uncommitted { Some(WORKING.into()) } else { revisions.first().map(|r| r.hash.clone()) };
            *this.revisions.borrow_mut() = revisions;
            if this.revisions.borrow().is_empty() {
                this.empty("No History", "No commit has this note yet.");
                return;
            }
            this.draw_list();
            this.load_selection();
        });
    }

    fn empty(&self, title: &str, description: &str) {
        self.clear_detail();
        self.detail.append(
            &adw::StatusPage::builder()
                .icon_name("document-open-recent-symbolic")
                .title(title)
                .description(description)
                .vexpand(true)
                .build(),
        );
    }

    fn clear_detail(&self) {
        while let Some(child) = self.detail.first_child() {
            self.detail.remove(&child);
        }
    }

    fn draw_list(self: &Rc<Self>) {
        let revisions = self.revisions.borrow().clone();
        let selection = self.selection.borrow().clone();
        let current = revisions.first().map(|r| r.path.clone()).unwrap_or_default();
        let mut rows = Vec::new();
        if *self.uncommitted.borrow() {
            rows.push((WORKING.to_string(), row("Not committed yet", "The text on disk", "document-edit-symbolic", "t-accent", None)));
        }
        for revision in &revisions {
            // The vault's commits all say "update notes @ <time> on <host>": the date names a commit better.
            let date = revision.date.with_timezone(&chrono::Local).format("%-d %b %Y, %H:%M").to_string();
            let moved = revision.path != current;
            let help = if moved {
                format!("{}\n{}\nThe note was {} then.", revision.subject, revision.hash, revision.path)
            } else {
                format!("{}\n{}", revision.subject, revision.hash)
            };
            let icon = if moved { "go-next-symbolic" } else { "media-record-symbolic" };
            let tint = if moved { "t-purple" } else { "t-blue" };
            rows.push((revision.hash.clone(), row(&date, &format!("{} · {}", revision.author, revision.subject), icon, tint, Some(&help))));
        }
        for (id, widget) in rows {
            let row = gtk::ListBoxRow::builder().child(&widget).name(&id).build();
            self.list.append(&row);
            if selection.as_deref() == Some(id.as_str()) {
                self.list.select_row(Some(&row));
            }
        }
    }

    /// The commit before [id], or none for the first one.
    fn parent(&self, id: &str) -> Option<Revision> {
        let revisions = self.revisions.borrow();
        if id == WORKING {
            return revisions.first().cloned();
        }
        let index = revisions.iter().position(|r| r.hash == id)?;
        revisions.get(index + 1).cloned()
    }

    fn text(&self, id: &str) -> Option<String> {
        if id == WORKING {
            return std::fs::read_to_string(self.root.join(&self.path)).ok();
        }
        self.texts.borrow().get(id).cloned()
    }

    /// The text before [id]'s change: empty for the commit that made the note.
    fn parent_text(&self, id: &str) -> Option<String> {
        match self.parent(id) {
            Some(parent) => self.texts.borrow().get(&parent.hash).cloned(),
            None => Some(String::new()),
        }
    }

    /// The texts the selection needs, read away from the window, then drawn.
    fn load_selection(self: &Rc<Self>) {
        let Some(id) = self.selection.borrow().clone() else { return };
        let wanted: Vec<Revision> = {
            let revisions = self.revisions.borrow();
            let texts = self.texts.borrow();
            [Some(id.clone()), self.parent(&id).map(|p| p.hash)]
                .into_iter()
                .flatten()
                .filter(|hash| hash != WORKING && !texts.contains_key(hash))
                .filter_map(|hash| revisions.iter().find(|r| r.hash == hash).cloned())
                .collect()
        };
        if wanted.is_empty() {
            self.draw_detail();
            return;
        }
        let this = self.clone();
        let root = self.root.clone();
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || {
                let git = Git::new(&root);
                wanted.into_iter().filter_map(|r| git.content(&r.path, &r.hash).ok().map(|text| (r.hash, text))).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            this.texts.borrow_mut().extend(read);
            this.draw_detail();
        });
    }

    fn draw_detail(self: &Rc<Self>) {
        self.clear_detail();
        let Some(id) = self.selection.borrow().clone() else {
            self.empty("Choose a Commit", "Its changes to the note show here.");
            return;
        };
        self.restore.set_sensitive(id != WORKING && self.texts.borrow().contains_key(&id));
        let (Some(new), Some(old)) = (self.text(&id), self.parent_text(&id)) else {
            self.detail.append(&gtk::Spinner::builder().spinning(true).vexpand(true).build());
            return;
        };
        if self.whole.is_active() {
            let label =
                gtk::Label::builder().label(&new).xalign(0.0).yalign(0.0).selectable(true).wrap(true).css_classes(["monospace"]).build();
            let scroll = gtk::ScrolledWindow::builder().child(&label).vexpand(true).css_classes(["daftar-diff"]).build();
            self.detail.append(&scroll);
            return;
        }
        let lines = LineDiff::lines(&old, &new);
        let (added, removed) = LineDiff::counts(&lines);
        let counts = gtk::Box::builder().spacing(10).build();
        counts.append(&gtk::Label::builder().label(format!("+{added}")).css_classes(["daftar-added-count", "numeric"]).build());
        counts.append(&gtk::Label::builder().label(format!("−{removed}")).css_classes(["daftar-removed-count", "numeric"]).build());
        let revision = self.revisions.borrow().iter().find(|r| r.hash == id).cloned();
        if let (Some(revision), Some(parent)) = (revision, self.parent(&id))
            && parent.path != revision.path
        {
            counts
                .append(&gtk::Label::builder().label(format!("moved from {}", parent.path)).css_classes(["dim-label", "caption"]).build());
        }
        self.detail.append(&counts);
        self.detail.append(&diff_view(&LineDiff::hunks(&lines, 3)));
    }

    fn confirm_restore(self: &Rc<Self>) {
        let alert = adw::AlertDialog::builder()
            .heading("Restore this version of the note?")
            .body("The note's text becomes the text of this commit. The next sync commits the change, so the text of now stays in the history.")
            .build();
        alert.add_responses(&[("cancel", "Cancel"), ("restore", "Restore")]);
        alert.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
        let this = self.clone();
        alert.connect_response(Some("restore"), move |_, _| this.restore());
        alert.present(Some(&self.dialog));
    }

    fn restore(&self) {
        let Some(id) = self.selection.borrow().clone().filter(|id| id != WORKING) else { return };
        let Some(text) = self.texts.borrow().get(&id).cloned() else { return };
        match (self.hooks.restore)(&self.path, &text) {
            Ok(()) => {
                let date = self
                    .revisions
                    .borrow()
                    .iter()
                    .find(|r| r.hash == id)
                    .map(|r| r.date.with_timezone(&chrono::Local).format("%-d %b %Y, %H:%M").to_string());
                (self.hooks.notice)(&format!("{} has its text of {} again.", Vault::title(&self.path), date.unwrap_or(id)));
                self.dialog.close();
            }
            Err(error) => (self.hooks.notice)(&format!("The note could not be restored: {error}")),
        }
    }
}

fn row(title: &str, detail: &str, icon: &str, tint: &str, help: Option<&str>) -> gtk::Box {
    let line = gtk::Box::builder().spacing(8).margin_top(3).margin_bottom(3).build();
    line.set_tooltip_text(help);
    line.append(&gtk::Image::builder().icon_name(icon).css_classes(["tint", tint]).build());
    let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    words.append(&gtk::Label::builder().label(title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
    words.append(
        &gtk::Label::builder()
            .label(detail)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    line.append(&words);
    line
}

/// The changed lines with the lines around them, as `git diff` shows them.
fn diff_view(hunks: &[Vec<DiffLine>]) -> gtk::ScrolledWindow {
    let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    if hunks.is_empty() {
        lines.append(
            &gtk::Label::builder()
                .label("This commit did not change the note's text.")
                .xalign(0.0)
                .margin_start(10)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    for (index, hunk) in hunks.iter().enumerate() {
        if index > 0 {
            lines.append(&gtk::Label::builder().label("⋯").xalign(0.0).margin_start(10).css_classes(["dim-label"]).build());
        }
        for line in hunk {
            lines.append(&diff_row(line));
        }
    }
    gtk::ScrolledWindow::builder().child(&lines).vexpand(true).css_classes(["daftar-diff"]).build()
}

fn diff_row(line: &DiffLine) -> gtk::Box {
    let (mark, class) = match line.kind {
        DiffKind::Added => ("+", Some("diff-added")),
        DiffKind::Removed => ("−", Some("diff-removed")),
        DiffKind::Same => ("", None),
    };
    let row = gtk::Box::builder().spacing(8).css_classes(["diff-line", "monospace"]).build();
    if let Some(class) = class {
        row.add_css_class(class);
    }
    let number = |n: Option<usize>| {
        gtk::Label::builder()
            .label(n.map(|n| n.to_string()).unwrap_or_default())
            .width_chars(4)
            .xalign(1.0)
            .css_classes(["diff-number"])
            .build()
    };
    row.append(&number(line.old));
    row.append(&number(line.new));
    row.append(&gtk::Label::builder().label(mark).width_chars(1).build());
    row.append(
        &gtk::Label::builder()
            .label(if line.text.is_empty() { " " } else { &line.text })
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .selectable(true)
            .build(),
    );
    row
}
