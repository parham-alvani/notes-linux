// The page runtime: takes a note from the app, renders it, and reports back
// what the person does with it. Every message to the app goes through
// `post`, and the app calls `daftar.show` and `daftar.scrollTo`.

import { render, renderInline, isClosed, hue, languageColor } from "./render.js";
import { renderView } from "./bases.js";
import { renderCanvas, bounds, fit } from "./canvas.js";
import { codeHead, findBar as findBarHtml, foldState, taskChip, baseTabs, canvasBar, zoomLabel } from "./controls.js";

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

/** A base's views, with a tab for each, into [holder]. */
function renderBase(holder, result, key, wanted) {
  const views = result.views ?? [];
  let chosen = chosenViews.get(key) ?? views.findIndex((view) => view.name === wanted);
  if (chosen < 0 || chosen >= views.length) chosen = 0;
  const problems = (result.problems ?? []).map((line) => `<li>${escape(line)}</li>`).join("");
  const warning = problems ? `<div class="callout query-warning" data-callout="warning"><div class="callout-title">Not understood</div><ul>${problems}</ul></div>` : "";
  const tabs = views.length > 1 ? baseTabs(views, chosen) : "";
  holder.innerHTML = warning + tabs + `<div class="base-view">${views[chosen] ? renderView(views[chosen]) : ""}</div>`;
  const choose = (at) => {
    chosenViews.set(key, at);
    renderBase(holder, result, key, wanted);
    holder.querySelector(".base-tab.is-active")?.focus();
  };
  holder.querySelectorAll(".base-tab").forEach((tab) => {
    tab.addEventListener("click", () => choose(Number(tab.dataset.at)));
    // A tab list's keys: the arrows go to the next and the previous view.
    tab.addEventListener("keydown", (event) => {
      const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
      if (!step) return;
      event.preventDefault();
      event.stopPropagation();
      choose((Number(tab.dataset.at) + step + views.length) % views.length);
    });
  });
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
  place(card, link);
  await drawDiagrams(card);
}

/** The card below [link], or above it when there is no room below. */
function place(card, link) {
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
}

/** A footnote's text in the card, where its number is: no jump to the end. */
function showFootnote(link) {
  const item = document.getElementById(decoded(link.getAttribute("href").slice(1)));
  if (!item || preview.link !== link) return;
  const card = previewCard();
  const body = item.cloneNode(true);
  body.querySelectorAll(".footnote-backref").forEach((back) => back.remove());
  card.innerHTML = `<div class="preview-head"><span class="preview-title">Footnote ${escape(link.textContent.replace(/[[\]]/g, ""))}</span></div><div class="preview-body"></div>`;
  card.querySelector(".preview-body").replaceChildren(...body.childNodes);
  place(card, link);
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
  const note = event.target.closest?.("sup.footnote-ref a");
  if (note && !note.closest(".preview")) {
    clearTimeout(preview.hide);
    if (preview.link === note) return;
    clearTimeout(preview.timer);
    preview.link = note;
    preview.timer = setTimeout(() => showFootnote(note), 250);
    return;
  }
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
  const link = event.target.closest?.("a.internal-link, sup.footnote-ref a");
  if (link && link === preview.link && !link.contains(event.relatedTarget)) hidePreviewSoon();
});
window.addEventListener("scroll", () => {
  if (preview.card?.classList.contains("is-open")) hidePreviewSoon();
}, { passive: true });

