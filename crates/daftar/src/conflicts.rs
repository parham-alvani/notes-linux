//! A pull that stopped on a conflict: the files it stopped on, and for a
//! note each conflict with the remote's lines beside the person's, as the
//! Mac's `ConflictSheet`. Git's "ours" and "theirs" are swapped in a rebase,
//! so this dialog never says them: it says "On the remote" and "Your change".

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use daftar_core::{Choice, ConflictPart, ConflictText, Conflicted, Resolution, Side, Vault};

use crate::sync::GitSync;

/// What the window does for the dialog.
pub struct Hooks {
    /// The editors save, so the files are read as they are on disk.
    pub save: Box<dyn Fn()>,
    /// Edit: the note opens in the editor with its marks.
    pub edit: Box<dyn Fn(&str)>,
    pub notice: Box<dyn Fn(&str)>,
}

struct State {
    git: Rc<GitSync>,
    root: PathBuf,
    hooks: Hooks,
    dialog: adw::Dialog,
    list: gtk::ListBox,
    detail: gtk::Box,
    remaining: gtk::Label,
    finish: gtk::Button,
    /// Each note's text as it was read, with its conflict marks.
    texts: RefCell<HashMap<String, String>>,
    /// The side chosen for each conflict of each note, by its number.
    choices: RefCell<HashMap<String, HashMap<usize, Choice>>>,
    /// The side kept of a whole file that has no marks to choose in.
    whole: RefCell<HashMap<String, Resolution>>,
    selection: RefCell<Option<String>>,
}

pub fn present(parent: &impl IsA<gtk::Widget>, git: Rc<GitSync>, hooks: Hooks) {
    let Some(root) = git.root() else { return };
    let dialog = adw::Dialog::builder().title("Sync Conflicts").content_width(980).content_height(660).build();
    let list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
    let detail = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).hexpand(true).build();
    let remaining = gtk::Label::builder().css_classes(["dim-label"]).build();
    let finish = gtk::Button::builder().label("Finish Sync").css_classes(["suggested-action", "pill"]).build();
    let state = Rc::new(State {
        git,
        root,
        hooks,
        dialog: dialog.clone(),
        list: list.clone(),
        detail: detail.clone(),
        remaining: remaining.clone(),
        finish: finish.clone(),
        texts: RefCell::new(HashMap::new()),
        choices: RefCell::new(HashMap::new()),
        whole: RefCell::new(HashMap::new()),
        selection: RefCell::new(None),
    });

    let header = gtk::Box::builder().spacing(12).build();
    let icon =
        gtk::Image::builder().icon_name("dialog-warning-symbolic").pixel_size(20).css_classes(["daftar-sheet-icon", "conflicts"]).build();
    header.append(&icon);
    let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    words.append(&gtk::Label::builder().label("Sync Conflicts").xalign(0.0).css_classes(["title-3"]).build());
    words.append(
        &gtk::Label::builder()
            .label("These files changed on the remote and on this computer. Choose what to keep; the sync waits until you finish.")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label"])
            .build(),
    );
    header.append(&words);

    let side = gtk::ScrolledWindow::builder().child(&list).width_request(260).hscrollbar_policy(gtk::PolicyType::Never).build();
    let detail_scroll = gtk::ScrolledWindow::builder().child(&detail).hexpand(true).vexpand(true).build();
    let body = gtk::Box::builder().spacing(14).vexpand(true).build();
    body.append(&side);
    body.append(&detail_scroll);

    let abort = gtk::Button::builder()
        .label("Abort Sync…")
        .css_classes(["destructive-action", "pill"])
        .tooltip_text("Stop the pull. Your commits stay as they were before it, and the next sync pulls again.")
        .build();
    let later = gtk::Button::builder().label("Later").css_classes(["pill"]).build();
    let bar = gtk::Box::builder().spacing(8).build();
    bar.append(&abort);
    bar.append(&gtk::Box::builder().hexpand(true).build());
    bar.append(&remaining);
    bar.append(&later);
    bar.append(&finish);

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
            let Some(path) = row.map(|row| row.widget_name().to_string()) else { return };
            if state.selection.borrow().as_deref() != Some(path.as_str()) {
                *state.selection.borrow_mut() = Some(path);
                state.draw_detail();
            }
        });
    }
    {
        let dialog = dialog.clone();
        later.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let state = state.clone();
        abort.connect_clicked(move |_| state.confirm_abort());
    }
    {
        let state = state.clone();
        finish.connect_clicked(move |_| state.finish());
    }
    state.load();
    dialog.present(Some(parent));
}

