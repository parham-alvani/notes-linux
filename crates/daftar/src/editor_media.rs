//! What the editor shows and takes beside the text: the pictures a line
//! embeds, drawn under it, and files dropped on the text.
//!
//! A picture is an overlay of the text view, not a character of the buffer,
//! so the file stays the markdown: the line it belongs to gets a tag whose
//! space below the line is the picture's height, and the picture is put in
//! that space. Dropping a file of the vault, from the sidebar or a file
//! manager, puts a link to it where it lands, as on the Mac; a file from
//! outside is copied into the attachment folder first.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use daftar_core::{EditorCommands, NoteParser, VaultMoves};

use super::{Editor, attachment_folder};
use crate::store::file_in;

/// The files a picture is drawn for.
const PICTURES: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];
/// The tallest a picture under a line is drawn.
const TALLEST: i32 = 420;
/// The space between pictures, and above and below them.
const GAP: i32 = 8;

/// Per line, the picture files under it and the size each is drawn at.
type Layout = BTreeMap<usize, Vec<(PathBuf, i32, i32)>>;

/// The pictures of the note, by the line they are under.
#[derive(Default)]
pub(super) struct Pictures {
    /// What is drawn now: per line, the files and their sizes.
    shown: RefCell<Layout>,
    widgets: RefCell<Vec<(usize, gtk::Picture, gtk::TextTag)>>,
    /// Pictures put on the view and hidden, for the next ones: the view does
    /// not give an overlay child back once it has it.
    spare: RefCell<Vec<gtk::Picture>>,
    /// Loaded pictures, until their file changes.
    cache: RefCell<HashMap<PathBuf, (SystemTime, gdk::Texture)>>,
    place_soon: Cell<bool>,
}

/// The lines of [text] that embed a picture, with the targets as written.
pub fn picture_lines(text: &str) -> BTreeMap<usize, Vec<String>> {
    let mut lines: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for link in NoteParser::parse(text).links.into_iter().filter(|l| l.embed && is_picture(&l.target)) {
        lines.entry(link.line).or_default().push(link.target);
    }
    lines
}

fn is_picture(target: &str) -> bool {
    target.rsplit_once('.').is_some_and(|(_, extension)| PICTURES.contains(&extension.to_lowercase().as_str()))
}

/// The size a picture of [width] by [height] is drawn at in [room] pixels
/// across: never wider than it is, nor than the room, nor taller than
/// [TALLEST].
pub fn fit(width: i32, height: i32, room: i32) -> (i32, i32) {
    if width <= 0 || height <= 0 {
        return (0, 0);
    }
    let scale = (room.max(1) as f64 / width as f64).min(TALLEST as f64 / height as f64).min(1.0);
    (((width as f64 * scale).round() as i32).max(1), ((height as f64 * scale).round() as i32).max(1))
}

/// The vault path of [file] when it is in the vault at [root].
pub fn vault_path(root: &Path, file: &Path) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let file = file.canonicalize().ok()?;
    let relative = file.strip_prefix(&root).ok()?;
    let path = relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    (!path.is_empty() && file.is_file()).then_some(path)
}

/// Copies [file] into the attachment folder of the vault at [root], under a
/// name no file in [files] has; its vault path.
pub fn import(root: &Path, file: &Path, files: &BTreeSet<String>) -> Result<String, String> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("no file name")?;
    let (base, extension) = match name.rsplit_once('.') {
        Some((base, extension)) if !base.is_empty() => (base.to_string(), extension.to_string()),
        _ => (name.clone(), String::new()),
    };
    let path = VaultMoves::free_name(&base, &extension, &attachment_folder(root), files);
    let target = file_in(&path, root).ok_or_else(|| format!("{name} has a name the vault cannot hold"))?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(file, &target).map_err(|e| format!("{name} could not be copied into the vault: {e}"))?;
    Ok(path)
}

