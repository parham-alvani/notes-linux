// Sorting a table by a column, as a view: the rule that orders the rows.
// No DOM, so node tests it; the page (`reader.js`) moves the rows. The file
// is never changed. The spec is `docs/specs/look.md` in notes-linux,
// "Sorting by a column".

/** Digits with an optional sign, `,` between thousands, one `.`, and a `%` at the end. */
const NUMBER = /^[+-]?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?%?$|^[+-]?\.\d+%?$/;

/** The number a cell's text is, or null when it is not one. */
export function cellNumber(text) {
  const value = text.trim();
  if (!NUMBER.test(value)) return null;
  return Number(value.replace(/[,%]/g, ""));
}

/**
 * The order of the rows whose cells in one column read [values], as indexes
 * into the file's order. [direction] is 1 from small to large, -1 from large
 * to small and 0 the file's order. A column whose cells that are not empty
 * are all numbers goes by the number; any other goes by its text, capital
 * and small letters as the same. Empty cells are last both ways, and rows
 * with the same value keep the file's order.
 */
export function sortOrder(values, direction) {
  const all = values.map((_, index) => index);
  if (!direction) return all;
  const texts = values.map((value) => String(value ?? "").trim());
  const filled = all.filter((index) => texts[index] !== "");
  const empty = all.filter((index) => texts[index] === "");
  const numbers = texts.map(cellNumber);
  const numeric = filled.length > 0 && filled.every((index) => numbers[index] !== null);
  const keys = numeric ? numbers : texts.map((text) => text.toLowerCase());
  const compare = (a, b) => (keys[a] < keys[b] ? -1 : keys[a] > keys[b] ? 1 : 0);
  // `sort` is stable, so equal values stay in the file's order in both directions.
  filled.sort((a, b) => direction * compare(a, b));
  return [...filled, ...empty];
}

/**
 * The sort after a click on [column]: small to large, then large to small,
 * then the file's order (null). A click on another column starts again.
 */
export function nextSort(sort, column) {
  if (!sort || sort.column !== column) return { column, direction: 1 };
  if (sort.direction === 1) return { column, direction: -1 };
  return null;
}

/** The mark after a sorted header's text; none in the file's order. */
export function sortMark(direction) {
  return direction === 1 ? "↑" : direction === -1 ? "↓" : "";
}

/** What `aria-sort` says of a header cell. */
export function sortWord(direction) {
  return direction === 1 ? "ascending" : direction === -1 ? "descending" : "none";
}

/**
 * The sorts to put back after the same note is drawn again: [kept] are
 * `{ at, headers, sort }` of the tables that were sorted, [tables] the
 * header texts of each table now, from the note's start. A table keeps its
 * sort when the table at its place has the same header texts.
 */
export function keptSorts(kept, tables) {
  return kept.filter(({ at, headers }) => {
    const now = tables[at];
    return now && now.length === headers.length && now.every((text, index) => text === headers[index]);
  });
}