impl State {
    /// A note both sides changed: its conflicts are chosen one by one.
    fn uses_marks(file: &Conflicted) -> bool {
        file.is_markdown() && file.has_marks()
    }

    fn files(&self) -> Vec<Conflicted> {
        self.git.conflicts()
    }

    fn count(&self, path: &str) -> usize {
        self.texts.borrow().get(path).map_or(0, |text| ConflictText::conflicts(text).len())
    }

    fn resolved(&self, file: &Conflicted) -> bool {
        if Self::uses_marks(file) && self.texts.borrow().contains_key(&file.path) {
            return self.choices.borrow().get(&file.path).map_or(0, HashMap::len) >= self.count(&file.path);
        }
        self.whole.borrow().contains_key(&file.path)
    }

    /// Reads the notes again, keeping the choices of a note whose text did not change.
    fn load(self: &Rc<Self>) {
        (self.hooks.save)();
        let files = self.files();
        let mut read = HashMap::new();
        for file in files.iter().filter(|file| Self::uses_marks(file)) {
            if let Ok(text) = std::fs::read_to_string(self.root.join(&file.path)) {
                read.insert(file.path.clone(), text);
            }
        }
        {
            let texts = self.texts.borrow();
            let mut choices = self.choices.borrow_mut();
            for (path, text) in &read {
                if texts.get(path) != Some(text) {
                    choices.remove(path);
                }
            }
            choices.retain(|path, _| files.iter().any(|file| &file.path == path));
        }
        *self.texts.borrow_mut() = read;
        self.whole.borrow_mut().retain(|path, _| files.iter().any(|file| &file.path == path));
        let keep = self.selection.borrow().clone().filter(|path| files.iter().any(|file| &file.path == path));
        *self.selection.borrow_mut() = keep.or_else(|| files.first().map(|file| file.path.clone()));
        self.draw();
    }

    fn draw(self: &Rc<Self>) {
        self.draw_list();
        self.draw_detail();
    }

