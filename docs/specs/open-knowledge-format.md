# The Open Knowledge Format: concepts in the vault, and the vault as a bundle

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`), and by `daftar-cli`.

The [Open Knowledge Format](https://github.com/open-knowledge-format/open-knowledge-format) (OKF, version 0.3, Apache-2.0) is knowledge as a folder of Markdown files with YAML front matter, written for agents and people alike: each file with a `type` is a **concept**, its front matter says where it came from, who confirmed it and when it goes stale, an `index.md` lists a folder and a `log.md` its changes. A vault is nearly that already. This spec is the two halves of Daftar's support: it **reads** OKF's front matter in any note (sections 1 to 4), and `daftar-cli okf export` **writes** a conformant bundle from the vault (section 5), which an agent on another computer can be given in place of the vault.

The vault stays what it is: Obsidian's Markdown, wikilinks and all. Nothing here changes a note that the person did not ask to change, and a note with no `type` is read as before. Section numbers written §N are OKF's.

## 1. A concept

A note is a concept when its front matter has a `type` that is not empty (§3.2). `OKF.concept(frontMatter:)` (`Okf::concept` in Rust) reads, with the consumer rules of §11.2:

| Key | Read as |
| --- | --- |
| `type` | the kind, any text; no list of kinds is known |
| `title`, `description`, `resource`, `tags` | as written; no `title` is the note's name |
| `status` | `draft`, `stable` or `deprecated`; absent is `stable`; any other value is kept as written and is **not** stable |
| `superseded_by` | a path (section 3), used only with `deprecated` |
| `stale_after` | a time; the concept is stale when now is at or after it |
| `generated` | `{by, at}`: the actor that wrote the text and when it last changed |
| `verified` | a list of `{by, at}`; one mapping is a list of one; an event without both keys is left out |

- **Null, an empty text and an empty list are an absent key.** One item where a list is asked is a list of one, and a scalar `tags` is one tag, never split at commas.
- **A time** is a date-time with an offset (`2026-06-30T14:00:00Z`, `…+02:00`) or a date alone (`2026-06-30`, the start of that day in UTC), quoted or as YAML's own. A date-time without an offset, or anything else, is malformed. A malformed `stale_after` is **stale**, never fresh, and is shown as malformed.
- **An actor** is `human:<id>`, another `<scheme>:<id>` (`team:`, `process:`, any other) or `<producer>/<version>`. Only `human:` is a person.
- **The trust tier** (§5.3) comes from the current events only: an event is current when its `at` is at or after `generated.at`, or when there is no `generated`. No current event is **unverified**; current events, none of a `human:`, is **machine-confirmed**; a current `human:` event is **human-reviewed**.
- A key OKF does not define is kept and shown as any property. Nothing is ever refused: a note that breaks a rule here is still read, opened and searched.

`index.md` and `log.md` are never concepts, whatever their front matter (§3.1). In the app they are notes like any other.

## 2. How it is shown

- **The inspector** (the properties form of [Properties](properties.md)) has a Concept row above the properties of a concept: its type, then chips for the trust tier ("Unverified", "Machine-confirmed", "Human-reviewed", with the last current verifier and day in the tooltip), the status when it is not `stable` ("Draft", "Deprecated", or the raw value), and "Stale since <day>" or "Stale: bad date". A note that is not a concept has no row.
- **The reader** has one line above a deprecated or stale concept's text, and nothing above any other: "Deprecated. Replaced by <link>." (the link from `superseded_by`, left out when it is absent or does not resolve) or "Stale since <day>: check it again."
- **Quick open and search** show a deprecated concept dimmed, after the others of the same rank.
- **Mark as Verified** (the palette, and a button in the Concept row): adds `{ by: "human:<id>", at: "<now, UTC, to the second>" }` to `verified` through the properties' write (`PropertySheet`), which keeps every other line byte for byte. `<id>` is `okf.actor` in `config.toml` (the app's settings on Android), else the local part of the clone's `user.email`, else the command is off and says to set it. An event of that actor already there is replaced, not added twice. It is offered on a concept only, and never in a read-only vault.

## 3. Links, as OKF reads them

OKF's links are Markdown links, `[text](target)`, not wikilinks. A vault may hold both, so the apps resolve a Markdown link to a file as §6 says:

1. A target with a scheme (`https:`) is a web address, as now.
2. A target that starts with `/` resolves from the **bundle root**: the nearest folder at or above the note whose `index.md` has `okf_version` in its front matter, else the vault's root.
3. Any other target resolves from the note's folder.

- The target is percent-decoded (`my%20note.md`), and a `#fragment` names a heading as a wikilink's does.
- A target that resolves outside the vault is never opened or read from the disk; it is shown as a broken link.
- A resolved Markdown link is a link like a wikilink: it counts in backlinks, the graph, `check`'s dangling links and a move's relinking, which rewrites it in the form it had (a `/` path stays a `/` path, a relative one stays relative), percent-encoded.
- `superseded_by` resolves by the same rules.

## 4. The audit, the MCP server and agents' writes

