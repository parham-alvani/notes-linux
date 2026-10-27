//! What a tab with no note shows, as on the Mac (`HomeView.swift`): the
//! time, a greeting, a field that opens a note, the notes read last, the
//! bookmarks and a quote, in cards over the person's picture. It replaces
//! Obsidian's Beautitab new tab and follows its settings in the vault; the
//! picture is `home.image` in the config file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use chrono::{DateTime, Local, Timelike};
use gtk::{gdk, gio, glib};

use daftar_core::{Beautitab, Quote, Vault, home};

use crate::store::VaultStore;

/// What the page needs from the window.
pub struct Handlers {
    /// A note picked in a list, or the field's best match.
    pub open: Box<dyn Fn(&str)>,
    /// Return in the field: the window opens the best match, or Quick Open
    /// with the text when there is none.
    pub search: Box<dyn Fn(&str)>,
}

pub struct HomePage {
    pub widget: gtk::Overlay,
    picture: gtk::Picture,
    scrim: gtk::Box,
    clock: gtk::Box,
    time: gtk::Label,
    date: gtk::Label,
    greeting: gtk::Label,
    search: gtk::Box,
    entry: gtk::Entry,
    lists: gtk::Box,
    quote: gtk::Box,
    settings: RefCell<Beautitab>,
    /// The picture shown: a newer refresh drops an older decode.
    shown: RefCell<Option<PathBuf>>,
    timer: RefCell<Option<glib::SourceId>>,
    handlers: Handlers,
}