    fn draw_list(self: &Rc<Self>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        let files = self.files();
        if files.is_empty() {
            let text = if self.git.is_rebasing() { "Every file is resolved. Finish the sync to go on." } else { "No sync is stopped." };
            self.list.set_placeholder(Some(&adw::StatusPage::builder().title("Nothing to Choose").description(text).build()));
        }
        let selection = self.selection.borrow().clone();
        for file in &files {
            let done = self.resolved(file);
            let line = gtk::Box::builder().spacing(8).margin_top(4).margin_bottom(4).build();
            let icon = gtk::Image::from_icon_name(if done { "object-select-symbolic" } else { "dialog-warning-symbolic" });
            icon.add_css_class(if done { "success" } else { "warning" });
            line.append(&icon);
            let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
            words.append(
                &gtk::Label::builder().label(Vault::title(&file.path)).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build(),
            );
            words.append(
                &gtk::Label::builder()
                    .label(self.status(file))
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
            line.append(&words);
            let row = gtk::ListBoxRow::builder().child(&line).name(&file.path).tooltip_text(&file.path).build();
            self.list.append(&row);
            if selection.as_deref() == Some(file.path.as_str()) {
                self.list.select_row(Some(&row));
            }
        }
        let left = files.iter().filter(|file| !self.resolved(file)).count();
        self.remaining.set_label(&match left {
            0 => "Every file is resolved.".to_string(),
            1 => "1 file to resolve".to_string(),
            n => format!("{n} files to resolve"),
        });
        self.finish.set_sensitive(left == 0 && self.git.is_rebasing() && !self.git.is_syncing());
    }

    fn status(&self, file: &Conflicted) -> String {
        if let Some(resolution) = self.whole.borrow().get(&file.path) {
            return if *resolution == Resolution::KeepRemote { "Keep the remote's".into() } else { "Keep yours".into() };
        }
        if Self::uses_marks(file) {
            let count = self.count(&file.path);
            if count == 0 {
                return "No marks left".into();
            }
            let chosen = self.choices.borrow().get(&file.path).map_or(0, HashMap::len);
            return format!("{chosen} of {count} chosen");
        }
        format!("{} on the remote, {} here", describe(file.remote), describe(file.mine))
    }

    fn draw_detail(self: &Rc<Self>) {
        while let Some(child) = self.detail.first_child() {
            self.detail.remove(&child);
        }
        let selection = self.selection.borrow().clone();
        let Some(file) = selection.and_then(|path| self.files().into_iter().find(|file| file.path == path)) else {
            self.detail.append(
                &adw::StatusPage::builder()
                    .icon_name("go-previous-symbolic")
                    .title("Choose a File")
                    .description("Its conflicts show here.")
                    .vexpand(true)
                    .build(),
            );
            return;
        };
        let text = self.texts.borrow().get(&file.path).cloned();
        match text {
            Some(text) if Self::uses_marks(&file) => self.note_detail(&file, &text),
            _ => self.whole_detail(&file),
        }
    }

    fn note_detail(self: &Rc<Self>, file: &Conflicted, text: &str) {
        let parts = ConflictText::parse(text);
        let count = ConflictText::count(&parts);
        let top = gtk::Box::builder().spacing(8).build();
        top.append(
            &gtk::Label::builder()
                .label(&file.path)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .css_classes(["heading"])
                .build(),
        );
        if count > 0 {
            let all = self.all_chosen(&file.path, count);
            let path = file.path.clone();
            let this = self.clone();
            top.append(&self.choice_buttons(all, move |choice| {
                this.choices.borrow_mut().insert(path.clone(), (0..count).map(|n| (n, choice)).collect());
                this.draw();
            }));
        }
        let edit = gtk::Button::builder()
            .label("Edit")
            .tooltip_text("Open the note in the editor with the conflict marks, to write the text yourself")
            .build();
        {
            let (this, path) = (self.clone(), file.path.clone());
            edit.connect_clicked(move |_| {
                this.dialog.close();
                (this.hooks.edit)(&path);
            });
        }
        top.append(&edit);
        self.detail.append(&top);
        if count == 0 {
            self.detail.append(
                &adw::StatusPage::builder()
                    .icon_name("object-select-symbolic")
                    .title("No Conflict Marks")
                    .description("The note has no conflict marks left. It is kept as it is.")
                    .vexpand(true)
                    .build(),
            );
            return;
        }
        let mut number = 0;
        for (at, part) in parts.iter().enumerate() {
            let ConflictPart::Conflict(conflict) = part else { continue };
            let chosen = self.choices.borrow().get(&file.path).and_then(|c| c.get(&number)).copied();
            let hunk = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["daftar-hunk"]).build();
            let head = gtk::Box::builder().spacing(8).build();
            head.append(
                &gtk::Label::builder()
                    .label(format!("Conflict {} of {count}", number + 1))
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["caption-heading", "dim-label"])
                    .build(),
            );
            {
                let (this, path, n) = (self.clone(), file.path.clone(), number);
                head.append(&self.choice_buttons(chosen, move |choice| {
                    this.choices.borrow_mut().entry(path.clone()).or_default().insert(n, choice);
                    this.draw();
                }));
            }
            hunk.append(&head);
            let before = context(&parts, at);
            if !before.is_empty() {
                hunk.append(&gtk::Label::builder().label(&before).xalign(0.0).wrap(true).css_classes(["monospace", "dim-label"]).build());
            }
            let sides = gtk::Box::builder().spacing(10).homogeneous(true).build();
            sides.append(&side("On the remote", &conflict.remote, "t-blue", matches!(chosen, Some(Choice::Remote | Choice::Both))));
            sides.append(&side("Your change", &conflict.mine, "t-accent", matches!(chosen, Some(Choice::Mine | Choice::Both))));
            hunk.append(&sides);
            self.detail.append(&hunk);
            number += 1;
        }
    }

    fn whole_detail(self: &Rc<Self>, file: &Conflicted) {
        let chosen = self.whole.borrow().get(&file.path).cloned();
        self.detail.append(
            &gtk::Label::builder()
                .label(&file.path)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .css_classes(["heading"])
                .build(),
        );
        let why = if file.has_marks() {
            "Daftar shows the lines of notes only. Keep one side of this file whole."
        } else {
            "One side deleted this file and the other changed it. Keep one side: keeping a deletion deletes the file."
        };
        self.detail.append(&gtk::Label::builder().label(why).xalign(0.0).wrap(true).css_classes(["dim-label"]).build());
        let sides = gtk::Box::builder().spacing(10).homogeneous(true).build();
        sides.append(&side("On the remote", what(file.remote), "t-blue", chosen == Some(Resolution::KeepRemote)));
        sides.append(&side("Your change", what(file.mine), "t-accent", chosen == Some(Resolution::KeepMine)));
        self.detail.append(&sides);
        let buttons = gtk::Box::builder().spacing(6).halign(gtk::Align::End).build();
        for (label, resolution) in [("Keep Remote", Resolution::KeepRemote), ("Keep Mine", Resolution::KeepMine)] {
            let button = gtk::Button::with_label(label);
            if chosen.as_ref() == Some(&resolution) {
                button.add_css_class("suggested-action");
            }
            let (this, path) = (self.clone(), file.path.clone());
            button.connect_clicked(move |_| {
                this.whole.borrow_mut().insert(path.clone(), resolution.clone());
                this.draw();
            });
            buttons.append(&button);
        }
        self.detail.append(&buttons);
    }

    fn choice_buttons(&self, chosen: Option<Choice>, choose: impl Fn(Choice) + 'static) -> gtk::Box {
        let choose = Rc::new(choose);
        let buttons = gtk::Box::builder().spacing(6).css_classes(["linked"]).build();
        for (label, choice, help) in [
            ("Keep Remote", Choice::Remote, "Keep the remote's lines"),
            ("Keep Mine", Choice::Mine, "Keep your lines"),
            ("Keep Both", Choice::Both, "Keep the remote's lines, then yours"),
        ] {
            let button = gtk::Button::builder().label(label).tooltip_text(help).css_classes(["small"]).build();
            if chosen == Some(choice) {
                button.add_css_class("suggested-action");
            }
            let choose = choose.clone();
            button.connect_clicked(move |_| choose(choice));
            buttons.append(&button);
        }
        buttons
    }

    fn all_chosen(&self, path: &str, count: usize) -> Option<Choice> {
        let choices = self.choices.borrow();
        let chosen = choices.get(path)?;
        let first = *chosen.get(&0)?;
        (0..count).all(|n| chosen.get(&n) == Some(&first)).then_some(first)
    }

    fn confirm_abort(self: &Rc<Self>) {
        let alert = adw::AlertDialog::builder()
            .heading("Abort the sync?")
            .body("The pull stops and your notes are as they were before it. Your changes stay committed on this computer, and the next sync tries the pull again.")
            .build();
        alert.add_responses(&[("cancel", "Cancel"), ("abort", "Abort Sync")]);
        alert.set_response_appearance("abort", adw::ResponseAppearance::Destructive);
        let this = self.clone();
        alert.connect_response(Some("abort"), move |_, _| {
            let again = this.clone();
            this.git.abort(move |result| match result {
                Ok(()) => {
                    (again.hooks.notice)("The sync was stopped. Your commits are as they were before the pull.");
                    again.dialog.close();
                }
                Err(error) => {
                    (again.hooks.notice)(&format!("The sync could not be stopped: {error}"));
                    again.load();
                }
            });
        });
        alert.present(Some(&self.dialog));
    }

    fn finish(self: &Rc<Self>) {
        (self.hooks.save)();
        let mut resolutions = Vec::new();
        for file in self.files() {
            if Self::uses_marks(&file) {
                // The note is read again: a change since it was shown is chosen again, not written over.
                let now = std::fs::read_to_string(self.root.join(&file.path)).ok();
                let shown = self.texts.borrow().get(&file.path).cloned();
                let (Some(now), Some(shown)) = (now, shown) else { continue };
                if now != shown {
                    (self.hooks.notice)(&format!(
                        "{} changed while it was shown. It is shown again; choose once more.",
                        Vault::title(&file.path)
                    ));
                    self.load();
                    return;
                }
                let choices = self.choices.borrow().get(&file.path).cloned().unwrap_or_default();
                resolutions.push((file, Resolution::Text(ConflictText::resolve(&ConflictText::parse(&now), &choices))));
            } else if let Some(resolution) = self.whole.borrow().get(&file.path).cloned() {
                resolutions.push((file, resolution));
            }
        }
        self.finish.set_sensitive(false);
        let this = self.clone();
        self.git.finish(resolutions, move |result| match result {
            Ok(true) => {
                (this.hooks.notice)("The conflicts are resolved and the sync is done.");
                this.dialog.close();
            }
            Ok(false) => {
                (this.hooks.notice)("The next commit stopped on a conflict too. Choose again.");
                this.load();
            }
            Err(error) => {
                (this.hooks.notice)(&format!("The sync could not go on: {error}"));
                this.load();
            }
        });
    }
}

