# Several vaults on the desktop: a list, and a change without a restart

Shared by Daftar for Linux (Rust, `notes-linux`) and macOS (Swift, `notes-mac`), and by `daftar-cli`. Daftar for Android has several vaults already, each separate, and is not changed by this spec.

The desktop apps open one vault, the `vault` line of `config.toml`, and Open Vault… replaces it: the vault before is forgotten, and going back means finding its folder again. A person now has more than one: the whole vault on the laptop, a read-only clone ([a read-only vault](read-only-vault.md)), a bundle exported for another computer ([the Open Knowledge Format](open-knowledge-format.md)). This spec keeps a list of vaults and a command to change between them.

**The change happens in the running app, by the path Open Vault… already uses** (`load` on Linux, `VaultStore.open` and `switchTo` on macOS): the editors are saved, the index, the search, the watcher and the vault's settings are made again for the new folder. No restart. A restart was considered and left as the fallback: it would replace a path that works with a slower one, and what it would buy (nothing of one vault left in the next) is asked of the change itself, by test 3.

One vault is open at a time, in every window of the app. Two vaults side by side are a second process (`open -n` on macOS, a second instance on Linux), not this spec.

## 1. The list

```toml
# The vault in use.
vault = "~/org"
# Every vault this computer knows, the one in use among them.
vaults = ["~/org", "~/notes-bundle"]
```