**`daftar-cli check`** with `okf = true` under `[check]` in `.daftar.toml` (or `--okf`) adds warnings, never errors, since OKF refuses nothing:

| Rule | When |
| --- | --- |
| `okf-stale` | a concept is stale; the message has the day |
| `okf-time` | a time key (`stale_after`, `generated.at`, `verified[].at`) is malformed |
| `okf-status` | `status` is none of the three |
| `okf-event` | a `verified` event, or `generated`, lacks `by` (or an event `at`) |
| `okf-superseded` | a link or wikilink goes to a deprecated concept that has a `superseded_by`; the message names the replacement, and `--fix` writes it |

**The MCP server**: `note_info` and `note_context` say `concept: <type>; <tier>; <status>; stale since …` for a concept. A new reading tool, `concepts`, lists them (`type`, `status`, `tier`, `stale`, `folder` as filters; each line the path, the type and the description), so an agent sees what there is before it opens a note, as an `index.md` would show it.

**An agent's write is marked.** When an MCP tool changes the text of a concept (`append_to_note`, `replace_in_note`, `replace_section`; not a task tick, a property or a move), the server sets `generated: { by: "<client name>/<client version>", at: "<now>" }` in the same write, from the `clientInfo` of `initialize` (lower case, spaces as `-`; no client info is `daftar-mcp/<version>`). The events in `verified` stay as history, and by the rule of section 1 the note is no longer human-reviewed until the person marks it again. A note that is not a concept gets no front matter.

## 5. The vault as a bundle: `daftar-cli okf export`

`daftar-cli okf export --output <folder>` writes an OKF 0.3 bundle from the vault, and `daftar-cli okf check <folder>` says whether a folder is one (§11.1: every front matter parses as a mapping, `index.md` and `log.md` have their shape; then the warnings of section 4). The vault's `.daftar.toml` says what goes in:

```toml
[okf]
# Folders to export, each with all under it; empty is the whole vault.
include = ["Learning", "Companies"]
# Folders and notes never exported, also under an included folder.
exclude = ["Identity", "Banking"]
# The bundle's own words, for the root index.md (§8.1).
title = ""
description = ""
license = ""

# The type given to a note that has none, by folder; the longest match wins.
[okf.types]
"Learning" = "Reference"
"Companies" = "Company"
```

`--include` and `--exclude` on the command line add to these. What is written:

- **Notes.** Each note in scope, at its own path. Left out always: hidden notes (`files.hidden`: `CLAUDE.md`, `AGENTS.md`, …), dot folders, the week notes of `Calendar/`, the scratchpad, and the templates. A note named `index.md` or `log.md` is written as `index-note.md` or `log-note.md`, and links to it follow.
- **Front matter.** What the note has, byte for byte. A note with no `type` gets one: `[okf.types]` by its folder, else `Note`. A note with no front matter gets a block with `type` alone. No `title`, `description`, trust or time is ever made up.
- **The body**, turned into what §4.2 allows (CommonMark, GFM tables, footnotes):
    - `[[Note]]`, `[[Note|text]]`, `[[Note#Heading]]`: `[text](/path/to/Note.md#heading)`, a `/` path from the bundle's root, percent-encoded; the text is the alias, else the note's name (with ` > Heading`). A link to a note that is not exported, or that resolves to none, becomes its text alone.
    - `![[picture.png]]` and `![](…)`: `![](/uploads/picture.png)`, and the file is copied as an asset. `![[Note]]` becomes a link to the note.
    - `> [!type] Title`: a quote whose first line is `> **Title**` (the type's name, capitalised, with no title); the body stays.
    - `==text==`, with or without a colour's emoji: `**text**`. `%%comments%%` are removed. A block id (` ^id`) is removed. Math, tasks, tags and code stay as they are.
- **`index.md`** in every folder written, with no front matter but the root's: a `# Notes` section, `* [Title](file.md) - description` for each concept by name (the `description` of its front matter; none is no ` - …`), then `# Folders`, `* [Name](name/)`. The folder's own note (`Acme/Acme.md`) is first in Notes. The root's has `okf_version: "0.3"`, `title` (else the vault's folder name), `description` and `license` when given, and `x-daftar-export: true`.
- **`log.md`** at the root, from git: a `## YYYY-MM-DD` heading for each day with changes, newest first, `--log-days` back (90), and under it `* **Creation**: [Title](path)` or `* **Update**: [Title](path)` for each exported note the day's commits added or changed, by path. No repository is no `log.md`.
- **The output folder** must not be in the vault. It must be empty, or a bundle this command wrote (its root `index.md` has `x-daftar-export: true`), else the command refuses. In a bundle it wrote, a file that this run does not write is removed (a `.git` folder and other dot paths are left alone), and the root `index.md`'s own front matter is kept, with only `okf_version` set again. A file whose text did not change is not written again, so a git repository in the output has small commits.
- It writes nothing in the vault, so it runs in a read-only clone too. `--dry-run` lists what it would write and remove.

