# Daftar task rules: reasons and custom statuses

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Everything stays plain Markdown that the Obsidian Tasks plugin (emoji format) still reads.

## 1. The reason field

A task may carry a reason: why it was cancelled, or why it is waiting (in progress, in review, …). It is a Dataview-style inline field in the task's description, after the text and before the Tasks plugin's emoji signifiers. The block id (`^abc`) stays last.

```markdown
- [-] Deploy the search cluster [reason:: no budget left] ➕ 2025-05-17 ❌ 2025-11-16
- [r] Ship the importer [reason:: waiting on the review of [[Importer]]] 📅 2026-11-02 ^imp
```

Reading:

- The key is `reason`, compared case-insensitively, then `::`, then the value, trimmed. Spaces around `::` are allowed (`[reason ::x]` too).
- The value ends at the `]` that balances the field's opening `[`, so `[[Wiki links]]` and `[md](links)` inside it work.
- Only the first `[reason:: …]` in the description is the reason; a later one stays text. An empty value means no reason.
- It is looked for in the description, after the trailing emoji signifiers have been taken off (as the existing metadata split does).
- The task's text, as the apps show and match it (the existing `text` / `indexedText` that board cards, Today, lists, queries' `description` and "locate the task again" use), is the description without the field. Where the field was taken out, the spaces around it fold into one, and the text is trimmed.

Writing (any edit an app makes):

- The field is written as `[reason:: value]`, one space after the text, before the signifiers: `text [reason:: value] <priority> <🔁 rule> … <dates>`.
- The value is trimmed, and newlines / tabs become single spaces. If its square brackets do not balance (a scan never goes below zero and ends at zero), every `[` becomes `(` and every `]` becomes `)`, so the field still closes where it should.
- An empty or blank value removes the field.

## 2. Statuses

The statuses come from the Tasks plugin's settings in the vault: `.obsidian/plugins/obsidian-tasks-plugin/data.json`, `statusSettings.coreStatuses` followed by `statusSettings.customStatuses`, each `{symbol, name, nextStatusSymbol, type}` with `type` one of `TODO`, `IN_PROGRESS`, `DONE`, `CANCELLED`, `NON_TASK`.

