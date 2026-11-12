# Send to Daftar: a selection from another app, to the scratchpad or as a task

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Two actions take text from any other app without switching to Daftar first; both write through the app's existing paths (the scratchpad and quick capture's append, the New Task form), so a line sent from a browser is written exactly as one typed in the app.

## 1. The two actions

| Action | Takes | Does |
| --- | --- | --- |
| **Add to Daftar Scratchpad** | the selected text, or a link | adds one line to the scratchpad, under today's section, and shows nothing |
| **New Daftar Task** | the selected text | brings Daftar forward with the **New Task** form (`new-task.md`) open, its text filled in; the person picks the note and section and adds it |

Both work **with no Daftar window open** and when the action is what starts the app: the vault is waited for while it loads (up to a minute), as quick capture does. Neither writes anything the person did not send.

## 2. What the selection becomes

- The text is **one line**: trimmed, its line breaks single spaces, as quick capture's line (`QuickCapture.line`).
- **A bare web address** (`http`, `https` or `mailto`, with a host for the first two, and nothing else) is written in angle brackets, `<https://example.com/a?b=1#c>`, as the vault's lint wants; brackets already there are not doubled. `file:`, `javascript:` and the like are not addresses: a `file:` link alone adds nothing, and text that only looks like one is words.
- **A selection that came with its link** (a link selected in a browser: the text and the URL on the pasteboard / in the share intent) is a Markdown link, `[Read the RFC](https://www.rfc-editor.org/rfc/rfc9110)`, a `]` in the text escaped as `\]`. The same address as text and link is one `<address>`.
- **To the scratchpad**, the line is a list item: `- ` before it, unless it is one already. **As a task**, a selected list item or task loses its marks (`- [ ] Pay rent` and `* Pay rent` start the form as `Pay rent`); the form adds the task's own.
- Nothing to send (empty, only spaces, only a mark) is refused with "There is no text to add." / "There is no text for a task.", and nothing is written.

## 3. Where the scratchpad line goes

The scratchpad is the one inbox note (`scratchpad.note`, `Scratchpad.md` at first; Linux and Android: their scratchpad path), made empty when it is not there. The line goes at the end of the section `## YYYY-MM-DD` of today (local date), with quick capture's rules (`QuickCapture.append` with that heading): the day's first line makes the section at the end of the note, after one blank line; the next ones go at its end. The editors save first and the file is read again at the write, through the app's ordinary write, so the index, the sync and the reading marks follow as for any edit, and the app's own write does not make the note "updated since you read".

```markdown
## 2026-11-11

- an older thought

## 2026-11-12
- [Read the RFC](https://www.rfc-editor.org/rfc/rfc9110)
- <https://example.com/>
```

A write that fails (no vault, a note that cannot be read) is not lost: the app opens its quick capture panel with the line in it and says why.

## 4. New Daftar Task

The form opens as New Task does from the app (the note on screen and the section of its cursor, else the scratchpad when it is there), with the text filled in and today as its created date. On macOS it goes through `daftar://new-task?text=<percent-encoded>`, handled by the window in front, or by a new window when none is open; the address only fills the form, so a page that opens it can write nothing. The address is sent to the running app itself, never to whichever app the system gives `daftar:` to.

## 5. Each platform

- **macOS: the Services menu.** `NSServices` in the app's `Info.plist`: "Add to Daftar Scratchpad" (`NSMessage` `addToScratchpad`, send types `public.utf8-plain-text` and `public.url`) and "New Daftar Task" (`newTask`, `public.utf8-plain-text`), each with an empty `NSRequiredContext` so it shows without being turned on in System Settings, and `NSPortName` `Daftar`. The app sets itself as `NSApp.servicesProvider` at launch. They appear in the app menu › Services and in the right-click menu of a selection (Safari, TextEdit, Mail…), and can be given a shortcut in System Settings › Keyboard › Keyboard Shortcuts › Services. After an install, `/System/Library/CoreServices/pbs -update` (or a log-out) makes the menu learn them.
- **Linux: desktop actions and a command.** The `.desktop` file gets `Actions=add-to-scratchpad;new-task;` with `Exec=daftar --add-to-scratchpad %u` and `Exec=daftar --new-task`, which read the **primary selection** (or the clipboard when it is empty) with `wl-paste --primary` / `xclip -o -selection primary`; the running instance takes them over D-Bus (`GApplication` `activate-action` with `add-to-scratchpad` / `new-task` and the text as the parameter), starting the app when it is not running. A desktop shortcut can be bound to `daftar --add-to-scratchpad`. `daftar-cli capture --scratchpad` is the same write for scripts.
- **Android: share targets.** Two activities with an `ACTION_SEND` intent filter for `text/plain`: "Add to Daftar Scratchpad" reads `EXTRA_TEXT` (and `EXTRA_SUBJECT` as the link's text when `EXTRA_TEXT` is a single address, as browsers share a page), writes through `VaultWriteRepository` like the scratchpad tile, shows a toast "Added to the scratchpad" and finishes without showing the app; "New Daftar Task" opens the New Task sheet with the text. `ACTION_PROCESS_TEXT` (the text selection menu) offers the same two.

## 6. Tests every app must have

1. `aSelectionIsOneListItem`: `"  Call Sara\n about the trip "` is `- Call Sara about the trip`; `- already an item` stays; Persian stays as written; `" \n "` and nothing are nil.
2. `aWebAddressIsInAngleBrackets`: `https://example.com/a?b=1#c` → `- <https://example.com/a?b=1#c>`; a link alone → `- <https://example.com/>`; `<https://example.com>` → `- <https://example.com>`; text and link the same → one address; `see https://example.com` stays words; a `file:///etc/hosts` link alone is nil; `javascript:alert(1)` and `https://` are words.
3. `aSelectedLinkIsAMarkdownLink`: `Daftar's [docs]` with `https://example.com/docs` → `- [Daftar's [docs\]](https://example.com/docs)`.
4. `aTaskStartsWithTheSelectionWithoutItsMarks`: `Renew the passport\nbefore May` → `Renew the passport before May`; `- [ ] Pay rent` and `* Pay rent` → `Pay rent`; `- ` and `` → nil.
5. `theScratchpadHasASectionForEachDay`: on 2026-11-12, `## 2026-11-11\n\n- old\n` becomes `## 2026-11-11\n\n- old\n\n## 2026-11-12\n- New\n`; `## 2026-11-12\n\n- old\n\n## 2026-11-13\n` becomes `## 2026-11-12\n\n- old\n- New\n\n## 2026-11-13\n`.
6. `theNewTaskAddressCarriesTheText` (macOS, and any app with the address): `Call Sara & Ali #home 100%` survives the round trip; `text=a%0Ab` is `a b`; no `text` is an empty form; `daftar://open?…` and `https://new-task?…` are not it.
7. On a made-up vault: a scratchpad that is not there is made (`## 2026-11-12\n- x\n`), and the sent line is written after a change made on disk meanwhile.
