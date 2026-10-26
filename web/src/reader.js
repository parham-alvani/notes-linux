// The page runtime: takes a note from the app, renders it, and reports back
// what the person does with it. Every message to the app goes through
// `post`, and the app calls `daftar.show` and `daftar.scrollTo`.

import { render, renderInline, isClosed } from "./render.js";

const app = window.webkit?.messageHandlers;

function post(message) {
  app?.daftar?.postMessage(message);
}

let current = { path: "" };
let mermaidLoading = null;

function loadMermaid() {
  if (window.mermaid) return Promise.resolve(window.mermaid);
  mermaidLoading ??= new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "mermaid.js";
    script.onload = () => resolve(window.mermaid);
    script.onerror = reject;
    document.head.appendChild(script);
  });
  return mermaidLoading;
}

async function drawDiagrams(root) {
  const nodes = [...root.querySelectorAll("pre.mermaid")];
  if (nodes.length === 0) return;
  const mermaid = await loadMermaid();
  // The diagram takes the reader's palette, naz or Daylight.
  const css = getComputedStyle(document.documentElement);
  const color = (name) => css.getPropertyValue(name).trim();
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  mermaid.initialize({
    startOnLoad: false,
    theme: "base",
    securityLevel: "strict",
    fontFamily: "-apple-system, sans-serif",
    themeVariables: {
      darkMode: dark,
      background: color("--background"),
      primaryColor: color("--panel"),
      primaryTextColor: color("--text"),
      primaryBorderColor: color("--accent"),
      secondaryColor: color("--code-background"),
      tertiaryColor: color("--background"),
      lineColor: color("--accent"),
      textColor: color("--text"),
      noteBkgColor: color("--panel"),
      noteTextColor: color("--text"),
    },
  });
  for (const node of nodes) {
    try {
      await mermaid.run({ nodes: [node] });
    } catch (error) {
      node.classList.add("mermaid-error");
    }
  }
}

/**
 * The heading [anchor] names. `## **Bold** [[X]]` is named as written, or as
 * it reads, `Bold X`, as Obsidian writes the link: the marks are not compared.
 */
