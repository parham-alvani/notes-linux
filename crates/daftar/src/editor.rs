//! Editing a note: the markdown in a GtkSourceView, colored with Daftar's
//! palette. The file is the record: what the editor loaded is kept, and a
//! save goes through only while the file on disk is still that text, so a
//! change made meanwhile by another app, Neovim or `git pull` is never
//! overwritten. A save runs a second after typing stops, on Ctrl+S, and when
//! the editor closes.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;

use crate::store::file_in;

/// How long typing must stop before the note is saved.
const AFTER_TYPING: Duration = Duration::from_secs(1);

/// What a save found.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    /// Nothing changed since the last save.
    Nothing,
    Written,
    /// The file on disk is not what the editor loaded; nothing was written.
    ChangedOnDisk,
    Failed(String),
}

/// The note open in the editor: its path, and the text the file had when it
/// was loaded or last saved.
struct Open {
    path: String,
    file: PathBuf,
    on_disk: String,
}

pub struct Editor {
    pub widget: gtk::ScrolledWindow,
    view: sourceview5::View,
    buffer: sourceview5::Buffer,
    open: RefCell<Option<Open>>,
    pending: RefCell<Option<glib::SourceId>>,
    notice: Box<dyn Fn(&str)>,
    conflict: Box<dyn Fn(&str)>,
}

