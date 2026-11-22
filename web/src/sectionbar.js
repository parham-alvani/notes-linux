// The section bar's rules: which headings are the path of the place being
// read, and which of them fit. No DOM, so node tests them; the page
// (`reader.js`) measures and draws. The spec is `docs/specs/look.md` in
// notes-linux, "Section bar".

/**
 * The path to the heading at [at] in [levels] (each heading's level, in the
 * note's order): its parents first, then itself, as indexes. A parent is the
 * nearest earlier heading of a lower level, so `# A`, `### C` gives A, C.
 */
export function headingPath(levels, at) {
  if (at < 0 || at >= levels.length) return [];
  const path = [at];
  let level = levels[at];
  for (let index = at - 1; index >= 0 && level > 1; index--) {
    if (levels[index] < level) {
      path.unshift(index);
      level = levels[index];
    }
  }
  return path;
}

/**
 * The texts of the path of the place at file line [line]: the last heading
 * at or above it, with its parents. [headings] are `{ level, text, line }`
 * in the note's order; one with `away` is folded away and is not counted.
 * Empty before the first heading.
 */
export function pathAt(headings, line) {
  const shown = headings.filter((heading) => !heading.away);
  let at = -1;
  for (let index = 0; index < shown.length && shown[index].line <= line; index++) at = index;
  return headingPath(shown.map((heading) => heading.level), at).map((index) => shown[index].text);
}

/**
 * How many parents are left out from the start so the path fits in [room]:
 * [widths] are the headings' widths in order, the last one the heading
 * itself, which always shows; [separator] is the width of a `›` with its
 * space, and [dots] the width of the `…` put where parents were left out.
 */
export function leftOut(widths, separator, dots, room) {
  const total = (from) => {
    const shown = widths.slice(from);
    const sum = shown.reduce((all, width) => all + width, 0) + separator * (shown.length - 1);
    return from > 0 ? sum + dots + separator : sum;
  };
  let from = 0;
  while (from < widths.length - 1 && total(from) > room) from++;
  return from;
}