impl HomePage {
    pub fn new(handlers: Handlers) -> Rc<HomePage> {
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).hexpand(true).vexpand(true).build();
        let scrim = gtk::Box::builder().css_classes(["home-scrim"]).hexpand(true).vexpand(true).visible(false).build();
        let label =
            |classes: &[&str]| gtk::Label::builder().css_classes(classes.to_vec()).justify(gtk::Justification::Center).wrap(true).build();
        let (time, date, greeting) = (label(&["home-time"]), label(&["home-date"]), label(&["home-greeting"]));
        greeting.set_margin_top(8);
        let clock = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["home-clock"]).build();
        clock.append(&time);
        clock.append(&date);
        clock.append(&greeting);

        let entry = gtk::Entry::builder().placeholder_text("Find a note").has_frame(false).hexpand(true).build();
        let search = gtk::Box::builder().spacing(8).css_classes(["home-glass", "home-search"]).build();
        search.set_tooltip_text(Some("Return opens the best match; an empty field opens Quick Open"));
        search.append(&gtk::Image::from_icon_name("system-search-symbolic"));
        search.append(&entry);
        search.append(&gtk::Label::builder().label("Ctrl+P").css_classes(["home-keys", "monospace", "caption"]).build());

        let lists = gtk::Box::builder().spacing(16).homogeneous(true).build();
        let quote = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["home-glass", "home-card"]).build();

        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(22).valign(gtk::Align::Center).build();
        column.append(&clock);
        column.append(&search);
        column.append(&lists);
        column.append(&quote);
        let clamp = adw::Clamp::builder()
            .maximum_size(560)
            .child(&column)
            .margin_top(40)
            .margin_bottom(40)
            .margin_start(40)
            .margin_end(40)
            .vexpand(true)
            .build();
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();

        let widget = gtk::Overlay::builder().css_classes(["daftar-home"]).child(&picture).build();
        widget.add_overlay(&scrim);
        widget.add_overlay(&scroller);
        let page = Rc::new(HomePage {
            widget,
            picture,
            scrim,
            clock,
            time,
            date,
            greeting,
            search,
            entry,
            lists,
            quote,
            settings: RefCell::new(Beautitab::default()),
            shown: RefCell::new(None),
            timer: RefCell::new(None),
            handlers,
        });
        {
            let weak = Rc::downgrade(&page);
            page.entry.connect_activate(move |entry| {
                let Some(page) = weak.upgrade() else { return };
                let text = entry.text().trim().to_string();
                entry.set_text("");
                (page.handlers.search)(&text);
            });
        }
        // The time moves on while the page is on screen.
        {
            let weak = Rc::downgrade(&page);
            page.widget.connect_map(move |_| {
                let Some(page) = weak.upgrade() else { return };
                page.tick();
                let inner = Rc::downgrade(&page);
                let id = glib::timeout_add_seconds_local(15, move || match inner.upgrade() {
                    Some(page) => {
                        page.tick();
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                });
                if let Some(old) = page.timer.replace(Some(id)) {
                    old.remove();
                }
            });
        }
        {
            let weak = Rc::downgrade(&page);
            page.widget.connect_unmap(move |_| {
                if let Some(id) = weak.upgrade().and_then(|page| page.timer.take()) {
                    id.remove();
                }
            });
        }
        page
    }

    /// Draws the page again from the vault: Beautitab's settings, the
    /// picture [image] names (`home.image`), the notes [opened] in the window
    /// lately and the ones read last.
    pub fn refresh(self: &Rc<Self>, store: &VaultStore, image: &str, opened: &[String]) {
        let settings = Beautitab::read(&store.root);
        self.clock.set_visible(settings.show_time || settings.show_greeting);
        self.time.set_visible(settings.show_time);
        self.date.set_visible(settings.show_time);
        self.greeting.set_visible(settings.show_greeting);
        self.search.set_visible(settings.show_search);

        while let Some(child) = self.lists.first_child() {
            self.lists.remove(&child);
        }
        let files = store.index().files.clone();
        if settings.show_recent_files {
            let recent = home::recent(opened, &store.places.recently_read(40), |p| files.contains(p) && !store.is_hidden(p), 8);
            if !recent.is_empty() {
                self.lists.append(&self.list("Recent", "document-open-recent-symbolic", &recent));
            }
        }
        if settings.show_bookmarks {
            let marked: Vec<String> = store
                .bookmarks
                .borrow()
                .entries()
                .into_iter()
                .filter(|e| !e.folder && files.contains(&e.path))
                .map(|e| e.path)
                .take(8)
                .collect();
            if !marked.is_empty() {
                self.lists.append(&self.list("Bookmarks", "user-bookmarks-symbolic", &marked));
            }
        }
        self.lists.set_visible(self.lists.first_child().is_some());

        while let Some(child) = self.quote.first_child() {
            self.quote.remove(&child);
        }
        let quote = settings
            .show_quote
            .then(|| settings.quotes.get(glib::random_int_range(0, settings.quotes.len() as i32) as usize).cloned())
            .flatten();
        self.quote.set_visible(quote.is_some());
        if let Some(Quote { text, author }) = quote {
            let line = gtk::Label::builder()
                .label(format!("“{text}”"))
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["home-quote"])
                .build();
            self.quote.append(&line);
            if !author.is_empty() {
                self.quote.append(&gtk::Label::builder().label(format!("— {author}")).css_classes(["home-date"]).build());
            }
        }

        // The config file's picture, else Beautitab's own when it is a file.
        let home_dir = glib::home_dir();
        let picture = if image.trim().is_empty() {
            settings.backgrounds.iter().filter_map(|b| home::picture_path(b, Some(&store.root), &home_dir)).find(|p| p.is_file())
        } else {
            home::picture_path(image, Some(&store.root), &home_dir)
        };
        *self.settings.borrow_mut() = settings;
        self.tick();
        self.show_picture(picture);
    }

    pub fn focus(&self) {
        if self.search.is_visible() {
            self.entry.grab_focus();
        }
    }

    fn list(self: &Rc<Self>, title: &str, icon: &str, paths: &[String]) -> gtk::Box {
        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["home-glass", "home-card"]).build();
        let head = gtk::Box::builder().spacing(6).css_classes(["home-heading"]).build();
        head.append(&gtk::Image::from_icon_name(icon));
        head.append(&gtk::Label::new(Some(title)));
        card.append(&head);
        for path in paths {
            let folder = path.rsplit_once('/').map(|(folder, _)| folder).unwrap_or("");
            let row = gtk::Box::builder().spacing(8).build();
            row.append(&gtk::Image::from_icon_name(if Vault::is_note(path) {
                "text-x-generic-symbolic"
            } else {
                "folder-documents-symbolic"
            }));
            row.append(&gtk::Label::builder().label(Vault::title(path)).ellipsize(gtk::pango::EllipsizeMode::End).xalign(0.0).build());
            if !folder.is_empty() {
                row.append(
                    &gtk::Label::builder()
                        .label(folder)
                        .ellipsize(gtk::pango::EllipsizeMode::Start)
                        .xalign(0.0)
                        .hexpand(true)
                        .css_classes(["home-folder"])
                        .build(),
                );
            }
            let button = gtk::Button::builder().child(&row).css_classes(["flat", "home-row"]).tooltip_text(path).build();
            let (page, path) = (Rc::downgrade(self), path.clone());
            button.connect_clicked(move |_| {
                if let Some(page) = page.upgrade() {
                    (page.handlers.open)(&path);
                }
            });
            card.append(&button);
        }
        card
    }

    fn tick(&self) {
        let now = Local::now();
        let settings = self.settings.borrow();
        self.time.set_label(&time(now, settings.twenty_four_hour.unwrap_or_else(system_twenty_four_hour)));
        let date = now.format("%A, %B %-d").to_string();
        self.date.set_label(&match crate::settings::persian_date(daftar_core::Day::today()) {
            Some(persian) => format!("{date}\n{persian}"),
            None => date,
        });
        self.greeting.set_label(&home::greeting(&settings.greeting_text, now.hour()));
    }

    fn show_picture(self: &Rc<Self>, path: Option<PathBuf>) {
        *self.shown.borrow_mut() = path.clone();
        let Some(path) = path else {
            self.picture.set_paintable(None::<&gdk::Paintable>);
            self.scrim.set_visible(false);
            return;
        };
        if let Some(texture) = cached(&path) {
            self.picture.set_paintable(Some(&texture));
            self.scrim.set_visible(true);
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = path.clone();
            let decoded = gio::spawn_blocking(move || gdk::Texture::from_filename(&file).ok()).await.ok().flatten();
            let Some(page) = weak.upgrade() else { return };
            if page.shown.borrow().as_deref() != Some(path.as_path()) {
                return;
            }
            match decoded {
                Some(texture) => {
                    keep(&path, &texture);
                    page.picture.set_paintable(Some(&texture));
                    page.scrim.set_visible(true);
                }
                None => {
                    page.picture.set_paintable(None::<&gdk::Paintable>);
                    page.scrim.set_visible(false);
                }
            }
        });
    }
}