function heading(root, anchor) {
  const plain = (text) => text.replace(/[*_`=~[\]#|^:%]/g, "").replace(/\s+/g, " ").trim().toLowerCase();
  const wanted = plain(anchor);
  const headings = [...root.querySelectorAll("[data-heading]")];
  return headings.find((h) => plain(h.dataset.heading) === wanted) ?? headings.find((h) => plain(h.textContent) === wanted);
}

/** The part of a note an embed names: the section under a heading, or all of it. */
function section(root, anchor) {
  if (!anchor) return root;
  if (anchor.startsWith("^")) {
    const block = root.querySelector(`[id="${CSS.escape(anchor)}"]`);
    const wrap = document.createElement("div");
    if (block) wrap.appendChild(block.cloneNode(true));
    return wrap;
  }
  const start = heading(root, anchor);
  const wrap = document.createElement("div");
  if (!start) return wrap;
  const level = Number(start.tagName.slice(1));
  for (let node = start.nextElementSibling; node; node = node.nextElementSibling) {
    if (/^H[1-6]$/.test(node.tagName) && Number(node.tagName.slice(1)) <= level) break;
    wrap.appendChild(node.cloneNode(true));
  }
  return wrap;
}

/** Fills each `![[note]]` embed, one level deep: an embed inside an embed stays a link. */
async function fillEmbeds(root) {
  const embeds = [...root.querySelectorAll(".embed")];
  for (const embed of embeds) {
    const body = embed.querySelector(".embed-body");
    try {
      const note = await app?.embed?.postMessage({ path: embed.dataset.path });
      if (!note) continue;
      const holder = document.createElement("div");
      holder.innerHTML = render(note.text, { path: embed.dataset.path, links: note.links });
      holder.querySelectorAll(".embed").forEach((inner) => {
        const link = Object.assign(document.createElement("a"), {
          className: "internal-link",
          href: "#",
          textContent: inner.querySelector(".embed-title")?.textContent ?? "",
        });
        // The link still goes where the embed did.
        Object.assign(link.dataset, { path: inner.dataset.path ?? "", target: inner.dataset.target ?? "", anchor: inner.dataset.anchor ?? "" });
        inner.replaceWith(link);
      });
      // A checkbox in an embed belongs to another file; it is shown, not ticked.
      holder.querySelectorAll("input[type=checkbox]").forEach((box) => (box.disabled = true));
      body.replaceChildren(...section(holder, embed.dataset.anchor).childNodes);
      await drawDiagrams(body);
    } catch (error) {
      body.textContent = "";
    }
  }
}

/**
 * Each ` ```tasks ` block, answered by the app: groups as headings, then the
 * tasks, each ticked in its own file. Lines the app could not read are listed
 * above the result, so a filter that did nothing is never silent.
 */
async function fillQueries(root) {
  for (const block of root.querySelectorAll(".tasks-query")) {
    let result;
    try {
      result = await app?.tasks?.postMessage({ source: block.dataset.source, path: current.path });
    } catch (error) {
      result = null;
    }
    if (!result) continue;
    const parts = [];
    if (result.notUnderstood.length > 0) {
      const lines = result.notUnderstood.map((line) => `<li><code>${escape(line)}</code></li>`).join("");
      parts.push(`<div class="callout query-warning" data-callout="warning"><div class="callout-title">Not understood, so not applied</div><ul>${lines}</ul></div>`);
    }
    let open = [];
    for (const task of result.tasks) {
      task.groups.forEach((name, level) => {
        if (open[level] === name) return;
        open = open.slice(0, level);
        open[level] = name;
        if (name) parts.push(`<div class="query-group level-${level}" dir="auto">${escape(name)}</div>`);
      });
      const checked = isClosed(task.status) ? " checked" : "";
      // Each date as a chip in its own colour; a due date in the past is red.
      const today = new Date().toLocaleDateString("sv");
      const chip = (kind, emoji, day) => `<span class="query-date ${kind}">${emoji} ${escape(day)}</span>`;
      const dates = [
        task.due && !isClosed(task.status) && chip(task.due < today ? "is-late" : "is-due", "📅", task.due),
        task.due && isClosed(task.status) && chip("is-past", "📅", task.due),
        task.scheduled && chip("is-scheduled", "⏳", task.scheduled),
        task.done && chip("is-done", "✅", task.done),
      ].filter(Boolean).join("");
      parts.push(
        `<li class="task-list-item${checked ? " is-checked" : ""}" data-task="${escape(task.status)}" dir="auto">` +
          `<input type="checkbox" class="task-list-item-checkbox" data-path="${escape(task.path)}" data-line="${task.line}" data-status="${escape(task.status)}"${checked}>` +
          `<span class="query-text">${renderInline(task.text, { path: task.path, links: result.links })}</span>` +
          (dates ? ` <span class="query-dates">${dates}</span>` : "") +
          ` <a class="internal-link query-source" href="#" data-path="${escape(task.path)}">${escape(task.title)}</a></li>`,
      );
    }
    if (result.tasks.length === 0) parts.push(`<div class="query-empty">No tasks match.</div>`);
    parts.push(`<div class="query-count">${result.tasks.length} ${result.tasks.length === 1 ? "task" : "tasks"}</div>`);
    block.innerHTML = `<ul class="query-list">${parts.join("")}</ul>`;
  }
}

/** The view of a base chosen last, by the base, so a redraw keeps it. */
const chosenViews = new Map();

function renderCell(cell) {
  if (cell.check !== undefined) return `<input type="checkbox" disabled${cell.check ? " checked" : ""}>`;
  return cell.parts
    .map((part) =>
      part.path
        ? `<a class="internal-link" href="#" data-path="${escape(part.path)}">${escape(part.text)}</a>`
        : escape(part.text),
    )
    .join("");
}

function renderView(view) {
  const head = view.columns.map((name) => `<th dir="auto">${escape(name)}</th>`).join("");
  const body = view.groups
    .map((group) => {
      const title = group.name == null ? "" : `<tr class="base-group"><th colspan="${view.columns.length}" dir="auto">${escape(group.name)} <span class="base-group-count">${group.rows.length}</span></th></tr>`;
      const rows = group.rows
        .map((row) => `<tr>${row.cells.map((cell) => `<td dir="auto"${cell.numeric ? ' class="numeric"' : ""}>${renderCell(cell)}</td>`).join("")}</tr>`)
        .join("");
      return title + rows;
    })
    .join("");
  const empty = view.count === 0 ? `<div class="query-empty">No files match.</div>` : "";
  const note = view.type === "table" ? "" : ` · shown as a table, not ${escape(view.type)}`;
  return `<div class="base-scroll"><table class="base-table"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>${empty}<div class="query-count">${view.count} ${view.count === 1 ? "result" : "results"}${note}</div>`;
}

/** A base's views, with a tab for each, into [holder]. */
function renderBase(holder, result, key, wanted) {
  const views = result.views ?? [];
  let chosen = chosenViews.get(key) ?? views.findIndex((view) => view.name === wanted);
  if (chosen < 0 || chosen >= views.length) chosen = 0;
  const problems = (result.problems ?? []).map((line) => `<li>${escape(line)}</li>`).join("");
  const warning = problems ? `<div class="callout query-warning" data-callout="warning"><div class="callout-title">Not understood</div><ul>${problems}</ul></div>` : "";
  const tabs = views.length > 1
    ? `<div class="base-tabs" role="tablist">${views.map((view, at) => `<button class="base-tab${at === chosen ? " is-active" : ""}" data-at="${at}" dir="auto">${escape(view.name)}</button>`).join("")}</div>`
    : "";
  holder.innerHTML = warning + tabs + `<div class="base-view">${views[chosen] ? renderView(views[chosen]) : ""}</div>`;
  holder.querySelectorAll(".base-tab").forEach((tab) =>
    tab.addEventListener("click", () => {
      chosenViews.set(key, Number(tab.dataset.at));
      renderBase(holder, result, key, wanted);
    }),
  );
}

/** Each `![[x.base]]` and ` ```base ` block, run by the app. */
async function fillBases(root) {
  for (const block of root.querySelectorAll(".base-query")) {
    let result;
    try {
      result = await app?.base?.postMessage({ path: block.dataset.path ?? "", source: block.dataset.source ?? "", context: current.path });
    } catch (error) {
      result = null;
    }
    if (!result) continue;
    const key = `${current.path}|${block.dataset.path ?? block.dataset.source}`;
    renderBase(block, result, key, block.dataset.view);
  }
}

/**
 * Hover a link to a note to see it: after a moment a card shows the note,
 * or the section the link names, rendered as the reader renders it. The
 * card stays while the pointer is on it, so its links can be followed.
 */
const preview = { card: null, timer: null, hide: null, link: null };

function previewCard() {
  if (preview.card) return preview.card;
  const card = document.createElement("div");
  card.className = "preview";
  card.addEventListener("mouseenter", () => clearTimeout(preview.hide));
  card.addEventListener("mouseleave", hidePreviewSoon);
  document.body.appendChild(card);
  preview.card = card;
  return card;
}

async function showPreview(link) {
  const path = link.dataset.path;
  let note;
  try {
    note = await app?.embed?.postMessage({ path });
  } catch (error) {
    note = null;
  }
  if (!note || preview.link !== link) return;
  const card = previewCard();
  const holder = document.createElement("div");
  holder.innerHTML = render(note.text, { path, links: note.links });
  // A preview is one level deep, like an embed.
  holder.querySelectorAll(".embed, .tasks-query, .base-query").forEach((inner) => inner.remove());
  holder.querySelectorAll("input[type=checkbox]").forEach((box) => (box.disabled = true));
  const part = section(holder, link.dataset.anchor);
  const title = (path.split("/").pop() ?? path).replace(/\.md$/i, "");
  card.innerHTML = `<div class="preview-head"><span class="preview-title" dir="auto">${escape(title)}</span>${link.dataset.anchor ? `<span class="preview-anchor" dir="auto">› ${escape(link.dataset.anchor)}</span>` : ""}<span class="preview-path">${escape(path.split("/").slice(0, -1).join(" › "))}</span></div><div class="preview-body"></div>`;
  card.querySelector(".preview-body").replaceChildren(...part.childNodes);
  if (!card.querySelector(".preview-body").childNodes.length) card.querySelector(".preview-body").innerHTML = `<p class="preview-empty">The note is empty.</p>`;
  decorate(card);
  // Below the link, or above it when there is no room below.
  const box = link.getBoundingClientRect();
  card.classList.add("is-open");
  const width = Math.min(460, window.innerWidth - 24);
  card.style.width = `${width}px`;
  const left = Math.min(Math.max(12, box.left), window.innerWidth - width - 12);
  const below = window.innerHeight - box.bottom;
  card.style.left = `${left}px`;
  if (below > 260 || below > box.top) {
    card.style.top = `${box.bottom + 8}px`;
    card.style.bottom = "auto";
    card.style.maxHeight = `${Math.min(360, below - 20)}px`;
  } else {
    card.style.bottom = `${window.innerHeight - box.top + 8}px`;
    card.style.top = "auto";
    card.style.maxHeight = `${Math.min(360, box.top - 20)}px`;
  }
  await drawDiagrams(card);
}

function hidePreviewSoon() {
  clearTimeout(preview.timer);
  clearTimeout(preview.hide);
  preview.hide = setTimeout(() => {
    preview.card?.classList.remove("is-open");
    preview.link = null;
  }, 250);
}

document.addEventListener("mouseover", (event) => {
  const link = event.target.closest?.("a.internal-link");
  if (!link || link.closest(".preview") || !/\.md$/i.test(link.dataset.path ?? "")) return;
  if (link.dataset.path === current.path && !link.dataset.anchor) return;
  clearTimeout(preview.hide);
  if (preview.link === link) return;
  clearTimeout(preview.timer);
  preview.link = link;
  preview.timer = setTimeout(() => showPreview(link), 450);
});
// Leaving the link hides the card, each time: a listener on the link
// itself would be spent by a quick leave and come back.
document.addEventListener("mouseout", (event) => {
  const link = event.target.closest?.("a.internal-link");
  if (link && link === preview.link && !link.contains(event.relatedTarget)) hidePreviewSoon();
});
window.addEventListener("scroll", () => {
  if (preview.card?.classList.contains("is-open")) hidePreviewSoon();
}, { passive: true });

/** Click an image or a diagram to see it as large as the window allows. */
function zoom(node) {
  const overlay = document.createElement("div");
  overlay.className = "zoom";
  const copy = node.tagName === "IMG" ? Object.assign(document.createElement("img"), { src: node.src, alt: node.alt }) : node.cloneNode(true);
  copy.removeAttribute("width");
  copy.removeAttribute("height");
  copy.removeAttribute("style");
  overlay.appendChild(copy);
  const close = () => {
    overlay.remove();
    document.removeEventListener("keydown", onKey, true);
  };
  const onKey = (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  };
  overlay.addEventListener("click", close);
  document.addEventListener("keydown", onKey, true);
  document.body.appendChild(overlay);
}

/** A colour of the palette for each tag, the same one every time. */
const TAG_COLORS = ["--orange", "--aqua", "--pink", "--purple", "--chartreuse", "--blue", "--spring-green", "--vivid-yellow", "--red"];

function hue(text) {
  let hash = 0;
  for (const char of text) hash = (hash * 31 + char.codePointAt(0)) >>> 0;
  return TAG_COLORS[hash % TAG_COLORS.length];
}

/** The colour of a code block's language label. */
const LANGUAGE_COLORS = {
  go: "--aqua", python: "--vivid-yellow", py: "--vivid-yellow", bash: "--chartreuse", sh: "--chartreuse", shell: "--chartreuse", zsh: "--chartreuse",
  fish: "--chartreuse", powershell: "--blue", yaml: "--pink", yml: "--pink", toml: "--pink", json: "--orange", sql: "--purple", rust: "--orange",
  zig: "--orange", c: "--blue", cpp: "--blue", typescript: "--blue", ts: "--blue", javascript: "--vivid-yellow", js: "--vivid-yellow",
  hcl: "--purple", terraform: "--purple", swift: "--orange", kotlin: "--purple", java: "--red", ruby: "--red", lua: "--blue",
  dockerfile: "--aqua", nix: "--blue", html: "--orange", css: "--pink", markdown: "--grey", md: "--grey", text: "--grey", diff: "--spring-green",
};

/** The Tasks plugin's signifiers, each drawn as a chip in its colour. */
const TASK_META = /(📅|⏳|🛫|✅|❌|➕)\uFE0F?\s*(\d{4}-\d{2}-\d{2})|(🔁)\uFE0F?\s*([^📅⏳🛫✅❌➕🔺⏫🔼🔽⏬#]+?)(?=\s*(?:[📅⏳🛫✅❌➕🔺⏫🔼🔽⏬#]|$))|(🔺|⏫|🔼|🔽|⏬)\uFE0F?/gu;
const META_KIND = { "📅": "is-due", "⏳": "is-scheduled", "🛫": "is-start", "✅": "is-done", "❌": "is-cancelled", "➕": "is-created" };
const PRIORITY = { "🔺": ["highest", "⇈"], "⏫": ["high", "⇈"], "🔼": ["medium", "↑"], "🔽": ["low", "↓"], "⏬": ["lowest", "⇊"] };

function taskChips(item, today) {
  const closed = isClosed(item.dataset.task ?? " ");
  const walker = document.createTreeWalker(item, NodeFilter.SHOW_TEXT, {
    // Only this task's own line, not the tasks nested under it.
    acceptNode: (node) =>
      node.parentElement.closest("li") === item && !node.parentElement.closest("code, a, .query-date, .task-priority") ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
  });
  const nodes = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) if (node.data.search(TASK_META) >= 0) nodes.push(node);
  for (const node of nodes) {
    const html = escape(node.data).replace(TASK_META, (match, emoji, day, repeat, rule, priority) => {
      if (priority) {
        const [name, arrow] = PRIORITY[priority];
        return `<span class="task-priority is-${name}" title="${name} priority">${arrow}</span>`;
      }
      if (repeat) return `<span class="query-date is-repeat">🔁 ${rule.trim()}</span>`;
      let kind = META_KIND[emoji];
      if (kind === "is-due" && !closed && day < today) kind = "is-late";
      if (kind === "is-due" && closed) kind = "is-past";
      return `<span class="query-date ${kind}">${emoji} ${day}</span>`;
    });
    const span = document.createElement("span");
    span.innerHTML = html;
    node.replaceWith(...span.childNodes);
  }
}

/** What the page adds after rendering: tag colours, task chips and code block headers. */
function decorate(root) {
  const today = new Date().toLocaleDateString("sv");
  root.querySelectorAll("li.task-list-item").forEach((item) => {
    if (item.dataset.chips) return;
    item.dataset.chips = "1";
    taskChips(item, today);
  });
  root.querySelectorAll("a.tag").forEach((tag) => tag.style.setProperty("--tag-color", `var(${hue(tag.dataset.tag ?? tag.textContent)})`));
  root.querySelectorAll("pre:not(.mermaid) > code").forEach((code) => {
    const pre = code.parentElement;
    if (pre.parentElement?.classList.contains("code-block")) return;
    const lang = /language-([\w+#-]+)/.exec(code.className)?.[1] ?? "";
    const block = document.createElement("div");
    block.className = "code-block";
    const color = Object.hasOwn(LANGUAGE_COLORS, lang.toLowerCase()) ? LANGUAGE_COLORS[lang.toLowerCase()] : "--accent";
    block.style.setProperty("--lang-color", `var(${color})`);
    const head = document.createElement("div");
    head.className = "code-head";
    head.innerHTML = `<span class="code-lang">${escape(lang || "code")}</span><button class="code-copy" title="Copy"><span class="icon"></span><span class="label">Copy</span></button>`;
    head.querySelector("button").addEventListener("click", (event) => {
      event.stopPropagation();
      post({ kind: "copy", text: code.textContent });
      const button = event.currentTarget;
      button.classList.add("is-done");
      button.querySelector(".label").textContent = "Copied";
      setTimeout(() => {
        button.classList.remove("is-done");
        button.querySelector(".label").textContent = "Copy";
      }, 1400);
    });
    pre.replaceWith(block);
    block.append(head, pre);
  });
}

/** The thin bar at the top that fills as the note is read. */
function progress() {
  const bar = document.getElementById("progress");
  if (!bar) return;
  const height = document.documentElement.scrollHeight - window.innerHeight;
  bar.style.transform = `scaleX(${height > 0 ? Math.min(1, window.scrollY / height) : 0})`;
}
window.addEventListener("scroll", progress, { passive: true });
window.addEventListener("resize", progress);

/**
 * Find in the note: a bar at the top, every match marked, the current one
 * brighter. Enter and ⌘G go forward, Shift-Enter and ⇧⌘G back, Escape closes.
 * The marks are CSS highlights, so the note's own HTML is never changed.
 */
const finder = { bar: null, input: null, count: null, ranges: [], at: -1 };

function findBar() {
  if (finder.bar) return finder.bar;
  const bar = document.createElement("div");
  bar.className = "find-bar";
  bar.innerHTML =
    `<span class="find-icon"></span><input type="search" placeholder="Find in note" spellcheck="false">` +
    `<span class="find-count"></span>` +
    `<button class="find-previous" title="Previous (⇧⌘G)"></button><button class="find-next" title="Next (⌘G)"></button>` +
    `<button class="find-close" title="Close (Esc)"></button>`;
  document.body.appendChild(bar);
  finder.bar = bar;
  finder.input = bar.querySelector("input");
  finder.count = bar.querySelector(".find-count");
  finder.input.addEventListener("input", () => search(finder.input.value));
  finder.input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      step(event.shiftKey ? -1 : 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeFind();
    }
  });
  bar.querySelector(".find-previous").addEventListener("click", () => step(-1));
  bar.querySelector(".find-next").addEventListener("click", () => step(1));
  bar.querySelector(".find-close").addEventListener("click", closeFind);
  return bar;
}

function search(text) {
  finder.ranges = [];
  finder.at = -1;
  if (text) {
    // Matched in the text as it is: a lowercased copy can be longer, and
    // its offsets would not fit the node.
    const needle = new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu");
    const walker = document.createTreeWalker(document.getElementById("note"), NodeFilter.SHOW_TEXT, {
      acceptNode: (node) => (hidden(node) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      for (const match of node.data.matchAll(needle)) {
        if (match[0].length === 0) break;
        const range = document.createRange();
        range.setStart(node, match.index);
        range.setEnd(node, match.index + match[0].length);
        finder.ranges.push(range);
      }
    }
  }
  CSS.highlights?.set("find", new Highlight(...finder.ranges));
  if (finder.ranges.length > 0) {
    // The first match below the top of the window that can be seen.
    const first = finder.ranges.findIndex((range) => {
      const box = range.getBoundingClientRect();
      return box.height > 0 && box.top >= 0;
    });
    finder.at = (first >= 0 ? first : 0) - 1;
    step(1);
  } else {
    CSS.highlights?.delete("find-current");
    showCount();
  }
}

/** Text find does not count: TeX source, a closed fold, the code header's labels. */
function hidden(node) {
  const parent = node.parentElement;
  if (!parent || parent.closest(".katex-mathml, annotation, .code-head, style, script")) return true;
  const fold = parent.closest("details");
  return Boolean(fold && !fold.open && !parent.closest("summary"));
}

function step(direction) {
  if (finder.ranges.length === 0) return;
  finder.at = (finder.at + direction + finder.ranges.length) % finder.ranges.length;
  const range = finder.ranges[finder.at];
  CSS.highlights?.set("find-current", new Highlight(range));
  const box = range.getBoundingClientRect();
  if (box.top < 80 || box.bottom > window.innerHeight - 40) window.scrollBy({ top: box.top - window.innerHeight / 3, behavior: "smooth" });
  showCount();
}

function showCount() {
  const total = finder.ranges.length;
  finder.count.textContent = finder.input.value ? (total ? `${finder.at + 1} of ${total}` : "No match") : "";
  finder.bar.classList.toggle("is-empty", Boolean(finder.input.value) && total === 0);
}

function openFind() {
  const bar = findBar();
  bar.classList.add("is-open");
  const selected = window.getSelection()?.toString().trim();
  if (selected && !selected.includes("\n")) finder.input.value = selected;
  finder.input.focus();
  finder.input.select();
  search(finder.input.value);
}

function closeFind() {
  finder.bar?.classList.remove("is-open");
  CSS.highlights?.delete("find");
  CSS.highlights?.delete("find-current");
  finder.ranges = [];
}

function renderProperties(properties) {
  if (!properties || properties.length === 0) return "";
  const rows = properties
    .map(([key, value]) => `<tr><th>${escape(key)}</th><td dir="auto">${escape(value)}</td></tr>`)
    .join("");
  return `<table class="properties">${rows}</table>`;
}

function escape(text) {
  const div = document.createElement("div");
  div.textContent = String(text);
  return div.innerHTML;
}

window.daftar = {
  /**
   * Shows a note: `{ path, title, text, links, properties }`, then scrolls
   * to `heading` or `line` when given, or stays where it was with
   * `keepScroll` -- a note shown again after it changed on disk.
   */
  async show(note) {
    const keep = note.keepScroll ? window.scrollY : null;
    current = note;
    const root = document.documentElement.style;
    if (note.textSize) root.setProperty("--text-size", `${note.textSize}px`);
    if (note.fontText) root.setProperty("--font-text", note.fontText);
    // SF Pro's alternates mean other things in other fonts.
    root.setProperty("--font-features", note.fontText?.startsWith("-apple-system") === false ? "normal" : '"ss01" 1, "ss07" 1, "cv07" 1');
    if (note.fontCode) root.setProperty("--font-mono", note.fontCode);
    const main = document.getElementById("note");
    main.innerHTML =
      `<h1 class="inline-title" dir="auto">${escape(note.title)}</h1>` +
      renderProperties(note.properties) +
      render(note.text, { path: note.path, links: note.links });
    // A base is a wide table: it gets the whole window.
    main.classList.toggle("is-base", Boolean(note.base));
    if (note.base) {
      const holder = Object.assign(document.createElement("div"), { className: "base-file" });
      main.appendChild(holder);
      renderBase(holder, note.base, note.path, "");
    }
    if (keep !== null) window.scrollTo(0, keep);
    else if (note.heading) this.scrollTo(note.heading);
    else if (note.line != null) this.scrollToLine(note.line);
    else window.scrollTo(0, 0);
    // The reading cursor: on the line the editor was at, on the same block
    // when the note is shown again, or at the top in a note just opened.
    reading.restore(note.line ?? (keep !== null ? reading.line : null));
    decorate(main);
    progress();
    await Promise.all([drawDiagrams(main), fillEmbeds(main), fillQueries(main), fillBases(main)]);
    decorate(main);
    // Embeds, queries and pictures made the page longer.
    progress();
    reading.redraw();
    if (finder.bar?.classList.contains("is-open")) search(finder.input.value);
  },

  /** Scrolls to a heading by its text, or to a `^block` id. */
  scrollTo(anchor) {
    const main = document.getElementById("note");
    let target;
    if (anchor.startsWith("^")) target = main.querySelector(`[id="${CSS.escape(anchor)}"]`);
    else target = heading(main, anchor);
    target?.scrollIntoView({ block: "start" });
  },

  /** Scrolls to the block that holds a file line. */
  scrollToLine(line) {
    const blocks = [...document.querySelectorAll("#note [data-line]")];
    const target = blocks.filter((b) => Number(b.dataset.line) <= line).pop();
    target?.scrollIntoView({ block: "start" });
  },

  /** Edit › Find, Find Next and Find Previous, from the app's menu. */
  findCommand(action) {
    if (action === "open" || !finder.bar?.classList.contains("is-open")) openFind();
    else step(action === "previous" ? -1 : 1);
  },

  /** Selects the first place [text] is in the note. */
  find(text) {
    window.getSelection()?.removeAllRanges();
    window.find?.(text, false, false, true);
  },
};

// A right click on a task opens the app's edit form for it.
document.addEventListener("contextmenu", (event) => {
  const item = event.target.closest("li.task-list-item");
  const box = item?.querySelector(":scope > input.task-list-item-checkbox, :scope > p > input.task-list-item-checkbox");
  if (!box || box.disabled) return;
  event.preventDefault();
  post({ kind: "editTask", line: Number(box.dataset.line), path: box.dataset.path ?? "" });
});

document.addEventListener("click", (event) => {
  const box = event.target.closest("input.task-list-item-checkbox");
  if (box && !box.disabled) {
    event.preventDefault();
    post({ kind: "toggleTask", line: Number(box.dataset.line), status: box.dataset.status, path: box.dataset.path ?? "" });
    return;
  }
  const link = event.target.closest("a");
  if (!link) {
    const picture = event.target.closest("#note img, #note pre.mermaid svg");
    if (picture && !event.target.closest(".zoom")) zoom(picture);
    return;
  }
  if (link.classList.contains("tag")) {
    event.preventDefault();
    post({ kind: "tag", tag: link.dataset.tag });
    return;
  }
  if (link.classList.contains("internal-link")) {
    event.preventDefault();
    preview.card?.classList.remove("is-open");
    preview.link = null;
    const path = link.dataset.path;
    if (path === current.path && link.dataset.anchor) {
      window.daftar.scrollTo(link.dataset.anchor);
      return;
    }
    post({ kind: "open", path, target: link.dataset.target ?? "", anchor: link.dataset.anchor ?? "", newTab: event.metaKey || event.ctrlKey });
    return;
  }
  const href = link.getAttribute("href") ?? "";
  if (href.startsWith("#")) {
    // A footnote, or a link to a heading in this note.
    event.preventDefault();
    const anchor = decoded(href.slice(1));
    const target = document.getElementById(anchor);
    if (target) target.scrollIntoView({ block: "center" });
    else window.daftar.scrollTo(anchor);
    return;
  }
  event.preventDefault();
  if (!href) return;
  // `[x](Notes/a.md)`: a note in the vault, by its path.
  if (!/^[a-z][a-z0-9+.-]*:/i.test(href)) {
    const [path, anchor = ""] = href.split("#");
    post({ kind: "open", path: "", target: decoded(path).replace(/^\.\//, ""), anchor: decoded(anchor), newTab: event.metaKey || event.ctrlKey });
    return;
  }
  post({ kind: "external", url: link.href });
});

// A middle click on a note's link opens it in a new tab, as a browser does;
// its press would paste the selection or start scrolling.
document.addEventListener("mousedown", (event) => {
  if (event.button === 1 && event.target.closest?.("a.internal-link")) event.preventDefault();
});
document.addEventListener("auxclick", (event) => {
  const link = event.button === 1 && event.target.closest?.("a.internal-link");
  if (!link) return;
  event.preventDefault();
  preview.card?.classList.remove("is-open");
  preview.link = null;
  post({ kind: "open", path: link.dataset.path, target: link.dataset.target ?? "", anchor: link.dataset.anchor ?? "", newTab: true });
});

/**
 * The reading cursor: one block at a time -- a paragraph, a list item, a
 * heading -- tinted, with the accent bar the editor's cursor line has.
 * `j` and `k` or the arrows move it, `g` and `G` or Home and End go to the
 * ends, a click puts it there. The app hears its line, so the editor opens
 * where the reading is.
 */
const reading = {
  block: null,
  line: null,
  box: null,

  /** The blocks in order: the innermost with a source line, not an embed's. */
  blocks() {
    return [...document.querySelectorAll("#note [data-line]")].filter(
      (el) => el.tagName !== "INPUT" && !el.parentElement.closest(".embed-body, .preview") && (el.tagName === "LI" || !el.querySelector("[data-line]:not(input)")),
    );
  },

  /** Where [block] is on the page: a list item without the list in it. */
  rect(block) {
    const box = block.getBoundingClientRect();
    let bottom = box.bottom;
    const nested = block.tagName === "LI" ? block.querySelector(":scope > ul, :scope > ol") : null;
    if (nested) bottom = nested.getBoundingClientRect().top;
    const main = document.getElementById("note").getBoundingClientRect();
    return { top: box.top + window.scrollY, height: Math.max(bottom - box.top, 4), left: main.left + window.scrollX, width: main.width };
  },

  place(block, scroll) {
    if (!block) return;
    this.block = block;
    this.line = Number(block.dataset.line);
    if (!this.box) {
      this.box = Object.assign(document.createElement("div"), { id: "reading-cursor" });
      document.body.appendChild(this.box);
    }
    this.redraw();
    if (scroll) {
      const { top, height } = this.rect(block);
      const above = top - 80 < window.scrollY;
      const below = top + height + 60 > window.scrollY + window.innerHeight;
      // A block taller than the window starts at the top.
      if (above || (below && height > window.innerHeight - 140)) window.scrollTo({ top: top - 80, behavior: "smooth" });
      else if (below) window.scrollTo({ top: top + height + 60 - window.innerHeight, behavior: "smooth" });
    }
    post({ kind: "cursor", line: this.line });
  },

  redraw() {
    if (!this.box) return;
    if (!this.block?.isConnected) {
      this.box.classList.remove("is-on");
      return;
    }
    const { top, height, left, width } = this.rect(this.block);
    Object.assign(this.box.style, { top: `${top - 3}px`, height: `${height + 6}px`, left: `${left - 14}px`, width: `${width + 28}px` });
    this.box.classList.add("is-on");
  },

  /** On the block of [line], after the note was drawn again; on the first
   * block on screen in a note just opened. */
  restore(line) {
    if (line == null) {
      const first = this.blocks().find((b) => b.getBoundingClientRect().bottom > 0) ?? null;
      this.block = first;
      this.line = first ? Number(first.dataset.line) : null;
      this.redraw();
      return;
    }
    const block = this.blocks().filter((b) => Number(b.dataset.line) <= line).pop() ?? this.blocks()[0];
    this.block = block ?? null;
    this.line = block ? Number(block.dataset.line) : null;
    this.redraw();
  },

  move(step) {
    const blocks = this.blocks();
    if (blocks.length === 0) return;
    let at = blocks.indexOf(this.block);
    const top = this.block?.getBoundingClientRect().top;
    // Scrolled away from it: the cursor starts again from what is on screen.
    if (at < 0 || top < -40 || top > window.innerHeight) {
      at = blocks.findIndex((b) => b.getBoundingClientRect().bottom > 60);
      if (at < 0) at = blocks.length - 1;
      return this.place(blocks[at], true);
    }
    this.place(blocks[Math.max(0, Math.min(blocks.length - 1, at + step))], true);
  },

  end(last) {
    const blocks = this.blocks();
    this.place(last ? blocks.at(-1) : blocks[0], true);
  },
};

document.addEventListener("keydown", (event) => {
  if (event.metaKey || event.ctrlKey || event.altKey || event.defaultPrevented) return;
  if (event.target.closest?.("input, textarea, [contenteditable], .find-bar") || document.querySelector(".zoom")) return;
  const moves = { j: 1, ArrowDown: 1, k: -1, ArrowUp: -1 };
  if (event.key in moves) reading.move(moves[event.key]);
  else if (event.key === "g" || event.key === "Home") reading.end(false);
  else if (event.key === "G" || event.key === "End") reading.end(true);
  else return;
  event.preventDefault();
});

document.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  const blocks = reading.blocks();
  for (let el = event.target; el && el.id !== "note"; el = el.parentElement) {
    if (blocks.includes(el)) return reading.place(el, false);
  }
});
window.addEventListener("resize", () => reading.redraw());

/** A percent-encoded part of a link, or the text as it is when it is not. */
function decoded(text) {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

post({ kind: "ready" });
