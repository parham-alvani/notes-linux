//! The Problems panel in the editor (`docs/specs/problems-and-format.md`
//! §5 and §7): a dot in the margin of each line with a finding, and Format
//! Note. The marks are GtkSourceView's line marks, drawn, never written: the
//! text, the cursor and undo do not change.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use daftar_core::{NoteFormat, Problem, ProblemLevel, Vault};
use gtk::{gdk_pixbuf, gio, glib};
use sourceview5::prelude::*;

use super::Editor;
use crate::vault_checks::VaultChecks;

const ERROR: &str = "daftar-error";
const WARNING: &str = "daftar-warning";

type Source = Box<dyn Fn(&str) -> BTreeMap<usize, Vec<Problem>>>;

/// Where the marks come from, and what each says on hover.
#[derive(Default)]
pub(super) struct Marks {
    source: RefCell<Option<Source>>,
    tips: Rc<RefCell<HashMap<String, String>>>,
}

/// A dot of [color], for the margin.
fn dot((red, green, blue): (u8, u8, u8)) -> gdk_pixbuf::Pixbuf {
    const SIZE: usize = 16;
    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    let (center, radius) = (7.5f64, 3.5f64);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = ((x as f64 - center).powi(2) + (y as f64 - center).powi(2)).sqrt();
            // A soft edge, a pixel wide.
            let alpha = (radius + 0.5 - distance).clamp(0.0, 1.0);
            let at = (y * SIZE + x) * 4;
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "from 0 to 255")]
            pixels[at..at + 4].copy_from_slice(&[red, green, blue, (alpha * 255.0).round() as u8]);
        }
    }
    gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(pixels),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        SIZE as i32,
        SIZE as i32,
        SIZE as i32 * 4,
    )
}

