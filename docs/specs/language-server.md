# The language server: Daftar's rules in any editor

`daftar-cli lsp` is a Language Server Protocol server for the vault, so Neovim, Helix or any editor with a client gets the lint, the audit, formatting, link completion, a link's note, hover, references and renaming a note with its links, by the same rules as the app. It is `daftar-cli`'s, in Rust (`crates/daftar-cli/src/lsp.rs`); Daftar for macOS runs `daftar-cli` and so has it too, and Android, where `daftar-cli` cannot run, has none.

## 1. Running it

`daftar-cli lsp` speaks the base protocol on stdin and stdout: each message is a `Content-Length` header, a blank line and a JSON-RPC 2.0 body. It answers in order, one message at a time, with no thread of its own; what it has to say beside an answer (diagnostics) goes out after it. It logs to stderr. The vault is found as for every command: `--vault`, else the vault the current folder is in (the nearest folder up with `.obsidian`), else `vault` in `~/.config/daftar/config.toml`; the client's `rootUri` is not read. `--hidden` lists the hidden files too, as elsewhere.

In Neovim:

```lua
vim.lsp.start({ name = 'daftar', cmd = { 'daftar-cli', 'lsp', '--vault', vim.fn.expand('~/org') } })
```

The lifecycle is the protocol's: `initialize` builds the index (`Vault::index`) and reads `.markdownlint.jsonc`; a request before it is refused with `-32002`, one after `shutdown` with `-32600`; `exit` ends the server, as the end of stdin does. A method it does not have is `-32601`; a notification it does not have is ignored, as are the client's answers to its own requests.

## 2. What it offers

`initialize` answers these capabilities, and `serverInfo.name` is `daftar`:

| Capability | Value |
| --- | --- |
| `positionEncoding` | `utf-16` |
| `textDocumentSync` | `openClose`, `change: 1` (the whole text), `save` without the text |
| `documentFormattingProvider` | true |
| `completionProvider` | trigger characters `[` and `#` |
| `definitionProvider`, `referencesProvider`, `hoverProvider` | true |
| `renameProvider` | with `prepareProvider` |

When the client can register file watching (`workspace.didChangeWatchedFiles.dynamicRegistration`), the server asks, after `initialized`, to be told of every file (`**/*`). A file the watcher names, and a note saved, is read again into the index; a change to `.markdownlint.jsonc` reads the whole index and the lint's config again.

Only a file in the vault is served: a URI outside it, or not `file:`, gets no diagnostics and a request on it fails. Only a note (`.md`) is kept open and linted.

## 3. Positions

A position is a line from 0, the text split at `\n`, and a character that is a **UTF-16 code unit**, as the protocol has it when no other encoding is agreed. A Persian letter is one unit; an emoji past U+FFFF (😀) is two. The core counts characters and the lint's column is a character from 1, so each is turned into units on its line's text. A unit inside a character (between the two halves of an emoji) is that character's start; a unit past the end of the line is its end. A URI is the vault's folder, absolute as given, joined with the vault path and percent-encoded but for letters, digits, `-`, `.`, `_`, `~` and `/`; a URI the client sends is decoded, and its path is taken from the vault's folder as given, else with symlinks resolved.

## 4. Diagnostics

A note's diagnostics are published when it opens, at each change and at each save, with the version of the client's text; they are emptied when it closes.

- **The lint** runs on the client's text at every change: `MarkdownLint` with the vault's `.markdownlint.jsonc` (or `.markdownlint.json`, else every rule), leaving out a note `.markdownlintignore` names. Severity 1 (error), `source` `daftar lint`, `code` the rule as `lint --json` writes it (`MD009/no-trailing-spaces`), the message its description and detail. The range is the problem's column to the end of its line, or the whole line with no column.
- **The audit** runs on the note as saved, when it opens and at each save: `check` with the vault's `.daftar.toml` (`[check]`), only this note, without the web, the archive or the figures. Severity 1 for an error, 2 for a warning, `source` `daftar check`, `code` the kind (`dangling-link`), the range the whole line (line 1 for a finding on the whole file). Between saves the audit's findings stay where they were.

## 5. Formatting

`textDocument/formatting` formats the client's text as `fmt --stdin --path=<note>` does: `MarkdownFormat` with the vault's `.prettierrc`, unchanged when `.prettierignore` names the note. The answer is one edit, the whole lines between what the two texts share at their start and at their end, or none when the note is formatted. The request's options (tab size) are not read: `.prettierrc` is.

## 6. Completion

Completion looks at the line before the cursor for the last `[[` with no `]]` after it.

