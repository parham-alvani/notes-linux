// The section bar: the heading path of the place being read, and which
// headings fit (`docs/specs/look.md` in notes-linux, "Section bar").
import { test } from "node:test";
import assert from "node:assert/strict";
import { headingPath, pathAt, leftOut } from "../src/sectionbar.js";
import { sectionCrumbs } from "../src/controls.js";

// `# A`, `## B`, `### C`, `## D`, each two lines after the last.
const note = [
  { level: 1, text: "A", line: 0 },
  { level: 2, text: "B", line: 2 },
  { level: 3, text: "C", line: 4 },
  { level: 2, text: "D", line: 6 },
];

test("the section bar is the heading path", () => {
  assert.deepEqual(pathAt(note, 5), ["A", "B", "C"]);
  assert.deepEqual(pathAt(note, 7), ["A", "D"]);
  assert.deepEqual(pathAt(note, 3), ["A", "B"]);
  // Before the first heading there is no bar.
  assert.deepEqual(pathAt(note.map((heading) => ({ ...heading, line: heading.line + 3 })), 1), []);
  assert.deepEqual(pathAt([], 9), []);
  // A parent is the nearest earlier heading of a lower level: a level may be skipped.
  const skipped = [{ level: 1, text: "A", line: 0 }, { level: 3, text: "C", line: 2 }];
  assert.deepEqual(pathAt(skipped, 3), ["A", "C"]);
  // A heading folded away is not in a path.
  const folded = note.map((heading) => (heading.text === "C" ? { ...heading, away: true } : heading));
  assert.deepEqual(pathAt(folded, 5), ["A", "B"]);
});

test("a heading's parents are found by level, not by nearness alone", () => {
  // `## X`, `# A`, `### C`, `## D`, `#### E`: X is not above A.
  const levels = [2, 1, 3, 2, 4];
  assert.deepEqual(headingPath(levels, 4), [1, 3, 4]);
  assert.deepEqual(headingPath(levels, 2), [1, 2]);
  assert.deepEqual(headingPath(levels, 0), [0]);
  assert.deepEqual(headingPath(levels, -1), []);
  assert.deepEqual(headingPath(levels, 9), []);
  // Two headings of one level are not each other's parent.
  assert.deepEqual(headingPath([2, 2], 1), [1]);
});

test("a path wider than the bar leaves parents out from the start, never the last heading", () => {
  // Three headings of 100 px, 10 px between them, 20 px for the dots.
  assert.equal(leftOut([100, 100, 100], 10, 20, 400), 0);
  assert.equal(leftOut([100, 100, 100], 10, 20, 320), 0);
  // 319 px: the first parent goes, and `… › B › C` is 240 px.
  assert.equal(leftOut([100, 100, 100], 10, 20, 319), 1);
  assert.equal(leftOut([100, 100, 100], 10, 20, 239), 2);
  // The last heading always shows, however little room there is.
  assert.equal(leftOut([100, 100, 100], 10, 20, 10), 2);
  assert.equal(leftOut([500], 10, 20, 100), 0);
});

test("the bar's headings are buttons with no place in the Tab order, each in its own direction", () => {
  const html = sectionCrumbs(["Plan", "سفر", "A <b>"]);
  assert.equal((html.match(/<button type="button" tabindex="-1" class="section-crumb/g) ?? []).length, 3);
  assert.equal((html.match(/dir="auto"/g) ?? []).length, 3);
  assert.match(html, /class="section-crumb is-last" data-at="2" dir="auto" aria-current="location">A &lt;b&gt;<\/button>$/);
  // `…` is there for the parents left out, hidden until some are.
  assert.match(html, /^<span class="section-dots" aria-hidden="true" hidden>…/);
  assert.equal((html.match(/class="section-sep"/g) ?? []).length, 3);
});
