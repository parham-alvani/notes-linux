//! Daftar for Linux: a native app for an Obsidian vault kept in a Git
//! repository. The sister of Daftar for macOS and Daftar for Android.

mod palette;
mod reader;
mod settings;
mod sidebar;
mod store;
mod sync;
mod watcher;
mod window;

use adw::prelude::*;
use gtk::glib;

/// The same id as Daftar for macOS.
const APP_ID: &str = "me.parham1995.notes";

fn main() -> glib::ExitCode {
    // `daftar Notes/Inbox.md` opens that note, as `daftar://open?path=` does
    // on the Mac: for a keybinding in the compositor or a script.
    let mut args = std::env::args();
    let program = args.next().unwrap_or_default();
    let open = args.next();
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| window::Window::present(app, open.clone()));
    app.set_accels_for_action("window.close", &["<Ctrl>w"]);
    app.set_accels_for_action("win.find", &["<Ctrl>f"]);
    app.set_accels_for_action("win.find-next", &["<Ctrl>g"]);
    app.set_accels_for_action("win.find-previous", &["<Ctrl><Shift>g"]);
    app.set_accels_for_action("win.quick-open", &["<Ctrl>p", "<Ctrl>o"]);
    app.set_accels_for_action("win.search", &["<Ctrl><Shift>f"]);
    app.set_accels_for_action("win.open-vault", &["<Ctrl><Shift>o"]);
    app.set_accels_for_action("win.sync", &["<Ctrl><Shift>s"]);
    let quit = gtk::gio::SimpleAction::new("quit", None);
    let handle = app.clone();
    quit.connect_activate(move |_, _| handle.quit());
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    app.run_with_args(&[program])
}
