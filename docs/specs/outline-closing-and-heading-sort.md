# Daftar outline rules: closing headings and sorting the headings under one

Shared by Daftar for macOS (`notes-mac`, commit `723e125`), Android (`notes-droid`, commit `ebb7aef`) and Linux (`notes-linux`, branch `outline-close-and-sort`). Both features act on the outline, the list of a note's headings beside it; neither folds the note's own text.

## 1. Closing a heading in the outline

- A heading has children when the heading right after it is deeper. Only such a heading can be closed; it shows an arrow (down when open, toward the reading direction's end when closed).
- A closed heading hides, in the outline only, every heading after it that is deeper than it, up to the next heading of its level or above.
- A heading closed under a closed one is hidden with it, and is still closed when that one opens.
- A heading with nothing under it has nothing to hide: a key for it changes nothing.
- The current section's mark goes to the closed heading it is hidden under.
- Close and Open act on one heading; "Close (Open) every heading of this level" on every heading of its level that has children. On the Mac, Option with a click on the arrow does the same; on Linux, Shift with a click.
- The closed headings are kept per note as `level:text` keys (`2:Plan`), which an edit above the heading does not move, apart from the reader's folds. They are kept across launches, and through a reindex of the note even when its text changed.
- On the Mac and Linux they follow the note's move or rename, as its folds do; on Android, where folds do not follow a rename yet, they do not either.

## 2. Sort Headings Under This

- The children of a heading are the headings of the highest level in its section (the lines up to the next heading of its level or above).
- The command puts them in the order of their names: natural (`Bean 9` before `Bean 10`) and case-insensitive (`apple` before `Tomato`), as Finder orders names, and stable for equal names.
- Each child moves with its whole section, the deeper headings and text under it included. The text between the heading and its first child stays where it is.
- Every line of the note is still there once after a sort; a second sort changes nothing.
- Nothing changes when the heading has fewer than two children, or they are in order already; the app says so: "“X” has fewer than two headings right under it.", "The headings under “X” are in order already."
- With no heading given, it acts on the heading of the section the cursor (or, with no editor, the reading place) is in; with none there it says "Put the cursor in a section to sort the headings under it." (Android: "Scroll into a section…").
- It is in the outline heading's menu (disabled with fewer than two children) and with the note's commands; on Linux also `daftar-cli sort-headings NOTE HEADING`, where HEADING is the heading's text or a line in its section. The editor's text is saved first, and the sorted text is written as any other edit of the app; on Linux, in the editor it is one step to undo.

## 3. Tests every app has

With the headings `# A`, `## A1`, `### A1a`, `## A2`, `# B`, `## B1`, `# C` (positions 0 to 6):

- `A` and `A1` have children; `A1a` and `C` do not.
- Nothing closed shows 0 to 6; `1:A` closed shows 0, 4, 5, 6; `2:A1` closed shows 0, 1, 3, 4, 5, 6; `1:A`, `2:A1` and `1:B` closed show 0, 4, 6; `1:C` and `3:A1a` closed show 0 to 6.
- With `1:A` closed, heading 2 is marked as 0, and heading 5 as itself.

With the note:

```markdown
# Plants

Intro stays first.

## Tomato

Red.

### Cherry tomato

## apple

Green.

## Bean 10

## Bean 9
Tall.

# Kitchen

## Zucchini
```

- The children of `Plants` are `Tomato`, `apple`, `Bean 10`, `Bean 9`.
- Sorting them gives the headings `Plants`, `apple`, `Bean 9`, `Bean 10`, `Tomato`, `Cherry tomato`, `Kitchen`, `Zucchini`; the text starts `# Plants\n\nIntro stays first.\n\n## apple\n\nGreen.\n`, and holds `## Tomato\n\nRed.\n\n### Cherry tomato\n` and `## Bean 9\nTall.\n`.
- Sorting again changes nothing; `Kitchen` (one child) and `Zucchini` (none) have nothing to sort.
