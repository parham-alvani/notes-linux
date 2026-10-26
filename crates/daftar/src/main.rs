//! Daftar for Linux: a native app for an Obsidian vault kept in a Git
//! repository. The sister of Daftar for macOS and Daftar for Android.

mod board;
mod editor;
mod editor_popup;
mod editor_style;
mod files;
mod graph;
mod home;
mod inspector;
mod neovim_session;
mod notifier;
mod palette;
mod places;
mod preferences;
mod reader;
mod settings;
mod sidebar;
mod store;
mod style;
mod sync;
mod task_edit;
mod terminal;
mod today;
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
    // A tab closes, and the last one closes the window. Ctrl+Tab, Ctrl+Page
    // Up and Down and Alt+1 to 9 are the tab view's own.
    app.set_accels_for_action("win.close-tab", &["<Ctrl>w"]);
    app.set_accels_for_action("win.new-tab", &["<Ctrl>t"]);
    app.set_accels_for_action("win.find", &["<Ctrl>f"]);
    app.set_accels_for_action("win.find-next", &["<Ctrl>g"]);
    app.set_accels_for_action("win.find-previous", &["<Ctrl><Shift>g"]);
    app.set_accels_for_action("win.quick-open", &["<Ctrl>p", "<Ctrl>o"]);
    app.set_accels_for_action("win.search", &["<Ctrl><Shift>f"]);
    app.set_accels_for_action("win.open-vault", &["<Ctrl><Shift>o"]);
    app.set_accels_for_action("win.today", &["<Ctrl>1"]);
    app.set_accels_for_action("win.board", &["<Ctrl>2"]);
    // The file commands act on the note on screen: an empty target.
    app.set_accels_for_action("win.new-note::", &["<Ctrl>n"]);
    app.set_accels_for_action("win.new-folder::", &["<Ctrl><Shift>n"]);
    app.set_accels_for_action("win.rename::", &["F2", "<Ctrl>r"]);
    app.set_accels_for_action("win.bookmark::", &["<Ctrl>d"]);
    app.set_accels_for_action("win.extract", &["<Ctrl><Alt>x"]);
    app.set_accels_for_action("win.inspector", &["<Ctrl><Alt>i"]);
    app.set_accels_for_action("win.commands", &["<Ctrl><Shift>p"]);
    app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
    app.set_accels_for_action("win.graph", &["<Ctrl><Alt>g"]);
    app.set_accels_for_action("win.terminal", &["<Ctrl>grave"]);
    app.set_accels_for_action("win.edit", &["<Ctrl>e"]);
    app.set_accels_for_action("win.save", &["<Ctrl>s"]);
    app.set_accels_for_action("win.sync", &["<Ctrl><Shift>s"]);
    app.set_accels_for_action("win.back", &["<Ctrl>bracketleft", "<Alt>Left"]);
    app.set_accels_for_action("win.forward", &["<Ctrl>bracketright", "<Alt>Right"]);
    app.set_accels_for_action("win.bigger-text", &["<Ctrl>plus", "<Ctrl>equal"]);
    app.set_accels_for_action("win.smaller-text", &["<Ctrl>minus"]);
    app.set_accels_for_action("win.actual-size", &["<Ctrl>0"]);
    let quit = gtk::gio::SimpleAction::new("quit", None);
    let handle = app.clone();
    // Each window closes as its button closes it: the editors save, and the
    // tabs and the reading places are kept. The app ends with the last.
    quit.connect_activate(move |_, _| {
        for window in handle.windows() {
            window.close();
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    // A notification's actions: a meeting's web link, or the Today page.
    let open_url = gtk::gio::SimpleAction::new("open-url", Some(glib::VariantTy::STRING));
    open_url.connect_activate(|_, target| {
        let Some(url) = target.and_then(|t| t.get::<String>()) else { return };
        if url.starts_with("https://") || url.starts_with("http://") {
            let _ = gtk::gio::AppInfo::launch_default_for_uri(&url, gtk::gio::AppLaunchContext::NONE);
        }
    });
    app.add_action(&open_url);
    let today = gtk::gio::SimpleAction::new("today", None);
    let handle = app.clone();
    today.connect_activate(move |_, _| {
        if let Some(window) = handle.active_window() {
            window.present();
            let _ = WidgetExt::activate_action(&window, "win.today", None);
        }
    });
    app.add_action(&today);
    let new_window = gtk::gio::SimpleAction::new("new-window", None);
    let handle = app.clone();
    new_window.connect_activate(move |_, _| window::Window::present(&handle, None));
    app.add_action(&new_window);
    app.set_accels_for_action("app.new-window", &["<Ctrl><Alt>n"]);
    app.run_with_args(&[program])
}
