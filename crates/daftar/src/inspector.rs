//! The inspector (Ctrl+Alt+I), as on the Mac: the notes that link to the
//! open one, and the unlinked mentions -- where other notes write its name or
//! an alias without a link -- each with a button that makes it a link.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::text::direction;
use daftar_core::{Failure, Mention, UnlinkedMentions, Vault, VaultEdits};

use crate::store::VaultStore;

/// A few mentions from each note, so one note that names it all the time
/// does not fill the list; and no more than this in all.
const PER_NOTE: usize = 5;
const MOST: usize = 300;

type Open = dyn Fn(&str, usize);
type Notice = dyn Fn(&str);

pub struct Inspector {
    pub widget: gtk::ScrolledWindow,
    content: gtk::Box,
    /// A number that grows with each redraw, so a slow search for an older
    /// note does not land on a newer one.
    generation: Cell<u64>,
    open: Box<Open>,
    notice: Box<Notice>,
}

impl Inspector {
    pub fn new(open: impl Fn(&str, usize) + 'static, notice: impl Fn(&str) + 'static) -> Rc<Inspector> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let widget = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).width_request(300).child(&content).build();
        Rc::new(Inspector { widget, content, generation: Cell::new(0), open: Box::new(open), notice: Box::new(notice) })
    }

    /// Draws the inspector for the note at [path], or says there is none.
    pub fn show(self: &Rc<Self>, store: &Rc<VaultStore>, path: Option<&str>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let Some(path) = path.filter(|p| store.index().notes.contains_key(*p)) else {
            self.content.append(
                &adw::StatusPage::builder()
                    .icon_name("sidebar-show-right-symbolic")
                    .title("No Note")
                    .description("Open a note to see its backlinks.")
                    .css_classes(["compact"])
                    .build(),
            );
            return;
        };

        // Backlinks, each with the line that links.
        let backlinks = store.index().backlinks(path).to_vec();
        self.content.append(&title("Backlinks", backlinks.len()));
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        for link in &backlinks {
            let line = store.text(&link.source).and_then(|t| t.lines().nth(link.line).map(|l| l.trim().to_string())).unwrap_or_default();
            list.append(&self.row(&link.source, link.line, &line, None));
        }
        if backlinks.is_empty() {
            self.content.append(&empty("No note links here."));
        } else {
            self.content.append(&list);
        }

        // Unlinked mentions, found away from the window.
        let mentions_title = title("Unlinked mentions", 0);
        self.content.append(&mentions_title);
        let waiting = empty("Looking…");
        self.content.append(&waiting);
        let (names, candidates) = candidates(store, path);
        let root = store.root.clone();
        let (this, store, path) = (self.clone(), store.clone(), path.to_string());
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || find(&root, &names, &candidates)).await.unwrap_or_default();
            if this.generation.get() != generation {
                return;
            }
            this.content.remove(&waiting);
            set_count(&mentions_title, found.len());
            if found.is_empty() {
                this.content.append(&empty("No other note names it without a link."));
                return;
            }
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
            for (source, mention) in found {
                list.append(&this.row(&source, mention.line, &mention.excerpt, Some((&store, &path, mention.clone()))));
            }
            this.content.append(&list);
        });
    }

    /// A row: the note, the line, and for a mention a button that links it.
    fn row(self: &Rc<Self>, source: &str, line: usize, text: &str, mention: Option<(&Rc<VaultStore>, &str, Mention)>) -> gtk::ListBoxRow {
        let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        lines.append(
            &gtk::Label::builder()
                .label(Vault::title(source))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["heading"])
                .build(),
        );
        let rtl = direction::starts_rtl(text);
        lines.append(
            &gtk::Label::builder()
                .label(text)
                .xalign(if rtl { 1.0 } else { 0.0 })
                .wrap(true)
                .lines(3)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        let open = gtk::Button::builder().child(&lines).css_classes(["flat"]).hexpand(true).build();
        {
            let (this, source) = (self.clone(), source.to_string());
            open.connect_clicked(move |_| (this.open)(&source, line));
        }
        let row = gtk::Box::builder().spacing(4).build();
        row.append(&open);
        if let Some((store, target, mention)) = mention {
            let link = gtk::Button::builder().label("Link").valign(gtk::Align::Center).css_classes(["flat", "accent"]).build();
            let (this, store, source, target) = (self.clone(), store.clone(), source.to_string(), target.to_string());
            link.connect_clicked(move |_| this.link(&store, &source, &target, &mention));
            row.append(&link);
        }
        gtk::ListBoxRow::builder().child(&row).activatable(false).build()
    }

    /// The mention made a link to [target], with the shortest name that still
    /// finds it; the file is read again and the edit refused if it moved.
    fn link(self: &Rc<Self>, store: &Rc<VaultStore>, source: &str, target: &str, mention: &Mention) {
        let name = link_name(store, source, target);
        match VaultEdits::apply(source, &store.root, |text| UnlinkedMentions::link(mention, &name, text)) {
            Ok(()) => store.wrote(),
            Err(Failure::TextMoved) => (self.notice)("The note changed before the click. Nothing was written; the list is made again."),
            Err(error) => (self.notice)(&format!("The link could not be written: {error}")),
        }
        store.refresh(&[source.to_string()]);
        self.show(store, Some(target));
    }
}