impl Editor {
    /// Files dropped on the text become links where they land.
    pub(super) fn connect_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        // Before the text view's own, which would put the files' names in.
        target.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        target.connect_drop(move |_, value, x, y| {
            let Some(this) = this.upgrade() else { return false };
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            this.drop_files(&list.files(), x, y)
        });
        self.view.add_controller(target);
    }

    fn drop_files(&self, dropped: &[gio::File], x: f64, y: f64) -> bool {
        let Some(store) = self.store.borrow().clone() else { return false };
        if self.open.borrow().is_none() {
            return false;
        }
        let mut files = store.index().files.clone();
        let mut paths = Vec::new();
        let mut imported = false;
        for file in dropped.iter().filter_map(|f| f.path()) {
            if let Some(path) = vault_path(&store.root, &file).filter(|p| file_in(p, &store.root).is_some()) {
                paths.push(path);
                continue;
            }
            match import(&store.root, &file, &files) {
                Ok(path) => {
                    files.insert(path.clone());
                    paths.push(path);
                    imported = true;
                }
                Err(error) => (self.notice)(&error),
            }
        }
        if paths.is_empty() {
            return false;
        }
        if imported {
            store.wrote();
        }
        let links = paths.iter().map(|p| EditorCommands::link(p, &files)).collect::<Vec<_>>().join(" ");
        if let Some(vim) = self.vim.borrow().as_ref() {
            vim.session.paste(&links);
            return true;
        }
        let (bx, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        // Past the end of a line, the end of that line.
        let at = self.view.iter_at_location(bx, by).unwrap_or_else(|| {
            let (mut line, _) = self.view.line_at_y(by);
            if !line.ends_line() {
                line.forward_to_line_end();
            }
            line
        });
        let at = at.offset() as usize;
        let end = at + links.chars().count();
        self.replace(at..at, &links, Some(end..end));
        self.view.grab_focus();
        true
    }

    /// The pictures follow the text and the width: drawn again when what a
    /// line embeds or the room for it changed, else only moved.
    pub(super) fn connect_pictures(self: &Rc<Self>) {
        // The width changes while the view lays itself out, when tags must
        // not change: the pictures follow it afterwards.
        if let Some(h) = self.view.hadjustment() {
            let this = Rc::downgrade(self);
            h.connect_page_size_notify(move |_| {
                let this = this.clone();
                glib::idle_add_local_once(move || {
                    if let Some(this) = this.upgrade() {
                        this.refresh_pictures();
                    }
                });
            });
        }
        // The heights of the lines are known a little at a time.
        if let Some(v) = self.view.vadjustment() {
            let this = Rc::downgrade(self);
            v.connect_changed(move |_| {
                if let Some(this) = this.upgrade() {
                    this.schedule_place();
                }
            });
        }
    }

    pub(super) fn refresh_pictures(self: &Rc<Self>) {
        let wanted = self.wanted_pictures();
        if *self.pictures.shown.borrow() == wanted {
            self.schedule_place();
            return;
        }
        self.put_away_pictures();
        let table = self.buffer.tag_table();
        let gap = self.view.pixels_below_lines();
        let mut widgets = Vec::new();
        for (&line, files) in &wanted {
            let Some(start) = self.buffer.iter_at_line(line as i32) else { continue };
            let mut end = start;
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            let height: i32 = files.iter().map(|(_, _, h)| h + GAP).sum();
            let tag = gtk::TextTag::builder().pixels_below_lines(gap + height + GAP).build();
            table.add(&tag);
            self.buffer.apply_tag(&tag, &start, &end);
            for (file, width, height) in files {
                let Some(texture) = self.texture(file) else { continue };
                let spare = self.pictures.spare.borrow_mut().pop();
                let picture = spare.unwrap_or_else(|| {
                    let picture = gtk::Picture::builder().can_shrink(true).content_fit(gtk::ContentFit::Contain).build();
                    picture.add_css_class("daftar-editor-picture");
                    self.view.add_overlay(&picture, 0, 0);
                    picture
                });
                picture.set_paintable(Some(&texture));
                picture.set_size_request(*width, *height);
                picture.set_visible(true);
                widgets.push((line, picture, tag.clone()));
            }
        }
        *self.pictures.widgets.borrow_mut() = widgets;
        *self.pictures.shown.borrow_mut() = wanted;
        self.schedule_place();
    }

    /// Places the pictures once the view has laid the lines out again, which
    /// it does at a higher priority than this.
    fn schedule_place(self: &Rc<Self>) {
        if self.pictures.place_soon.replace(true) {
            return;
        }
        let this = Rc::downgrade(self);
        glib::idle_add_local_full(glib::Priority::LOW, move || {
            if let Some(this) = this.upgrade() {
                this.pictures.place_soon.set(false);
                this.place_pictures();
            }
            glib::ControlFlow::Break
        });
    }

    /// Each picture in the space under its line.
    fn place_pictures(&self) {
        let widgets = self.pictures.widgets.borrow();
        let shown = self.pictures.shown.borrow();
        let mut below: HashMap<usize, i32> = HashMap::new();
        for (line, picture, tag) in widgets.iter() {
            let Some(iter) = self.buffer.iter_at_line(*line as i32) else { continue };
            // The bottom of the line's last row, where the space under it starts.
            let (y, height) = self.view.line_yrange(&iter);
            let top = y + height - tag.pixels_below_lines();
            let offset = below.entry(*line).or_insert(GAP);
            let index = widgets.iter().filter(|(l, _, _)| l == line).position(|(_, p, _)| p == picture).unwrap_or(0);
            let picture_height = shown.get(line).and_then(|files| files.get(index)).map_or(0, |(_, _, h)| *h);
            self.view.move_overlay(picture, self.view.left_margin(), top + *offset);
            *offset += picture_height + GAP;
        }
        // A child that moved leaves its old place drawn otherwise.
        self.view.queue_draw();
    }

    /// What should be drawn now: per line, each picture it embeds that is a
    /// file of the vault, at the size it fits the room in.
    fn wanted_pictures(&self) -> Layout {
        let Some(store) = self.store.borrow().clone() else { return BTreeMap::new() };
        let Some(source) = self.open.borrow().as_ref().map(|open| open.path.clone()) else { return BTreeMap::new() };
        let room = self.view.hadjustment().map_or(0.0, |h| h.page_size()) as i32 - self.view.left_margin() - self.view.right_margin();
        let room = if room > 40 { room } else { 600 };
        let index = store.index();
        let mut wanted = BTreeMap::new();
        for (line, targets) in picture_lines(&self.text()) {
            let files: Vec<(PathBuf, i32, i32)> = targets
                .iter()
                .filter_map(|target| index.resolve(target, &source))
                .filter_map(|path| file_in(&path, &store.root))
                .filter_map(|file| {
                    let texture = self.texture(&file)?;
                    let (width, height) = fit(texture.width(), texture.height(), room);
                    (width > 0).then_some((file, width, height))
                })
                .collect();
            if !files.is_empty() {
                wanted.insert(line, files);
            }
        }
        wanted
    }

    /// The picture in [file], loaded once until the file changes.
    fn texture(&self, file: &Path) -> Option<gdk::Texture> {
        let changed = fs::metadata(file).and_then(|m| m.modified()).ok()?;
        if let Some((when, texture)) = self.pictures.cache.borrow().get(file)
            && *when == changed
        {
            return Some(texture.clone());
        }
        let texture = gdk::Texture::from_filename(file).ok()?;
        self.pictures.cache.borrow_mut().insert(file.to_path_buf(), (changed, texture.clone()));
        Some(texture)
    }

    /// Every picture hidden and spare, and the space under the lines gone.
    fn put_away_pictures(&self) {
        let table = self.buffer.tag_table();
        let mut spare = self.pictures.spare.borrow_mut();
        // The pictures of a line share its tag.
        let mut tags: Vec<gtk::TextTag> = Vec::new();
        for (_, picture, tag) in self.pictures.widgets.borrow_mut().drain(..) {
            picture.set_visible(false);
            picture.set_paintable(gdk::Paintable::NONE);
            spare.push(picture);
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        for tag in tags {
            table.remove(&tag);
        }
    }

    /// No pictures, for a note that is closed.
    pub(super) fn clear_pictures(&self) {
        self.put_away_pictures();
        self.pictures.shown.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_with_pictures_and_only_pictures() {
        let text = "# Trip\n![[beach.png]] and ![[map.JPG|300]]\n![[Notes/Plan]]\n```\n![[code.png]]\n```\n![sky](uploads/sky%20blue.webp)\n![[doc.pdf]]";
        let lines = picture_lines(text);
        assert_eq!(lines.keys().copied().collect::<Vec<_>>(), [1, 6]);
        assert_eq!(lines[&1], ["beach.png", "map.JPG"]);
        assert_eq!(lines[&6], ["uploads/sky blue.webp"]);
    }

    #[test]
    fn a_picture_fits_the_room_and_is_never_blown_up() {
        assert_eq!(fit(200, 100, 600), (200, 100));
        assert_eq!(fit(1200, 600, 600), (600, 300));
        assert_eq!(fit(400, 1600, 600), (105, 420));
        assert_eq!(fit(0, 10, 600), (0, 0));
    }

    #[test]
    fn a_dropped_file_from_outside_is_copied_to_the_attachments() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::write(root.path().join(".obsidian/app.json"), r#"{"attachmentFolderPath":"uploads"}"#).unwrap();
        fs::create_dir_all(root.path().join("Notes")).unwrap();
        fs::write(root.path().join("Notes/Plan.md"), "# Plan").unwrap();
        let outside = tempfile::tempdir().unwrap();
        let photo = outside.path().join("photo.png");
        fs::write(&photo, b"png").unwrap();

        assert_eq!(vault_path(root.path(), &root.path().join("Notes/Plan.md")).as_deref(), Some("Notes/Plan.md"));
        assert_eq!(vault_path(root.path(), &photo), None);
        assert_eq!(vault_path(root.path(), &root.path().join("Notes")), None);

        let files: BTreeSet<String> = ["uploads/photo.png".to_string()].into();
        let path = import(root.path(), &photo, &files).unwrap();
        assert_eq!(path, "uploads/photo 1.png");
        assert_eq!(fs::read(root.path().join(&path)).unwrap(), b"png");
    }
}
