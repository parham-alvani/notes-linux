// The markup of the page's own controls: the code header, the find bar,
// the fold chevron, the task chips, the base tabs. Each is a real button
// or has a role and a name, so VoiceOver and the keyboard reach it. Kept
// apart from the page runtime so node can test it.

import { escapeHtml as escape } from "./render.js";

/** A code block's header: its language and a Copy button. */
export function codeHead(lang) {
  return (
    `<span class="code-lang">${escape(lang || "code")}</span>` +
    `<button type="button" class="code-copy" title="Copy" aria-label="Copy code"><span class="icon" aria-hidden="true"></span><span class="label">Copy</span></button>`
  );
}

/** The find bar: a search field, its count (said as it changes), and three buttons. */
export function findBar() {
  return (
    `<span class="find-icon" aria-hidden="true"></span><input type="search" placeholder="Find in note" aria-label="Find in note" spellcheck="false">` +
    `<span class="find-count" role="status" aria-live="polite"></span>` +
    `<button type="button" class="find-previous" title="Previous (⇧⌘G)" aria-label="Previous match"></button>` +
    `<button type="button" class="find-next" title="Next (⌘G)" aria-label="Next match"></button>` +
    `<button type="button" class="find-close" title="Close (Esc)" aria-label="Close find"></button>`
  );
}

/** The fold chevron's attributes for a heading's [text], open or shut. */
export function foldState(text, shut) {
  return { label: `${shut ? "Unfold" : "Fold"} “${text}”`, expanded: shut ? "false" : "true" };
}

const DATE_WORDS = { "📅": "due", "⏳": "scheduled", "🛫": "starts", "✅": "done", "❌": "cancelled", "➕": "created" };
const META_KIND = { "📅": "is-due", "⏳": "is-scheduled", "🛫": "is-start", "✅": "is-done", "❌": "is-cancelled", "➕": "is-created" };
export const PRIORITY = { "🔺": ["highest", "⇈"], "⏫": ["high", "⇈"], "🔼": ["medium", "↑"], "🔽": ["low", "↓"], "⏬": ["lowest", "⇊"] };

/**
 * One of the Tasks plugin's fields as a chip in its colour: a date, a
 * repeat rule or a priority. A late due date is red to the eye and
 * "overdue" to a screen reader; an arrow is "high priority".
 */
export function taskChip({ emoji, day, rule, priority, closed, today }) {
  if (priority) {
    const [name, arrow] = PRIORITY[priority];
    return `<span class="task-priority is-${name}" title="${name} priority" role="img" aria-label="${name} priority">${arrow}</span>`;
  }
  // [rule] comes from text escaped already.
  if (rule !== undefined) return `<span class="query-date is-repeat" role="img" aria-label="repeats ${rule.trim()}">🔁 ${rule.trim()}</span>`;
  let kind = META_KIND[emoji];
  const late = kind === "is-due" && !closed && day < today;
  if (late) kind = "is-late";
  if (kind === "is-due" && closed) kind = "is-past";
  const words = `${DATE_WORDS[emoji]} ${day}${late ? ", overdue" : ""}`;
  return `<span class="query-date ${kind}" role="img" aria-label="${words}">${emoji} ${day}</span>`;
}

/** A base's views as tabs: a tab list, the chosen tab selected and the only one in the Tab order. */
export function baseTabs(views, chosen) {
  const tabs = views
    .map((view, at) => {
      const on = at === chosen;
      return `<button type="button" role="tab" class="base-tab${on ? " is-active" : ""}" data-at="${at}" aria-selected="${on}" tabindex="${on ? 0 : -1}" dir="auto">${escape(view.name)}</button>`;
    })
    .join("");
  return `<div class="base-tabs" role="tablist" aria-label="Views">${tabs}</div>`;
}

/** The canvas's buttons, each with a name; − and + are not words. */
export function canvasBar(title) {
  return (
    `<div class="canvas-bar" role="toolbar" aria-label="Canvas"><span class="canvas-title" dir="auto">${escape(title)}</span>` +
    `<button type="button" class="canvas-zoom-out" title="Zoom out (−)" aria-label="Zoom out">−</button>` +
    `<button type="button" class="canvas-zoom-in" title="Zoom in (+)" aria-label="Zoom in">+</button>` +
    `<button type="button" class="canvas-fit" title="Show the whole canvas (0)" aria-label="Show the whole canvas">Fit</button></div>`
  );
}

/** What a picture opened large is called: its alt text, or "Picture". */
export function zoomLabel(alt, diagram) {
  if (diagram) return "Diagram. Escape closes it.";
  return `${alt ? alt : "Picture"}. Escape closes it.`;
}

/**
 * The bar of a picture: zoom out, the zoom in words, zoom in, fit, 100 %,
 * and on a picture file's own page, a button that opens it in its own app.
 */
export function imageBar({ external = false } = {}) {
  return (
    `<div class="image-bar" role="toolbar" aria-label="Zoom">` +
    `<button type="button" data-zoom="out" title="Zoom out (−)" aria-label="Zoom out">−</button>` +
    `<span class="image-percent" aria-label="Zoom">100 %</span>` +
    `<button type="button" data-zoom="in" title="Zoom in (+)" aria-label="Zoom in">+</button>` +
    `<button type="button" data-zoom="fit" title="Fit in the window (0)" aria-label="Fit in the window">Fit</button>` +
    `<button type="button" data-zoom="actual" title="Actual size (1)" aria-label="Actual size">1:1</button>` +
    (external ? `<button type="button" class="image-external" title="Open in the default app" aria-label="Open in the default app">Open in Default App</button>` : "") +
    `</div>`
  );
}

/** The line under a picture file: "1200 × 800 · 245 KB · PNG image", with what is known. */
export function imageInfo(width, height, size, type) {
  const parts = [];
  if (width > 0 && height > 0) parts.push(`${width} × ${height}`);
  if (size) parts.push(size);
  if (type) parts.push(type);
  return escape(parts.join(" · "));
}
