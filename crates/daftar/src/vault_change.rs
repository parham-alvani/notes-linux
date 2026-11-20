//! The change from one vault to another in the running app
//! (`docs/specs/several-vaults.md`): what is made again for the new folder,
//! with no widget in it, so it is tested with no display. The window's
//! `load` runs `open` for every vault it shows, at launch and at a change:
//! the settings the vault sets, the hidden files, the store with its index
//! and search, whether the clone is read-only, and the vault in use in
//! `config.toml`. Nothing of the vault before is left in what it returns.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use serde_json::json;

use daftar_core::{SearchIndex, VaultIndex};

use crate::editor::Editor;
use crate::settings::AppSettings;
use crate::store::VaultStore;

/// The vault that is open now.
pub struct Opened {
    pub store: Rc<VaultStore>,
    /// The clone is read-only (`daftar.readonly`): the app pulls and never writes.
    pub read_only: bool,
}

/// Whether the folder at [root] can be opened; a vault that is missing (on
/// a disk that is away, or gone) is not, and the notice says so.
pub fn may_open(root: &Path) -> Result<(), String> {
    if root.is_dir() { Ok(()) } else { Err(format!("{} is not there. The vault stays as it is.", root.display())) }
}

/// Whether [editor] lets its note go before a change of vault: it saves,
/// and when the save fails, or the file changed under it, it keeps its
/// text and the change does not happen.
pub fn lets_go(editor: &Editor) -> bool {
    !editor.close().kept()
}

/// Everything read again for the vault at [root], whose notes [index] and
/// [search] hold: the settings its `.daftar.toml` sets (and none of the
/// vault before), the hidden files, the icons, bookmarks, templates and
/// daily notes of the store, and whether the clone is read-only. With
/// [record], a change of vault: `vault` is written in `config.toml`, and
/// `vaults` when the vault is new; a launch writes nothing.
pub fn open(settings: &mut AppSettings, root: &Path, index: VaultIndex, search: Arc<SearchIndex>, record: Option<&Path>) -> Opened {
    settings.set_vault(Some(root));
    if let Some(home) = record {
        // The settings say why when they cannot be written; the vault is open all the same.
        let _ = settings.record_vault(root, home);
    }
    let store = Rc::new(VaultStore::new(root.to_path_buf(), index, search));
    let names = settings.file.strings("files.hidden").unwrap_or_else(|| daftar_core::HIDDEN_NAMES.map(String::from).to_vec());
    store.set_hidden(&names, settings.file.bool("files.show_hidden").unwrap_or(false));
    Opened { read_only: daftar_core::Git::read_only(root), store }
}

/// The tabs a window had on one vault, kept by the vault's folder: the
/// note of each tab, of its right pane, the pane with the focus, and the
/// tab in front.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tabs {
    /// The note of each tab's first pane.
    pub notes: Vec<String>,
    pub splits: Vec<Option<String>>,
    pub focused: Vec<usize>,
    pub chosen: usize,
}

impl Tabs {
    /// Where the tabs of the vault at [root] are kept, in [state] (the
    /// app's state folder): a file named by the folder, so each vault has
    /// its own, and a vault that moved is a new one.
    #[cfg(test)]
    pub fn file_in(state: &Path, root: &Path) -> PathBuf {
        let name = crate::places::state_file(root, "tabs");
        state.join(name.file_name().unwrap_or_default())
    }

    pub fn file(root: &Path) -> PathBuf {
        crate::places::state_file(root, "tabs")
    }

    pub fn save(&self, file: &Path) {
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let text = json!({ "tabs": self.notes, "splits": self.splits, "focused": self.focused, "chosen": self.chosen }).to_string();
        let _ = std::fs::write(file, text);
    }

