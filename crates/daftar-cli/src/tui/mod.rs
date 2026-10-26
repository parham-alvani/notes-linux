//! `daftar-cli tui`: the vault in a terminal, in Daftar's colors. The tree,
//! the note, the quick switcher, search, tasks, backlinks and tags, with
//! the same rules as the app, from `daftar-core`.
//!
//! `app.rs` keeps the state and is tested without a terminal; `render.rs`
//! draws it; `keys.rs` maps keys to it; `markdown.rs` turns a note into
//! rows; `theme.rs` has the colors of `web/src/reader.css`.

mod app;
mod keys;
mod markdown;
mod render;
mod theme;

use std::path::{Path, PathBuf};
use std::process::Command;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use daftar_core::ConfigFile;

pub use app::App;
use app::Job;
pub use theme::Theme;

/// Runs the TUI on [root] until `q`.
pub fn run(root: PathBuf) -> Result<(), String> {
    let config = ConfigFile::new(std::fs::read_to_string(ConfigFile::default_path()).unwrap_or_default());
    let theme = Theme::named(config.string("appearance.theme").as_deref());
    let mut app = App::new(root, theme);
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<(), String> {
    loop {
        terminal.draw(|frame| render::draw(frame, app)).map_err(|e| e.to_string())?;
        if app.quit {
            return Ok(());
        }
        // The frame above says what is about to happen; now it happens.
        if let Some(job) = app.job.take() {
            match job {
                Job::Sync => app.sync(),
                Job::Edit(path, line) => {
                    ratatui::restore();
                    let result = edit(&app.root.join(&path), line);
                    *terminal = ratatui::init();
                    terminal.clear().map_err(|e| e.to_string())?;
                    app.edited(&path, result);
                }
            }
            continue;
        }
        match event::read().map_err(|e| e.to_string())? {
            // Windows reports a release too; a key acts once.
            Event::Key(key) if key.kind == KeyEventKind::Press => keys::handle(app, key),
            _ => {}
        }
    }
}

/// The note in `$VISUAL` or `$EDITOR`, at [line] when the editor takes one.
fn edit(file: &Path, line: usize) -> Result<(), String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| if cfg!(windows) { "notepad".into() } else { "vi".into() });
    let mut words = editor.split_whitespace();
    let program = words.next().ok_or("$EDITOR is empty")?;
    let mut command = Command::new(program);
    command.args(words);
    let name = Path::new(program).file_stem().and_then(|n| n.to_str()).unwrap_or("");
    match name {
        "vi" | "vim" | "nvim" | "nano" | "emacs" | "kak" | "micro" => {
            command.arg(format!("+{}", line + 1)).arg(file);
        }
        "hx" | "helix" => {
            command.arg(format!("{}:{}", file.display(), line + 1));
        }
        _ => {
            command.arg(file);
        }
    }
    let status = command.status().map_err(|e| format!("{program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{program} exited with {status}")) }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::app::{Focus, Overlay, PickerKind};
    use super::*;

    /// A small vault, made up for the tests.
    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("Tasks")).unwrap();
        fs::create_dir_all(root.join("Projects/Kite")).unwrap();
        fs::write(
            root.join("Tasks/Work.md"),
            "# Work\n\n## Alpha\n\n- [ ] Ship the plan #work ⏳ 2026-10-20\n- [/] Half way through [[Home]]\n- [x] Old one ✅ 2026-10-01\n",
        )
        .unwrap();
        fs::write(root.join("Tasks/Tasks.md"), "# Tasks\n\n> [!todo] Now\n>\n> ```tasks\n> not done\n> group by filename\n> ```\n")
            .unwrap();
        fs::write(root.join("Home.md"), "---\ntags: [home]\n---\n# Home\n\nThe kitchen needs paint. See [[Work]].\n").unwrap();
        fs::write(root.join("Projects/Kite/Kite.md"), "# Kite\n\nA folder note about the kite.\n").unwrap();
        dir
    }

    fn app(dir: &tempfile::TempDir) -> App {
        App::new(dir.path().to_path_buf(), Theme::NAZ)
    }

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| render::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(app: &mut App, code: KeyCode) {
        keys::handle(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn the_tree_has_folders_first() {
        let dir = vault();
        let mut app = app(&dir);
        let names: Vec<&str> = app.tree.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Projects", "Tasks", "Home"]);
        let shown = screen(&mut app);
        assert!(shown.contains("▸ Projects"), "{shown}");
        assert!(shown.contains("Daftar"), "{shown}");
        // A folder opens, and its folder note with it.
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.note.as_ref().map(|n| n.path.as_str()), Some("Projects/Kite/Kite.md"));
    }

    #[test]
    fn a_note_shows_its_heading_tasks_and_links() {
        let dir = vault();
        let mut app = app(&dir);
        app.open("Tasks/Work.md", None, true);
        let shown = screen(&mut app);
        assert!(shown.contains("█ Work"), "{shown}");
        assert!(shown.contains("☐ Ship the plan #work ⏳"), "{shown}");
        assert!(shown.contains("◐ Half way through Home"), "{shown}");
        assert!(shown.contains("⎇") || shown.contains("no git"), "{shown}");
    }

    #[test]
    fn enter_follows_a_link_and_backspace_returns() {
        let dir = vault();
        let mut app = app(&dir);
        app.open("Tasks/Work.md", None, true);
        screen(&mut app);
        press(&mut app, KeyCode::Char(']'));
        press(&mut app, KeyCode::Char(']'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.note.as_ref().map(|n| n.path.as_str()), Some("Home.md"));
        let shown = screen(&mut app);
        assert!(shown.contains("#home"), "{shown}");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.note.as_ref().map(|n| n.path.as_str()), Some("Tasks/Work.md"));
    }

    #[test]
    fn space_ticks_a_task_and_refuses_a_changed_line() {
        let dir = vault();
        let mut app = app(&dir);
        app.open("Tasks/Work.md", None, true);
        screen(&mut app);
        press(&mut app, KeyCode::Char(']'));
        press(&mut app, KeyCode::Char(' '));
        let text = fs::read_to_string(dir.path().join("Tasks/Work.md")).unwrap();
        assert!(text.contains("- [x] Ship the plan #work ⏳ 2026-10-20 ✅"), "{text}");
        assert!(screen(&mut app).contains("☑ Ship the plan"));
        // The file changes under the view: the next tick is refused.
        fs::write(dir.path().join("Tasks/Work.md"), "# Work\n\n## Alpha\n\nNo tasks now.\n").unwrap();
        press(&mut app, KeyCode::Char(' '));
        assert!(app.message.as_ref().is_some_and(|(m, error)| *error && m.contains("not on that line")), "{:?}", app.message);
    }

    #[test]
    fn the_quick_switcher_ranks_and_opens() {
        let dir = vault();
        let mut app = app(&dir);
        press(&mut app, KeyCode::Char('p'));
        typed(&mut app, "wor");
        let Some(Overlay::Picker(picker)) = &app.overlay else { panic!("no switcher") };
        assert_eq!(picker.kind, PickerKind::Switcher);
        assert_eq!(picker.items[0].path.as_deref(), Some("Tasks/Work.md"));
        let shown = screen(&mut app);
        assert!(shown.contains("› wor"), "{shown}");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.note.as_ref().map(|n| n.path.as_str()), Some("Tasks/Work.md"));
        assert_eq!(app.focus, Focus::Note);
    }

    #[test]
    fn search_finds_text() {
        let dir = vault();
        let mut app = app(&dir);
        press(&mut app, KeyCode::Char('/'));
        typed(&mut app, "kitchen");
        let Some(Overlay::Picker(picker)) = &app.overlay else { panic!("no search") };
        assert_eq!(picker.items[0].path.as_deref(), Some("Home.md"));
        assert!(screen(&mut app).contains("kitchen"));
    }

    #[test]
    fn tasks_run_the_notes_queries_with_groups() {
        let dir = vault();
        let mut app = app(&dir);
        app.open("Tasks/Tasks.md", None, true);
        press(&mut app, KeyCode::Char('t'));
        let shown = screen(&mut app);
        assert!(shown.contains("Tasks · Tasks"), "{shown}");
        assert!(shown.contains("Query 1"), "{shown}");
        assert!(shown.contains("Work"), "{shown}");
        assert!(shown.contains("Ship the plan"), "{shown}");
        assert!(!shown.contains("Old one"), "{shown}");
        // Space ticks the task under the selection, in the list.
        press(&mut app, KeyCode::Char(' '));
        let text = fs::read_to_string(dir.path().join("Tasks/Work.md")).unwrap();
        assert!(text.contains("- [x] Ship the plan"), "{text}");
    }

    #[test]
    fn every_open_task_by_day_without_a_query() {
        let dir = vault();
        let mut app = app(&dir);
        press(&mut app, KeyCode::Char('t'));
        let shown = screen(&mut app);
        assert!(shown.contains("Open tasks"), "{shown}");
        assert!(shown.contains("Late") || shown.contains("Coming"), "{shown}");
        assert!(shown.contains("No date"), "{shown}");
    }

    #[test]
    fn backlinks_and_tags() {
        let dir = vault();
        let mut app = app(&dir);
        app.open("Home.md", None, true);
        press(&mut app, KeyCode::Char('b'));
        let shown = screen(&mut app);
        assert!(shown.contains("Links to Home"), "{shown}");
        assert!(shown.contains("Half way through Home"), "{shown}");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('#'));
        assert!(screen(&mut app).contains("#home"));
        press(&mut app, KeyCode::Enter);
        let Some(Overlay::Picker(picker)) = &app.overlay else { panic!("no tag list") };
        assert_eq!(picker.items.iter().filter_map(|i| i.path.as_deref()).collect::<Vec<_>>(), ["Home.md"]);
    }

    #[test]
    fn help_and_light_theme_draw() {
        let dir = vault();
        let mut app = App::new(dir.path().to_path_buf(), Theme::DAYLIGHT);
        press(&mut app, KeyCode::Char('?'));
        assert!(screen(&mut app).contains("Keys"));
        press(&mut app, KeyCode::Char('x'));
        assert!(app.overlay.is_none());
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    /// Draws every note of the vault in `$DAFTAR_TUI_VAULT`, and every list,
    /// reading only: `DAFTAR_TUI_VAULT=~/org cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_real_vault_draws() {
        let Some(root) = std::env::var_os("DAFTAR_TUI_VAULT") else { return };
        let mut app = App::new(root.into(), Theme::NAZ);
        let mut paths: Vec<String> = app.index.notes.keys().cloned().collect();
        paths.sort();
        for path in &paths {
            app.open(path, None, false);
            screen(&mut app);
            press(&mut app, KeyCode::Char('G'));
            screen(&mut app);
        }
        for key in ['t', 'b', '#'] {
            press(&mut app, KeyCode::Char(key));
            screen(&mut app);
            press(&mut app, KeyCode::Esc);
        }
        press(&mut app, KeyCode::Char('p'));
        typed(&mut app, "kub");
        screen(&mut app);
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('/'));
        typed(&mut app, "backup");
        screen(&mut app);
        eprintln!("drew {} notes", paths.len());
    }
}