/// "Error: message (code)", one a line.
pub fn tooltip(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|p| format!("{}: {} ({})", if p.level == ProblemLevel::Error { "Error" } else { "Warning" }, p.message, p.code))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Editor {
    /// The margin for the marks: red for a line with an error, else yellow.
    pub(super) fn install_marks(&self) {
        self.view.set_show_line_marks(true);
        for (category, color, priority) in [(ERROR, (0xe0, 0x1b, 0x24), 2), (WARNING, (0xe5, 0xa5, 0x0a), 1)] {
            let attributes = sourceview5::MarkAttributes::new();
            attributes.set_pixbuf(&dot(color));
            let tips = self.marks.tips.clone();
            attributes.connect_query_tooltip_text(move |_, mark| {
                mark.name().and_then(|name| tips.borrow().get(name.as_str()).cloned()).unwrap_or_default()
            });
            self.view.set_mark_attributes(category, &attributes, priority);
        }
    }

    /// The cursor to [line] (from 0), in view, with the keys.
    pub fn show_line(&self, line: usize) {
        self.go_to_line(line);
    }

    /// [source] gives the findings of a note by source line.
    pub fn set_marks_source(&self, source: impl Fn(&str) -> BTreeMap<usize, Vec<Problem>> + 'static) {
        *self.marks.source.borrow_mut() = Some(Box::new(source));
        self.refresh_marks();
    }

    /// The marks of the note open, as the last check read it.
    pub fn refresh_marks(&self) {
        let (start, end) = self.buffer.bounds();
        for category in [ERROR, WARNING] {
            self.buffer.remove_source_marks(&start, &end, Some(category));
        }
        self.marks.tips.borrow_mut().clear();
        let Some(path) = self.path() else { return };
        let lines = match self.marks.source.borrow().as_ref() {
            Some(source) => source(&path),
            None => return,
        };
        for (line, problems) in lines {
            let Some(at) = self.buffer.iter_at_line(i32::try_from(line).unwrap_or(i32::MAX)) else { continue };
            let error = problems.iter().any(|p| p.level == ProblemLevel::Error);
            let name = format!("daftar-problem-{line}");
            self.buffer.create_source_mark(Some(&name), if error { ERROR } else { WARNING }, &at);
            self.marks.tips.borrow_mut().insert(name, tooltip(&problems));
        }
    }

    /// Edit › Format Note on the note in the editor: its text formatted with
    /// `daftar-cli fmt --stdin`, and only what differs replaced, as one step
    /// to undo; then the note is saved. In Neovim mode, Neovim writes first
    /// and takes the lines that differ.
    pub fn format_note(self: &Rc<Self>, checks: &Rc<VaultChecks>) {
        let Some(path) = self.path() else { return };
        if self.is_neovim() {
            self.format_in_neovim(path, checks.clone());
            return;
        }
        let text = self.text();
        let (this, checks) = (self.clone(), checks.clone());
        glib::spawn_future_local(async move {
            let Some(out) = checks.formatted_for_command(text.clone(), &path).await else { return };
            let title = Vault::title(&path);
            if out == text {
                (this.notice)(&format!("{title} is formatted already."));
                return;
            }
            if this.path().as_deref() != Some(path.as_str()) || this.is_neovim() || this.text() != text {
                (this.notice)(&format!("{title} changed while it was formatted. Nothing was changed."));
                return;
            }
            this.replace_changed(&text, &out);
            this.save();
        });
    }

    /// [old], on screen, becomes [new]: only the one range that differs is
    /// replaced, as one step to undo. The cursor stays on its letter, or goes
    /// to the change's end when it was inside it.
    fn replace_changed(&self, old: &str, new: &str) {
        let (a, b): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
        let range: Range<usize> = prefix..a.len() - suffix;
        let inserted: String = b[prefix..b.len() - suffix].iter().collect();
        let cursor = moved_cursor(self.cursor(), &range, b.len() - suffix - prefix);
        self.buffer.begin_user_action();
        let (mut start, mut end) = (self.iter(range.start), self.iter(range.end));
        self.buffer.delete(&mut start, &mut end);
        let mut at = self.iter(range.start);
        self.buffer.insert(&mut at, &inserted);
        self.buffer.end_user_action();
        self.buffer.place_cursor(&self.iter(cursor));
    }

    fn format_in_neovim(self: &Rc<Self>, path: String, checks: Rc<VaultChecks>) {
        let file = {
            let vim = self.vim.borrow();
            let Some(vim) = vim.as_ref() else { return };
            vim.session.save_and_wait(Duration::from_secs(2));
            self.open.borrow().as_ref().map(|open| open.file.clone())
        };
        let Some(text) = file.and_then(|file| fs::read_to_string(file).ok()) else { return };
        let this = self.clone();
        glib::spawn_future_local(async move {
            let Some(out) = checks.formatted_for_command(text.clone(), &path).await else { return };
            let title = Vault::title(&path);
            let Some(change) = NoteFormat::changed_lines(&text, &out) else {
                (this.notice)(&format!("{title} is formatted already."));
                return;
            };
            let expected = NoteFormat::lines(&text).join("\n");
            let answer = this.vim.borrow().as_ref().and_then(|vim| vim.session.replace_lines(&expected, change.lines, change.new));
            let replaced = match answer {
                Some(answer) => gio::spawn_blocking(move || answer.recv_timeout(Duration::from_secs(2)))
                    .await
                    .is_ok_and(|reply| reply.is_ok_and(|result| result.is_ok_and(|value| value.as_bool() == Some(true)))),
                None => false,
            };
            if !replaced {
                (this.notice)(&format!("{title} changed while it was formatted. Nothing was changed."));
            }
        });
    }
}

