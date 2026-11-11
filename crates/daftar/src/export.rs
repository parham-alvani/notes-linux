//! Export as PDF (Ctrl+Shift+E): `daftar-cli export` on the note, to where
//! the person chooses. The command line makes the PDF, with the logos of
//! `export.logo` and of the note's folder, so the app and a script make the
//! same file. Before the place, a dialog asks for the command's two page
//! options, `--contents` and `--landscape`, set at first from
//! `export.contents` and `export.landscape`, which keep the last choice.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use daftar_core::{ConfigFile, Value};
use gtk::{gio, glib};

use crate::settings::AppSettings;

/// The page options the dialog asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `--contents`: a table of the headings before the note.
    pub contents: bool,
    /// `--landscape`: the page on its side.
    pub landscape: bool,
}

impl Options {
    /// The last choice, from `export.contents` and `export.landscape`.
    pub fn from(file: &ConfigFile) -> Options {
        Options { contents: file.bool("export.contents").unwrap_or(false), landscape: file.bool("export.landscape").unwrap_or(false) }
    }

    /// The flags these options add to `daftar-cli export`.
    pub fn flags(self) -> Vec<&'static str> {
        [(self.contents, "--contents"), (self.landscape, "--landscape")].into_iter().filter(|(on, _)| *on).map(|(_, flag)| flag).collect()
    }
}

/// The command's arguments, after the program, for the note at [path] in
/// the vault at [root]. The note comes last, after `--`: a note at the
/// vault's top whose name starts with `-` (`-o.md`, `--help.md`) is a note,
/// not an option. The flags of [options] go before it.
pub fn arguments<'a>(root: &'a Path, path: &'a str, output: &'a Path, options: Options) -> Vec<&'a OsStr> {
    let mut arguments: Vec<&OsStr> = vec!["--vault".as_ref(), root.as_os_str(), "export".as_ref(), "-o".as_ref(), output.as_os_str()];
    arguments.extend(options.flags().into_iter().map(AsRef::<OsStr>::as_ref));
    arguments.extend([OsStr::new("--"), OsStr::new(path)]);
    arguments
}

pub fn pdf(window: &gtk::Window, settings: Rc<RefCell<AppSettings>>, root: &Path, path: &str, notice: impl Fn(&str) + 'static) {
    let Some(cli) = cli() else {
        notice("Export as PDF runs daftar-cli, which is neither beside Daftar nor on $PATH.");
        return;
    };
    let notice: Rc<dyn Fn(&str)> = Rc::new(notice);
    let last = Options::from(&settings.borrow().file);
    let dialog = adw::AlertDialog::new(Some("Export as PDF"), Some(&daftar_core::Vault::title(path)));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("go", "Export…");
    dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("go"));
    dialog.set_close_response("cancel");
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    let contents = adw::SwitchRow::builder()
        .title("Table of contents")
        .subtitle("The headings, each with its page, before the note")
        .active(last.contents)
        .build();
    let landscape = adw::SwitchRow::builder()
        .title("Landscape")
        .subtitle("The page on its side, for a screen or a projector")
        .active(last.landscape)
        .build();
    list.append(&contents);
    list.append(&landscape);
    dialog.set_extra_child(Some(&list));
    let (parent, root, path) = (window.clone(), root.to_path_buf(), path.to_string());
    dialog.connect_response(None, move |_, response| {
        if response != "go" {
            return;
        }
        let chosen = Options { contents: contents.is_active(), landscape: landscape.is_active() };
        if chosen != last {
            let mut settings = settings.borrow_mut();
            let kept = settings
                .set("export.contents", Value::Bool(chosen.contents))
                .and_then(|()| settings.set("export.landscape", Value::Bool(chosen.landscape)));
            if let Err(error) = kept {
                notice(&format!("The choice was not kept in config.toml: {error}"));
            }
        }
        save(&parent, cli.clone(), root.clone(), path.clone(), chosen, notice.clone());
    });
    dialog.present(Some(window));
}

