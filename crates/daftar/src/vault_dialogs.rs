//! The dialogs that change many notes at once, as on the Mac
//! (`TagRenameSheet`, `VaultReplaceSheet`): renaming a tag, and finding and
//! replacing in the whole vault. Each says what it will change before it
//! writes, and each note is read again before its write.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{ReplaceMatch, ReplaceOptions, ReplacePattern, TagRename, Vault, VaultReplace};

use crate::store::{VaultStore, file_in};

/// Rename a Tag: [tag] and the tags under it, in every note's text and
/// `tags` property. [done] hears how many notes were written.
pub fn rename_tag(parent: &impl IsA<gtk::Widget>, store: Rc<VaultStore>, tag: &str, done: impl Fn(String) + 'static) {
    let dialog = adw::AlertDialog::new(Some("Rename a Tag"), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("go", "Rename");
    dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("go"));
    dialog.set_close_response("cancel");
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let old = adw::EntryRow::builder().title("Tag").text(tag.trim_start_matches('#')).build();
    let new = adw::EntryRow::builder().title("New name").activates_default(true).build();
    list.append(&old);
    list.append(&new);
    let note = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
    let extra = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
    extra.append(&list);
    extra.append(&note);
    dialog.set_extra_child(Some(&extra));
    let check = {
        let (dialog, old, new, note, store) = (dialog.clone(), old.clone(), new.clone(), note.clone(), store.clone());
        move || {
            let (from, to) = (old.text().trim().trim_start_matches('#').to_string(), new.text().trim().trim_start_matches('#').to_string());
            let count = if TagRename::is_valid(&from) { store.notes_tagged(&from).len() } else { 0 };
            note.set_label(&match () {
                _ if !from.is_empty() && !TagRename::is_valid(&from) => {
                    "A tag is letters, digits, _, - and /, not only digits.".to_string()
                }
                _ if !to.is_empty() && !TagRename::is_valid(&to) => {
                    "The new name is letters, digits, _, - and /, not only digits.".to_string()
                }
                _ if count == 0 => "No note has this tag.".to_string(),
                _ => format!(
                    "{} {} #{from}, or a tag under it. Code and links are left as they are.",
                    count,
                    if count == 1 { "note has" } else { "notes have" }
                ),
            });
            dialog.set_response_enabled("go", count > 0 && TagRename::is_valid(&to) && from != to);
        }
    };
    check();
    {
        let check = check.clone();
        old.connect_changed(move |_| check());
    }
    new.connect_changed(move |_| check());
    let focus = new.clone();
    glib::idle_add_local_once(move || {
        focus.grab_focus();
    });
    dialog.connect_response(Some("go"), move |_, _| {
        let (from, to) = (old.text().trim().trim_start_matches('#').to_string(), new.text().trim().trim_start_matches('#').to_string());
        let (written, failed) = store.rename_tag(&from, &to);
        let mut text = match written.len() {
            1 => format!("#{from} is #{to} in 1 note."),
            n => format!("#{from} is #{to} in {n} notes."),
        };
        if !failed.is_empty() {
            text.push_str(&format!(" {} could not be written.", failed.len()));
        }
        done(text);
    });
    dialog.present(Some(parent));
}

/// The matches of one note, and which of them are chosen.
struct NoteMatches {
    path: String,
    matches: Vec<ReplaceMatch>,
}

struct ReplaceState {
    pattern: Option<ReplacePattern>,
    notes: Vec<NoteMatches>,
    /// `(path, match id)` of the matches to replace.
    chosen: BTreeSet<(String, usize)>,
    /// Bumped by each search: a slower one that comes back late is dropped.
    run: u64,
    pending: Option<glib::SourceId>,
}