/// Where the cursor at [cursor] goes when [range] becomes [inserted]
/// characters: before it, it stays; after it, it moves with the text; inside
/// it, to the change's end.
fn moved_cursor(cursor: usize, range: &Range<usize>, inserted: usize) -> usize {
    if cursor <= range.start {
        cursor
    } else if cursor >= range.end {
        cursor + inserted - range.len()
    } else {
        range.start + inserted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use crate::vault_checks::tests::{Fake, NEW, idle, on_a_loop_of_its_own, wait};

    /// An editor on the stand-in's vault, in a window, with the template's settings.
    fn editor_on(fake: &Fake, path: &str, neovim: bool) -> (Rc<Editor>, gtk::Window) {
        gtk::init().unwrap();
        adw::init().unwrap();
        crate::style::install();
        let mut text = daftar_core::ConfigFile::TEMPLATE.to_string();
        if neovim {
            text = text.replace("vim_mode = false", "vim_mode = true").replace("nvim_config = \"user\"", "nvim_config = \"clean\"");
        }
        let settings = Rc::new(RefCell::new(AppSettings::of(fake.vault.path().join("config.toml"), &text)));
        let notices = fake.notices.clone();
        let editor = Editor::new(settings, move |text| notices.borrow_mut().push(text.to_string()), |_| {});
        let checks = fake.checks.clone();
        editor.set_marks_source(move |path| checks.lines(path));
        let window = gtk::Window::builder().default_width(900).default_height(500).child(&editor.widget).build();
        window.present();
        assert!(editor.load(fake.vault.path(), path, Some(0)));
        (editor, window)
    }

    /// Format Note in the editor, and in Neovim mode. Needs a display and
    /// `nvim`: run it under Broadway, `gtk4-broadwayd :7` and
    /// `GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 cargo test -p daftar --
    /// --ignored`. One test, as GTK keeps to the thread that started it.
    #[test]
    #[ignore = "needs a display and nvim"]
    fn format_note_in_the_editor() {
        on_a_loop_of_its_own(|context| {
            format_note_in_the_editor_is_one_undo_step(context);
            format_note_in_neovim_takes_the_lines_and_u_undoes_them(context);
        });
    }

    /// One step to undo, and "formatted already" the second time.
    fn format_note_in_the_editor_is_one_undo_step(context: &glib::MainContext) {
        {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            fs::write(fake.vault.path().join("Home.md"), "# Home\n\n*  item\ntext\n").unwrap();
            let (editor, window) = editor_on(&fake, "Home.md", false);
            // The cursor on the `t` of `text`, after the change.
            editor.buffer.place_cursor(&editor.iter(17));
            editor.format_note(&fake.checks);
            assert!(wait(context, || fake.file("Home.md") == "# Home\n\n- item\ntext\n"));
            assert_eq!(editor.text(), "# Home\n\n- item\ntext\n");
            assert_eq!(editor.cursor(), 16, "the cursor stays on its letter");
            assert!(editor.buffer.can_undo());
            editor.buffer.undo();
            assert_eq!(editor.text(), "# Home\n\n*  item\ntext\n", "one undo takes the whole format back");
            editor.buffer.redo();
            editor.format_note(&fake.checks);
            assert!(wait(context, || !fake.notices.borrow().is_empty()));
            assert_eq!(*fake.notices.borrow(), ["Home is formatted already."]);
            assert!(wait(context, || idle(&fake.checks)));
            window.destroy();
        }
    }

    #[test]
    fn the_cursor_stays_on_its_letter() {
        // `*  item` → `- item`: characters 4..7 (`*  `) become 2 (`- `).
        assert_eq!(moved_cursor(2, &(4..7), 2), 2);
        assert_eq!(moved_cursor(10, &(4..7), 2), 9);
        assert_eq!(moved_cursor(5, &(4..7), 2), 6);
    }

    #[test]
    fn the_margin_says_the_problems_of_a_line() {
        let problems = daftar_core::Problems::parse(concat!(
            r#"{"tool":"lint","level":"warning","path":"A.md","line":3,"code":"MD009","message":"spaces at the end"}"#,
            "\n",
            r#"{"tool":"check","level":"error","path":"A.md","line":3,"code":"link","message":"no note Plan"}"#,
        ));
        let lines = daftar_core::Problems::lines(&problems, "A.md");
        assert_eq!(tooltip(&lines[&2]), "Error: no note Plan (link)\nWarning: spaces at the end (MD009)");
    }

    /// In Neovim mode the buffer takes the formatted lines and `u` undoes
    /// them; leaving a note typed in formats it once Neovim wrote it.
    fn format_note_in_neovim_takes_the_lines_and_u_undoes_them(context: &glib::MainContext) {
        {
            let fake = Fake::new(NEW);
            fake.checks.opened(fake.vault.path());
            fs::write(fake.vault.path().join("Home.md"), "# Home\n\n*  item\ntext\n").unwrap();
            let (editor, window) = editor_on(&fake, "Home.md", true);
            assert!(wait(context, || editor.vim.borrow().as_ref().is_some_and(|vim| vim.mirror.is_some())), "Neovim started");
            editor.format_note(&fake.checks);
            assert!(wait(context, || fake.file("Home.md") == "# Home\n\n- item\ntext\n"));
            assert!(wait(context, || editor.text() == "# Home\n\n- item\ntext\n"));
            editor.vim.borrow().as_ref().unwrap().session.input("u");
            assert!(wait(context, || editor.text() == "# Home\n\n*  item\ntext\n"), "one u takes it back");
            // Typed in, then left: Neovim writes, and the file is formatted after.
            let checks = fake.checks.clone();
            editor.connect_left(move |path, loaded| checks.format_on_leave(path, loaded, daftar_core::FormatMoment::Closed));
            editor.vim.borrow().as_ref().unwrap().session.input("Go*  more<Esc>");
            assert!(wait(context, || editor.text().ends_with("*  more\n")));
            editor.close();
            assert!(wait(context, || fake.file("Home.md") == "# Home\n\n- item\ntext\n- more\n"), "{:?}", fake.file("Home.md"));
            assert!(wait(context, || idle(&fake.checks)));
            window.destroy();
        }
    }
}
