# Daftar: notes that open folded

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). In the reading view a section folds shut from its heading. A vault of long reference notes is easier to enter when a note opens as its own table of contents: every section shut, and each heading saying what is under it. This spec adds that as a choice of the vault, which an app can override, and the words a folded heading says.

The notes are not changed: a fold is how a note is shown, never a mark in its Markdown, so Obsidian, `daftar-cli` and anything else that reads the vault still read the whole note.

## 1. The vault's setting

```toml
[reader]
folded = true
```

`reader.folded` in the `.daftar.toml` at the vault's root is `false` (the default: a note opens with every section open, as before) or `true` (a note opens with every section shut). Anything else, and a file or a key that is not there, is `false`. The file is the one `daftar-cli` reads its `[check]` and `[audit]` sections from; `daftar-cli` ignores `[reader]`. Because the file is in the vault, every device that opens the vault follows it.

An app reads the file when it opens the vault and again when the file changes (a pull, an edit), and draws the note on screen again.

## 2. The app's setting

```toml
[appearance]
fold_sections = "vault"
```

`appearance.fold_sections` is `"vault"` (the default: as the vault says), `"always"` or `"never"`. Anything else is `"vault"`. It is a line in each app's config template, after `remote_images`:

```toml
# A note opens with its sections folded, each heading saying what is
# under it: "vault" as the vault says (reader.folded in its
# .daftar.toml), "always" or "never". A note whose sections you fold
# or open yourself is kept as you left it.
fold_sections = "vault"
```

Android, which has no config file, has the same three choices in its settings.

## 3. Which notes open folded

Each app already keeps, for each note, the sections the person left shut (a list of `level:text` keys). The rule adds one distinction: a note with **no list kept** and a note with **an empty list kept**.

- A note with no list kept opens as the settings say: every section shut when notes open folded, every section open otherwise.
- A note with a list kept opens with exactly those sections shut, whatever the settings say. The person's own folds win.
- The list is kept at every fold or unfold the person makes, and when the page opens a fold to show something (a link to a heading, the reading place, a line from the editor, find). While notes open folded an empty list is kept too, so a note the person opened whole stays open the next time. While notes open unfolded an empty list is not kept (it is the default), as before.
- "Every section shut" is every heading with something under it before the next heading as big or bigger, at every level. So the note shows its top headings only, and opening one shows its text and its own headings, shut.
- A note opens where it was being read. When that place is inside a shut section, the sections that hold it are opened, as for any place the page is asked to show.
- Text that is not a note's file (a card of Review Cards, a preview) is never folded.

## 4. What a folded heading says

A heading that is shut and on the page says, after its text, what is under it, small and faint, on the heading's line:

```text
How the text begins… · 240 words · 3 sections
```

- **How the text begins**: the text of the first block under the heading that has any (not a heading), with white space collapsed. Up to 64 characters; a longer one is cut at the last space before that, any `.,;:!?،؛` or space at the cut is dropped, and `…` is added. A single word longer than 64 characters is cut at 64.
- **Words**: the words of every block under the heading down to the next heading as big or bigger, without the headings' own text. Left out when the beginning shows all of them. `1 word`, `2 words`.
- **Sections**: the headings under it, at any depth. Left out when there are none. `1 section`, `2 sections`.
- The parts are joined with ` · `. A section with no text and no headings says `…`.
- The hint is not part of the heading's name: a link copied from the heading, the outline, and read aloud leave it out. It is not printed.

The page is shared (`web/src`), so this is written once: `foldHint(first, words, sections)` in `controls.js`, pure, and `folds.hint(heading)` in `reader.js`, which counts. The host sends `folds` (the kept list, or `null` when none is kept) and `folded` (whether notes open folded) with `daftar.show(note)`; the page shuts every section when `folds` is `null` and `folded` is true.

## 5. Tests every app must have

The page's tests (`web/test/folds.test.mjs`) cover the words:

| `first` | `words` | `sections` | Result |
| --- | --- | --- | --- |
| `Coffee and tea are fine.` | 240 | 3 | `Coffee and tea are fine. · 240 words · 3 sections` |
| `One line only.` | 3 | 0 | `One line only.` |
| `One line only.` | 3 | 1 | `One line only. · 1 section` |
| `Because it is a mail drop it cannot carry the empadronamiento, which needs the address we sleep at.` | 18 | 0 | `Because it is a mail drop it cannot carry the empadronamiento… · 18 words` |
| eighty `x` | 1 | 0 | sixty-four `x`, then `… · 1 word` |
| empty | 0 | 4 | `4 sections` |
| empty | 0 | 0 | `…` |

Each host has tests that:

- `reader.folded` is read from `.daftar.toml` (`false` when the file or the key is not there, and when the key is in another section), beside `[check]` and `[audit]`.
- `appearance.fold_sections` is read (`"vault"` when absent or unknown) and decides with the vault's word.
- No list kept and an empty list kept are told apart, and still are after the places are written and read.
- In the page: a note with no list kept opens with every heading that holds something shut and only its top headings on the page; opening one shows the next level shut and keeps the list; a note with an empty list kept opens whole; with `"never"` nothing is shut.