/// Asks where the PDF goes, then runs the command there.
fn save(window: &gtk::Window, cli: PathBuf, root: PathBuf, path: String, options: Options, notice: Rc<dyn Fn(&str)>) {
    let title = daftar_core::Vault::title(&path);
    let dialog = gtk::FileDialog::builder().title("Export as PDF").modal(true).initial_name(format!("{title}.pdf")).build();
    dialog.save(Some(window), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(output) = file.path() else {
            notice("Export as PDF writes to a folder on this computer.");
            return;
        };
        let mut command = vec![cli.as_os_str()];
        command.extend(arguments(&root, &path, &output, options));
        let process = match gio::Subprocess::newv(&command, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE) {
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
                Ok((_, errors)) => notice(&failure(&errors.map(|e| e.to_string()).unwrap_or_default())),
                Err(error) => notice(&format!("The PDF was not made: {error}")),
            }
        });
    });
}

/// What a failed run says: the last line of its stderr, without the
/// command's name. A usage error (clap's `error:` line, then a usage and a
/// "try '--help'" line) says its `error:` line, and a `daftar-cli` too old
/// for the page options (before 0.21.0) says to upgrade it.
pub fn failure(errors: &str) -> String {
    let lines: Vec<&str> = errors.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if let Some(usage) = lines.iter().find_map(|line| line.strip_prefix("error: ")) {
        if ["--contents", "--landscape"].iter().any(|flag| usage.starts_with(&format!("unexpected argument '{flag}'"))) {
            return "Table of contents and Landscape need daftar-cli 0.21.0 or later.".to_string();
        }
        return format!("daftar-cli: {usage}");
    }
    lines.last().map_or_else(|| "daftar-cli failed.".to_string(), |line| line.trim_start_matches("daftar-cli: ").to_string())
}

/// The `daftar-cli` beside this binary (both packages install the two
/// together), else the one on $PATH.
pub fn cli() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.with_file_name("daftar-cli");
    if beside.is_file() {
        return Some(beside);
    }
    glib::find_program_in_path("daftar-cli")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_come_from_the_config_file() {
        let file = ConfigFile::new("[export]\ncontents = true\nlandscape = false\n".to_string());
        assert_eq!(Options::from(&file), Options { contents: true, landscape: false });
        assert_eq!(Options::from(&ConfigFile::new(String::new())), Options::default());
    }

    #[test]
    fn a_failure_is_the_last_line_of_stderr() {
        assert_eq!(failure("warning: something\ndaftar-cli: the logo /x.svg is not a file\n\n"), "the logo /x.svg is not a file");
        assert_eq!(failure(""), "daftar-cli failed.");
    }

    #[test]
    fn an_old_cli_says_to_upgrade_for_the_page_options() {
        let old = "error: unexpected argument '--contents' found\n\nUsage: daftar-cli export [OPTIONS] <NOTE>\n\nFor more information, try '--help'.\n";
        assert_eq!(failure(old), "Table of contents and Landscape need daftar-cli 0.21.0 or later.");
        let bad = "error: unexpected argument '--bad' found\n\nUsage: daftar-cli export [OPTIONS] <NOTE>\n\nFor more information, try '--help'.\n";
        assert_eq!(failure(bad), "daftar-cli: unexpected argument '--bad' found");
    }

    #[test]
    fn the_arguments_add_a_flag_for_each_option_before_the_note() {
        let (root, output) = (Path::new("/vault"), Path::new("/tmp/Note.pdf"));
        let plain = arguments(root, "Notes/Note.md", output, Options::default());
        assert_eq!(plain, ["--vault", "/vault", "export", "-o", "/tmp/Note.pdf", "--", "Notes/Note.md"].map(OsStr::new));
        let both = arguments(root, "Notes/Note.md", output, Options { contents: true, landscape: true });
        assert_eq!(
            both,
            ["--vault", "/vault", "export", "-o", "/tmp/Note.pdf", "--contents", "--landscape", "--", "Notes/Note.md"].map(OsStr::new)
        );
        // A note whose name starts with a dash is still the note.
        assert_eq!(arguments(root, "--help.md", output, Options::default()).last(), Some(&OsStr::new("--help.md")));
    }
}