impl Editor {
    /// [notice] says a line to the person; [conflict] says a save was refused
    /// because the file changed on disk, and may offer `reload`.
    pub fn new(notice: impl Fn(&str) + 'static, conflict: impl Fn(&str) + 'static) -> Rc<Editor> {
        let buffer = sourceview5::Buffer::new(None);
        if let Some(language) = sourceview5::LanguageManager::default().language("markdown") {
            buffer.set_language(Some(&language));
        }
        buffer.set_highlight_matching_brackets(false);
        let view = sourceview5::View::builder()
            .buffer(&buffer)
            .wrap_mode(gtk::WrapMode::WordChar)
            .auto_indent(true)
            .indent_width(2)
            .tab_width(4)
            .insert_spaces_instead_of_tabs(true)
            .smart_backspace(true)
            .top_margin(24)
            .bottom_margin(240)
            .left_margin(32)
            .right_margin(32)
            .pixels_below_lines(2)
            .css_classes(["daftar-editor"])
            .build();
        let clamp = adw::Clamp::builder().maximum_size(860).tightening_threshold(860).child(&view).build();
        let widget =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).css_classes(["daftar-page"]).build();
        let editor = Rc::new(Editor {
            widget,
            view,
            buffer,
            open: RefCell::new(None),
            pending: RefCell::new(None),
            notice: Box::new(notice),
            conflict: Box::new(conflict),
        });
        install_schemes(&editor.buffer);
        {
            let this = Rc::downgrade(&editor);
            editor.buffer.connect_changed(move |_| {
                if let Some(this) = this.upgrade() {
                    this.schedule_save();
                }
            });
        }
        editor
    }

    /// Loads the note at [path] from the vault at [root], with [line] in view.
    pub fn load(&self, root: &Path, path: &str, line: Option<usize>) -> bool {
        self.save();
        let Some(file) = file_in(path, root) else { return false };
        let Ok(text) = fs::read_to_string(&file) else { return false };
        self.cancel_pending();
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        *self.open.borrow_mut() = Some(Open { path: path.to_string(), file, on_disk: text });
        let mut at = self.buffer.iter_at_line(line.unwrap_or(0) as i32).unwrap_or_else(|| self.buffer.start_iter());
        self.buffer.place_cursor(&at);
        let view = self.view.clone();
        glib::idle_add_local_once(move || {
            view.scroll_to_iter(&mut at, 0.1, true, 0.0, 0.3);
            view.grab_focus();
        });
        true
    }

    /// The path of the note being edited.
    pub fn path(&self) -> Option<String> {
        self.open.borrow().as_ref().map(|open| open.path.clone())
    }

    /// The line the cursor is on, from 0, to show the same place when reading.
    pub fn line(&self) -> usize {
        self.buffer.iter_at_mark(&self.buffer.get_insert()).line().max(0) as usize
    }

    /// Writes the buffer when it changed, unless the file changed under it.
    pub fn save(&self) -> Saved {
        self.cancel_pending();
        let mut open = self.open.borrow_mut();
        let Some(note) = open.as_mut() else { return Saved::Nothing };
        if !self.buffer.is_modified() {
            return Saved::Nothing;
        }
        let text = self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), true).to_string();
        let saved = write_if_unchanged(&note.file, &note.on_disk, &text);
        match &saved {
            Saved::Written => {
                note.on_disk = text;
                self.buffer.set_modified(false);
            }
            Saved::ChangedOnDisk => (self.conflict)(&format!("{} changed on disk, so your edit was not saved.", note.path)),
            Saved::Failed(error) => (self.notice)(&format!("{} was not saved: {error}", note.path)),
            Saved::Nothing => {}
        }
        saved
    }

    /// Saves, then lets go of the note.
    pub fn close(&self) -> Saved {
        let saved = self.save();
        if saved != Saved::ChangedOnDisk {
            *self.open.borrow_mut() = None;
        }
        saved
    }

    /// The file changed on disk: an editor with nothing unsaved reads it again.
    pub fn changed_on_disk(&self) {
        if !self.buffer.is_modified() {
            self.reload();
        }
    }

    /// Reads the file again, where the cursor was, and drops what was not saved.
    pub fn reload(&self) {
        self.cancel_pending();
        let Some(file) = self.open.borrow().as_ref().map(|open| open.file.clone()) else { return };
        let Ok(text) = fs::read_to_string(&file) else { return };
        if self.open.borrow().as_ref().is_some_and(|open| open.on_disk == text) && !self.buffer.is_modified() {
            return;
        }
        let line = self.line();
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.on_disk = text;
        }
        if let Some(at) = self.buffer.iter_at_line(line as i32) {
            self.buffer.place_cursor(&at);
        }
    }

    fn schedule_save(self: &Rc<Self>) {
        if self.open.borrow().is_none() || !self.buffer.is_modified() {
            return;
        }
        self.cancel_pending();
        let this = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(AFTER_TYPING, move || {
            if let Some(this) = this.upgrade() {
                this.pending.take();
                this.save();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    fn cancel_pending(&self) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
    }
}

/// Writes [text] to [file] through a temporary file and a rename, only while
/// the file still holds [expected].
pub fn write_if_unchanged(file: &Path, expected: &str, text: &str) -> Saved {
    match fs::read_to_string(file) {
        Ok(current) if current != expected => return Saved::ChangedOnDisk,
        Ok(_) => {}
        Err(error) => return Saved::Failed(error.to_string()),
    }
    let Some(name) = file.file_name() else { return Saved::Failed("no file name".into()) };
    let temporary = file.with_file_name(format!(".{}.daftar-tmp", name.to_string_lossy()));
    if let Err(error) = fs::write(&temporary, text) {
        return Saved::Failed(error.to_string());
    }
    // The person's permissions stay the file's.
    if let Ok(metadata) = fs::metadata(file) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    match fs::rename(&temporary, file) {
        Ok(()) => Saved::Written,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Saved::Failed(error.to_string())
        }
    }
}

/// Daftar's two schemes, written where GtkSourceView looks, and the one for
/// the current scheme chosen now and whenever it changes.
fn install_schemes(buffer: &sourceview5::Buffer) {
    let folder = glib::user_cache_dir().join("daftar").join("styles");
    let _ = fs::create_dir_all(&folder);
    for (id, name, palette) in [("daftar-naz", "Daftar naz", &NAZ), ("daftar-daylight", "Daftar Daylight", &DAYLIGHT)] {
        let _ = fs::write(folder.join(format!("{id}.xml")), scheme(id, name, palette));
    }
    let manager = sourceview5::StyleSchemeManager::default();
    manager.append_search_path(&folder.to_string_lossy());
    manager.force_rescan();
    let style = adw::StyleManager::default();
    let apply = {
        let buffer = buffer.clone();
        move |dark: bool| {
            let id = if dark { "daftar-naz" } else { "daftar-daylight" };
            buffer.set_style_scheme(sourceview5::StyleSchemeManager::default().scheme(id).as_ref());
        }
    };
    apply(style.is_dark());
    style.connect_dark_notify(move |style| apply(style.is_dark()));
}

/// background, text, muted, accent, strong, em, link, code, rule, selection.
type Palette = [&'static str; 10];
const NAZ: Palette = ["#323232", "#f5f5f0", "#a8a8a0", "#ffb040", "#ffb070", "#b8ffd0", "#80e5ff", "#ffd850", "#4a4a4a", "#5a4a30"];
const DAYLIGHT: Palette = ["#faf9f5", "#2a2a28", "#6a6a62", "#a85200", "#a85200", "#0a7a48", "#00707a", "#8a6a00", "#d8d5cc", "#f0dcc0"];

/// A GtkSourceView style scheme in the reader's colors, for markdown.
fn scheme(id: &str, name: &str, p: &Palette) -> String {
    let [background, text, muted, accent, strong, em, link, code, rule, selection] = p;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="{id}" name="{name}" version="1.0">
  <style name="text" foreground="{text}" background="{background}"/>
  <style name="selection" background="{selection}"/>
  <style name="cursor" foreground="{accent}"/>
  <style name="current-line" background="{rule}"/>
  <style name="line-numbers" foreground="{muted}" background="{background}"/>
  <style name="def:heading" foreground="{accent}" bold="true" scale="large"/>
  <style name="def:strong-emphasis" foreground="{strong}" bold="true"/>
  <style name="def:emphasis" foreground="{em}" italic="true"/>
  <style name="def:inline-code" foreground="{code}"/>
  <style name="def:preformatted-section" foreground="{code}"/>
  <style name="def:link-text" foreground="{link}"/>
  <style name="def:link-destination" foreground="{muted}" underline="single"/>
  <style name="def:link-symbol" foreground="{muted}"/>
  <style name="def:list-marker" foreground="{accent}" bold="true"/>
  <style name="def:thematic-break" foreground="{muted}"/>
  <style name="def:shebang" foreground="{accent}"/>
  <style name="def:preprocessor" foreground="{link}"/>
  <style name="def:special-char" foreground="{muted}"/>
  <style name="def:comment" foreground="{muted}" italic="true"/>
  <style name="def:constant" foreground="{em}"/>
  <style name="def:note" foreground="{muted}"/>
</style-scheme>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_never_overwrites_a_change_on_disk() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("Note.md");
        fs::write(&file, "one\n").unwrap();
        assert_eq!(write_if_unchanged(&file, "one\n", "two\n"), Saved::Written);
        assert_eq!(fs::read_to_string(&file).unwrap(), "two\n");
        // Another app wrote meanwhile: the editor's text stays out.
        fs::write(&file, "three\n").unwrap();
        assert_eq!(write_if_unchanged(&file, "two\n", "mine\n"), Saved::ChangedOnDisk);
        assert_eq!(fs::read_to_string(&file).unwrap(), "three\n");
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_schemes_are_valid_xml_with_the_palette() {
        let text = scheme("daftar-naz", "Daftar naz", &NAZ);
        assert!(text.contains(r##"<style name="def:heading" foreground="#ffb040""##));
        assert_eq!(text.matches("<style ").count(), 21);
    }
}