/// One side of a conflict: its title, and its lines in the code font.
fn side(title: &str, text: &str, tint: &str, kept: bool) -> gtk::Box {
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
    let head = gtk::Box::builder().spacing(4).build();
    head.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["caption", "daftar-side-title", tint]).build());
    if kept {
        head.append(&gtk::Image::builder().icon_name("object-select-symbolic").css_classes(["daftar-side-title", tint]).build());
    }
    column.append(&head);
    let shown = text.strip_suffix('\n').unwrap_or(text);
    let body = gtk::Label::builder()
        .label(if shown.is_empty() { "(nothing)" } else { shown })
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .vexpand(true)
        .css_classes(["monospace", "daftar-side", tint])
        .build();
    if kept {
        body.add_css_class("kept");
    }
    if shown.is_empty() {
        body.add_css_class("dim-label");
    }
    column.append(&body);
    column
}

fn describe(side: Side) -> &'static str {
    match side {
        Side::Changed => "changed",
        Side::Added => "added",
        Side::Deleted => "deleted",
    }
}

fn what(side: Side) -> &'static str {
    match side {
        Side::Changed => "Changed the file",
        Side::Added => "Added the file",
        Side::Deleted => "Deleted the file",
    }
}

/// The last two lines before the conflict at [at], to place it.
fn context(parts: &[ConflictPart], at: usize) -> String {
    let Some(ConflictPart::Plain(text)) = at.checked_sub(1).and_then(|before| parts.get(before)) else { return String::new() };
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    lines[lines.len().saturating_sub(2)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conflict_is_placed_by_the_lines_before_it() {
        let parts = ConflictText::parse("# Plan\nfirst\nsecond\n<<<<<<< HEAD\nr\n=======\nm\n>>>>>>> x\n");
        assert_eq!(context(&parts, 1), "first\nsecond");
        assert_eq!(context(&parts, 0), "");
        let at_top = ConflictText::parse("<<<<<<< HEAD\nr\n=======\nm\n>>>>>>> x\n");
        assert_eq!(context(&at_top, 0), "");
    }
}