    /// The tabs kept in [file]; none when the vault was never open here.
    pub fn read(file: &Path) -> Option<Tabs> {
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        let list = |key: &str| saved.get(key).and_then(|list| list.as_array()).cloned().unwrap_or_default();
        Some(Tabs {
            notes: list("tabs").iter().map(|path| path.as_str().unwrap_or_default().to_string()).collect(),
            splits: list("splits").iter().map(|path| path.as_str().map(String::from)).collect(),
            focused: list("focused").iter().map(|at| at.as_u64().unwrap_or(0) as usize).collect(),
            chosen: saved.get("chosen").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use daftar_core::{Git, TaskFields, TaskStatuses, Vault, Vaults};

    // Every fixture here is made up. The repository is public; the vault is not.

    const STATUSES: &str =
        r#"{"statusSettings":{"customStatuses":[{"symbol":"r","name":"In Review","nextStatusSymbol":"x","type":"IN_PROGRESS"}]}}"#;

    /// A home folder with a `config.toml` and two vaults: `A` has a note
    /// `Alpha.md`, a task status of its own, a text size and a hidden name
    /// in its `.daftar.toml`, and is a read-only clone; `B` has `Beta.md`
    /// and none of those.
    fn two_vaults() -> (tempfile::TempDir, PathBuf, PathBuf, AppSettings) {
        let home = tempfile::tempdir().unwrap();
        let (a, b) = (home.path().join("A"), home.path().join("work/B"));
        fs::create_dir_all(a.join(".obsidian/plugins/obsidian-tasks-plugin")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("Alpha.md"), "# Alpha\n\nOnly in the first vault.\n\n- [r] Read it\n").unwrap();
        fs::write(a.join("A2.md"), "# A2\n").unwrap();
        fs::write(a.join("Hidden by A.md"), "# Hidden\n").unwrap();
        fs::write(a.join(".obsidian/plugins/obsidian-tasks-plugin/data.json"), STATUSES).unwrap();
        fs::write(a.join(".daftar.toml"), "[appearance]\ntext_size = 20\n\n[files]\nhidden = [\"Hidden by A.md\"]\n").unwrap();
        fs::write(b.join("Beta.md"), "# Beta\n\nOnly in the second vault.\n").unwrap();
        fs::write(b.join("Hidden by A.md"), "# Shown here\n").unwrap();
        fs::write(b.join("AGENTS.md"), "# Rules\n").unwrap();
        let git = Git::new(&a);
        for arguments in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"],
            &["config", "user.name", "Test"],
            &["config", "commit.gpgsign", "false"],
            &["add", "-A"],
            &["commit", "-qm", "start"],
            &["config", "daftar.readonly", "true"],
        ] {
            git.run(arguments).unwrap();
        }
        let path = home.path().join("config.toml");
        let computer = "vault = \"~/A\"\n\n[appearance]\ntext_size = 14\n\n[files]\nhidden = [\"AGENTS.md\"]\n".to_string();
        fs::write(&path, &computer).unwrap();
        let settings = AppSettings::of(path, &computer);
        (home, a, b, settings)
    }

    /// The change to the vault at [root], as the window makes it: its
    /// notes are indexed, and everything is made again for it.
    fn change(settings: &mut AppSettings, root: &Path, home: &Path) -> Opened {
        let search = Arc::new(SearchIndex::new());
        let index = Vault::index(root, Some(&search));
        open(settings, root, index, search, Some(home))
    }

    /// Whether the statuses in use have the vault's own `[r]`. Other tests
    /// index their vaults at the same time, and the statuses are the
    /// process's, so the answer is asked right after this vault's own index.
    fn has_review_status(root: &Path, wanted: bool) -> bool {
        (0..200).any(|_| {
            Vault::index(root, None);
            TaskStatuses::current().all.iter().any(|status| status.symbol == 'r') == wanted
        })
    }

    #[test]
    fn a_change_leaves_nothing_of_the_vault_before() {
        let (home, a, b, mut settings) = two_vaults();
        let home = home.path();
        let found =
            |opened: &Opened, word: &str| -> Vec<String> { opened.store.search.search(word, 10).into_iter().map(|hit| hit.path).collect() };
        let task = TaskFields { text: "Added".into(), ..Default::default() };
        let in_use =
            |settings: &AppSettings| settings.vaults(home).entries().into_iter().find(|entry| entry.in_use).map(|entry| entry.path);
        let is_a = |opened: &Opened, settings: &AppSettings| {
            // Quick open and search find `Alpha`, and not `Beta`.
            assert_eq!(crate::palette::best(&opened.store, "alpha", &[]).as_deref(), Some("Alpha.md"));
            assert_eq!(crate::palette::best(&opened.store, "beta", &[]), None);
            assert_eq!(found(opened, "Alpha"), ["Alpha.md"]);
            assert!(found(opened, "Beta").is_empty());
            assert!(opened.store.index().tasks().iter().any(|task| task.path == "Alpha.md"));
            // The vault's own status, text size and hidden name; a read-only clone.
            assert!(has_review_status(&a, true));
            assert_eq!(settings.text_size(), 20);
            assert!(settings.is_from_vault("appearance.text_size"));
            assert!(opened.store.is_hidden("Hidden by A.md") && !opened.store.is_hidden("AGENTS.md"));
            assert!(opened.read_only);
            assert_eq!(opened.store.add_task("Alpha.md", None, &task), Err(daftar_core::Failure::ReadOnly));
            assert_eq!(in_use(settings).as_deref(), Some(a.as_path()));
        };
        let opened = change(&mut settings, &a, home);
        is_a(&opened, &settings);
        let alpha = fs::read(a.join("Alpha.md")).unwrap();

        // To B: nothing of A is left.
        let opened = change(&mut settings, &b, home);
        assert_eq!(crate::palette::best(&opened.store, "beta", &[]).as_deref(), Some("Beta.md"));
        assert_eq!(crate::palette::best(&opened.store, "alpha", &[]), None);
        assert_eq!(found(&opened, "Beta"), ["Beta.md"]);
        assert!(found(&opened, "Alpha").is_empty());
        assert!(opened.store.index().notes.keys().all(|path| path != "Alpha.md" && path != "A2.md"));
        assert!(opened.store.index().tasks().is_empty());
        // The statuses are the defaults, and the text size and the hidden names the computer's.
        assert!(has_review_status(&b, false));
        assert_eq!(settings.text_size(), 14);
        assert!(!settings.is_from_vault("appearance.text_size"));
        assert!(opened.store.is_hidden("AGENTS.md") && !opened.store.is_hidden("Hidden by A.md"));
        // Not read-only: a write to `Beta.md` succeeds.
        assert!(!opened.read_only);
        assert_eq!(opened.store.add_task("Beta.md", None, &task), Ok(()));
        assert!(fs::read_to_string(b.join("Beta.md")).unwrap().ends_with("- [ ] Added\n"));
        // `vault` in `config.toml` is B, and the list has both, in the order they came.
        assert_eq!(in_use(&settings).as_deref(), Some(b.as_path()));
        let written = Vaults::new(&fs::read_to_string(&settings.path).unwrap(), home);
        assert_eq!(written.list(), ["~/A", "~/work/B"]);
        assert_eq!(written.in_use().as_deref(), Some("~/work/B"));
        assert_eq!(opened.store.root, b);

        // Back to A: each is A's again, and the write is refused.
        let opened = change(&mut settings, &a, home);
        is_a(&opened, &settings);
        assert_eq!(fs::read(a.join("Alpha.md")).unwrap(), alpha);
        assert_eq!(Vaults::new(&fs::read_to_string(&settings.path).unwrap(), home).list(), ["~/A", "~/work/B"]);
        // A launch writes nothing: the file is as the change left it.
        let before = fs::read_to_string(&settings.path).unwrap();
        let search = Arc::new(SearchIndex::new());
        let index = Vault::index(&b, Some(&search));
        let launched = open(&mut settings, &b, index, search, None);
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), before);
        assert_eq!(launched.store.root, b);
    }

