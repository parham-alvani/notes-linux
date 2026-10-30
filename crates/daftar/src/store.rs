//! The open vault: its folder, its index and its search, and the one way the
//! app writes a file.

use std::cell::{Ref, RefCell};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use daftar_core::{
    BoardDrop, Bookmarks, CalendarDay, CalendarWeek, Clock, DailyNotes, Day, ExcludedFiles, Failure, Iconic, PeriodicNotes, ReplaceMatch,
    ReplacePattern, SearchIndex, Sections, TagRename, TaskFields, TaskSorter, Templates, Vault, VaultEdits, VaultIndex, VaultReplace,
    VaultTask,
};

use crate::places::PlaceBook;

pub struct VaultStore {
    pub root: PathBuf,
    index: RefCell<VaultIndex>,
    pub search: Arc<SearchIndex>,
    /// Told after the app wrote a note, for the sync that follows an edit.
    wrote: RefCell<Option<Box<dyn Fn()>>>,
    /// Obsidian's `.obsidian/bookmarks.json`.
    pub bookmarks: RefCell<Bookmarks>,
    /// The Iconic plugin's icons and colors.
    pub iconic: RefCell<Iconic>,
    /// Where each note was being read, and when.
    pub places: Rc<PlaceBook>,
    /// The Daily notes plugin's folder and format, as periods.
    pub periodic: PeriodicNotes,
    /// The Templates plugin's folder and formats.
    pub templates: Templates,
    /// What the lists leave out: Obsidian's excluded files and the files for
    /// AI agents (`files.hidden`), or nothing while hidden files are shown.
    excluded: RefCell<ExcludedFiles>,
}

impl VaultStore {
    pub fn new(root: PathBuf, index: VaultIndex, search: Arc<SearchIndex>) -> VaultStore {
        let (bookmarks, iconic, places) = (Bookmarks::load(&root), Iconic::load(&root), PlaceBook::load(&root));
        let (periodic, templates) = (PeriodicNotes::new(DailyNotes::read(&root)), Templates::read(&root));
        VaultStore {
            root,
            index: RefCell::new(index),
            search,
            wrote: RefCell::new(None),
            bookmarks: RefCell::new(bookmarks),
            iconic: RefCell::new(iconic),
            places,
            periodic,
            templates,
            excluded: RefCell::new(ExcludedFiles::default()),
        }
    }

    /// The files the lists leave out: Obsidian's excluded files and those
    /// named [names] in any folder; none when [show] is on.
    pub fn set_hidden(&self, names: &[String], show: bool) {
        *self.excluded.borrow_mut() = if show { ExcludedFiles::default() } else { ExcludedFiles::load_with_names(&self.root, names) };
    }

    /// Whether the lists leave [path] out. A link to it still opens it.
    pub fn is_hidden(&self, path: &str) -> bool {
        self.excluded.borrow().contains(path)
    }

