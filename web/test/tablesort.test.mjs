// A table sorted by a click on a header cell, as a view
// (`docs/specs/look.md` in notes-linux, "Sorting by a column").
import { test } from "node:test";
import assert from "node:assert/strict";
import { cellNumber, sortOrder, nextSort, sortMark, sortWord, keptSorts } from "../src/tablesort.js";

/** The cells in the order a sort gives. */
const sorted = (cells, direction) => sortOrder(cells, direction).map((index) => cells[index]);

test("a column sorts by numbers or by text", () => {
  assert.deepEqual(sorted(["10", "9", "100"], 1), ["9", "10", "100"]);
  assert.deepEqual(sorted(["10", "9", "100"], -1), ["100", "10", "9"]);
  assert.deepEqual(sorted(["1,200", "85%", "-3"], 1), ["-3", "85%", "1,200"]);
  assert.deepEqual(sorted(["b", "A", "c"], 1), ["A", "b", "c"]);
  // One cell that is no number makes it a text column.
  assert.deepEqual(sorted(["10", "x", "9"], 1), ["10", "9", "x"]);
  // An empty cell is last in both directions.
  assert.deepEqual(sorted(["2", "", "1"], 1), ["1", "2", ""]);
  assert.deepEqual(sorted(["2", "", "1"], -1), ["2", "1", ""]);
  assert.deepEqual(sorted(["b", "  ", "a"], -1), ["b", "a", "  "]);
  // Two rows with the same value keep the file's order, both ways.
  assert.deepEqual(sortOrder(["b", "a", "B", "a"], 1), [1, 3, 0, 2]);
  assert.deepEqual(sortOrder(["b", "a", "B", "a"], -1), [0, 2, 1, 3]);
  // The third click gives the file's order, and the cells are the same before and after.
  const cells = ["10", "9", "100"];
  assert.deepEqual(sortOrder(cells, 0), [0, 1, 2]);
  assert.deepEqual(cells, ["10", "9", "100"]);
});

test("a date written YYYY-MM-DD is in order as text", () => {
  assert.deepEqual(sorted(["2026-11-22", "2025-01-05", "2026-02-01"], 1), ["2025-01-05", "2026-02-01", "2026-11-22"]);
});

test("a number is digits with a sign, thousands, one point and a percent sign", () => {
  assert.equal(cellNumber("10"), 10);
  assert.equal(cellNumber(" -3 "), -3);
  assert.equal(cellNumber("+4.5"), 4.5);
  assert.equal(cellNumber("1,200"), 1200);
  assert.equal(cellNumber("1,234,567.5"), 1234567.5);
  assert.equal(cellNumber("85%"), 85);
  assert.equal(cellNumber(".5"), 0.5);
  for (const text of ["", "x", "1.2.3", "1,20", "12,3456", "1 200", "10 kg", "%", "-", "2026-11-22", "1e3"]) {
    assert.equal(cellNumber(text), null, text);
  }
});

test("the clicks go small to large, large to small, then the file's order", () => {
  let sort = nextSort(null, 1);
  assert.deepEqual(sort, { column: 1, direction: 1 });
  sort = nextSort(sort, 1);
  assert.deepEqual(sort, { column: 1, direction: -1 });
  sort = nextSort(sort, 1);
  assert.equal(sort, null);
  // A click on another column starts at the first.
  assert.deepEqual(nextSort({ column: 1, direction: -1 }, 0), { column: 0, direction: 1 });
  assert.equal(sortMark(1), "↑");
  assert.equal(sortMark(-1), "↓");
  assert.equal(sortMark(0), "");
  assert.deepEqual([sortWord(1), sortWord(-1), sortWord(0)], ["ascending", "descending", "none"]);
});

test("a sort lasts through a redraw while the table at its place has the same header texts", () => {
  const kept = [
    { at: 0, headers: ["Name", "Age"], sort: { column: 1, direction: 1 } },
    { at: 2, headers: ["City"], sort: { column: 0, direction: -1 } },
  ];
  assert.deepEqual(keptSorts(kept, [["Name", "Age"], ["x"], ["City"]]), kept);
  // A header renamed, a column added or a table gone: the sort is gone.
  assert.deepEqual(keptSorts(kept, [["Name", "Years"], ["x"], ["City"]]), [kept[1]]);
  assert.deepEqual(keptSorts(kept, [["Name", "Age", "Town"], ["x"]]), []);
  assert.deepEqual(keptSorts([], [["Name"]]), []);
});
