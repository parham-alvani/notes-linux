<div align="center">
  <img src="data/me.parham1995.notes.png" width="128" alt="">
  <h1>Daftar for Linux</h1>
</div>

## Introduction

Daftar is a native Linux app for an [Obsidian](https://obsidian.md) vault that is kept in a Git repository. It is the Linux part of [Daftar for macOS](https://github.com/parham-alvani/notes-mac) and [Daftar for Android](https://github.com/parham-alvani/notes-droid), and it keeps their rules for links, tasks and the calendar week notes, and their colors (the naz palette on dark, Daylight on light).

It is written in Rust with GTK 4 and libadwaita, and draws notes with the same reader as the Mac app, in WebKitGTK. The aim is to replace Obsidian on a Linux desktop, a tiling Wayland compositor first (Hyprland, Sway, niri).

## Status

This is the first part of the port. It has:

- **Reader:** Obsidian's markdown with wikilinks, embeds, callouts, highlights, tags, footnotes, math (KaTeX), Mermaid diagrams and colored code, from `web/`, the Mac app's renderer. Each paragraph takes its own direction, so Persian text reads right to left with no plugin. Pictures, videos and sounds from the vault play in the note.
- **Tasks:** a click on a checkbox completes a task as the Tasks plugin does, a repeating one too, and refuses when the line changed on disk.
- **Sidebar:** the folder tree with folder notes; a folder's note opens with the folder.
- **Find:** Ctrl+F in the note, Ctrl+G and Ctrl+Shift+G for the next and the previous match.
- **Command line:** `daftar-cli`, below.
- **Core:** the parser, the link resolver, the task lines, the vault index, the quick switcher's ranking, full-text search (SQLite FTS5) and the config file, in `daftar-core`, with the Mac app's tests ported.

Next come search and quick open in the window, file watching, tasks queries and Bases, the Today page, Git sync, the editor and the Neovim mode.

## Command line

`daftar-cli` reads the same vault with the same rules, for a shell, a script or a launcher. It takes the vault from the config file, or from `--vault` or `$DAFTAR_VAULT`.

```bash
daftar-cli find kub fire          # notes by name, as the quick switcher ranks them
daftar-cli search "backup plan"   # the text of every note
daftar-cli tasks --due today      # open tasks due or scheduled by today, as path:line
daftar-cli toggle Inbox 12        # ticks the task on line 12 of Inbox, as a click does
daftar-cli backlinks Kubernetes   # the notes that link to it
daftar-cli broken                 # links that go nowhere
daftar-cli tags                   # every tag, with its count
daftar-cli images --dry-run       # the picture cleanup of notes-cli
```

`images` is [notes-cli](https://github.com/parham-alvani/notes-cli) in Rust: it removes the pictures in `uploads/` that no note names, makes the others JPEGs under 1 MB named after their note and their content's hash, and rewrites the notes to the new names. Unlike notes-cli, a converted picture is named by the hash of the JPEG it became, so a second run changes nothing.

## Configuration

Every setting is in one file, `~/.config/daftar/config.toml`. The app makes it on the first start, with a comment on each setting. It is the same file Daftar for macOS reads, so one copy in your dotfiles serves both; each app leaves alone the keys it does not have.

`daftar Notes/Inbox.md` opens that note, for a keybinding in the compositor or a script.

## Build

The build needs Rust, GTK 4, libadwaita, WebKitGTK 6.0, Node.js and [just](https://github.com/casey/just). On Arch Linux:

```bash
pacman -S rust gtk4 libadwaita webkitgtk-6.0 nodejs npm just
```

```bash
just test     # the Rust tests and the renderer tests
just run      # build and start it
just install  # into ~/.local: daftar, daftar-cli, the desktop entry and the icon
```

## License

GPL-3.0. See [LICENSE](LICENSE).
