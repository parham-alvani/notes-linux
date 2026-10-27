<div align="center">
  <img src="data/me.parham1995.notes.png" width="128" alt="">
  <h1>Daftar for Linux</h1>
</div>

## Introduction

Daftar is a native Linux app for an [Obsidian](https://obsidian.md) vault that is kept in a Git repository. It is the Linux part of [Daftar for macOS](https://github.com/parham-alvani/notes-mac) and [Daftar for Android](https://github.com/parham-alvani/notes-droid), and it keeps their rules for links, tasks and the calendar week notes, and their colors (the naz palette on dark, Daylight on light).

It is written in Rust with GTK 4 and libadwaita, and draws notes with the same reader as the Mac app, in WebKitGTK. The aim is to replace Obsidian on a Linux desktop, a tiling Wayland compositor first (Hyprland, Sway, niri).

## Status

This is the first part of the port. It has:

- **Reader:** Obsidian's markdown with wikilinks, embeds, callouts, highlights, tags, footnotes, math (KaTeX), Mermaid diagrams and colored code, from `web/`, the Mac app's renderer. Each paragraph takes its own direction, so Persian text reads right to left with no plugin. Pictures, videos and sounds from the vault play in the note. A section folds shut with its heading's arrow or `z`, and stays folded the next time the note opens. The keys walk the page: `j` and `k` move the reading cursor, Space pages, Enter follows the block's link, `x` ticks its task and `e` edits the note at that block. A hover on a footnote's number shows its text. A street address or a phone number written as plain text is a link, to OpenStreetMap or to the phone (`tel:`); Linux has no data detector, so fewer addresses are found than on the Mac. The page is usable with a screen reader: landmarks, labelled controls and a live region for what changes.
- **Persian:** each block takes the direction of the text it shows, not of its syntax, so `- [x] تماس` and `[[Note|یادداشت]]` read right to left in the reader and stand on the right in the editor; a list, a quote and a table put their marks on the side their first item reads from, and a date in a Persian line keeps its order (`2026-10-30`, never `30-10-2026`). Persian is drawn in Vazirmatn in the reader and the editor when it is installed. Search finds a word however it was typed: `ي` or `ی`, `ك` or `ک`, `۱۲` or `12`, and `می‌خواهم` with the half space, a space or none. With the Persian layout on, the shortcuts are the keys in the same place (Ctrl+ح is Ctrl+P), and in Neovim mode Normal mode's keys are too (`langmap`, unless your config sets one), while Insert mode types Persian.
- **Canvas:** an Obsidian `.canvas` opens as a read-only board: text cards rendered as notes, file cards showing the note or picture, link cards, groups with their colors, and the arrows with their labels. Scroll or drag to move around, Ctrl+scroll, a pinch or the + and − buttons to zoom, Fit to see all of it; a card's link opens its note.
- **Tasks:** a click on a checkbox completes a task as the Tasks plugin does, a repeating one too, and refuses when the line changed on disk.
- **Tasks queries:** a ` ```tasks ` block lists the tasks it asks for, as the Tasks plugin does: filters on status, dates (`due before next week`, `scheduled on or before this week`), path, folder, heading, text and tags, joined with `AND`, `OR` and `NOT`; `filter by function` and `group by function` with the small part of JavaScript such lines use; `group by`, `sort by` and `limit`. A line it cannot answer is shown above the list, never dropped in silence.
- **Bases:** a `.base` file, a ` ```base ` block and `![[file.base]]` show their views as Obsidian Bases does: filters with `and`, `or` and `not`, formulas, the expression language with its functions and methods (dates, durations, lists, strings, regular expressions, `file.*`), `sort`, `groupBy` and `limit`, and the property types in `.obsidian/types.json`. A `cards` view draws a card per file with its cover (`image`: a picture in the vault, a web address or a `#hex` colour, with `imageFit`, `imageAspectRatio` and `cardSize`), and a `list` view a line per file (`markers`, `indentProperties`, `separator`). A filter or a formula it cannot read is listed, never applied in silence.
- **Sidebar:** the folder tree with folder notes; a folder's note opens with the folder. The Iconic plugin's colors and emoji show on its rows, and its Lucide icons as the nearest icon GTK's theme has. A `logo.svg` (or `logo.png`, `.webp`, `.jpg`) in a folder is that folder's icon, for example a company's logo on its folder; the file itself is not listed. Above the tree, the vault's bookmarks (`.obsidian/bookmarks.json`); Ctrl+D bookmarks the note.
- **Hidden files:** files that are not for people are not in the file tree, quick open, search, the graph, the backlinks, the recent notes, Today or the board. These are Obsidian's excluded files (`userIgnoreFilters`, for example an `uploads/` folder and all that is in it) and files for AI agents, by name in any folder: `CLAUDE.md`, `AGENTS.md`, `GEMINI.md` and `CLAUDE.local.md` at first (`files.hidden` in the settings file, or Settings › Files). A link or an embed to a hidden file still opens it or shows it. Ctrl+Shift+. (Show Hidden Files) shows them all.
- **Tags and missing links:** a tag clicked in a note or the inspector searches for it: `#tag` in the search (Ctrl+Shift+F) lists the notes with the tag or one under it. A click on a link to a note that is not there yet makes the note, beside the one it is in or at the path the link names, and opens it in the editor.
- **Files:** Ctrl+N makes a note in the current folder and opens it in the editor, Ctrl+Shift+N a folder, F2 or Ctrl+R renames, and a right click in the sidebar also moves to another folder and to the Trash. A rename or a move rewrites every link to what moved, as Obsidian's "always update links" does, and the dialog says first how many notes link there. Bookmarks and reading places follow.
- **Reading places:** a note opens where it was read last, kept in `$XDG_STATE_HOME/daftar`.
- **Inspector:** Ctrl+Alt+I, as on the Mac: the note's outline, each level in its heading's color, with the section on screen marked (a click goes to the heading); the notes that link to it; the unlinked mentions, where other notes write its name or an alias without a link, each with a Link button; the links it makes, those to no note yet in red; the addresses it writes, each opening a map; and its tags, each a search.
- **Quick open:** Ctrl+P finds a note or a file by name as Obsidian's quick switcher does: words in any order, letters that skip, aliases, and `comp/acme` for a path. Ctrl+Shift+F searches the text of every note: the words side by side come first, then the words near each other, then the notes that hold them anywhere.
- **Follows the disk:** a note changed by another app, by Neovim or by `git pull` is read again, and the one on screen is drawn again where you were reading.
- **Find:** Ctrl+F in the note, Ctrl+G and Ctrl+Shift+G for the next and the previous match.
- **Editor:** Ctrl+E edits the note on screen, colored as the Mac editor colors it, and Ctrl+E again reads it where the cursor is. Return continues a list or a task list and ends it on an empty item; Tab and Shift+Tab indent an item, or go from cell to cell of a table and line its columns up; Ctrl+B, Ctrl+I, Ctrl+Shift+H, Ctrl+Shift+X (strikethrough) and Ctrl+Shift+C (code) mark the selection, Ctrl+K links it, Ctrl+Return ticks the task on the line, Alt+Up and Alt+Down move lines. `[[` offers notes as the quick switcher ranks them, `/` offers snippets (headings, tasks, callouts, tables, today's date), brackets and marks close themselves, a web address pasted on words links them, and a pasted picture is saved in the vault's attachment folder and embedded. A picture a line embeds (`![[beach.png]]`, `![](uploads/map.jpg)`) is drawn under the line, as wide as it fits, and follows the text as you type. Ctrl+Alt+X extracts the selection to a new note in the note's folder, named after its first line, and puts a link to it in its place, as on the Mac. A file dragged from the sidebar or a file manager onto the text becomes a link where it lands, a picture an embed; a file from outside the vault is copied into the attachment folder first. A bar under the text counts the words. It saves a second after you stop typing, on Ctrl+S and when you leave, and never over a change made on disk meanwhile: it says so and offers to reload.
- **More editor keys:** Ctrl+Alt+1 to Ctrl+Alt+6 make the line a heading of that level (the same level again, or Ctrl+Alt+0, makes it plain text), Alt+Shift+Up and Alt+Shift+Down duplicate the lines, and Ctrl+Shift+K deletes them. A web page or a document pasted as rich text keeps its headings, emphasis, links, pictures, lists, quotes, code and tables as Markdown, as Obsidian converts a paste; Ctrl+Shift+V pastes the plain text.
- **Periodic notes:** Ctrl+3 opens the note of today's period, as the Daily notes plugin names it (`.obsidian/daily-notes.json`): a format with no day in it (`YYYY-[W]ww`, `YYYY-MM`) makes it a week's or a month's note. A missing one is made from the plugin's template. Ctrl+Alt+Left and Ctrl+Alt+Right go to the periodic note before and after the open one, over the gaps; on a periodic note the header bar has the same arrows, and a menu of the period's note and the same day, week or month in earlier years.
- **Scratchpad:** Ctrl+Alt+S (or "Today's Scratchpad" in the commands) opens today's scratch note, `Scratchpad/YYYY-MM-DD.md`, in the editor, and makes it empty when it is not there yet. `scratchpad.folder` in the settings file names another folder.
- **Templates:** New Note can start from a note of the Templates plugin's folder (`.obsidian/templates.json`, else `Templates/`), with `{{title}}`, `{{date}}`, `{{time}}` and `{{date:FORMAT}}` filled as Obsidian fills them.
- **Sort tasks:** "Sort Tasks (Done First)" in the command palette puts the done and cancelled tasks first in each section of the note, with the lines under them, as the vault's Task Sorter plugin does; in the editor it is a step to undo.
- **Rename a tag:** from the command palette, a tag and the tags under it (`#old/sub`) are renamed in every note, in the text and in the `tags` property; code, links and web addresses are left alone. The dialog says first how many notes have it.
- **Find and replace in the vault:** Ctrl+Shift+H (in the editor it still highlights): every match in every note, its line before and after, each one chosen or not; plain text or a regular expression with `$1`, with match case, whole word (in every script, Persian too) and code left alone. Each note is read again before the write and left alone when its matches changed since the search.
- **Postpone:** the Tasks plugin's menu, in Edit Task and on a board card: the due date, else the scheduled, else the start date, by a day to a month; a date already past moves on from today.
- **Updated since you read:** the notes a pull or another app changed since you last read them are listed above the bookmarks, the newest first, until you open them or mark them read. The app's own writes do not count.
- **Neovim mode:** with Settings › Editor › Edit with Neovim (`editor.vim_mode`), the editor is your own Neovim (`nvim --embed`, with your config, or `editor.nvim_config`): keys go to it, the view shows its buffer with the same colors and Persian text, its mode shows in the title and the bar with its command line, and Neovim writes the file. Settings shows the nvim it runs and picks your config or none; a change applies to the note being edited at once. It moves as Neovide does: the cursor glides to where Neovim puts it (a block in Normal mode, a bar while inserting), and the page scrolls smoothly with Neovim's jumps; the wheel scrolls Neovim's window too, bringing the cursor along. Plain Ctrl keys (Ctrl+O, Ctrl+R, Ctrl+W, Ctrl+[) are Neovim's while it edits; Ctrl+E, Ctrl+S, Ctrl+Q and the Ctrl+Shift and Ctrl+Alt shortcuts stay with the app. Neovim's completion menu (Ctrl+N, `:e <Tab>`) shows at the cursor or over the command line, and a message of more lines (`:ls`, `:version`, an error) shows whole until the next key. Plugins that draw their own floating windows (blink.cmp, Telescope, LSP hover) do not show: the editor draws the note, not Neovim's screen.
- **Neovide:** "Open in Neovide" in the commands (Ctrl+Shift+P) opens the note in Neovide at the line you were reading, with your own config; the editor here closes first, and what Neovide saves shows in the reader. The command shows when `neovide` is installed.
- **Today:** Ctrl+1, the meetings from the phone's week note in `Calendar/` (now, next and over, the free time, overlaps, and a Join button for a web link), then the overdue tasks, the ones due today, the ones in progress and the next seven days, in cards; a box ticks a task, its text opens it.
- **Board:** Ctrl+2, the tasks in columns by status, by day or by note, filtered by note or by words. A card dragged to another status or day column changes its line as the Tasks plugin writes it, and only while the line is still the task the card showed; a click opens the task, a right click edits or moves it.
- **Edit Task:** a right click on a task, in a note, on Today or on the board: the text, the status, the priority, the due, scheduled and start dates, and the repeat rule. The line is read again before the save, which is refused when it changed.
- **Git sync:** a button in the sidebar shows the branch and what waits to be pushed, pulled or committed; Ctrl+Shift+S commits, pulls with a rebase and pushes, as `daftar-cli sync` does. It pulls at launch (`git.pull_on_launch`), syncs every `git.sync_interval` minutes when that is set, and shortly after a task is ticked.
- **Tabs:** Ctrl+T opens a tab on the note on screen, or on the home page, as on the Mac; Ctrl+click or a middle click on a link, and "Open in New Tab" in the sidebar's menu, open the note in a new tab. Ctrl+W closes a tab and the last one closes the window; Ctrl+Tab, Ctrl+Page Up and Down and Alt+1 to 9 go from tab to tab. Each tab has its own history, editor, Today and board, and the tabs come back at the next launch.
- **Split view:** Ctrl+\ shows a second note pane beside the note, as the Mac's Split Right does, and again closes it; each pane has its own note, editor and history, and the header, the commands, Back and Forward and the inspector follow the focused one, marked by a line on top. Alt+click on a link, or "Open in Split" in the sidebar's menu, opens the note in the other pane; Ctrl+Alt+\ moves the focus there. The editor saves when its pane loses the focus and takes the file again when it gets it back, so one note edited in both panes loses nothing; in Neovim mode only one pane edits. A split comes back with its tab at the next launch.
- **Home:** a tab with no note shows the time, a greeting, a field that opens the best match (or Quick Open with the text), the notes read last, the bookmarks and a quote, over your picture (`home.image`, chosen in Settings). The Home button beside Back and Forward, Ctrl+Shift+D or Alt+Home shows it in the pane, and Back returns to the note. It replaces Obsidian's Beautitab new tab and follows its settings in the vault.
- **Back and Forward:** Ctrl+[ and Ctrl+], or Alt+Left and Alt+Right, in the tab. Ctrl+= and Ctrl+- set the text size, Ctrl+0 resets it.
- **Command palette:** Ctrl+Shift+P, every command by name, matched as the switcher matches.
- **Graph:** Ctrl+Alt+G, the notes a few links from the open one, or the whole vault, each top folder in its color as on the Mac; drag to move, scroll to zoom, click a note to open it.
- **Terminal:** Ctrl+\`, your login shell under the note, started in the vault's folder, in Daftar's colors (`terminal.shell`, `terminal.font_size`).
- **Sync conflicts:** when a pull stops on a conflict, Daftar keeps the rebase open, stops every sync and shows the conflicted files, with a ⚠ badge on the git button. For a note, each conflict shows the lines on the remote beside your change; Keep Remote, Keep Mine or Keep Both (yours after the remote's) for each conflict or for the whole note, or Edit opens the note with the conflict marks. Other files keep one side whole, and a deletion is a side. Finish Sync commits the result, goes on with the rebase and pushes; Abort Sync puts your commits back as they were before the pull. A rebase left open is shown when the app starts. `daftar-cli sync` lists the files and refuses too, and `daftar-cli sync --abort` aborts.
- **History:** Ctrl+Alt+Y lists the commits that changed the note, across renames, with the lines each one added and removed or the whole text then, and restores an older text; the restore is an ordinary edit, so the next sync commits it and nothing in the history is lost. `daftar-cli history` lists them too.
- **Quick capture:** `daftar --capture` opens a small window from any app, in the running Daftar or on its own: Return adds the line to today's daily note (made from the Daily notes plugin's template when it is not there), Ctrl+Return adds it as a task. The note and a heading to add it under are `[capture]` settings. Bind it in the compositor and float the window, in Hyprland `bind = CTRL ALT, N, exec, daftar --capture` and `windowrule = float, title:^(Quick Capture)$`. `daftar-cli capture` adds a line from a shell.
- **Read aloud:** Ctrl+Alt+R reads the note from the reading cursor on, a paragraph at a time, with speech-dispatcher (`spd-say`) or `espeak-ng`, and a Persian voice for a Persian paragraph. Escape on the page or Ctrl+Alt+R again stops.
- **Export as PDF:** Ctrl+Shift+E prints the note on screen to an A4 PDF where you choose.
- **WakaTime:** a setting (off at first) sends the time spent reading and writing each note to WakaTime with `wakatime-cli` and the key in `~/.wakatime.cfg`, as Obsidian's WakaTime plugin does.
- **Notifications:** a meeting alert a few minutes before each meeting in the week note, with Join, and a morning digest of the tasks due (`[notifications]`). Unlike on the Mac, they ring while Daftar runs.
- **Settings:** Ctrl+, for the theme, the text size, the fonts, the home picture and git, written back to `config.toml` line by line. Ctrl+Alt+N opens another window.
- **Command line:** `daftar-cli`, below.
- **Core:** the parser, the link resolver, the task lines, the vault index, the quick switcher's ranking, full-text search (SQLite FTS5) and the config file, in `daftar-core`, with the Mac app's tests ported.


## Command line

`daftar-cli` reads the same vault with the same rules, for a shell, a script or a launcher. It finds the vault as git finds a repository, from the folder it runs in (the nearest one up with `.obsidian`); elsewhere it takes `--vault`, `$DAFTAR_VAULT` or `vault` in the config file.

Unlike the app, it needs no GTK and runs on Linux, macOS and Windows: each [release](https://github.com/parham-alvani/notes-linux/releases) has it for Linux (x86_64, arm64), macOS (Apple silicon, Intel) and Windows (x86_64). From the source, `cargo install --path crates/daftar-cli` or `just install-cli` builds it anywhere Rust does. On Windows the config file is in `%USERPROFILE%\.config\daftar\config.toml`.

With Homebrew, on macOS or Linux: `brew install parham-alvani/tap/daftar-cli`.

On Arch, from the AUR: `yay -S daftar-bin` for the app and the command line, or `yay -S daftar-cli-bin` for the command line alone (x86_64, aarch64).

```bash
daftar-cli find kub fire          # notes by name, as the quick switcher ranks them
daftar-cli search "backup plan"   # the text of every note
daftar-cli tasks --due today      # open tasks due or scheduled by today, as path:line
daftar-cli query 'not done' 'due before next week' 'group by filename'
daftar-cli query --note Tasks/Tasks.md  # the note's tasks blocks, as the app draws them
daftar-cli toggle Inbox 12        # ticks the task on line 12 of Inbox, as a click does
daftar-cli postpone Inbox 12 week # moves its date on, as the Tasks plugin's Postpone does
daftar-cli sort-tasks Inbox       # done and cancelled tasks first in each section
daftar-cli daily                  # today's periodic note, made from its template when missing
daftar-cli rename-tag project work --dry-run   # the notes #project (and #project/…) is in
daftar-cli replace 'Ana' 'Sara' --whole-word --dry-run  # each match's line, before → after
daftar-cli backlinks Kubernetes   # the notes that link to it
daftar-cli broken                 # links that go nowhere
daftar-cli tags                   # every tag, with its count
daftar-cli sync                   # commit, pull with a rebase, push: obsidian-git's workflow
daftar-cli sync --abort           # end a sync stopped on a conflict, as before the pull
daftar-cli status                 # the branch, ahead and behind, and the files changed
daftar-cli history Plan.md        # the commits that changed a note; --show <hash> prints its text then
daftar-cli capture -t Call Sara   # a task in today's daily note, as quick capture adds it
daftar-cli tui                    # the vault in the terminal; `daftar-cli` alone does the same
daftar-cli images --dry-run       # the picture cleanup of notes-cli
```

`daftar-cli tui`, or `daftar-cli` alone on a terminal, opens the vault in the terminal, in Daftar's colors (naz on dark, Daylight on light, as `appearance.theme` says): the folder tree, the note drawn with its headings, lists, tasks, callouts, tables, code and properties, and a status bar with the git branch. `Ctrl+P` or `p` opens a note by name, `/` searches the text, `t` lists the note's tasks queries (every open task when it has none), `b` its backlinks and `#` the tags. `j`/`k` move, `]`/`[` jump to the next link or task, `Enter` follows a link, `Backspace` goes back, `H` shows the home page (the time, a greeting, the notes read here and in the app last, the bookmarks and a quote, as the app's home page and the vault's Beautitab settings have them), `Space` ticks a task as a click does, `e` opens the note in `$EDITOR`, `s` syncs, `?` lists every key and `q` quits. The mouse works too: a click opens a note from the tree or a list and ticks a task's box, a second click on the line under the cursor follows its link, and the wheel scrolls. The vault is read and synced on a worker thread behind a spinner, so the keys keep working, and it is watched: a note changed on disk, by an editor, Obsidian or a `git pull`, shows at once, the cursor where it was.

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