- `[[query` offers files as the quick switcher ranks them (`QuickSwitcher::rank`, the names, aliases and folders, the hidden files left out), 50 at most; with nothing typed yet, the notes changed last. Each item's label is the file's name, its detail the vault path, and it replaces what was typed after `[[` with the name when no other file has it, else the path without `.md`. The list is incomplete, so the client asks again as more is typed; `filterText` is what was typed and `sortText` the rank, so the client keeps the server's order.
- `[[Note#query` offers the headings of the note `Note` goes to (this note's own, from the client's text, for `[[#`) whose text holds the query, folded, in the note's order; an item replaces what was typed after `#`.
- After `|` (the alias) or `#^` (a block) nothing is offered.

## 7. Links

A link is `[[target#anchor|alias]]`, `![[embed]]`, or `[text](path#anchor)` with no scheme, not in a code span; links in fenced code are not counted. Its note is the one the index's resolver gives from this note (aliases included); an empty target is this note.

- **Definition** goes to the link's file, at the line of its heading (compared as `check` compares them: the marks left out and the case folded, or as a `#slug`; `[[Note#A#B]]` is `B`) or its block (`^id` at the end of a line), else its first line. A link that goes nowhere has no definition.
- **Hover** on a link shows, under the file's vault path, the first 15 lines of its heading's section, of its block's line, or of the note without its front matter, with `…` when there is more. A file that is not a note shows its path.
- **References** at a link are the links to its note, else the links to this note: every link in every note the index has as a backlink, at the link's range, the hidden notes left out; with `includeDeclaration`, the note's first line comes first.

## 8. Renaming

`textDocument/prepareRename` on a link to a note marks the link's target and offers the note's name; anywhere else in a note it offers this note's name. A link that goes nowhere, or a note not saved in the vault yet, cannot be renamed.

`textDocument/rename` with a new name moves the note, and every link to it follows, as the app's Rename does (`VaultMoves::plan` and `VaultMoves::relinked`). A name stays in the note's folder; a name with `/` is a path from the vault's folder; `.md` is added. A name that is empty, has one of `/ \ : # ^ [ ] |`, starts with `.`, or goes up with `..`, and a name another file has, are refused. The server writes nothing: the answer is one workspace edit with `documentChanges`, first a `rename` of the file, then for each note whose links change the one edit of its changed lines (on the client's text and version when it is open, `null` otherwise; the moved note's own edit is on its new URI), and the vault's bookmarks (`.obsidian/bookmarks.json`) when one follows the note. A client without `rename` in `workspace.workspaceEdit.resourceOperations` is refused with `-32803` and a message saying so.

## 9. Tests every implementation must have

1. Framing: messages with `Content-Length` (and another header) are answered in order, and nothing is read after `exit`.
2. A request before `initialize` is `-32002`, after `shutdown` `-32600`; an unknown method `-32601`; a body that is not JSON `-32700`.
3. With dynamic registration, `initialized` asks to watch the files; the client's answer is taken quietly.
4. A note opened with `سلام 😀 ` (a trailing space) has `MD009/no-trailing-spaces` at character 7, and its `[[Nowhere]]` a `dangling-link` warning; a change is linted at once, the audit stays until the save; a saved `[[Seeds]]` that names two notes is an `ambiguous-link`; closing empties the diagnostics.
5. Formatting `# Note\n* one\n* two\n\nend\n` is one edit, lines 1 to 3, `\n- one\n- two\n`; a formatted note has none.
6. `سلام 😀 [[pla` completes `Plants` first, replacing characters 10 to 13; a name two notes share is offered with its folder; `[[Plants#` offers its headings in order; `[[#` this note's.
7. Definition on `[[Plants#Mint]]` after the emoji goes to the heading's line, on `#^sun` to the block's, on a markdown link `#basil` by its slug; on a link that goes nowhere, and off a link, none.
8. Hover shows a heading's section, or the note.
9. References on a note list each link to it, two in one line, at their ranges.
10. Rename on `[[Plants#Mint]]` to `Herbs`: the range is the target, the placeholder `Plants`; the edit renames `Garden/Plants.md` to `Garden/Herbs.md` and rewrites the links of the open note (with its version), of a closed one (`null`), and the bookmarks; nothing is written. Anywhere else it renames this note; a path moves it; a bad name and a taken one are refused; a client without resource operations is refused.
11. UTF-16: Persian, an emoji, a unit between an emoji's halves and past the end of a line convert both ways; a URI with Persian and a space is a vault path again, and one outside the vault is none.