/// The hour as Beautitab's setting asks: `14:05`, or `2:05 PM`.
pub fn time(now: DateTime<Local>, twenty_four_hour: bool) -> String {
    if twenty_four_hour { now.format("%H:%M").to_string() } else { now.format("%-I:%M %p").to_string() }
}

/// GNOME's `clock-format`, when its schema is there; else a 24-hour clock.
fn system_twenty_four_hour() -> bool {
    let Some(schema) = gio::SettingsSchemaSource::default().and_then(|source| source.lookup("org.gnome.desktop.interface", true)) else {
        return true;
    };
    if !schema.has_key("clock-format") {
        return true;
    }
    gio::Settings::new("org.gnome.desktop.interface").string("clock-format") != "12h"
}

thread_local! {
    /// The decoded pictures, by file, so a new tab never decodes one again;
    /// a file changed on disk is decoded again.
    static PICTURES: RefCell<HashMap<PathBuf, (Option<SystemTime>, gdk::Texture)>> = RefCell::new(HashMap::new());
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn cached(path: &Path) -> Option<gdk::Texture> {
    let now = stamp(path);
    PICTURES.with(|pictures| pictures.borrow().get(path).filter(|(at, _)| *at == now).map(|(_, texture)| texture.clone()))
}

fn keep(path: &Path, texture: &gdk::Texture) {
    PICTURES.with(|pictures| {
        let mut pictures = pictures.borrow_mut();
        // One picture is in use at a time; a few are kept for the Settings preview.
        if pictures.len() > 4 {
            pictures.clear();
        }
        pictures.insert(path.to_path_buf(), (stamp(path), texture.clone()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_time_follows_the_setting() {
        let at = Local.with_ymd_and_hms(2026, 10, 27, 14, 5, 0).unwrap();
        assert_eq!(time(at, true), "14:05");
        assert_eq!(time(at, false), "2:05 PM");
        let morning = Local.with_ymd_and_hms(2026, 10, 27, 9, 30, 0).unwrap();
        assert_eq!(time(morning, true), "09:30");
        assert_eq!(time(morning, false), "9:30 AM");
    }
}
