//! Check the Vault: `daftar-cli check` on the open vault, as its CI runs it,
//! with each finding a row that opens its note at the line. The command line
//! keeps the rules, so the app, a script and the vault's CI find the same.

use std::path::Path;

use adw::prelude::*;
use gtk::{gio, glib};

/// One line of `check`: `path:line`, a tab, the level, the rule and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    /// From 1, as the command prints it.
    pub line: usize,
    pub error: bool,
    pub rule: String,
    pub message: String,
}

/// The findings in what `check` printed; a line of another shape is left out.
pub fn parse(output: &str) -> Vec<Finding> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let (place, level, rule, message) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
            let (path, line) = place.rsplit_once(':')?;
            Some(Finding {
                path: path.to_string(),
                line: line.parse().ok()?,
                error: level == "error",
                rule: rule.to_string(),
                message: message.to_string(),
            })
        })
        .collect()
}

/// Runs the check off the window, then shows what it found; [open] opens a
/// note at a line from 0.
pub fn present(window: &gtk::Window, root: &Path, open: impl Fn(&str, usize) + 'static, notice: impl Fn(&str) + 'static) {
    let Some(cli) = crate::export::cli() else {
        notice("Check the Vault runs daftar-cli, which is neither beside Daftar nor on $PATH.");
        return;
    };
    let arguments = [cli.as_os_str(), "--vault".as_ref(), root.as_os_str(), "check".as_ref()];
    let process = match gio::Subprocess::newv(&arguments, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE) {
        Ok(process) => process,
        Err(error) => {
            notice(&format!("The vault was not checked: {error}"));
            return;
        }
    };
    notice("Checking the vault…");
    let window = window.clone();
    glib::spawn_future_local(async move {
        let (found, said) = match process.communicate_utf8_future(None).await {
            Ok((found, said)) => (found.map(|f| f.to_string()).unwrap_or_default(), said.map(|s| s.to_string()).unwrap_or_default()),
            Err(error) => {
                notice(&format!("The vault was not checked: {error}"));
                return;
            }
        };
        // The last line on stderr counts them: `daftar-cli: 2 errors, 1 warnings in 40 notes`.
        let summary = said
            .lines()
            .map(|line| line.trim_start_matches("daftar-cli: "))
            .find(|line| line.contains(" errors, "))
            .unwrap_or_default()
            .to_string();
        show(&window, parse(&found), &summary, open);
    });
}

fn show(window: &gtk::Window, mut findings: Vec<Finding>, summary: &str, open: impl Fn(&str, usize) + 'static) {
    // Errors first, each kind in the vault's order.
    findings.sort_by(|a, b| b.error.cmp(&a.error).then(a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
    for finding in &findings {
        let row = adw::ActionRow::builder()
            .title(&finding.message)
            .subtitle(format!("{}:{} · {}", finding.path, finding.line, finding.rule))
            .use_markup(false)
            .activatable(true)
            .build();
        let icon = if finding.error { "dialog-error-symbolic" } else { "dialog-warning-symbolic" };
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        list.append(&row);
    }
    let dialog = adw::Dialog::builder().title("Check the Vault").content_width(720).content_height(560).build();
    {
        let dialog = dialog.clone();
        list.connect_row_activated(move |_, row| {
            let Some(finding) = usize::try_from(row.index()).ok().and_then(|i| findings.get(i)) else { return };
            dialog.close();
            open(&finding.path, finding.line.saturating_sub(1));
        });
    }
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let heading = if list.first_child().is_none() { "Nothing to fix.".to_string() } else { summary.to_string() };
    body.append(&gtk::Label::builder().label(heading).xalign(0.0).css_classes(["dim-label"]).build());
    body.append(&list);
    let scrolled = gtk::ScrolledWindow::builder().child(&body).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scrolled));
    dialog.set_child(Some(&view));
    dialog.present(Some(window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_is_read_from_its_line() {
        let found = parse(
            "Notes/A b.md:12\terror\ttask-date\t📅 2026-02-30 is not a day\nnot a finding\nB.md:3\twarning\tdangling-link\t[[Gone]] is no note\n",
        );
        assert_eq!(
            found,
            [
                Finding {
                    path: "Notes/A b.md".into(),
                    line: 12,
                    error: true,
                    rule: "task-date".into(),
                    message: "📅 2026-02-30 is not a day".into()
                },
                Finding { path: "B.md".into(), line: 3, error: false, rule: "dangling-link".into(), message: "[[Gone]] is no note".into() },
            ]
        );
    }
}
