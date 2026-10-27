// What a screen reader and the keyboard get from the reader page: every
// control is a real button or has a role and a name.

import { test } from "node:test";
import assert from "node:assert/strict";
import { render, taskLabel, baseName } from "../src/render.js";
import { codeHead, findBar, foldState, taskChip, baseTabs, canvasBar, zoomLabel, zoomBar, zoomPercent } from "../src/controls.js";
import { renderView } from "../src/bases.js";

const links = { Inbox: "Notes/Inbox.md", "logo.png": "uploads/logo.png" };

test("a task box is named by its task and says what a box cannot", () => {
  const html = render("- [ ] Call [[Inbox|Sara]] about *the trip* 📅 2026-10-27\n- [/] Build\n- [-] Drop\n- [x] Done\n");
  assert.match(html, /type="checkbox"[^>]*data-status=" " aria-label="Call Sara about the trip">/);
  assert.match(html, /data-status="\/" aria-label="Build, in progress">/);
  assert.match(html, /data-status="-" checked aria-label="Drop, cancelled">/);
  assert.match(html, /data-status="x" checked aria-label="Done">/);
  assert.equal(taskLabel("Ask [a](https://x) ⏫", "?"), "Ask a, status ?");
});

test("a picture's alt text is the embed's words or the file's name", () => {
  assert.match(render("![[logo.png]]", { links }), /alt="logo"/);
  assert.match(render("![[logo.png|The logo]]", { links }), /alt="The logo"/);
  assert.match(render("![[logo.png|300]]", { links }), /alt="logo" width="300"/);
  assert.equal(baseName("a/b/c.d.png"), "c.d");
});

test("a missing note says so in words, not only in its colour", () => {
  assert.match(render("[[Nowhere]]"), /aria-label="Nowhere, not a note yet"/);
  assert.match(render("![[Gone.png]]"), /role="link" aria-disabled="true" aria-label="Gone.png, not in the vault"/);
});

test("a callout says its kind and is a note; a folding one is a details", () => {
  const html = render("> [!warning] Mind the step\n> Body\n");
  assert.match(html, /<div class="callout" data-callout="warning" dir="auto" role="note">/);
  assert.match(html, /<span class="callout-icon" role="img" aria-label="Warning"><\/span>/);
  assert.match(render("> [!tip]- Fold\n> Body\n"), /<details class="callout"[^>]*dir="auto">.*<summary/s);
});

test("the page's controls are buttons with names", () => {
  assert.match(codeHead("swift"), /<button type="button" class="code-copy"[^>]*aria-label="Copy code"><span class="icon" aria-hidden="true">/);
  const bar = findBar();
  assert.match(bar, /<input type="search"[^>]*aria-label="Find in note"/);
  assert.match(bar, /class="find-count" role="status" aria-live="polite"/);
  for (const name of ["Previous match", "Next match", "Close find"]) assert.match(bar, new RegExp(`<button type="button"[^>]*aria-label="${name}"`));
  assert.deepEqual(foldState("Plans", false), { label: "Fold “Plans”", expanded: "true" });
  assert.deepEqual(foldState("Plans", true), { label: "Unfold “Plans”", expanded: "false" });
  const canvas = canvasBar("Board");
  for (const name of ["Zoom out", "Zoom in", "Show the whole canvas"]) assert.match(canvas, new RegExp(`aria-label="${name}"`));
  assert.equal(zoomLabel("logo", false), "logo. Escape closes it.");
  assert.equal(zoomLabel("", true), "Diagram. Escape closes it.");
  // Every button of the picture viewer has a name; ‹ › only with more than one picture.
  for (const count of [1, 3]) {
    const bar = zoomBar(count);
    for (const button of bar.match(/<button[^>]*>/g)) assert.match(button, /aria-label="[^"]+"/);
    assert.equal(bar.includes("zoom-next"), count > 1);
  }
  assert.equal(zoomPercent(1.254), "125%");
});

test("a task chip says its colour: overdue, priority, repeat", () => {
  assert.match(taskChip({ emoji: "📅", day: "2026-10-20", closed: false, today: "2026-10-25" }), /class="query-date is-late" role="img" aria-label="due 2026-10-20, overdue"/);
  assert.match(taskChip({ emoji: "📅", day: "2026-10-20", closed: true, today: "2026-10-25" }), /aria-label="due 2026-10-20"/);
  assert.match(taskChip({ emoji: "⏳", day: "2026-10-30", closed: false, today: "2026-10-25" }), /aria-label="scheduled 2026-10-30"/);
  assert.match(taskChip({ priority: "⏫" }), /role="img" aria-label="high priority">⇈/);
  assert.match(taskChip({ rule: " every week " }), /aria-label="repeats every week">🔁 every week/);
});

test("a base's views are tabs, and its cells and cards have names", () => {
  const tabs = baseTabs([{ name: "Table" }, { name: "Cards" }], 1);
  assert.match(tabs, /role="tablist" aria-label="Views"/);
  assert.match(tabs, /role="tab" class="base-tab" data-at="0" aria-selected="false" tabindex="-1"/);
  assert.match(tabs, /role="tab" class="base-tab is-active" data-at="1" aria-selected="true" tabindex="0"/);
  const rows = [{ path: "Notes/Inbox.md", cells: [{ check: true, parts: [] }] }];
  assert.match(renderView({ kind: "table", columns: ["Done"], groups: [{ rows }], count: 1 }), /<th scope="col" dir="auto">Done<\/th>.*aria-label="Done"/s);
  assert.match(renderView({ kind: "cards", columns: ["Done"], groups: [{ rows }], count: 1 }), /class="base-card" role="group" aria-label="Inbox"/);
});