    #[test]
    fn each_vault_keeps_its_tabs() {
        let (home, a, b, _) = two_vaults();
        let state = home.path().join("state");
        // A shows `Alpha.md` and `A2.md` in two tabs, the second in front; then the change to B.
        let of_a = Tabs { notes: vec!["Alpha.md".into(), "A2.md".into()], splits: vec![None, None], focused: vec![0, 0], chosen: 1 };
        of_a.save(&Tabs::file_in(&state, &a));
        // A vault opened for the first time has no tabs of its own: the home page.
        assert_eq!(Tabs::read(&Tabs::file_in(&state, &b)), None);
        let of_b = Tabs { notes: vec!["Beta.md".into()], splits: vec![Some("Hidden by A.md".into())], focused: vec![1], chosen: 0 };
        of_b.save(&Tabs::file_in(&state, &b));
        // Back: A shows its two tabs with the same one in front, and B, opened again, `Beta.md`.
        assert_eq!(Tabs::read(&Tabs::file_in(&state, &a)), Some(of_a));
        assert_eq!(Tabs::read(&Tabs::file_in(&state, &b)), Some(of_b));
        assert_ne!(Tabs::file_in(&state, &a), Tabs::file_in(&state, &b));
        // A vault that moved is a new vault to its tabs.
        assert_eq!(Tabs::read(&Tabs::file_in(&state, &home.path().join("moved/A"))), None);
        assert_eq!(Tabs::file(&a).file_name(), Tabs::file_in(&state, &a).file_name());
    }