/** Click an image or a diagram to see it as large as the window allows. */
function zoom(node) {
  const overlay = document.createElement("div");
  overlay.className = "zoom";
  // A dialog that takes the focus and gives it back when it closes.
  const back = document.activeElement;
  Object.assign(overlay, { tabIndex: -1 });
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  overlay.setAttribute("aria-label", zoomLabel(node.getAttribute("alt"), node.tagName !== "IMG"));
  const copy = node.tagName === "IMG" ? Object.assign(document.createElement("img"), { src: node.src, alt: node.alt }) : node.cloneNode(true);
  copy.removeAttribute("width");
  copy.removeAttribute("height");
  copy.removeAttribute("style");
  overlay.appendChild(copy);
  const close = () => {
    overlay.remove();
    document.removeEventListener("keydown", onKey, true);
    back?.focus?.({ preventScroll: true });
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
  overlay.focus();
}

/** Says [text] with VoiceOver through the page's live region. */
function announce(text) {
  const region = document.getElementById("announce");
  if (!region) return;
  region.textContent = "";
  setTimeout(() => (region.textContent = text), 50);
}

/** Text whose first letter is right to left, as a Persian note's title. */
const RTL_FIRST = /^[^\p{L}]*[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFF]/u;

/** "smooth", or "auto" with Reduce Motion. */
const motion = () => (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth");

/** The Tasks plugin's signifiers, each drawn as a chip in its colour. */
const TASK_META = /(📅|⏳|🛫|✅|❌|➕)\uFE0F?\s*(\d{4}-\d{2}-\d{2})|(🔁)\uFE0F?\s*([^📅⏳🛫✅❌➕🔺⏫🔼🔽⏬#]+?)(?=\s*(?:[📅⏳🛫✅❌➕🔺⏫🔼🔽⏬#]|$))|(🔺|⏫|🔼|🔽|⏬)\uFE0F?/gu;

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
    const html = escape(node.data).replace(TASK_META, (match, emoji, day, repeat, rule, priority) =>
      taskChip({ emoji, day, rule: repeat ? rule : undefined, priority, closed, today }),
    );
    const span = document.createElement("span");
    // Chips side by side keep their gap as a margin: a space between them
    // would be struck through with a done task's text.
    span.innerHTML = html.replace(/(<\/span>)\s+(?=<span class="query-date)/g, "$1");
    node.replaceWith(...span.childNodes);
  }
}

/**
 * Sections fold shut from their heading, as in Obsidian's reading view: a
 * chevron before the heading, or `z` on the reading cursor's section. A fold
 * is named `level:text`, so an edit above it does not open it, and the app
 * keeps the folds of each note.
 */
const folds = {
  open: new Set(),
  shut: new Set(),

  key(heading) {
    return `${heading.tagName.slice(1)}:${heading.dataset.heading}`;
  },

  headings() {
    return [...document.querySelectorAll("#note > [data-heading]")];
  },

  /** Hides what each folded heading holds: everything after it up to a heading as big or bigger. */
  apply() {
    let shutLevel = Infinity;
    for (const child of document.getElementById("note").children) {
      const level = /^H[1-6]$/.test(child.tagName) && child.dataset.heading !== undefined ? Number(child.tagName[1]) : null;
      if (level !== null && level <= shutLevel) shutLevel = Infinity;
      const away = shutLevel !== Infinity;
      child.classList.toggle("folded-away", away);
      if (level !== null) {
        const shut = this.shut.has(this.key(child));
        child.classList.toggle("is-folded", shut);
        const toggle = child.querySelector(":scope > .fold-toggle");
        if (toggle) {
          const state = foldState(child.dataset.heading, shut);
          toggle.setAttribute("aria-label", state.label);
          toggle.setAttribute("aria-expanded", state.expanded);
        }
        if (shut && !away) shutLevel = level;
      }
    }
    reading.redraw();
    progress();
  },

  toggle(heading, save = true) {
    const key = this.key(heading);
    if (this.shut.has(key)) this.shut.delete(key);
    else this.shut.add(key);
    this.apply();
    if (save) post({ kind: "folds", folds: [...this.shut] });
  },

  /** All shut, or all open. */
  all(shut) {
    this.shut = new Set(shut ? this.headings().filter((h) => this.holds(h)).map((h) => this.key(h)) : []);
    this.apply();
    post({ kind: "folds", folds: [...this.shut] });
  },

  /** A heading with something under it before the next heading as big. */
  holds(heading) {
    const level = Number(heading.tagName[1]);
    const next = heading.nextElementSibling;
    return next !== null && !(next.dataset.heading !== undefined && Number(next.tagName[1]) <= level);
  },

  /** Opens every fold that hides [el]. */
  reveal(el) {
    let changed = false;
    while (el.closest?.(".folded-away")) {
      const shut = this.headings().filter((h) => h.classList.contains("is-folded") && h.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING);
      const last = shut.at(-1);
      if (!last) break;
      this.shut.delete(this.key(last));
      this.apply();
      changed = true;
    }
    if (changed) post({ kind: "folds", folds: [...this.shut] });
  },

  /** The heading of the section [block] is in. */
  sectionOf(block) {
    let top = block;
    while (top && top.parentElement?.id !== "note") top = top.parentElement;
    for (let el = top; el; el = el.previousElementSibling) {
      if (el.dataset?.heading !== undefined && /^H[1-6]$/.test(el.tagName)) return el;
    }
    return null;
  },
};

/**
 * Addresses and phone numbers the app found in the note's text, made links
 * where the page shows them: an address opens Maps, a number the phone.
 * Text in links, code, maths and tags is left alone.
 */
function linkDetected(root, items) {
  if (!items?.length) return;
  const wanted = [...new Map(items.map((item) => [item.text, item])).values()].sort((a, b) => b.text.length - a.text.length);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) =>
      node.parentElement.closest("a, code, pre, .katex, .mermaid, .inline-title, .properties, button") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
  });
  const nodes = [];
  while (walker.nextNode()) nodes.push(walker.currentNode);
  for (const node of nodes) {
    let text = node.textContent;
    const hits = [];
    for (const item of wanted) {
      for (let at = text.indexOf(item.text); at >= 0; at = text.indexOf(item.text, at + item.text.length)) {
        if (!hits.some((hit) => at < hit.at + hit.item.text.length && hit.at < at + item.text.length)) hits.push({ at, item });
      }
    }
    if (hits.length === 0) continue;
    hits.sort((a, b) => a.at - b.at);
    const parts = [];
    let from = 0;
    for (const { at, item } of hits) {
      parts.push(document.createTextNode(text.slice(from, at)));
      const link = Object.assign(document.createElement("a"), { className: `detected detected-${item.kind}`, textContent: item.text, href: "#" });
      link.dataset.kind = item.kind;
      link.dataset.value = item.value;
      link.title = item.kind === "phone" ? `Call ${item.text}` : "Show in Maps";
      parts.push(link);
      from = at + item.text.length;
    }
    parts.push(document.createTextNode(text.slice(from)));
    node.replaceWith(...parts);
  }
}

/** The blocks being read aloud, so the app can name one by its number. */
const speech = { blocks: [] };

/** What the page adds after rendering: tag colours, task chips and code block headers. */
function decorate(root) {
  if (root.id === "note") {
    root.querySelectorAll(":scope > [data-heading]").forEach((heading) => {
      if (heading.querySelector(".fold-toggle") || !folds.holds(heading)) return;
      const button = Object.assign(document.createElement("button"), { type: "button", className: "fold-toggle", title: "Fold or unfold this section (z)" });
      const state = foldState(heading.dataset.heading, heading.classList.contains("is-folded"));
      button.setAttribute("aria-label", state.label);
      button.setAttribute("aria-expanded", state.expanded);
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        folds.toggle(heading);
      });
      heading.prepend(button);
    });
  }
  // A link to each heading, to copy: `[[Note#Heading]]`.
  if (root.id === "note") {
    root.querySelectorAll(":scope > [data-heading]").forEach((heading) => {
      if (heading.querySelector(".heading-link")) return;
      const button = Object.assign(document.createElement("button"), { type: "button", className: "heading-link", title: "Copy a link to this heading", textContent: "#" });
      button.setAttribute("aria-label", "Copy a link to this heading");
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        const text = [...heading.childNodes].filter((node) => node.tagName !== "BUTTON").map((node) => node.textContent).join("").trim();
        post({ kind: "copy", text: `[[${current.title}#${text}]]` });
        announce("Link copied");
        button.textContent = "✓";
        setTimeout(() => (button.textContent = "#"), 1200);
      });
      heading.prepend(button);
    });
  }
  const today = new Date().toLocaleDateString("sv");
  root.querySelectorAll("li.task-list-item").forEach((item) => {
    if (item.dataset.chips) return;
    item.dataset.chips = "1";
    taskChips(item, today);
  });
  // An in-progress task is a half box: neither checked nor not.
  root.querySelectorAll('input.task-list-item-checkbox[data-status="/"]').forEach((box) => (box.indeterminate = true));
  // A picture opens large with a click, or with Return or Space on it.
  root.querySelectorAll("img, pre.mermaid svg").forEach((picture) => {
    if (picture.closest(".zoom, .preview, .base-card-cover") || picture.hasAttribute("tabindex")) return;
    picture.setAttribute("tabindex", "0");
    if (picture.tagName !== "IMG") {
      picture.setAttribute("role", "img");
      picture.setAttribute("aria-label", "Diagram");
    }
  });
  root.querySelectorAll("a.tag").forEach((tag) => tag.style.setProperty("--tag-color", `var(${hue(tag.dataset.tag ?? tag.textContent)})`));
  root.querySelectorAll("pre:not(.mermaid) > code").forEach((code) => {
    const pre = code.parentElement;
    if (pre.parentElement?.classList.contains("code-block")) return;
    const lang = /language-([\w+#-]+)/.exec(code.className)?.[1] ?? "";
    const block = document.createElement("div");
    block.className = "code-block";
    block.style.setProperty("--lang-color", `var(${languageColor(lang)})`);
    const head = document.createElement("div");
    head.className = "code-head";
    head.innerHTML = codeHead(lang);
    head.querySelector("button").addEventListener("click", (event) => {
      event.stopPropagation();
      post({ kind: "copy", text: code.textContent });
      announce("Copied");
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

/** The section at the top of the window, told to the app when it changes, for the outline. */
let lastSection = null;
function reportSection() {
  const headings = [...document.querySelectorAll("#note > [data-heading]")].filter((h) => !h.classList.contains("folded-away"));
  let at = null;
  for (const heading of headings) {
    if (heading.getBoundingClientRect().top > 90) break;
    at = Number(heading.dataset.line);
  }
  if (at === lastSection) return;
  lastSection = at;
  post({ kind: "section", line: at ?? -1 });
}
window.addEventListener("scroll", reportSection, { passive: true });

/** The thin bar at the top that fills as the note is read. */
function progress() {
  const bar = document.getElementById("progress");
  if (!bar) return;
  const height = document.documentElement.scrollHeight - window.innerHeight;
  const part = height > 0 ? Math.min(1, window.scrollY / height) : 0;
  bar.style.transform = `scaleX(${part})`;
  bar.setAttribute("aria-valuenow", String(Math.round(part * 100)));
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
  bar.setAttribute("role", "search");
  bar.innerHTML = findBarHtml();
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
  mark("find", finder.ranges);
  if (finder.ranges.length > 0) {
    // The first match below the top of the window that can be seen.
    const first = finder.ranges.findIndex((range) => {
      const box = range.getBoundingClientRect();
      return box.height > 0 && box.top >= 0;
    });
    finder.at = (first >= 0 ? first : 0) - 1;
    step(1);
  } else {
    mark("find-current", []);
    showCount();
  }
}

/**
 * [ranges] as the highlight [name]. One Highlight is kept and changed in
 * place, and the page drawn again: WebKitGTK leaves the old marks painted
 * when a highlight is replaced by a new one.
 */
function mark(name, ranges) {
  if (!globalThis.CSS?.highlights) return;
  let highlight = CSS.highlights.get(name);
  if (!highlight) {
    highlight = new Highlight();
    CSS.highlights.set(name, highlight);
  }
  highlight.clear();
  for (const range of ranges) highlight.add(range);
  const note = document.getElementById("note");
  if (note) {
    note.classList.toggle("repaint");
    void note.offsetHeight;
  }
}

/** Text find does not count: TeX source, a closed fold, the code header's labels. */
function hidden(node) {
  const parent = node.parentElement;
  if (!parent || parent.closest(".katex-mathml, annotation, .code-head, .heading-link, style, script")) return true;
  const fold = parent.closest("details");
  return Boolean(fold && !fold.open && !parent.closest("summary"));
}

function step(direction) {
  if (finder.ranges.length === 0) return;
  finder.at = (finder.at + direction + finder.ranges.length) % finder.ranges.length;
  const range = finder.ranges[finder.at];
  mark("find-current", [range]);
  const box = range.getBoundingClientRect();
  if (box.top < 80 || box.bottom > window.innerHeight - 40) window.scrollBy({ top: box.top - window.innerHeight / 3, behavior: motion() });
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
  mark("find", []);
  mark("find-current", []);
  finder.ranges = [];
}

/** The front matter as a card; a value with `[[links]]` links, as in Obsidian. */
function renderProperties(properties, note) {
  if (!properties || properties.length === 0) return "";
  const cell = (value) =>
    String(value).includes("[[") ? renderInline(String(value), { path: note.path, links: note.links }) : escape(value);
  const rows = properties.map(([key, value]) => `<tr><th>${escape(key)}</th><td dir="auto">${cell(value)}</td></tr>`).join("");
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
    main.classList.toggle("is-canvas", Boolean(note.canvas));
    if (note.canvas) {
      await showCanvas(main, note);
      return;
    }
    board.el = null;
    main.innerHTML =
      `<h1 class="inline-title" dir="${RTL_FIRST.test(note.title) ? "rtl" : "auto"}">${escape(note.title)}</h1>` +
      renderProperties(note.properties, note) +
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
    folds.shut = new Set(note.folds ?? []);
    linkDetected(main, note.detected);
    decorate(main);
    folds.apply();
    // A link to a heading inside a folded section opens the section.
    if (keep === null && note.heading) this.scrollTo(note.heading);
    reading.restore(note.line ?? (keep !== null ? reading.line : null));
    progress();
    await Promise.all([drawDiagrams(main), fillEmbeds(main), fillQueries(main), fillBases(main)]);
    decorate(main);
    // Embeds, queries and pictures made the page longer.
    progress();
    reading.redraw();
    lastSection = undefined;
    reportSection();
    if (finder.bar?.classList.contains("is-open")) search(finder.input.value);
  },

  /** Scrolls to a heading by its text, or to a `^block` id. */
  scrollTo(anchor) {
    const main = document.getElementById("note");
    let target;
    if (anchor.startsWith("^")) target = main.querySelector(`[id="${CSS.escape(anchor)}"]`);
    else target = heading(main, anchor);
    if (target) folds.reveal(target);
    target?.scrollIntoView({ block: "start" });
  },

  /**
   * The blocks to read aloud, from the reading cursor on: each with the
   * text a voice should say, code and diagrams left out.
   */
  speechBlocks() {
    const blocks = reading.blocks();
    const from = Math.max(0, blocks.indexOf(reading.block));
    speech.blocks = blocks;
    const out = [];
    blocks.forEach((block, index) => {
      if (index < from || block.closest("pre, .code-block, .mermaid, .katex-display, .properties")) return;
      const copy = block.cloneNode(true);
      copy.querySelectorAll("ul, ol, button, .task-chips, .footnote-ref, sup, input").forEach((node) => node.remove());
      const text = copy.textContent.replace(/\s+/g, " ").trim();
      if (text) out.push({ index, text });
    });
    return out;
  },

  /** The voice is on block [index] of the last `speechBlocks()`; -1 when it stopped. */
  speaking(index) {
    document.querySelectorAll(".is-speaking").forEach((el) => el.classList.remove("is-speaking"));
    const block = speech.blocks[index];
    if (!block?.isConnected) return;
    block.classList.add("is-speaking");
    reading.place(block, true);
  },

  /** Scrolls to the block that holds a file line. */
  scrollToLine(line) {
    const blocks = [...document.querySelectorAll("#note [data-line]")];
    const target = blocks.filter((b) => Number(b.dataset.line) <= line).pop();
    if (target) folds.reveal(target);
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

/**
 * A canvas on screen: its board, and the pan and zoom of its world. The
 * view of each canvas is kept, so a canvas drawn again after a change on
 * disk stays where it was.
 */
const board = { el: null, world: null, path: "", scale: 1, x: 0, y: 0, nodes: [], drag: null, moved: false, pinch: 1 };
const boardViews = new Map();

function applyBoard() {
  if (!board.world) return;
  board.world.style.transform = `translate(${board.x}px, ${board.y}px) scale(${board.scale})`;
  board.el.style.setProperty("--canvas-scale", board.scale);
  boardViews.set(board.path, { scale: board.scale, x: board.x, y: board.y });
}

/** Zooms by [factor] keeping the point under ([cx], [cy]) where it is. */
function zoomBoard(factor, cx, cy) {
  const scale = Math.min(4, Math.max(0.05, board.scale * factor));
  const rect = board.el.getBoundingClientRect();
  const px = (cx ?? rect.left + rect.width / 2) - rect.left, py = (cy ?? rect.top + rect.height / 2) - rect.top;
  board.x = px - ((px - board.x) * scale) / board.scale;
  board.y = py - ((py - board.y) * scale) / board.scale;
  board.scale = scale;
  applyBoard();
}

function fitBoard() {
  Object.assign(board, fit(bounds(board.nodes), board.el.clientWidth, board.el.clientHeight));
  applyBoard();
}

/** A `.canvas` file: read-only, panned with a scroll or a drag, zoomed with a pinch, ⌘-scroll or + and −. */
async function showCanvas(main, note) {
  const title = note.title.replace(/\.canvas$/i, "");
  main.innerHTML =
    `<div class="canvas-board" role="region" aria-label="Canvas ${escape(title)}" aria-describedby="canvas-summary">${renderCanvas(note.canvas, { path: note.path, links: note.links })}` +
    `${canvasBar(title)}</div>`;
  Object.assign(board, { el: main.querySelector(".canvas-board"), world: main.querySelector(".canvas-world"), path: note.path, nodes: note.canvas.nodes ?? [] });
  board.el.querySelector(".canvas-zoom-in").addEventListener("click", () => zoomBoard(1.25));
  board.el.querySelector(".canvas-zoom-out").addEventListener("click", () => zoomBoard(0.8));
  board.el.querySelector(".canvas-fit").addEventListener("click", fitBoard);
  const kept = note.keepScroll ? boardViews.get(note.path) : null;
  if (kept) {
    Object.assign(board, kept);
    applyBoard();
  } else fitBoard();
  // The board is read-only: a task on it is shown, not ticked.
  const lock = () => main.querySelectorAll("input[type=checkbox]").forEach((box) => (box.disabled = true));
  lock();
  decorate(main);
  await Promise.all([drawDiagrams(main), fillEmbeds(main), fillQueries(main), fillBases(main)]);
  lock();
  decorate(main);
}

const onBoard = (event) => board.el?.isConnected && board.el.contains(event.target);

/** A card's body that can still scroll that way takes the scroll; the board takes the rest. */
function scrollsInside(target, dx, dy) {
  for (let el = target; el && el !== board.el; el = el.parentElement) {
    if (!el.classList?.contains("canvas-node-body")) continue;
    if (dy < 0 && el.scrollTop > 0) return true;
    if (dy > 0 && el.scrollTop + el.clientHeight < el.scrollHeight - 1) return true;
    if (dx < 0 && el.scrollLeft > 0) return true;
    if (dx > 0 && el.scrollLeft + el.clientWidth < el.scrollWidth - 1) return true;
    return false;
  }
  return false;
}

document.addEventListener(
  "wheel",
  (event) => {
    if (!onBoard(event)) return;
    // A trackpad pinch comes as a wheel with Control; ⌘ and a mouse wheel zoom too.
    if (event.ctrlKey || event.metaKey) {
      event.preventDefault();
      zoomBoard(Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.002)), event.clientX, event.clientY);
      return;
    }
    if (scrollsInside(event.target, event.deltaX, event.deltaY)) return;
    event.preventDefault();
    board.x -= event.deltaX;
    board.y -= event.deltaY;
    applyBoard();
  },
  { passive: false },
);

// Safari's own pinch.
document.addEventListener("gesturestart", (event) => {
  if (!onBoard(event)) return;
  event.preventDefault();
  board.pinch = 1;
});
document.addEventListener("gesturechange", (event) => {
  if (!onBoard(event)) return;
  event.preventDefault();
  zoomBoard(event.scale / board.pinch, event.clientX, event.clientY);
  board.pinch = event.scale;
});

// A drag on the background, on a group, or with the middle button pans.
document.addEventListener("pointerdown", (event) => {
  if (!onBoard(event) || event.target.closest(".canvas-bar")) return;
  const onCard = event.target.closest(".canvas-node");
  if (onCard && event.button !== 1) return;
  board.drag = { x: event.clientX, y: event.clientY, bx: board.x, by: board.y, id: event.pointerId };
  board.moved = false;
  board.el.setPointerCapture(event.pointerId);
  board.el.classList.add("is-panning");
});
document.addEventListener("pointermove", (event) => {
  if (!board.drag || event.pointerId !== board.drag.id) return;
  const dx = event.clientX - board.drag.x, dy = event.clientY - board.drag.y;
  if (Math.abs(dx) + Math.abs(dy) > 3) board.moved = true;
  board.x = board.drag.bx + dx;
  board.y = board.drag.by + dy;
  applyBoard();
});
const endDrag = () => {
  board.drag = null;
  board.el?.classList.remove("is-panning");
};
document.addEventListener("pointerup", endDrag);
document.addEventListener("pointercancel", endDrag);

// A click on a file card, away from its links, opens the file.
document.addEventListener("click", (event) => {
  if (!onBoard(event)) return;
  const card = event.target.closest(".canvas-node[data-open]");
  if (!card || event.target.closest("a, button, input, img, video, audio, iframe, summary")) return;
  if (board.moved || window.getSelection()?.toString()) return;
  post({ kind: "open", path: card.dataset.open, target: "", anchor: "", newTab: event.metaKey || event.ctrlKey });
});

// A card reached with Tab comes into view; the board pans, it does not scroll.
document.addEventListener("focusin", (event) => {
  if (!onBoard(event)) return;
  const card = event.target.closest(".canvas-node");
  if (!card) return;
  const box = card.getBoundingClientRect(), view = board.el.getBoundingClientRect();
  const inside = box.left >= view.left && box.right <= view.right && box.top >= view.top && box.bottom <= view.bottom;
  if (inside) return;
  board.x += view.left + view.width / 2 - (box.left + box.width / 2);
  board.y += view.top + view.height / 2 - (box.top + box.height / 2);
  applyBoard();
});

/** The board's keys: + and − zoom, 0 fits, the arrows pan. True when taken. */
function boardKey(event) {
  const step = 60;
  const keys = {
    "+": () => zoomBoard(1.25), "=": () => zoomBoard(1.25), "-": () => zoomBoard(0.8), "0": fitBoard,
    ArrowLeft: () => { board.x += step; applyBoard(); }, ArrowRight: () => { board.x -= step; applyBoard(); },
    ArrowUp: () => { board.y += step; applyBoard(); }, ArrowDown: () => { board.y -= step; applyBoard(); },
  };
  if (!keys[event.key]) return false;
  keys[event.key]();
  event.preventDefault();
  return true;
}

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
  if (link?.classList.contains("detected")) {
    event.preventDefault();
    post({ kind: "detected", type: link.dataset.kind, value: link.dataset.value });
    return;
  }
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
    post({ kind: "open", path, target: link.dataset.target ?? "", anchor: link.dataset.anchor ?? "", newTab: event.metaKey || event.ctrlKey, otherPane: event.altKey });
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
    post({ kind: "open", path: "", target: decoded(path).replace(/^\.\//, ""), anchor: decoded(anchor), newTab: event.metaKey || event.ctrlKey, otherPane: event.altKey });
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
      (el) => el.tagName !== "INPUT" && !el.parentElement.closest(".embed-body, .preview") && !el.closest(".folded-away") && (el.tagName === "LI" || !el.querySelector("[data-line]:not(input)")),
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
      if (above || (below && height > window.innerHeight - 140)) window.scrollTo({ top: top - 80, behavior: motion() });
      else if (below) window.scrollTo({ top: top + height + 60 - window.innerHeight, behavior: motion() });
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

  /** Space: a page on, and the cursor on the first block of that page. */
  page(step) {
    window.scrollBy({ top: step * (window.innerHeight - 100), behavior: motion() });
    clearTimeout(this.paging);
    this.paging = setTimeout(() => {
      const blocks = this.blocks();
      const first = blocks.find((b) => b.getBoundingClientRect().top >= 70) ?? blocks.at(step > 0 ? -1 : 0);
      this.place(first, false);
    }, 350);
  },
};

document.addEventListener("keydown", (event) => {
  if (event.metaKey || event.ctrlKey || event.altKey || event.defaultPrevented) return;
  if (event.target.closest?.("input, textarea, [contenteditable], .find-bar") || document.querySelector(".zoom")) return;
  // Return or Space on a picture opens it large; on a file card, opens the file.
  if (event.key === "Enter" || event.key === " ") {
    const target = event.target;
    if (target.matches?.("#note img[tabindex], #note svg[tabindex]")) {
      event.preventDefault();
      zoom(target);
      return;
    }
    if (target.matches?.(".canvas-node[data-open]")) {
      event.preventDefault();
      post({ kind: "open", path: target.dataset.open, target: "", anchor: "", newTab: false });
      return;
    }
  }
  // A focused control takes its own Return and Space, not the reading cursor's.
  if ((event.key === "Enter" || event.key === " ") && event.target.closest?.("button, a[href], summary, [role=tab]")) return;
  // A canvas has no reading cursor: its keys pan and zoom.
  if (board.el?.isConnected) {
    boardKey(event);
    return;
  }
  if (event.key === "Escape" && document.querySelector(".is-speaking")) {
    post({ kind: "stopSpeaking" });
    event.preventDefault();
    return;
  }
  const moves = { j: 1, ArrowDown: 1, k: -1, ArrowUp: -1 };
  if (event.key in moves) reading.move(moves[event.key]);
  else if (event.key === "[" || event.key === "]") {
    // The previous or the next heading, as a reading cursor move.
    const headings = reading.blocks().filter((b) => /^H[1-6]$/.test(b.tagName));
    const line = reading.line ?? -1;
    const target = event.key === "]" ? headings.find((h) => Number(h.dataset.line) > line) : headings.filter((h) => Number(h.dataset.line) < line).pop();
    if (!target) return;
    reading.place(target, true);
  } else if (event.key === "g" || event.key === "Home") reading.end(false);
  else if (event.key === "G" || event.key === "End") reading.end(true);
  else if (event.key === " ") reading.page(event.shiftKey ? -1 : 1);
  else if (event.key === "e" && reading.line != null) post({ kind: "edit", line: reading.line });
  else if (event.key === "z" && reading.block) {
    const heading = /^H[1-6]$/.test(reading.block.tagName) ? reading.block : folds.sectionOf(reading.block);
    if (!heading || !folds.holds(heading)) return;
    folds.toggle(heading);
    reading.place(heading, true);
  } else if (event.key === "Z") folds.all(folds.shut.size === 0);
  else if (event.key === "Enter" && reading.block) {
    // The block's first link, as a click on it.
    const link = reading.block.querySelector("a.internal-link, a[href]:not(.footnote-backref):not(.heading-link)");
    if (!link) return;
    link.click();
  } else if (event.key === "x" && reading.block) {
    const box = reading.block.querySelector(":scope > input.task-list-item-checkbox, :scope > p > input.task-list-item-checkbox");
    if (!box || box.disabled) return;
    box.click();
  } else return;
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
