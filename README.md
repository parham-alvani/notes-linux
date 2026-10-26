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
- **Tasks queries:** a ` ```tasks ` block lists the tasks it asks for, as the Tasks plugin does: filters on status, dates (`due before next week`, `scheduled on or before this week`), path, folder, heading, text and tags, joined with `AND`, `OR` and `NOT`; `filter by function` and `group by function` with the small part of JavaScript such lines use; `group by`, `sort by` and `limit`. A line it cannot answer is shown above the list, never dropped in silence.
- **Bases:** a `.base` file, a ` ```base ` block and `![[file.base]]` show their views as Obsidian Bases does: filters with `and`, `or` and `not`, formulas, the expression language with its functions and methods (dates, durations, lists, strings, regular expressions, `file.*`), `sort`, `groupBy` and `limit`, and the property types in `.obsidian/types.json`. A filter or a formula it cannot read is listed, never applied in silence.
- **Sidebar:** the folder tree with folder notes; a folder's note opens with the folder.
- **Quick open:** Ctrl+P finds a note or a file by name as Obsidian's quick switcher does: words in any order, letters that skip, aliases, and `comp/acme` for a path. Ctrl+Shift+F searches the text of every note.
- **Follows the disk:** a note changed by another app, by Neovim or by `git pull` is read again, and the one on screen is drawn again where you were reading.
- **Find:** Ctrl+F in the note, Ctrl+G and Ctrl+Shift+G for the next and the previous match.
- **Editor:** Ctrl+E edits the note on screen, colored as the Mac editor colors it, and Ctrl+E again reads it where the cursor is. Return continues a list or a task list and ends it on an empty item; Tab and Shift+Tab indent an item, or go from cell to cell of a table and line its columns up; Ctrl+B, Ctrl+I and Ctrl+Shift+H mark the selection, Ctrl+K links it, Ctrl+Return ticks the task on the line, Alt+Up and Alt+Down move lines. `[[` offers notes as the quick switcher ranks them, `/` offers snippets (headings, tasks, callouts, tables, today's date), brackets and marks close themselves, a web address pasted on words links them, and a pasted picture is saved in the vault's attachment folder and embedded. A bar under the text counts the words. It saves a second after you stop typing, on Ctrl+S and when you leave, and never over a change made on disk meanwhile: it says so and offers to reload.
- **Neovim mode:** with `editor.vim_mode = true`, the editor is your own Neovim (`nvim --embed`, with your config, or `editor.nvim_config`): keys go to it, the view shows its buffer with the same colors and Persian text, its mode shows in the title and the bar with its command line, and Neovim writes the file.
- **Today:** Ctrl+1, the meetings from the phone's week note in `Calendar/` (now, next and over, the free time, overlaps, and a Join button for a web link), then the overdue tasks, the ones due today, the ones in progress and the next seven days, in cards; a box ticks a task, its text opens it.
- **Board:** Ctrl+2, the tasks in columns by status, by day or by note, filtered by note or by words. A card dragged to another status or day column changes its line as the Tasks plugin writes it, and only while the line is still the task the card showed; a click opens the task, a right click edits or moves it.
- **Edit Task:** a right click on a task, in a note, on Today or on the board: the text, the status, the priority, the due, scheduled and start dates, and the repeat rule. The line is read again before the save, which is refused when it changed.
- **Git sync:** a button in the sidebar shows the branch and what waits to be pushed, pulled or committed; Ctrl+Shift+S commits, pulls with a rebase and pushes, as `daftar-cli sync` does. It pulls at launch (`git.pull_on_launch`), syncs every `git.sync_interval` minutes when that is set, and shortly after a task is ticked.
- **Back and Forward:** Ctrl+[ and Ctrl+], or Alt+Left and Alt+Right. Ctrl+= and Ctrl+- set the text size, Ctrl+0 resets it.
- **Command palette:** Ctrl+Shift+P, every command by name, matched as the switcher matches.
- **Graph:** Ctrl+Alt+G, the notes a few links from the open one, or the whole vault, each top folder in its color as on the Mac; drag to move, scroll to zoom, click a note to open it.
- **Terminal:** Ctrl+\`, your login shell under the note, started in the vault's folder, in Daftar's colors (`terminal.shell`, `terminal.font_size`).
- **Notifications:** a meeting alert a few minutes before each meeting in the week note, with Join, and a morning digest of the tasks due (`[notifications]`). Unlike on the Mac, they ring while Daftar runs.
- **Settings:** Ctrl+, for the theme, the text size, the fonts and git, written back to `config.toml` line by line. Ctrl+Alt+N opens another window.
- **Command line:** `daftar-cli`, below.
- **Core:** the parser, the link resolver, the task lines, the vault index, the quick switcher's ranking, full-text search (SQLite FTS5) and the config file, in `daftar-core`, with the Mac app's tests ported.

Next come file actions (new, rename, move, delete) and bookmarks.

## Command line

`daftar-cli` reads the same vault with the same rules, for a shell, a script or a launcher. It finds the vault as git finds a repository, from the folder it runs in (the nearest one up with `.obsidian`); elsewhere it takes `--vault`, `$DAFTAR_VAULT` or `vault` in the config file.

Unlike the app, it needs no GTK and runs on Linux, macOS and Windows: each [release](https://github.com/parham-alvani/notes-linux/releases) has it for Linux (x86_64, arm64), macOS (Apple silicon, Intel) and Windows (x86_64). From the source, `cargo install --path crates/daftar-cli` or `just install-cli` builds it anywhere Rust does. On Windows the config file is in `%USERPROFILE%\.config\daftar\config.toml`.

With Homebrew, on macOS or Linux: `brew install parham-alvani/tap/daftar-cli`.

```bash
daftar-cli find kub fire          # notes by name, as the quick switcher ranks them
daftar-cli search "backup plan"   # the text of every note
daftar-cli tasks --due today      # open tasks due or scheduled by today, as path:line
daftar-cli query 'not done' 'due before next week' 'group by filename'
daftar-cli query --note Tasks/Tasks.md  # the note's tasks blocks, as the app draws them
daftar-cli toggle Inbox 12        # ticks the task on line 12 of Inbox, as a click does
daftar-cli backlinks Kubernetes   # the notes that link to it
daftar-cli broken                 # links that go nowhere
daftar-cli tags                   # every tag, with its count
daftar-cli sync                   # commit, pull with a rebase, push: obsidian-git's workflow
daftar-cli status                 # the branch, ahead and behind, and the files changed
daftar-cli tui                    # the vault in the terminal; `daftar-cli` alone does the same
daftar-cli images --dry-run       # the picture cleanup of notes-cli
```

`daftar-cli tui`, or `daftar-cli` alone on a terminal, opens the vault in the terminal, in Daftar's colors (naz on dark, Daylight on light, as `appearance.theme` says): the folder tree, the note drawn with its headings, lists, tasks, callouts, tables, code and properties, and a status bar with the git branch. `Ctrl+P` or `p` opens a note by name, `/` searches the text, `t` lists the note's tasks queries (every open task when it has none), `b` its backlinks and `#` the tags. `j`/`k` move, `]`/`[` jump to the next link or task, `Enter` follows a link, `Backspace` goes back, `Space` ticks a task as a click does, `e` opens the note in `$EDITOR`, `s` syncs, `?` lists every key and `q` quits.

`images` is [notes-cli](https://github.com/parham-alvani/notes-cli) in Rust: it removes the pictures in `uploads/` that no note names, makes the others JPEGs under 1 MB named after their note and their content's hash, and rewrites the notes to the new names. Unlike notes-cli, a converted picture is named by the hash of the JPEG it became, and a picture already named `{name}_{hash}` keeps its name when its hash is corrected, so a second run changes nothing.

`sync` commits with obsidian-git's message (`update notes @ 2026-10-25 09:30:00 on x-wing`, or `-m`), pulls with `--rebase --autostash` and pushes, through the system `git`: your git config, credential helper and SSH agent apply, and it never waits on a password prompt.

## Configuration

Every setting is in one file, `~/.config/daftar/config.toml`. The app makes it on the first start, with a comment on each setting. It is the same file Daftar for macOS reads, so one copy in your dotfiles serves both; each app leaves alone the keys it does not have.

`daftar Notes/Inbox.md` opens that note, for a keybinding in the compositor or a script.

## Build

The build needs Rust, GTK 4, libadwaita, WebKitGTK 6.0, GtkSourceView 5, VTE, Node.js and [just](https://github.com/casey/just). On Arch Linux:

```bash
pacman -S rust gtk4 libadwaita webkitgtk-6.0 gtksourceview5 vte4 nodejs npm just
```

```bash
just test     # the Rust tests and the renderer tests
just run      # build and start it
just install  # into ~/.local: daftar, daftar-cli, the desktop entry and the icon
```

## License

GPL-3.0. See [LICENSE](LICENSE).
