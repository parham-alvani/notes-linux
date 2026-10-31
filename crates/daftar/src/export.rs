//! Export as PDF (Ctrl+Shift+E): `daftar-cli export` on the note, to where
//! the person chooses. The command line makes the PDF, with the logos of
//! `export.logo` and of the note's folder, so the app and a script make the
//! same file.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::{gio, glib};

pub fn pdf(window: &gtk::Window, root: &Path, path: &str, notice: impl Fn(&str) + 'static) {
    let Some(cli) = cli() else {
        notice("Export as PDF runs daftar-cli, which is neither beside Daftar nor on $PATH.");
        return;
    };
    let title = daftar_core::Vault::title(path);
    let dialog = gtk::FileDialog::builder().title("Export as PDF").modal(true).initial_name(format!("{title}.pdf")).build();
    let (root, path) = (root.to_path_buf(), path.to_string());
    dialog.save(Some(window), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(output) = file.path() else {
            notice("Export as PDF writes to a folder on this computer.");
            return;
        };
        let arguments =
            [cli.as_os_str(), "--vault".as_ref(), root.as_os_str(), "export".as_ref(), path.as_ref(), "-o".as_ref(), output.as_os_str()];
        let process = match gio::Subprocess::newv(&arguments, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE) {
            Ok(process) => process,
            Err(error) => {
                notice(&format!("The PDF was not made: {error}"));
                return;
            }
        };
        let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        glib::spawn_future_local(async move {
            let said = process.communicate_utf8_future(None).await;
            match said {
                Ok(_) if process.is_successful() => notice(&format!("Saved {name}.")),
                Ok((_, errors)) => {
                    let errors = errors.map(|e| e.to_string()).unwrap_or_default();
                    let last = errors.lines().rfind(|line| !line.trim().is_empty()).unwrap_or("daftar-cli failed");
                    notice(last.trim_start_matches("daftar-cli: "));
                }
                Err(error) => notice(&format!("The PDF was not made: {error}")),
            }
        });
    });
}

/// The `daftar-cli` beside this binary (both packages install the two
/// together), else the one on $PATH.
fn cli() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.with_file_name("daftar-cli");
    if beside.is_file() {
        return Some(beside);
    }
    glib::find_program_in_path("daftar-cli")
}