/// `[[Title]]` when that finds [target] from [source], else its path.
fn link_name(store: &VaultStore, source: &str, target: &str) -> String {
    let title = Vault::title(target);
    if store.index().resolve(&title, source).as_deref() == Some(target) { title } else { target.trim_end_matches(".md").to_string() }
}

/// The note's names, and the notes that may write one: the full-text search
/// for each name, without the note itself and the notes that link to it.
fn candidates(store: &VaultStore, path: &str) -> (Vec<String>, Vec<String>) {
    let index = store.index();
    let mut names = vec![Vault::title(path)];
    if let Some(note) = index.notes.get(path) {
        names.extend(note.parsed.aliases.iter().cloned());
    }
    let linking: std::collections::BTreeSet<&str> = index.backlinks(path).iter().map(|b| b.source.as_str()).collect();
    let mut sources = std::collections::BTreeSet::new();
    for name in names.iter().filter(|n| n.chars().count() >= UnlinkedMentions::SHORTEST) {
        for hit in store.search.search(&format!("\"{}\"", name.replace('"', "")), MOST) {
            sources.insert(hit.path);
        }
    }
    let candidates = sources.into_iter().filter(|s| s != path && !linking.contains(s.as_str()) && Vault::is_note(s)).collect();
    (names, candidates)
}

/// The mentions of [names] in each of [candidates], read from the disk.
fn find(root: &Path, names: &[String], candidates: &[String]) -> Vec<(String, Mention)> {
    let root: PathBuf = root.to_path_buf();
    let mut out = Vec::new();
    for source in candidates {
        let Ok(text) = std::fs::read_to_string(root.join(source)) else { continue };
        out.extend(UnlinkedMentions::find(names, &text).into_iter().take(PER_NOTE).map(|m| (source.clone(), m)));
        if out.len() >= MOST {
            break;
        }
    }
    out
}

fn title(text: &str, count: usize) -> gtk::Box {
    let head = gtk::Box::builder().spacing(8).margin_top(8).build();
    head.append(&gtk::Label::builder().label(text).xalign(0.0).css_classes(["heading"]).build());
    head.append(&gtk::Label::builder().label(count.to_string()).css_classes(["dim-label", "numeric", "caption"]).build());
    head
}

fn set_count(head: &gtk::Box, count: usize) {
    if let Some(label) = head.last_child().and_downcast::<gtk::Label>() {
        label.set_label(&count.to_string());
    }
}

fn empty(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim-label"]).build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn mentions_come_a_few_from_each_note() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.md"), "Roadmap here.\nRoadmap again.\n".repeat(4)).unwrap();
        fs::write(root.path().join("B.md"), "No such word.\n").unwrap();
        let found = find(root.path(), &["Roadmap".into()], &["A.md".into(), "B.md".into(), "Gone.md".into()]);
        assert_eq!(found.len(), PER_NOTE);
        assert!(found.iter().all(|(source, m)| source == "A.md" && m.text == "Roadmap"));
    }
}