- When the file is missing or unreadable, or lacks one of them, these four are there, in this order, before any other custom ones: ` ` "Todo" (TODO, next `x`), `x` "Done" (DONE, next ` `), `/` "In Progress" (IN_PROGRESS, next `x`), `-` "Cancelled" (CANCELLED, next ` `). A symbol listed twice: the first wins. A symbol that is not exactly one character is skipped.
- `X` is DONE "Done" unless the settings say otherwise.
- A symbol not in the settings is TODO, and its name is the symbol in brackets, e.g. `[>]`. (This is the plugin's rule and what the apps do today.)
- Closed = type DONE or CANCELLED. NON_TASK counts as closed too (it is not something to do). Everything else is open.
- The settings are read when the vault is opened and again when that file changes (or on the app's usual vault refresh); a bad file falls back to the defaults, it never fails the app.
- Example: `r` "In Review", type IN_PROGRESS, next `x`.

## 3. Changing a task

The click on a checkbox (tick) is unchanged: an open task becomes `x` with `✅ today` (a repeating one leaves its next occurrence above), a closed one becomes ` ` without its ✅/❌ dates.

Setting a status `S` (from a status picker, a board column, the CLI):

- If `S` is DONE-typed and the task is open: exactly the tick above (recurrence and all), then the status char becomes `S` if it is not `x`.
- Otherwise the status char becomes `S`; the ✅ date is kept if the task was already DONE-typed, set to today if `S` is DONE-typed and it was not, removed otherwise; likewise the ❌ date for CANCELLED-typed.
- Same status as now: nothing changes (unless a new reason is given; then only the reason changes).

The reason and status changes:

- A reason belongs to the status it was given with. Whenever an app changes a task's status (tick, picker, board drop, CLI) the old reason is removed, unless the change itself gives a new one.
- A repeating task's next occurrence never carries a reason, and its completed copy loses the reason too (the status changed).
- Editing only the reason (edit form, CLI `reason`) keeps the status and dates.

## 4. Asking for the reason

- Cancelling a task — choosing a CANCELLED-typed status in a picker or menu, dropping a card on the Cancelled column, a "Cancel task" command — asks for the reason first: a small dialog/sheet with one text field (prefilled with the task's current reason, if any, focused), "Cancel Task" (default, Return) and "Back"/"Don't Cancel" (Escape: nothing changes). An empty field cancels without a reason.
- The task edit form (where it exists) has a "Reason" text field for every status, so a reason can be written for In Progress / In Review / any status, and edited or cleared.

## 5. Showing it

- Wherever a task row is shown outside the raw editor (board cards, Today, task lists, query results, the reader/preview, widgets if they show tasks), the text is shown without the field, and the reason, when there is one, follows it in a muted/secondary style with a speech-balloon mark: `💬 no budget left` (or the platform's equivalent icon, SF Symbol `text.bubble`, Material `Icons.Outlined.ChatBubbleOutline`, symbolic icon in GTK). Wiki links in the reason may stay plain text.
- A status that is not one of the four defaults shows its name (e.g. "In Review") as a small badge/label where the status is shown (board card, task row, status picker lists every status by name with its symbol).
- In the raw Markdown editor, `[reason:: …]` stays as written; the field may be styled muted.
- The board's "In progress"/"Doing" column holds every IN_PROGRESS-typed status (each card labelled with its name when it is not `/`); its Done column every DONE-typed, Cancelled every CANCELLED-typed. Dropping on a column sets its default symbol (`/`, `x`, `-`), keeping a card's own symbol when it is already of that type.

## 6. Queries (` ```tasks ` blocks)

Added to what each app's TasksQuery understands (lines case-insensitive like the rest):

- `has reason`, `no reason`
- `reason includes <text>`, `reason does not include <text>` (case-insensitive substring)
- `status.name is <name>`, `status.name is not <name>`, `status.name includes <text>`, `status.name does not include <text>` (Tasks plugin syntax; case-insensitive)
- `status.type is …` / `is not …` now uses the type from the settings (so `[r]` is IN_PROGRESS), and also accepts `non_task`.
- `done` / `not done` use the settings' closed rule.
- `group by status.name` / `status.type` use the settings' names/types (already named groups).
- Expressions (`filter by function`, `group by function`): `task.reason` (string, or null when none), `task.status.name`, `task.status.type`, `task.status.symbol`, `task.isDone` (type DONE).
- `description includes` does not see the reason field (the text is without it).

## 7. Tests to have (in each app)

- Parse: `- [-] Deploy [reason:: no budget] ➕ 2025-05-17 ❌ 2025-11-16` → text `Deploy`, reason `no budget`, cancelled `2025-11-16`, created `2025-05-17`.
- Parse with a wikilink: `- [r] Ship [reason:: waiting on [[Importer]]] 📅 2026-11-02 ^imp` → text `Ship`, reason `waiting on [[Importer]]`, due `2026-11-02`.
- Field in the middle: `- [ ] Call [reason:: busy] the bank` → text `Call the bank`, reason `busy`.
- Write: edit that sets reason `a ] b` on `- [ ] T 📅 2026-11-02` → `- [ ] T [reason:: a ) b] 📅 2026-11-02`; clearing removes it; a block id stays last.
- Cancel with reason: `- [ ] Write ⏳ 2026-10-04` set to `-` reason `no time`, today 2026-10-28 → `- [-] Write [reason:: no time] ⏳ 2026-10-04 ❌ 2026-10-28`.
- Tick a cancelled task with a reason → `- [ ] Write ⏳ 2026-10-04` (reason and ❌ gone).
- Recurring task with a reason ticked → neither line has the reason.
- Statuses: parse the sample settings JSON below; `r` is IN_PROGRESS "In Review", open; a missing file gives the four defaults; `>` is TODO named `[>]`.
- Queries: `has reason`, `reason includes budget`, `status.name is In Review`, `status.type is in_progress` matching `[r]`, `task.reason` in an expression.

```json
{"statusSettings":{"coreStatuses":[{"symbol":" ","name":"Todo","nextStatusSymbol":"x","availableAsCommand":true,"type":"TODO"},{"symbol":"x","name":"Done","nextStatusSymbol":" ","availableAsCommand":true,"type":"DONE"}],"customStatuses":[{"symbol":"/","name":"In Progress","nextStatusSymbol":"x","availableAsCommand":true,"type":"IN_PROGRESS"},{"symbol":"-","name":"Cancelled","nextStatusSymbol":" ","availableAsCommand":true,"type":"CANCELLED"},{"symbol":"r","name":"In Review","nextStatusSymbol":"x","availableAsCommand":false,"type":"IN_PROGRESS"}]}}
```