/// Find and Replace in the Vault (Ctrl+Shift+H): the matches of every note,
/// each one chosen or not, then replaced. [done] hears what was written.
pub fn replace_in_vault(parent: &impl IsA<gtk::Widget>, store: Rc<VaultStore>, find: &str, done: impl Fn(String) + 'static) {
    let dialog = adw::Dialog::builder().title("Find and Replace in the Vault").content_width(760).content_height(640).build();
    let fields = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let find_row = adw::EntryRow::builder().title("Find").text(find).build();
    let replace_row = adw::EntryRow::builder().title("Replace with").build();
    fields.append(&find_row);
    fields.append(&replace_row);
    let toggles = gtk::Box::builder().spacing(12).build();
    let option = |label: &str, active: bool| {
        let check = gtk::CheckButton::builder().label(label).active(active).build();
        toggles.append(&check);
        check
    };
    let case = option("Match case", false);
    let word = option("Whole word", false);
    let regex = option("Regular expression", false);
    let skip_code = option("Skip code", true);
    let status = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
    let results = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let scroller = gtk::ScrolledWindow::builder().child(&results).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(16)
        .margin_end(16)
        .build();
    body.append(&fields);
    body.append(&toggles);
    body.append(&status);
    body.append(&scroller);
    let replace = gtk::Button::builder().label("Replace").css_classes(["suggested-action"]).sensitive(false).build();
    let header = adw::HeaderBar::new();
    header.pack_end(&replace);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));

    let state = Rc::new(RefCell::new(ReplaceState { pattern: None, notes: vec![], chosen: BTreeSet::new(), run: 0, pending: None }));
    let options = {
        let (case, word, regex, skip_code) = (case.clone(), word.clone(), regex.clone(), skip_code.clone());
        move || ReplaceOptions {
            match_case: case.is_active(),
            whole_word: word.is_active(),
            regular_expression: regex.is_active(),
            skip_code: skip_code.is_active(),
        }
    };

    // The count on the button, as the choices change.
    let update_button: Rc<dyn Fn()> = {
        let (state, replace) = (state.clone(), replace.clone());
        Rc::new(move || {
            let count = state.borrow().chosen.len();
            replace.set_label(&if count == 0 { "Replace".to_string() } else { format!("Replace {count}") });
            replace.set_sensitive(count > 0);
        })
    };

    // The list drawn from the state.
    let draw: Rc<dyn Fn()> = {
        let (state, results, status, update_button) = (state.clone(), results.clone(), status.clone(), update_button.clone());
        Rc::new(move || {
            while let Some(child) = results.first_child() {
                results.remove(&child);
            }
            let notes = std::mem::take(&mut state.borrow_mut().notes);
            let total: usize = notes.iter().map(|n| n.matches.len()).sum();
            for note in &notes {
                let head = gtk::Label::builder()
                    .label(format!("{} · {}", Vault::title(&note.path), note.matches.len()))
                    .tooltip_text(&note.path)
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .margin_top(8)
                    .margin_bottom(4)
                    .margin_start(12)
                    .build();
                let row = gtk::ListBoxRow::builder().child(&head).activatable(false).build();
                results.append(&row);
                for found in &note.matches {
                    let key = (note.path.clone(), found.id());
                    let check = gtk::CheckButton::builder().active(state.borrow().chosen.contains(&key)).valign(gtk::Align::Center).build();
                    let markup = format!(
                        "<span alpha=\"60%\">{}</span>  {}<s>{}</s><b>{}</b>{}",
                        found.line + 1,
                        glib::markup_escape_text(&found.prefix),
                        glib::markup_escape_text(&found.found.replace('\n', "↵")),
                        glib::markup_escape_text(&found.replacement.replace('\n', "↵")),
                        glib::markup_escape_text(&found.suffix),
                    );
                    let label = gtk::Label::builder().use_markup(true).label(&markup).xalign(0.0).wrap(true).hexpand(true).build();
                    let line = gtk::Box::builder().spacing(8).margin_start(12).margin_end(12).margin_top(2).margin_bottom(2).build();
                    line.append(&check);
                    line.append(&label);
                    {
                        let (state, update_button) = (state.clone(), update_button.clone());
                        check.connect_toggled(move |check| {
                            {
                                let mut state = state.borrow_mut();
                                if check.is_active() {
                                    state.chosen.insert(key.clone());
                                } else {
                                    state.chosen.remove(&key);
                                }
                            }
                            update_button();
                        });
                    }
                    results.append(&gtk::ListBoxRow::builder().child(&line).activatable(false).build());
                }
            }
            status.set_label(&match (notes.len(), total) {
                (_, 0) => "No match.".to_string(),
                (1, 1) => "1 match in 1 note.".to_string(),
                (1, n) => format!("{n} matches in 1 note."),
                (notes, n) => format!("{n} matches in {notes} notes."),
            });
            state.borrow_mut().notes = notes;
            update_button();
        })
    };

    // A search, a moment after the last change, read from disk off the window.
    let search: Rc<dyn Fn()> = {
        let (state, store, find_row, replace_row, status, draw) =
            (state.clone(), store.clone(), find_row.clone(), replace_row.clone(), status.clone(), draw.clone());
        Rc::new(move || {
            let pattern = match ReplacePattern::new(&find_row.text(), &replace_row.text(), options()) {
                Ok(pattern) => pattern,
                Err(error) => {
                    status.set_label(&error.to_string());
                    let mut state = state.borrow_mut();
                    state.notes.clear();
                    state.chosen.clear();
                    state.pattern = None;
                    drop(state);
                    draw();
                    return;
                }
            };
            let run = {
                let mut state = state.borrow_mut();
                state.run += 1;
                state.pattern = pattern.clone();
                state.run
            };
            let Some(pattern) = pattern else {
                state.borrow_mut().notes.clear();
                state.borrow_mut().chosen.clear();
                draw();
                status.set_label("");
                return;
            };
            status.set_label("Searching…");
            let root = store.root.clone();
            let paths: Vec<String> = store.index().files.iter().filter(|p| Vault::is_note(p)).cloned().collect();
            let (state, draw) = (state.clone(), draw.clone());
            glib::spawn_future_local(async move {
                let found = gio::spawn_blocking(move || {
                    paths
                        .into_iter()
                        .filter_map(|path| {
                            let text = std::fs::read_to_string(file_in(&path, &root)?).ok()?;
                            let matches = VaultReplace::matches_of(&text, &pattern);
                            (!matches.is_empty()).then_some(NoteMatches { path, matches })
                        })
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                {
                    let mut state = state.borrow_mut();
                    if state.run != run {
                        return;
                    }
                    state.chosen = found.iter().flat_map(|n| n.matches.iter().map(|m| (n.path.clone(), m.id()))).collect();
                    state.notes = found;
                }
                draw();
            });
        })
    };
    let later: Rc<dyn Fn()> = {
        let (state, search) = (state.clone(), search.clone());
        Rc::new(move || {
            if let Some(id) = state.borrow_mut().pending.take() {
                id.remove();
            }
            let (state_now, search) = (state.clone(), search.clone());
            let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
                state_now.borrow_mut().pending = None;
                search();
            });
            state.borrow_mut().pending = Some(id);
        })
    };
    for row in [&find_row, &replace_row] {
        let later = later.clone();
        row.connect_changed(move |_| later());
    }
    for check in [&case, &word, &regex, &skip_code] {
        let later = later.clone();
        check.connect_toggled(move |_| later());
    }

    {
        let (state, store, search) = (state.clone(), store.clone(), search.clone());
        replace.connect_clicked(move |_| {
            let (pattern, notes, chosen) = {
                let state = state.borrow();
                let Some(pattern) = state.pattern.clone() else { return };
                let notes: Vec<(String, Vec<ReplaceMatch>)> = state.notes.iter().map(|n| (n.path.clone(), n.matches.clone())).collect();
                (pattern, notes, state.chosen.clone())
            };
            let (mut written, mut replaced, mut refused) = (0, 0, Vec::new());
            for (path, matches) in &notes {
                let ids: BTreeSet<usize> = matches.iter().map(|m| m.id()).filter(|id| chosen.contains(&(path.clone(), *id))).collect();
                if ids.is_empty() {
                    continue;
                }
                match store.replace(path, &pattern, matches, &ids) {
                    Ok(()) => {
                        written += 1;
                        replaced += ids.len();
                    }
                    Err(_) => refused.push(Vault::title(path)),
                }
            }
            let mut text = format!(
                "{replaced} {} replaced in {written} {}.",
                if replaced == 1 { "match" } else { "matches" },
                if written == 1 { "note" } else { "notes" }
            );
            if !refused.is_empty() {
                text.push_str(&format!(" {} changed since the search and were left alone.", refused.join(", ")));
            }
            done(text);
            search();
        });
    }
    if !find.is_empty() {
        search();
    }
    let focus = find_row.clone();
    glib::idle_add_local_once(move || {
        focus.grab_focus();
    });
    dialog.present(Some(parent));
}
