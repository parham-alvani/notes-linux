# The app's settings from the vault

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`), and by `daftar-cli`.

The app's settings are one computer's: `$XDG_CONFIG_HOME/daftar/config.toml` on the desktop, the app's own store on Android. A vault opened on a new computer starts from the defaults, and every setting is made again by hand. This spec lets the vault carry them: the committed `.daftar.toml` at its root may hold the same sections and keys as `config.toml`, and a clone is set up by its first pull.

`.daftar.toml` already holds what the vault says about itself (`[check]`, `[audit]`, `[reader]`, and `[agents]` of [a read-only vault](read-only-vault.md)). Those stay as they are. This spec adds the app's own keys beside them.

## 1. Which value is used

For each key, the first of these that has it:

1. **The vault's `.daftar.toml`**, when the key is one the vault may set (section 2) and the computer follows the vault.
2. **The computer's `config.toml`** (on Android, the app's settings).
3. **The default.**

- **The vault wins over the computer.** `config.toml` is written from a template with every key in it, so "the computer's value when it has one" would never let the vault speak.
- **A computer can decline.** `vault_settings = false` at the top of `config.toml` (a switch in the app's settings on Android) makes the app read no app setting from any vault; `vault_settings = ["appearance.theme", "git.sync_interval"]` declines only those keys. Not set is `true`. `[check]`, `[audit]`, `[reader]` and `[agents]` are not app settings and are always read.
- **A value of the wrong type, or out of the key's range, is as not there**, and the next source is used. A key the app does not know is left alone, as in `config.toml`.
- **The file is read again when it changes** (a pull, an edit), as `config.toml` is, and at each sync.

## 2. What the vault may set

A repository is text that other people and other computers can change. It must never choose a program to run, a file outside the vault, or what leaves the computer. So the keys are a list, not "everything":

| Keys | From the vault |
| --- | --- |
| `appearance.theme`, `text_size`, `font`, `code_font`, `text_width`, `reading_line`, `fold_sections`, `persian_calendar` | yes |
| `appearance.remote_images` | only `false`: a vault can stop pictures from the web, never turn them on |
| `calendar.place` | yes |
| `editor.live_preview`, `editor.format_on_save` | yes |
| `git.sync_interval`, `git.pull_on_launch` | yes |
| `git.name`, `git.email` (new, vault only) | the author of commits made in a clone that has no `user.name` or `user.email` of its own (section 3) |
| `notifications.*` | yes |
| `terminal.font_size` | yes |
| `home.image`, `export.logo` | only a path in the vault: relative, with no `..`, and not a link that leaves the vault once resolved |
| `export.footer`, `export.contents`, `export.landscape` | yes |
| `scratchpad.*`, `capture.*` | yes; a path is in the vault by the same rule |
| `files.hidden`, `files.show_hidden` | yes |
| `vault`, `vault_settings` | never |
| `editor.vim_mode`, `editor.neovide`, `editor.nvim`, `editor.nvim_config`, `terminal.shell` | never: they name or start a program |
| `wakatime.*` | never: it sends the notes' paths to a service |

A key the list refuses is ignored in `.daftar.toml`, and `daftar-cli doctor` warns of it by name (`vault-config-refused`), so a vault that tries is seen. A new setting is added to this table, with its line, in the same change that adds it to the template; a key not in the table is refused.

Whether a clone is read-only, its address and its key are never the vault's to say: they are one clone's, in its git config ([a read-only vault](read-only-vault.md)).

## 3. The author from the vault

`git.name` and `git.email` in `.daftar.toml` are for the phone and the new computer, whose git has no author or a work one. For each commit the app or `daftar-cli` makes:

1. the clone's own `user.name` and `user.email` (`git config --local`), when set;
2. else the vault's `git.name` and `git.email`, passed as `-c user.name=… -c user.email=…`;
3. else whatever git finds (the global config), as now.

Each of the two is chosen on its own. On Android the app's own author setting is the first of the three.

## 4. How it is shown

- **In Settings, a key the vault sets is shown with its value and "From the vault"**, and its control is off. Its tooltip names `.daftar.toml` and says how to decline (`vault_settings`).
- **Changing a setting in the app writes `config.toml`, never `.daftar.toml`.** The vault's file is changed as a note is: by hand or by an editor, then committed. So a read-only clone is never written, and two computers never commit each other's taste.
- `daftar-cli config` (new) prints every key with its value and where it came from (`vault`, `computer`, `default`), `--json` one array of `{"key","value","from"}`; a refused key of `.daftar.toml` is listed as `refused`.

## 5. Tests every app must have

Ported by name (snake case in Rust, camelCase in Kotlin). `Settings.resolved(vault:computer:)` is one pure function over the two texts:

1. `theVaultsValueWins`: computer `text_size = 14`, vault `[appearance]\ntext_size = 16` gives 16; with no vault key, 14; with neither, the default.
2. `aComputerCanDecline`: with `vault_settings = false` the computer's 14 is used; with `vault_settings = ["appearance.text_size"]` it is 14 and the vault's `theme` is still used; `[reader] folded = true` is read in both.
3. `theVaultStartsNoProgram`: vault `[terminal]\nshell = "/tmp/x"`, `[editor]\nnvim = "/tmp/x"`, `nvim_config = "x.lua"`, `vim_mode = true`, `neovide = true`, `[wakatime]\nenabled = true`, `vault = "/tmp"` and `vault_settings = true` each leave the computer's value, and each is listed as refused.
4. `theVaultOnlyTurnsWebPicturesOff`: computer `true`, vault `false` gives `false`; computer `false`, vault `true` gives `false`.
5. `aPathFromTheVaultStaysInIt`: `home.image = "uploads/home.webp"` is that file in the vault; `"/etc/passwd"`, `"~/x.png"`, `"../x.png"`, `"uploads/../../x.png"` and a link in the vault to a file outside it are refused, and the computer's value is used.
6. `aWrongValueIsAsNotThere`: vault `text_size = "big"` and `text_size = -3` give the computer's value.
7. `theAuthorIsTheClonesThenTheVaults`, on a repository with a test global config whose author is `Work <work@example.com>`: with nothing else, the commit is Work's; with the vault's `git.name` and `git.email` it is the vault's; with the clone's `user.email` set too, the email is the clone's and the name the vault's.
8. `aSettingChangedInTheAppGoesToTheComputer`: a change of `text_size` in the app writes `config.toml` and leaves `.daftar.toml` byte for byte.
