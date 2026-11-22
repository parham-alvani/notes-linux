// The section bar: the heading path of the place being read, and which
// headings fit (`docs/specs/look.md` in notes-linux, "Section bar").
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

// What stays at the top of the view hides the note's lines that pass under
// it: at 92% of the background, the dark palettes showed the text through
// the bar's words.
const css = readFileSync(new URL("../src/reader.css", import.meta.url), "utf8");
/** The declarations of the one rule with this selector. */
const rule = (selector) => {
  const found = css.split("}").filter((part) => part.slice(part.lastIndexOf("\n\n") + 1).split("{")[0].replace(/\/\*[^]*?\*\//g, "").trim() === selector);
  assert.equal(found.length, 1, selector);
  return found[0].slice(found[0].lastIndexOf("{") + 1);
};
const declared = (selector, property) => rule(selector).match(new RegExp(`(?:^|[;\\s])${property}:\\s*([^;]+);`))?.[1];

test("the section bar and the table header that stays are not see-through", () => {
  assert.equal(declared("#section-bar", "background"), "var(--background)");
  assert.equal(declared("#section-bar", "border-bottom"), "1px solid var(--table-hairline)");
  assert.equal(declared("#table-head", "background"), "var(--panel)");
  assert.equal(declared("table:not(.properties) thead th", "background")?.split(";")[0], "var(--panel)");
  // Shown, the bar is whole: nothing makes it clear again.
  assert.match(rule("#section-bar.is-on"), /opacity: 1;/);
  // Daylight, naz and naz-soft each set the two colours, with no alpha.
  for (const name of ["--background", "--panel"]) {
    const values = [...css.matchAll(new RegExp(`^\\s*${name}:\\s*([^;]+);`, "gm"))].map((match) => match[1]);
    assert.equal(values.length, 3, name);
    for (const value of values) assert.match(value, /^#[0-9a-f]{6}$/, `${name}: ${value}`);
  }
});

test("the layers at the top of the view stack in one order", () => {
  const z = (selector) => Number(declared(selector, "z-index"));
  // The table's header goes under the bar, the progress line is on the bar's
  // edge, and find, a card and a picture opened large are over all of them.
  const order = ["#table-head", "#section-bar", "#progress", ".find-bar", ".preview", ".zoom"].map(z);
  assert.ok(order.every((value) => Number.isInteger(value)), String(order));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
  assert.equal(new Set(order).size, order.length);
  // The reading cursor's layer has no place in that order: it is under every one of them.
  assert.equal(declared("#reading-layer", "z-index"), undefined);
});
