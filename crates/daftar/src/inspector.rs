//! The inspector (Ctrl+Alt+I), as on the Mac: the note's outline with the
//! section on screen marked, the notes that link to it, the unlinked
//! mentions -- where other notes write its name or an alias without a link,
//! each with a button that makes it a link -- the links it makes (those to
//! no note yet in red), the places it names and its tags.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::text::direction;
use daftar_core::{Detected, DetectedKind, Failure, Mention, UnlinkedMentions, Vault, VaultEdits};

use crate::store::VaultStore;

/// A few mentions from each note, so one note that names it all the time
/// does not fill the list; and no more than this in all.
const PER_NOTE: usize = 5;
const MOST: usize = 300;

type Open = dyn Fn(&str, usize);
type Notice = dyn Fn(&str);
type Tag = dyn Fn(&str);

/// The outline drawn last: its list, and each row's heading line.
struct Outline {
    list: gtk::ListBox,
    lines: Vec<usize>,
}

pub struct Inspector {
    pub widget: gtk::ScrolledWindow,
    content: gtk::Box,
    /// A number that grows with each redraw, so a slow search for an older
    /// note does not land on a newer one.
    generation: Cell<u64>,
    open: Box<Open>,
    notice: Box<Notice>,
    tag: Box<Tag>,
    outline: RefCell<Option<Outline>>,
    /// The heading line of the section on screen in the reader.
    section: Cell<Option<usize>>,
}

impl Inspector {
    /// [open] goes to a note's line, [tag] searches for a tag.
    pub fn new(open: impl Fn(&str, usize) + 'static, notice: impl Fn(&str) + 'static, tag: impl Fn(&str) + 'static) -> Rc<Inspector> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let widget = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).width_request(300).child(&content).build();
        Rc::new(Inspector {
            widget,
            content,
            generation: Cell::new(0),
            open: Box::new(open),
            notice: Box::new(notice),
            tag: Box::new(tag),
            outline: RefCell::new(None),
            section: Cell::new(None),
        })
    }

    /// The reader's section on screen, marked in the outline; `None` above the first heading.
    pub fn set_section(&self, line: Option<usize>) {
        self.section.set(line);
        let outline = self.outline.borrow();
        let Some(outline) = outline.as_ref() else { return };
        match line.and_then(|line| outline.lines.iter().position(|l| *l == line)) {
            Some(at) => outline.list.select_row(outline.list.row_at_index(at as i32).as_ref()),
            None => outline.list.unselect_all(),
        }
    }

    /// Draws the inspector for the note at [path], or says there is none.
    pub fn show(self: &Rc<Self>, store: &Rc<VaultStore>, path: Option<&str>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.outline.take();
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

        let note = store.index().notes.get(path).cloned();
        let Some(note) = note else { return };

        // The outline, each level further in; a click goes to the heading.
        if !note.parsed.headings.is_empty() {
            self.content.append(&title("Outline", note.parsed.headings.len()));
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::Single)
                .css_classes(["navigation-sidebar", "daftar-outline"])
                .build();
            for heading in &note.parsed.headings {
                let rtl = direction::starts_rtl(&heading.text);
                let label = gtk::Label::builder()
                    .label(&heading.text)
                    .xalign(if rtl { 1.0 } else { 0.0 })
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .margin_start(((heading.level.max(1) - 1) * 12) as i32)
                    .css_classes([format!("outline-h{}", heading.level.min(6))])
                    .build();
                list.append(&gtk::ListBoxRow::builder().child(&label).tooltip_text(&heading.text).build());
            }
            let lines: Vec<usize> = note.parsed.headings.iter().map(|h| h.line).collect();
            {
                let (this, path, lines) = (self.clone(), path.to_string(), lines.clone());
                list.connect_row_activated(move |_, row| {
                    if let Some(line) = lines.get(row.index() as usize) {
                        (this.open)(&path, *line);
                    }
                });
            }
            self.content.append(&list);
            *self.outline.borrow_mut() = Some(Outline { list, lines });
            self.set_section(self.section.get());
        }

        // Backlinks, each with the line that links.
        let backlinks: Vec<_> = store.index().backlinks(path).iter().filter(|b| !store.is_hidden(&b.source)).cloned().collect();
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

