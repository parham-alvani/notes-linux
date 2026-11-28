# A read-only vault, and the clone's own git identity

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`), and by `daftar-cli`.

A vault is a clone of a Git repository, and not every computer that reads it should change it: a work laptop, a server, a phone that crosses a border. Such a clone gets a deploy key without write access, so the remote refuses its pushes. This spec is the app's side of that: a clone that is **read-only** pulls and never writes, and a clone can carry its own name, email and SSH key, apart from the computer's.

What one clone is (read-only, its address, its author, its key) is in the clone's own git config (`.git/config`), so it is never committed, and the app, `daftar-cli`, the hooks and a shell all read the same value. What the vault asks of every read-only clone (section 5: no agent reads it there) is in the vault's `.daftar.toml`, which is committed, so the rule reaches a clone with its first pull.

## 1. The settings

| Key (local git config) | Value | Read by |
| --- | --- | --- |
| `daftar.readonly` | a git boolean; not set is `false` | `git config --local --type=bool --get daftar.readonly` |
| `remote.<name>.url` | the vault's address: where this clone pulls from and pushes to. `<name>` is the remote of the branch's upstream, else `origin` | git itself |
| `user.name`, `user.email` | the author of this clone's commits; with none, the vault's `git.name` and `git.email` ([the app's settings from the vault](vault-config.md)) | git itself |
| `core.sshCommand` | the `ssh` of this clone, with its key: `ssh -i <key> -o IdentitiesOnly=yes` | git itself, and the app (section 2) |

- A vault that is not a git repository has none of them: it is never read-only, and the settings are not offered.
- A value that is not a git boolean (`daftar.readonly = perhaps`) reads as read-only: when in doubt the app does not write.
- The global and system configs are not read for `daftar.readonly`. A computer cannot make every vault read-only by accident, and the answer does not change with `$HOME`.
- On Android the clone is the app's own, and its key and author are in the app's settings already; it keeps them there, and keeps `daftar.readonly` in the clone's config as the others do.

## 2. The ssh command

Today the desktop apps set `GIT_SSH_COMMAND` when the person did not, and git puts that variable above `core.sshCommand`: a key set for the clone is ignored. The rule, in one pure function (`Git.sshCommand(configured:environment:)`, `Git::ssh_command`):

1. `GIT_SSH_COMMAND` in the environment, when it is set and not empty, is used as it is. The person asked for it.
2. Else the clone's `core.sshCommand` (any scope git would read: `git config --get core.sshCommand`), when set and not empty, with the app's options added after it.
3. Else `ssh` with the app's options.

The app's options are the ones each app has now (`-o BatchMode=yes`, and on macOS `ConnectTimeout` and `ServerAliveInterval`), each added only when the command does not name that option already, compared without case. The result goes in `GIT_SSH_COMMAND` for every git the app runs.

## 3. What read-only means

**The app writes no file of the vault and makes no commit.** Everything that only reads works as before: the reader, search, search by meaning, the outline, tasks and their queries, Today, the board, history, export, Quick Look.

- **Sync is a pull.** `git pull --ff-only` (with `--no-rebase`, so a `pull.rebase` in the person's git config rebases nothing), with no commit before it and no push after. The pull at launch and the timer run the same. Nothing is stashed, reset or rebased: when the pull cannot fast-forward (the clone has commits of its own and the remote has others, or, with no commit of its own, an uncommitted change that the pull would write over), it fails with `readOnlyDiverged` and names the files, and the person mends it in a shell. A read-only clone that is only ahead of its remote has nothing to do, and the sync succeeds.
- **Every write is refused at the one place the app writes** (`VaultEdits` and `VaultStore.write`, their Kotlin twins), with the failure `readOnly`, before the file is read. That covers the editor's save, ticking a task and setting its status, Edit Task and New Task, quick capture and the scratchpad, Send to Daftar, a new note, the daily note, a template, moving, renaming and deleting, properties, block ids, pasting a picture, Clip Web Page, Format Note and format on leave, Fix and Fix All, restoring a revision, and on Android the week notes in `Calendar/`.
- **What is not in the vault is still written**: reading places and folds, the drill's state, the search and meaning indexes, the notified meetings, the app's settings.
- **`daftar-cli`**: a command that writes (`append`, `new`, `mv`, `set-property`, `toggle`, `set-status`, `cancel`, `postpone`, `replace`, `rename-tag`, `fmt` without `--check`, `lint --fix`, `check --fix`, `mentions --fix`, `inbox move`, `clip`, `images`, `review --write`, `sort-headings`, `sort-tasks`, `capture`, and `daily` without `--no-create`, which makes the day's note) writes nothing, prints `this vault is read-only (daftar.readonly)` on stderr and exits 1; with `--dry-run` it runs. `sync` pulls as above and says so. `status` and `doctor` have a `read-only` line. `mcp` runs as with `--read-only`, and `lsp` answers formatting and rename with no edit. `hook pre-commit` refuses the commit.
- **Turning it on or off** changes only `daftar.readonly`. Changes the clone already has stay as they are; `doctor` lists them.

## 4. How it is shown

- **A badge**, "Read-only", by the vault's name (the window's title or sidebar header on the desktop, the top bar on Android), with the tooltip "This vault is read-only on this computer. Daftar pulls and never writes."
- **The actions that write are off**, not hidden: Edit, New Note, New Task, capture, Format Note, Move, Rename, Delete, Fix. A task's box in the reader does not tick; a click gives the notice "This vault is read-only." once per note.
- **Sync** is labelled Pull, and its notice says "Pulled" or "Up to date", only when the person asked for the sync: the timer and the pull at launch are quiet.
- **Settings › Vault › Git**, for the vault that is open, only when it is a repository: a Read-only switch, Address, Name, Email, and SSH key (a file chooser, and Generate, which runs `ssh-keygen -t ed25519 -N "" -C "daftar <vault> on <host>"` into `~/.ssh/daftar_<vault>_ed25519`, never over a file that is there, and shows the public key with Copy; in the file's name `<vault>` keeps its letters, digits, `-` and `.`, and every other character is `_`). Each field writes its key of section 1 with `git config --local`; an emptied field unsets it. The key field writes `core.sshCommand` as `ssh -i <path> -o IdentitiesOnly=yes`, the path quoted for a shell, and shows a command it did not write as text, without changing it: Choose, Generate and Clear are off for it. Address writes `remote.<name>.url` with `git remote set-url` (with no remote yet, `git remote add origin`, and the branch's upstream is written then, as config: `branch.<b>.remote` and `branch.<b>.merge`, not at the next pull); it is trimmed, an empty one is refused, and nothing is fetched until the next sync, which says so when the new address does not answer or holds another history. The address is how a clone moves to a mirror when the first host cannot be reached.
- **A push the remote refuses for a read-only key** (stderr has `marked as read only`, GitHub's deploy key message, or `Write access to repository not granted`) fails as `pushRefusedReadOnly`: the notice says the key cannot write and offers "Make This Vault Read-only". The commits already made stay.

## 5. No agent in a read-only clone

A read-only clone is on a computer the person trusts less, and an agent there (Claude through `daftar-cli mcp`) would read the whole vault and carry it into a conversation. The vault can refuse that, for every read-only clone at once, in its committed `.daftar.toml`:

```toml
# A read-only clone is not served to an agent: `daftar-cli mcp` does not start there.
[agents]
read_only_clones = false
```

- **Not set is `true`**: a read-only clone serves the reading tools, as `mcp --read-only` does. A clone that is not read-only is never touched by this key.
- **With `false`, in a read-only clone, `daftar-cli mcp` does not serve.** It answers `initialize` (and any other request) with a JSON-RPC error, code `-32000`, whose message is `this vault is not open to agents in a read-only clone (agents.read_only_clones in .daftar.toml)`, prints the same on stderr and exits 1. No tool is listed and no note is read. Over HTTP (`mcp --http`, [the MCP server over HTTP](mcp-http.md)) it does not start, and a pull that brings the key closes the server it runs: every request is answered with that error.
- **The apps' own agent features follow it**: anything of the app's that sends a note's text to a model (the Mac's `NoteAssistant`; none on Linux and Android today) is off in such a clone, and says why. What the system does by itself with the text on screen (the Mac's Translate and Writing Tools) does not count.
- **Every other command works**: the person's own `search`, `find`, `tui` and the apps read as before. The key is about agents, not about reading.
- **It is a rule, not a lock.** The notes are plain files, and a program with access to the folder reads them without Daftar; so does a person who edits `.daftar.toml` in that clone. The key stops the door Daftar opens. A vault that must not be read on a computer must not be cloned there, or only in part.
- `doctor` and `status` say `agents: refused in this read-only clone` when it applies. `VaultSettings` reads the key (`agents_in_read_only_clones`), and the Mac's `VaultSettings` with it.

## 6. Tests every app must have

Ported by name (snake case in Rust, camelCase in Kotlin):

1. `theSshCommandKeepsTheClonesKey`:
    - no config, no environment: `ssh` and the app's options;
    - configured `ssh -i ~/.ssh/notes_ro -o IdentitiesOnly=yes`: that, then the app's options;
    - configured `ssh -o batchmode=no`: `BatchMode` is not added again;
    - environment `ssh -v`, configured anything: `ssh -v`, nothing added;
    - environment or configured empty or blank: as not set.
2. `aCloneIsReadOnlyByItsOwnConfig`, on a repository in a temporary folder: not set is not read-only; `true`, `yes`, `on`, `1` are; `false`, `no`, `off`, `0` are not; `perhaps` is; the same key in a global config (a test `GIT_CONFIG_GLOBAL`) is not read; a folder that is no repository is not read-only.
3. `aReadOnlyVaultRefusesEveryWrite`: with `daftar.readonly` on, a write, a task tick, a capture, a new note, a move and a delete each fail with `readOnly`, and afterwards every file of the vault has the bytes it had and `git status --porcelain` is empty.
4. `aReadOnlySyncOnlyPulls`, on two clones of a bare remote: a commit pushed from the first reaches the read-only second by sync; the second's `git rev-list @{u}..HEAD` stays empty and the remote has no new commit; a file changed by hand in the second is not committed, and is still changed after the sync.
5. `aReadOnlySyncDoesNotMendADivergedClone`: the second clone has a commit of its own and the remote another: sync fails with `readOnlyDiverged`, and the commit, the branch and the working tree are as they were.
6. `aRefusedPushIsNamed` (the parser only, no network): stderr with `ERROR: The key you are authenticating with has been marked as read only.` and with `ERROR: Write access to repository not granted.` both give `pushRefusedReadOnly`; `Permission denied (publickey).` does not.
7. `daftar-cli` (Rust only): each writing command of section 3 on a read-only vault exits 1 with the message and changes nothing; `--dry-run` runs; `mcp` lists no writing tool.
8. `theAddressIsTheUpstreamsRemote`, on a clone of a bare remote: the address read is the remote's; set to a second bare remote with the same history, it is `remote.origin.url` and the next sync pulls a commit pushed there; with the upstream on a remote named `backup`, `remote.backup.url` is the one read and written and `origin` is not changed; an empty or blank address is refused and the old one stays.
9. `aReadOnlyCloneCanRefuseAgents`: `VaultSettings` of `[agents]\nread_only_clones = false\n` refuses, of no key, of `true` and of the key under another section does not; `mcp` on a read-only clone with the key answers `initialize` with the error and no tool call reads a note; the same vault not read-only serves every tool; a read-only clone without the key serves the reading tools.
