# Daftar properties: the front matter as a form

Shared by Daftar for macOS (`notes-mac`, `Properties.swift` and `PropertiesView.swift`, commits `6694a38` and `4c26615`) and Linux (`notes-linux`, `properties.rs` and `properties_form.rs`). Android does not have it yet; it should be built against this text. The Mac's `PropertiesTests` and Linux's `properties::tests` are the same cases by the same names.

## 1. Reading

- The front matter is the lines between a first line of `---` and the next line of `---` or `...`, as written: the fences keep their `\r` and trailing spaces, and the closing fence may be the note's last line with no newline after it.
- It is a form only when every line fits one: `key: value`, `key:` with a block list under it (`- item`, at any one indent, the key's own too), `key: [a, b]`, blank lines, and keys or values in single or double quotes. Each property's lines are kept as written.
- Anything else stays YAML, and the app shows the front matter as text with the reason: a comment, an anchor or alias, a tag (`!!str`), text over more than one line (`|`, `>`, or a value that goes on), a map inside a map or inside a list, a list inside a list (`[[Link]]` unquoted in a flow list), a key there twice, a list that is not of properties, a quote that does not close, an escape other than `\\ \" \/ \n \r \t`, a colon that YAML reads as a map.
- A property's type is the one `.obsidian/types.json` names (its key matched without case), else `tags`/`tag` are tags, `aliases`/`alias` aliases and `cssclasses`/`cssclass` a list, else the value's shape: a list is a list, `true`/`false` a checkbox, a numeral a number, `YYYY-MM-DD` a date, `YYYY-MM-DDTHH:MM` (or with a space and seconds) a date and time, anything else text.

## 2. Writing

- An edit is set a value, add a property (after the others), remove one (its blank lines stay with the one before it), or rename one in place.
- A property the edit did not change prints byte for byte as it was read; a changed one is written in its own style: its quote (plain, single or double) and a flow `[a, b]` or block `- a` list with the item prefix it had. An item of a list that did not change keeps its text. A value that plain text would misread (an indicator at its start, `null`, `~`, `: `, ` #`, a tab or newline, a `,` or bracket in a flow list) gets double quotes. A key that needs them gets quotes; a name that cannot be quoted is refused.
- Setting a value to the one it has changes nothing; an empty text is `key:`. Removing the last property removes the fences.
- A note with no front matter gets one when a property is added, with the note's line ending (`\r\n` when the note has one).
- The body is never touched. The edit is made to the file as it is on disk now, and only when its front matter is still the one the form was read from; otherwise nothing is written and the form is read again.
- The editor's text is saved first, and the write is the app's own: it does not mark the note "updated since you read".

## 3. How it is shown

- On the Mac, above the editor, the YAML hidden in the text, with a setting to show the YAML instead. On Linux, at the top of the inspector (Ctrl+Alt+I), above the outline, for the note on screen in the reader or the editor.
- A row per property: its name, a control for its type (a check box, a date field with a calendar, a list as chips with a field that adds one, a number or text field that writes on Enter or when it loses the focus), and a menu to rename or remove it. Add Property asks for a name, offering the vault's property names that match it (the most used first), and a type; a check box starts `false`, anything else empty, and the type picked is kept for the empty property until it has a value.
- Front matter the form cannot show is said so, with the reason, and edited as text.

## 4. Tests every app has

`a form reads and prints the same bytes`, `YAML the form cannot show stays raw and says why`, `a note without front matter gets one when a property is added`, `an edit keeps the style of every other line`, `rename, remove and add keep the order`, `an edit is refused when the front matter changed on disk`, `a written value reads back the same`, `types are inferred as Obsidian does`, `types.json wins`, `dates are written as Obsidian writes them`, `the front matter length` (in the unit the editor counts), and `names are ranked and completed`, each with the Mac's cases.