**Why it matters here**: the bundle is the part of the vault that can be given away. A repository made of it (a CI job that runs the export on each push and pushes the output) is what a computer the person trusts less clones, or what an agent there reads, in place of the vault and its `Identity/` and `Banking/`.

On Android there is no `daftar-cli`: sections 1 to 3 apply, and 4 and 5 do not.

## 6. Tests every app must have

Ported by name (snake case in Rust, camelCase in Kotlin). 1 to 7 are for every app; 8 to 11 are Rust only (`daftar-cli`).

1. `aNoteWithATypeIsAConcept`: `type: Metric` is one; no front matter, no `type`, `type:` empty and `type: null` are none; `index.md` and `log.md` with `type: Metric` are none.
2. `theTrustTierComesFromCurrentEvents`: no `verified` is unverified; `verified: { by: "human:parham", at: "2026-06-25T09:00:00Z" }` (a bare mapping) is human-reviewed; with `generated: { by: claude-code/2.1, at: "2026-06-26T00:00:00Z" }` after it, unverified; a `process:nightly` event after that, machine-confirmed; a `team:x` event alone, machine-confirmed; an event with no `at` is left out; an event at the very second of `generated.at` is current.
3. `aTimeHasAnOffsetOrIsADay`: `"2026-06-30T14:00:00Z"`, `"2026-06-30T16:00:00+02:00"` and YAML's own unquoted form are the same instant; `2026-06-30` (quoted or not) is `2026-06-30T00:00:00Z`; `"2026-06-30T14:00:00"`, `"30/06/2026"` and `"soon"` are malformed.
4. `aConceptIsStaleAtItsInstant`, with now `2026-09-23T00:00:00Z`: `stale_after` of that instant is stale, a second later is fresh, absent, null and empty are fresh, `"soon"` is stale and malformed.
5. `anUnknownStatusIsNotStable`: absent is stable; `draft`, `stable`, `deprecated` are themselves; `archived` is kept as `archived` and is not stable; a scalar `tags: a, b` is the one tag `a, b`.
6. `aMarkdownLinkResolvesAsOKFSays`, in a vault with `Docs/index.md` (front matter `okf_version: "0.3"`), `Docs/tables/orders.md`, `Docs/metrics/revenue.md` and `Other/x.md`, from `Docs/metrics/revenue.md`: `/tables/orders.md` is `Docs/tables/orders.md`; `../tables/orders.md` is the same; `./revenue.md#Definition` is the note and its heading; `my%20note.md` is `my note.md` beside it; from `Other/x.md`, `/Docs/tables/orders.md` resolves from the vault's root; `../../../etc/passwd` and `/../x.md` resolve to nothing and are never read.
7. `markAsVerifiedWritesOneEvent`: on a concept with `verified: { by: "process:nightly", at: … }` and other keys, marking as `human:parham` gives a list of the two, every other line byte for byte; marking again replaces the `human:parham` event; on a note with no `type` it is refused.
8. `anAgentsWriteIsMarkedAsGenerated`: `replace_in_note` on a human-reviewed concept by a client `Claude Code` `2.1.0` sets `generated.by` to `claude-code/2.1.0` and the tier to unverified, with `verified` kept; the same on a note that is not a concept adds no front matter; `toggle_task` on a concept changes no front matter.
9. `theAuditWarnsOfStaleAndSuperseded`: with `okf = true`, a stale concept gives `okf-stale`, `status: archived` gives `okf-status`, a wikilink to a deprecated concept with `superseded_by: /New.md` gives `okf-superseded` naming `New`, and `--fix` makes the link `[[New]]` with its text kept; without `okf = true`, none.
10. `anExportIsAConformantBundle`, on a vault of `Learning/A.md` (no front matter, `[[B|the other]]`, `![[p.png]]`, a `> [!warning] Mind` callout, `==🔴hot==`, `%%gone%%`), `Learning/B.md` (`type: Guide`, `description: Second.`), `Learning/Sub/Sub.md`, `Identity/Secret.md`, `CLAUDE.md` and `uploads/p.png`, with `include = ["Learning"]` and `"Learning" = "Reference"`: `A.md` starts with `---\ntype: Reference\n---` and has `[the other](/Learning/B.md)`, `![](/uploads/p.png)`, `> **Mind**`, `**hot**` and no `gone`; `B.md`'s front matter is unchanged; `uploads/p.png` is copied; `Learning/index.md` lists `* [A](A.md)`, `* [B](B.md) - Second.` and `* [Sub](Sub/)`; the root `index.md` has `okf_version: "0.3"` and `x-daftar-export: true`; no `Identity`, no `CLAUDE.md`; a link from `A` to `[[Secret]]` is the text `Secret`; `okf check` of the output passes; the vault's files are byte for byte as before.
11. `anExportOnlyWritesWhereItMay`: an output inside the vault is refused; a folder with files that is not an export is refused and untouched; a second export after `B.md` is removed from the vault removes `B.md` from the bundle and leaves a `.git` folder there alone; a second export with nothing changed writes no file (the change times stay).