        // Unlinked mentions, found away from the window, in their place.
        let mentions = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        self.content.append(&mentions);
        let mentions_title = title("Unlinked mentions", 0);
        mentions.append(&mentions_title);
        let waiting = empty("Looking…");
        mentions.append(&waiting);
        self.outgoing(store, path, &note);
        self.places_and_tags(store, path, &note);
        let (names, candidates) = candidates(store, path);
        let root = store.root.clone();
        let (this, store, path) = (self.clone(), store.clone(), path.to_string());
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || find(&root, &names, &candidates)).await.unwrap_or_default();
            if this.generation.get() != generation {
                return;
            }
            mentions.remove(&waiting);
            set_count(&mentions_title, found.len());
            if found.is_empty() {
                mentions.append(&empty("No other note names it without a link."));
                return;
            }
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
            for (source, mention) in found {
                list.append(&this.row(&source, mention.line, &mention.excerpt, Some((&store, &path, mention.clone()))));
            }
            mentions.append(&list);
        });
    }

    /// The links the note makes, each once: to a note, a file it embeds, or
    /// in red to a note not there yet.
    fn outgoing(self: &Rc<Self>, store: &Rc<VaultStore>, path: &str, note: &daftar_core::NoteEntry) {
        let mut seen = std::collections::HashSet::new();
        let links: Vec<(String, Option<String>, bool)> = note
            .parsed
            .links
            .iter()
            .filter(|link| !link.target.is_empty() && seen.insert(link.target.clone()))
            .filter_map(|link| {
                let found = store.index().resolve(&link.target, path);
                (found.as_deref() != Some(path)).then(|| (link.target.clone(), found, link.embed))
            })
            .collect();
        if links.is_empty() {
            return;
        }
        self.content.append(&title("Outgoing links", links.len()));
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        for (target, found, embed) in links {
            let row = gtk::Box::builder().spacing(8).margin_top(6).margin_bottom(6).margin_start(8).margin_end(8).build();
            let icon = match (&found, embed) {
                (None, _) => "list-add-symbolic",
                (Some(_), true) => "image-x-generic-symbolic",
                (Some(_), false) => "go-next-symbolic",
            };
            row.append(&gtk::Image::from_icon_name(icon));
            let name = found.as_deref().map_or_else(|| target.clone(), Vault::title);
            let label = gtk::Label::builder().label(&name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
            row.append(&label);
            let item = gtk::ListBoxRow::builder().child(&row).build();
            if let Some(found) = found {
                item.set_activatable(true);
                item.set_tooltip_text(Some(&found));
                let (this, found) = (self.clone(), found.clone());
                let click = gtk::GestureClick::new();
                click.connect_released(move |_, _, _, _| (this.open)(&found, 0));
                item.add_controller(click);
            } else {
                item.set_activatable(false);
                item.set_tooltip_text(Some("Not in the vault yet"));
                row.add_css_class("error");
            }
            list.append(&item);
        }
        self.content.append(&list);
    }

    /// The addresses the note writes, each opening a map, and its tags,
    /// each searching for the notes with it.
    fn places_and_tags(self: &Rc<Self>, store: &Rc<VaultStore>, path: &str, note: &daftar_core::NoteEntry) {
        let places: Vec<String> = store
            .text(path)
            .map(|text| {
                Detected::find(&text).into_iter().filter(|item| item.kind == DetectedKind::Address).map(|item| item.value).collect()
            })
            .unwrap_or_default();
        if !places.is_empty() {
            self.content.append(&title("Places in this note", places.len()));
            let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
            for place in places {
                let row = gtk::Box::builder().spacing(8).margin_top(6).margin_bottom(6).margin_start(8).margin_end(8).build();
                row.append(&gtk::Image::from_icon_name("mark-location-symbolic"));
                row.append(&gtk::Label::builder().label(&place).xalign(0.0).wrap(true).hexpand(true).build());
                let item = gtk::ListBoxRow::builder().child(&row).tooltip_text("Show on a map").build();
                let click = gtk::GestureClick::new();
                let widget = self.widget.clone();
                click.connect_released(move |_, _, _, _| {
                    if let Some(url) = crate::reader::detected_url("address", &place) {
                        let window = widget.root().and_downcast::<gtk::Window>();
                        gtk::UriLauncher::new(&url).launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
                    }
                });
                item.add_controller(click);
                list.append(&item);
            }
            self.content.append(&list);
        }
        if !note.parsed.tags.is_empty() {
            self.content.append(&title("Tags", note.parsed.tags.len()));
            // One line of links that wraps, as the Mac's row of tags does.
            let markup = note
                .parsed
                .tags
                .iter()
                .map(|tag| {
                    let tag = glib::markup_escape_text(tag);
                    format!("<a href=\"{}\">#{tag}</a>", glib::markup_escape_text(&format!("tag:{tag}")))
                })
                .collect::<Vec<_>>()
                .join("   ");
            let tags = gtk::Label::builder().use_markup(true).label(&markup).wrap(true).xalign(0.0).css_classes(["daftar-tags"]).build();
            let this = self.clone();
            tags.connect_activate_link(move |_, uri| {
                (this.tag)(uri.strip_prefix("tag:").unwrap_or(uri));
                glib::Propagation::Stop
            });
            self.content.append(&tags);
        }
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
    let candidates =
        sources.into_iter().filter(|s| s != path && !linking.contains(s.as_str()) && Vault::is_note(s) && !store.is_hidden(s)).collect();
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