    pub fn excluded(&self) -> Ref<'_, ExcludedFiles> {
        self.excluded.borrow()
    }

    /// The file tree without the hidden files.
    pub fn tree(&self) -> Vec<daftar_core::TreeNode> {
        let index = self.index();
        let excluded = self.excluded.borrow();
        if excluded.is_empty() {
            return index.tree().to_vec();
        }
        excluded.tree(&index.files, &index.empty_folders)
    }

    /// The tags as the sidebar shows them, without the hidden notes'.
    pub fn tag_tree(&self) -> Vec<daftar_core::TagNode> {
        self.excluded.borrow().tag_tree(&self.index())
    }

    /// The open tasks' notes' tasks, without the hidden notes', for Today and the board.
    pub fn listed_tasks(&self) -> Vec<VaultTask> {
        let excluded = self.excluded.borrow();
        self.index().tasks().into_iter().filter(|task| !excluded.contains(&task.path)).collect()
    }

    /// The periodic note of the period [day] falls in, made from the Daily
    /// notes plugin's template when it is not there yet; its vault path.
    pub fn periodic_note(&self, day: Day) -> Result<String, String> {
        let path = self.periodic.path_for(day);
        if self.index().files.contains(&path) {
            return Ok(path);
        }
        let template = &self.periodic.config.template;
        let source = if template.is_empty() {
            String::new()
        } else {
            self.text(&if Vault::is_note(template) { template.clone() } else { format!("{template}.md") }).unwrap_or_default()
        };
        let name = Vault::title(&path);
        let body = self.templates.fill(&source, &name, day, Clock::now());
        let made = crate::files::create_note(&self.root, &self.index(), &name, crate::files::parent(&path), &body);
        let made = made.map_err(|e| e.to_string())?;
        self.refresh(std::slice::from_ref(&made));
        self.wrote();
        Ok(made)
    }

    /// [day]'s scratch note in [folder], made empty when it is not there yet.
    pub fn scratchpad(&self, day: Day, folder: &str) -> Result<String, String> {
        let folder = folder.trim().trim_matches('/');
        let name = day.to_string();
        let path = if folder.is_empty() { format!("{name}.md") } else { format!("{folder}/{name}.md") };
        if self.index().files.contains(&path) {
            return Ok(path);
        }
        let made = crate::files::create_note(&self.root, &self.index(), &name, folder, "").map_err(|e| e.to_string())?;
        self.refresh(std::slice::from_ref(&made));
        self.wrote();
        Ok(made)
    }

    /// A note for the link [target] in [source] that goes nowhere yet: at the
    /// path the link names, else beside [source]; its vault path.
    pub fn create_note_for_link(&self, target: &str, source: &str) -> Result<String, String> {
        let clean = target.split(['#', '|']).next().unwrap_or("").trim();
        let mut parts: Vec<&str> = clean.split('/').filter(|p| !p.is_empty()).collect();
        let last = parts.pop().ok_or_else(|| format!("“{target}” is not a name for a note."))?;
        let name = last.strip_suffix(".md").unwrap_or(last);
        let folder = if parts.is_empty() { crate::files::parent(source).to_string() } else { parts.join("/") };
        let made = crate::files::create_note(&self.root, &self.index(), name, &folder, "").map_err(|e| e.to_string())?;
        self.refresh(std::slice::from_ref(&made));
        self.wrote();
        Ok(made)
    }

    /// The template notes of the vault, for New Note.
    pub fn template_notes(&self) -> Vec<String> {
        self.templates.templates(&self.index().files)
    }

    /// [template]'s text filled for a note named [name], made now.
    pub fn filled(&self, template: &str, name: &str) -> String {
        self.text(template).map(|text| self.templates.fill(&text, name, Day::today(), Clock::now())).unwrap_or_default()
    }

    /// The notes read before that changed on disk since, by a pull or
    /// another app: the newest change first, with when, seconds since 1970.
    pub fn updated_since_read(&self) -> Vec<(String, i64)> {
        let index = self.index();
        let mut updated: Vec<(i64, String)> = index
            .notes
            .values()
            .filter_map(|note| {
                if self.is_hidden(&note.path) {
                    return None;
                }
                let read = self.places.last_read(&note.path)?;
                let modified = note.modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
                // Two seconds, as on the Mac: a save the app made a moment after a read is not news.
                (modified - read > 2).then(|| (modified, note.path.clone()))
            })
            .collect();
        updated.sort_by(|a, b| b.cmp(a));
        updated.into_iter().map(|(modified, path)| (path, modified)).collect()
    }

    /// The tasks of [path] sorted as the vault's Task Sorter plugin does:
    /// done and cancelled first in each section. False when none moved.
    pub fn sort_tasks(&self, path: &str) -> Result<bool, Failure> {
        let mut sorted = false;
        self.write(path, |text| {
            Ok(match TaskSorter::sort(text) {
                Some(result) => {
                    sorted = true;
                    result
                }
                None => text.to_string(),
            })
        })?;
        Ok(sorted)
    }

    /// The headings right under heading [index] of [path], or under the
    /// heading of the section [line] is in, in the order of their names.
    /// `Ok(Some(why))` when nothing changed.
    pub fn sort_headings(&self, path: &str, index: Option<usize>, line: usize) -> Result<Option<String>, Failure> {
        let mut why = None;
        self.write(path, |text| {
            Ok(Sections::sort_headings(text, index, line).unwrap_or_else(|reason| {
                why = Some(reason);
                text.to_string()
            }))
        })?;
        Ok(why)
    }

    /// The section of heading [index] of [path] at place [position] among
    /// the headings of its level, in the file. False when it did not move.
    pub fn move_section(&self, path: &str, index: usize, position: usize) -> Result<bool, Failure> {
        let mut moved = false;
        self.write(path, |text| {
            Ok(match Sections::move_section(text, index, position) {
                Some(result) => {
                    moved = true;
                    result.text
                }
                None => text.to_string(),
            })
        })?;
        Ok(moved)
    }

    /// The notes that have the tag [tag] or one under it.
    pub fn notes_tagged(&self, tag: &str) -> Vec<String> {
        let mut paths: Vec<String> =
            self.index().notes.values().filter(|n| TagRename::has(&n.parsed.tags, tag)).map(|n| n.path.clone()).collect();
        paths.sort();
        paths
    }

    /// Renames the tag [old], and those under it, in every note that has it,
    /// several notes at a time. The notes written, and those that failed.
    pub fn rename_tag(&self, old: &str, new: &str) -> (Vec<String>, Vec<String>) {
        if !TagRename::is_valid(old) || !TagRename::is_valid(new) {
            return (vec![], vec![]);
        }
        let paths: Vec<String> = self.notes_tagged(old).into_iter().filter(|p| file_in(p, &self.root).is_some()).collect();
        let (mut written, mut failed) = (Vec::new(), Vec::new());
        for (path, outcome) in paths.iter().zip(VaultEdits::apply_all(&paths, &self.root, |text| TagRename::rename(text, old, new))) {
            match outcome {
                Ok(true) => written.push(path.clone()),
                Ok(false) => {}
                Err(_) => failed.push(path.clone()),
            }
            self.places.mark_read(path);
        }
        if !written.is_empty() {
            self.refresh(&written);
            self.wrote();
        }
        (written, failed)
    }

    /// Replaces the [chosen] matches of [note], read again and refused when
    /// its matches are not the [shown] ones.
    pub fn replace(&self, path: &str, pattern: &ReplacePattern, shown: &[ReplaceMatch], chosen: &BTreeSet<usize>) -> Result<(), Failure> {
        self.write(path, |text| VaultReplace::replace(text, pattern, shown, chosen).map_err(|_| Failure::TextMoved))
    }

    pub fn connect_wrote(&self, wrote: impl Fn() + 'static) {
        *self.wrote.borrow_mut() = Some(Box::new(wrote));
    }

    pub fn index(&self) -> Ref<'_, VaultIndex> {
        self.index.borrow()
    }

    /// Tells the sync that the app wrote to the vault.
    pub fn wrote(&self) {
        if let Some(wrote) = self.wrote.borrow().as_ref() {
            wrote();
        }
    }

    /// The text of the file at [path], when it is in the vault.
    pub fn text(&self, path: &str) -> Option<String> {
        fs::read(file_in(path, &self.root)?).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Ticks or opens a task. Only a note in the index is written, and only
    /// when its line still holds the task the page showed.
    pub fn toggle_task(&self, path: &str, line: usize, status: char) -> Result<(), Failure> {
        if !self.index().notes.contains_key(path) {
            return Err(Failure::Unreadable);
        }
        VaultEdits::apply(path, &self.root, |text| VaultEdits::toggle_task(text, line, status, Day::today()))?;
        self.refresh(&[path.to_string()]);
        if self.places.last_read(path).is_some() {
            self.places.mark_read(path);
        }
        self.wrote();
        Ok(())
    }

    /// A task dropped on a board column. The card's task must still be on its
    /// line, with the same status and words, or nothing is written.
    pub fn move_task(&self, task: &VaultTask, drop: &BoardDrop) -> Result<(), Failure> {
        self.write(&task.path, |text| VaultEdits::move_task(text, task.task.line, &task.task, drop, Day::today()))
    }

    /// The Edit Task dialog's save: refused when the line is no longer [before].
    pub fn edit_task(&self, path: &str, line: usize, before: &TaskFields, fields: &TaskFields) -> Result<(), Failure> {
        self.write(path, |text| VaultEdits::edit_task(text, line, before, fields))
    }

    /// One edit of a note in the index, through `VaultEdits::apply`; the index
    /// reads the note again either way, so a refused edit shows what is there.
    fn write(&self, path: &str, edit: impl FnOnce(&str) -> Result<String, Failure>) -> Result<(), Failure> {
        if !self.index().notes.contains_key(path) || file_in(path, &self.root).is_none() {
            return Err(Failure::Unreadable);
        }
        let result = VaultEdits::apply(path, &self.root, edit);
        self.refresh(&[path.to_string()]);
        // The app's own change does not make the note "updated since you read".
        if result.is_ok() && self.places.last_read(path).is_some() {
            self.places.mark_read(path);
        }
        if result.is_ok()
            && let Some(wrote) = self.wrote.borrow().as_ref()
        {
            wrote();
        }
        result
    }

    /// The days from [start], [count] of them, in the phone's week notes.
    pub fn calendar_days(&self, start: Day, count: i64) -> Vec<CalendarDay> {
        let mut paths: Vec<String> = Vec::new();
        for offset in 0..count {
            let path = CalendarWeek::path(start.adding_days(offset));
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let end = start.adding_days(count);
        paths
            .iter()
            .filter(|path| self.index().files.contains(*path))
            .filter_map(|path| self.text(path).map(|text| CalendarWeek::parse(&text, path)))
            .flatten()
            .filter(|day| day.day >= start && day.day < end)
            .collect()
    }

    /// Reads the files at [paths] again.
    pub fn refresh(&self, paths: &[String]) {
        let changed: BTreeSet<String> = paths.iter().cloned().collect();
        Vault::update_in_place(&mut self.index.borrow_mut(), &changed, &self.root, Some(&self.search));
    }
}

/// [path] inside [root], or `None` for an absolute path, a `.` or `..` part,
/// or a symlink that points out of the vault.
pub fn file_in(path: &str, root: &Path) -> Option<PathBuf> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    let base = root.canonicalize().ok()?;
    let file = base.join(path);
    if file.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    // A file that is not there yet: its nearest folder that is.
    let mut existing = file.as_path();
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let resolved = existing.canonicalize().ok()?;
    resolved.starts_with(&base).then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_cannot_leave_the_vault() {
        let outside = std::env::temp_dir().join(format!("daftar-outside-{}", std::process::id()));
        let root = outside.join("vault");
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(outside.join("secret.md"), "no").unwrap();
        fs::write(root.join("Notes/a.md"), "yes").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Notes/out")).unwrap();
        assert!(file_in("Notes/a.md", &root).is_some());
        assert!(file_in("Notes/new.md", &root).is_some());
        assert!(file_in("../secret.md", &root).is_none());
        assert!(file_in("Notes/../../secret.md", &root).is_none());
        assert!(file_in("/etc/passwd", &root).is_none());
        assert!(file_in("Notes/out/secret.md", &root).is_none());
        fs::remove_dir_all(&outside).unwrap();
    }
}
