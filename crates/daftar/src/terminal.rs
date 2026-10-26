//! Ctrl+`: a terminal under the note, as on the Mac: the person's own login
//! shell, started in the vault's folder the first time the pane opens, in
//! naz on dark and Daylight on light. Hiding the pane keeps the shell.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use vte4::prelude::*;

/// The sixteen colors, dark then light, as the Mac's terminal has them.
const NAZ: [u32; 16] = [
    0x3A3A35, 0xFF5070, 0xA0FF40, 0xF0E890, 0x40D0FF, 0xF0A0F0, 0x80E5FF, 0xD0D0D0, 0x707080, 0xFF7088, 0x20FFA0, 0xFFE040, 0x60D8FF,
    0xFF69B4, 0xB8FFD0, 0xF5F5F0,
];
const DAYLIGHT: [u32; 16] = [
    0x2A2A28, 0xC01030, 0x5A7A00, 0x8A6A00, 0x0070A0, 0x8040A0, 0x00707A, 0x6A6A62, 0x4A4A46, 0xC01030, 0x0A7A48, 0xA85200, 0x0070A0,
    0xB02A70, 0x00707A, 0x2A2A28,
];

fn rgba(value: u32) -> gdk::RGBA {
    gdk::RGBA::new(((value >> 16) & 0xFF) as f32 / 255.0, ((value >> 8) & 0xFF) as f32 / 255.0, (value & 0xFF) as f32 / 255.0, 1.0)
}

pub struct TerminalPane {
    pub widget: gtk::Box,
    terminal: vte4::Terminal,
    started: Cell<bool>,
    folder: RefCell<Option<PathBuf>>,
    shell: String,
}

impl TerminalPane {
    /// [shell] empty is `$SHELL`; [font_size] in points, for the code font [font].
    pub fn new(shell: String, font: &str, font_size: i64) -> Rc<TerminalPane> {
        let terminal = vte4::Terminal::builder().hexpand(true).vexpand(true).scrollback_lines(10_000).build();
        let family = if font.is_empty() { "monospace" } else { font };
        terminal.set_font(Some(&gtk::pango::FontDescription::from_string(&format!("{family} {font_size}"))));
        let scroller = gtk::ScrolledWindow::builder().child(&terminal).vexpand(true).build();
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).height_request(160).build();
        widget.append(&scroller);
        widget.set_visible(false);
        let pane = Rc::new(TerminalPane { widget, terminal, started: Cell::new(false), folder: RefCell::new(None), shell });
        let style = adw::StyleManager::default();
        pane.paint(style.is_dark());
        {
            let pane = Rc::downgrade(&pane);
            style.connect_dark_notify(move |style| {
                if let Some(pane) = pane.upgrade() {
                    pane.paint(style.is_dark());
                }
            });
        }
        {
            // The shell ended: the next open starts another.
            let weak = Rc::downgrade(&pane);
            pane.terminal.connect_child_exited(move |_, _| {
                let pane = weak.clone();
                if let Some(pane) = pane.upgrade() {
                    pane.started.set(false);
                    pane.widget.set_visible(false);
                }
            });
        }
        pane
    }

    /// The folder a new shell starts in: the vault.
    pub fn set_folder(&self, folder: PathBuf) {
        *self.folder.borrow_mut() = Some(folder);
    }

    /// Shows or hides the pane; the first show starts the shell.
    pub fn toggle(&self) {
        let show = !self.widget.is_visible();
        self.widget.set_visible(show);
        if show {
            if !self.started.get() {
                self.start();
            }
            self.terminal.grab_focus();
        }
    }

    fn start(&self) {
        let shell = if self.shell.is_empty() { std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()) } else { self.shell.clone() };
        let folder = self.folder.borrow().clone().or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let mut environment: Vec<String> = std::env::vars()
            .filter(|(key, _)| !matches!(key.as_str(), "TERM" | "COLORTERM" | "TERM_PROGRAM"))
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        environment.extend(["TERM=xterm-256color".into(), "COLORTERM=truecolor".into(), "TERM_PROGRAM=Daftar".into()]);
        let environment: Vec<&str> = environment.iter().map(String::as_str).collect();
        // A login shell, as a terminal app starts it.
        self.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            folder.as_deref().and_then(|f| f.to_str()),
            &[shell.as_str(), "-l"],
            &environment,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            gio::Cancellable::NONE,
            |_| {},
        );
        self.started.set(true);
    }

    fn paint(&self, dark: bool) {
        let colors: Vec<gdk::RGBA> = (if dark { NAZ } else { DAYLIGHT }).iter().map(|c| rgba(*c)).collect();
        let (foreground, background) = if dark { (0xF5F5F0, 0x323232) } else { (0x2A2A28, 0xFAF9F5) };
        let palette: Vec<&gdk::RGBA> = colors.iter().collect();
        self.terminal.set_colors(Some(&rgba(foreground)), Some(&rgba(background)), &palette);
        self.terminal.set_color_cursor(Some(&rgba(if dark { 0xFFB040 } else { 0xA85200 })));
    }
}