- `vaults` is a list of folders in `config.toml`, each as the person would write it (`~` kept). `vault` stays the one in use, so an older Daftar reads the file as before.
- **A vault is one folder, compared after `~` and `.`/`..` are resolved** (a standardized path, links not followed). The list never holds the same folder twice, in whatever spelling.
- **The vault in use is always in the list.** A `config.toml` with a `vault` and no `vaults` (every one today) is read as a list of that one, and the list is written at the first change.
- **Opening a vault adds it**, at the end, once. The order is the person's: the list is not sorted, and a change of vault does not move anything. A folder the list has keeps the text it has; a new one that was chosen in a file dialog is written with `~` when it is under the home folder, and whole when it is not. A start of the app writes nothing: the list is written at the first change.
- **A vault is shown by its folder's name** (`org`); when two share a name, each with its parent folder (`org — ~/work`), the parent written with `~` when it is under the home folder.
- **A folder that is gone stays in the list**, shown as missing and not opened; it may be on a disk that is not there today. Forget Vault removes an entry from the list and touches no file.
- `vaults` is the computer's only: like `vault`, a repository's `.daftar.toml` can never set it ([the app's settings from the vault](vault-config.md), the never row).

## 2. The commands

- **Switch Vault** (the palette; File › Switch Vault, a submenu on macOS; on Linux the palette's Switch Vault… opens the list as a dialog, each row the name over the folder): the vaults of the list, the one in use marked, a missing one off, then Open Vault… as the last entry. Open Vault… keeps its shortcut and now adds to the list.
- **Forget Vault** (the palette, and a button by each row of Settings › Vault): removes a vault from the list after a question. The vault in use cannot be forgotten.
- **Settings › Vault** lists the vaults with Switch and Forget, above the Git group of the vault in use. On Linux both close the settings first, since the page is the vault's that is being left.
- `daftar-cli vaults` prints the list, a line each: the name, the folder, and `in use`, `missing` or nothing; `--json` is one array of `{"name","path","in_use","missing"}`. `--vault` takes a folder as now, or the name of a listed vault when exactly one has it: a value that is a folder is the folder, whatever the list says; else a value with no `/` is looked for among the names, more than one is refused with each of them named, and none is the folder it always was (and the error that it is no folder). `daftar-cli` never changes the vault in use: the app does.

## 3. The change

Changing to a vault of the list, or opening a new one, is one path:

1. **The editors save first.** When a save fails, or the person keeps an editor open on a question, the change does not happen and the vault stays.
2. **A sync that is running finishes first**; the change waits for it, with a notice that it does. No sync is started by a change, but the new vault's pull at launch runs when `git.pull_on_launch` is on, as at a start.
3. **The tabs of the vault being left are kept**, and the tabs the new vault had are opened again, each with its reading place. A vault opened for the first time shows the home page. With more than one window, every window changes: the window the person is in keeps its tabs for the vault being left and opens the new vault's, and the others show the home page in one tab.
4. **Everything is read again for the new folder**: the index and the search, the file watcher, the vault's `.daftar.toml` (the settings it sets, `[reader]`, `[agents]`), whether the clone is read-only, the task statuses, the hidden files, the icons, the bookmarks, the templates, the daily notes' settings and the home page's.
5. **`vault` is written**, and `vaults` when the vault is new, once the change is not refused: a missing folder or an editor that keeps its text leaves `config.toml` as it was.
6. **Nothing of the vault before is left**: no note of it in quick open, search, the graph or the tasks, no setting it set, no read-only mark, no task status of its own, no finding in the Problems panel, no notification waiting for one of its meetings.

State that is kept per vault, by the vault's folder (the hash the state files already use): the tabs, the reading places and folds, the drill's state, the search and meaning indexes, the notified meetings. A vault that is renamed or moved is a new vault to them.

**The fallback.** If a kind of state cannot be reset in the running app, the change may restart the app for that platform (save, write `vault`, start again, quit). It is the last resort, and the spec text says so when an app takes it. Linux does not take it: every state of section 3 is made again in the running app. What stays as the person left it, and is not the vault's: a shell already running in the terminal pane (a new one starts in the new folder), and a dialog that was open over the window.

## 4. Tests every app must have

Ported by name (snake case in Rust). `Vaults` is the pure rule over the text of `config.toml`. On Linux, tests 3, 4 and 5 run with no display on the part of the change that has no widget (`vault_change.rs`: `open`, which the window's `load` runs for every vault, the tabs' file by vault, and `may_open`), and test 6 needs a display, so it is run on its own (`--ignored`) and not by `just test`:

1. `theVaultInUseIsOneOfTheList`: `vault = "~/org"` with no `vaults` is the list `["~/org"]`; `vaults = ["~/a"]` with `vault = "~/b"` is `["~/a", "~/b"]`; no `vault` and no `vaults` is an empty list and no vault in use.
2. `openingAVaultAddsItOnce`: with the home folder `/home/p`, opening `/home/p/org`, `~/org`, `~/org/` and `~/x/../org` on the list `["~/org"]` adds nothing and keeps the text `~/org`; opening `~/notes` gives `["~/org", "~/notes"]`; forgetting `~/notes` gives `["~/org"]`; forgetting the vault in use is refused.
3. `aChangeLeavesNothingOfTheVaultBefore`, on two synthetic vaults: `A` has the note `Alpha.md`, a custom task status `[r]`, `[appearance] text_size = 20` in its `.daftar.toml`, a hidden file name of its own and `daftar.readonly` on; `B` has `Beta.md` and none of those. After a change from `A` to `B`: quick open and search find `Beta` and not `Alpha`; the statuses are the defaults; `text_size` is the computer's; the hidden names are the computer's; the vault is not read-only and a write to `Beta.md` succeeds; `vault` in `config.toml` is `B`. After the change back to `A`, each is `A`'s again and the write is refused.
4. `eachVaultKeepsItsTabs`: with `A` showing `Alpha.md` and `A2.md` in two tabs, a change to `B`, `Beta.md` opened there, and a change back: `A` shows its two tabs with the same one in front, and `B`, opened again, shows `Beta.md`.
5. `aMissingVaultIsListedAndNotOpened`: a listed folder that does not exist is in the list as missing; a change to it is refused with a notice and the vault in use stays; it is still in the list afterwards.
6. `anUnsavedEditorStopsTheChange`: with an editor whose save fails, the change is refused, the vault stays and the editor's text is kept.
7. `twoVaultsOfOneNameShowTheirParents`: `~/org` and `~/work/org` are shown as `org — ~` and `org — ~/work`; `--vault org` is refused as ambiguous and names both; `--vault notes` with one `~/notes` listed opens it.
8. `theVaultCannotNameVaults`: `vaults = ["/tmp/x"]` in a vault's `.daftar.toml` is not used and is listed as refused.