    #[test]
    fn a_missing_vault_is_listed_and_not_opened() {
        let (home, a, _, mut settings) = two_vaults();
        let home = home.path();
        change(&mut settings, &a, home);
        let away = home.join("away");
        let listed = "vault = \"~/A\"\nvaults = [\"~/A\", \"~/away\"]\n".to_string();
        fs::write(&settings.path, &listed).unwrap();
        // It is in the list, as missing.
        let entries = settings.vaults(home).entries();
        assert_eq!(
            entries.iter().map(|e| (e.name.as_str(), e.is_missing(), e.in_use)).collect::<Vec<_>>(),
            [("A", false, true), ("away", true, false)]
        );
        // A change to it is refused with a notice, before anything is written; the vault in use stays.
        let refused = may_open(&away).unwrap_err();
        assert!(refused.contains("is not there"), "{refused}");
        assert_eq!(may_open(&a), Ok(()));
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), listed);
        // It is still in the list afterwards, until it is forgotten; the one in use never is.
        assert_eq!(settings.vaults(home).list(), ["~/A", "~/away"]);
        assert!(settings.forget_vault("~/A", home).is_err());
        settings.forget_vault("~/away", home).unwrap();
        assert_eq!(settings.vaults(home).list(), ["~/A"]);
        assert!(a.join("Alpha.md").exists());
    }

    /// Needs a display: `gtk4-broadwayd :7`, then `GDK_BACKEND=broadway
    /// BROADWAY_DISPLAY=:7 cargo test -p daftar -- --ignored
    /// an_unsaved_editor_stops_the_change`, alone, as GTK keeps to the
    /// thread that started it.
    #[test]
    #[ignore = "needs a display"]
    fn an_unsaved_editor_stops_the_change() {
        use gtk::prelude::*;
        use std::cell::RefCell;
        let (home, a, b, mut settings) = two_vaults();
        let home = home.path();
        // B is the vault here: A is read-only, and no editor opens there.
        change(&mut settings, &b, home);
        let before = fs::read_to_string(&settings.path).unwrap();
        gtk::init().unwrap();
        adw::init().unwrap();
        crate::style::install();
        let shared = Rc::new(RefCell::new(AppSettings::of(settings.path.clone(), daftar_core::ConfigFile::TEMPLATE)));
        let editor = Editor::new(shared, |_| {}, |_| {});
        let window = gtk::Window::builder().default_width(600).default_height(400).child(&editor.widget).build();
        window.present();
        assert!(editor.load(&b, "Beta.md", Some(0)));
        editor.type_at_end("Typed and not saved.\n");
        let typed = editor.text();
        // Another app wrote the note meanwhile: the save fails, and keeps the text.
        fs::write(b.join("Beta.md"), "# Beta\n\nWritten by another app.\n").unwrap();
        assert!(!lets_go(&editor));
        // The change is refused: the vault stays, and the editor's text is kept.
        assert_eq!(editor.text(), typed);
        assert_eq!(editor.path().as_deref(), Some("Beta.md"));
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), before);
        assert_eq!(fs::read_to_string(b.join("Beta.md")).unwrap(), "# Beta\n\nWritten by another app.\n");
        assert!(a.join("Alpha.md").exists());
        window.close();
    }
}
