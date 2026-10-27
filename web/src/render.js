// Markdown to HTML the way Obsidian reads it. Pure: no DOM, so `node --test`
// runs it as it is. The page runtime (reader.js) does the rest -- mermaid,
// embeds, clicks -- after the HTML is in the document.

import MarkdownIt from "markdown-it";
import footnote from "markdown-it-footnote";
import katex from "katex";
import hljs from "highlight.js/lib/common";

const IMAGE = /\.(png|jpe?g|gif|webp|svg|bmp|avif|heic)$/i;
const VIDEO = /\.(mp4|m4v|mov|webm|ogv)$/i;
const AUDIO = /\.(mp3|m4a|wav|ogg|oga|flac|aac|opus)$/i;
const CALLOUT = /^\[!([\w-]+)\]([+-]?)[ \t]*(.*)$/;
const BLOCK_ID = /\s\^([A-Za-z0-9-]+)\s*$/;
const TASK = /^\[([^\]\n])\][ \t]+/;
const TAG = /^#([\p{L}\p{N}_\/-]+)/u;

/** Done or cancelled. In progress and any custom status are still open. */
export function isClosed(status) {
  return status === "x" || status === "X" || status === "-";
}

/** A colour of the palette for each tag, the same one every time. */
const TAG_COLORS = ["--orange", "--aqua", "--pink", "--purple", "--chartreuse", "--blue", "--spring-green", "--vivid-yellow", "--red"];

export function hue(text) {
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

export function languageColor(lang) {
  return Object.hasOwn(LANGUAGE_COLORS, lang.toLowerCase()) ? LANGUAGE_COLORS[lang.toLowerCase()] : "--accent";
}

/** The line count of the front matter, so a token's line maps back to the file. */
export function frontMatterLines(text) {
  if (!text.startsWith("---\n") && !text.startsWith("---\r\n")) return 0;
  const lines = text.split("\n");
  for (let i = 1; i < lines.length; i++) {
    if (/^(---|\.\.\.)\s*$/.test(lines[i])) return i + 1;
  }
  return 0;
}

/**
 * `%% comments %%` taken out, keeping every newline so that line numbers do
 * not move. Fenced code is left as it is: a `%%` there is code.
 */
export function stripComments(text) {
  const lines = text.split("\n");
  let fence = null;
  let open = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const marker = /^\s*(```+|~~~+)/.exec(line);
    if (!open && marker) {
      if (fence === null) fence = marker[1];
      else if (marker[1].startsWith(fence[0]) && marker[1].length >= fence.length) fence = null;
      continue;
    }
    if (fence !== null) continue;
    let out = "";
    let at = 0;
    while (at < line.length) {
      const next = line.indexOf("%%", at);
      if (next < 0) {
        if (!open) out += line.slice(at);
        break;
      }
      if (!open) out += line.slice(at, next);
      open = !open;
      at = next + 2;
    }
    lines[i] = out;
  }
  return lines.join("\n");
}

/** `target#heading|alias` in its three parts. */
export function parseWikilink(inner) {
  const bar = inner.indexOf("|");
  const ref = bar < 0 ? inner : inner.slice(0, bar);
  const alias = bar < 0 ? null : inner.slice(bar + 1);
  const hash = ref.indexOf("#");
  const target = (hash < 0 ? ref : ref.slice(0, hash)).trim();
  const anchor = hash < 0 ? null : ref.slice(hash + 1).trim();
  return { target, anchor, alias };
}

/** A file's name without its folder and extension: an image's alt text when the embed names none. */
export function baseName(path) {
  return (String(path).split("/").pop() ?? "").replace(/\.[^.]+$/, "");
}

/** A task's status for a screen reader: the box says checked or not, this says the rest. */
export function taskStatusWords(status) {
  if (status === " " || status === "x" || status === "X") return "";
  if (status === "/") return "in progress";
  if (status === "-") return "cancelled";
  return `status ${status}`;
}

/** Markdown as the words it shows: links as their text, no emphasis marks, one line. */
export function plainText(source) {
  return String(source)
    .replace(/!?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target, alias) => alias ?? target)
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_~=`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

const TASK_FIELDS = /[📅⏳🛫✅❌➕🔁🔺⏫🔼🔽⏬]/u;

/**
 * The name a task's checkbox has: its sentence without marks, links as
 * their words and the Tasks plugin's fields left out, then the status a
 * checkbox cannot show (in progress, cancelled).
 */
export function taskLabel(source, status) {
  const cut = source.search(TASK_FIELDS);
  const text = plainText(cut >= 0 ? source.slice(0, cut) : source);
  const words = taskStatusWords(status);
  return words ? `${text}, ${words}` : text;
}

export function escapeHtml(text) {
  return text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

/** A vault path as a URL the page can load: `vault:///uploads/a%20b.png`. */
export function vaultUrl(path) {
  return "vault:///" + path.split("/").map(encodeURIComponent).join("/");
}

function wikilinks(md) {
  md.inline.ruler.before("link", "wikilink", (state, silent) => {
    const src = state.src;
    let pos = state.pos;
    const embed = src.charCodeAt(pos) === 0x21; /* ! */
    if (embed) pos++;
    if (src.charCodeAt(pos) !== 0x5b || src.charCodeAt(pos + 1) !== 0x5b) return false;
    const end = src.indexOf("]]", pos + 2);
    if (end < 0) return false;
    const inner = src.slice(pos + 2, end);
    if (inner.includes("\n") || inner.length === 0) return false;
    if (!silent) {
      const token = state.push(embed ? "wikiembed" : "wikilink", "", 0);
      token.meta = parseWikilink(inner);
    }
    state.pos = end + 2;
    return true;
  });

  md.renderer.rules.wikilink = (tokens, idx, _options, env) => {
    const { target, anchor, alias } = tokens[idx].meta;
    const path = target === "" ? env.path : linkOf(env, target);
    const label = alias ?? (target === "" ? anchor : anchor ? `${target} > ${anchor}` : target);
    const cls = path ? "internal-link" : "internal-link is-unresolved";
    return (
      `<a class="${cls}" href="#" data-path="${escapeHtml(path ?? "")}"` +
      ` data-target="${escapeHtml(target)}" data-anchor="${escapeHtml(anchor ?? "")}"` +
      // A missing note is a colour to the eye: words to a screen reader.
      `${path ? "" : ` title="Click to make this note" aria-label="${escapeHtml(label)}, not a note yet"`}>${escapeHtml(label)}</a>`
    );
  };

  md.renderer.rules.wikiembed = (tokens, idx, _options, env) => {
    const { target, anchor, alias } = tokens[idx].meta;
    const path = linkOf(env, target);
    if (path && IMAGE.test(path)) {
      // `![[a.png|300]]` and `![[a.png|300x200]]` set the size; any other
      // text after the bar is the alt text.
      const size = alias && /^(\d+)(?:x(\d+))?$/.exec(alias);
      const width = size ? ` width="${size[1]}"` : "";
      const height = size && size[2] ? ` height="${size[2]}"` : "";
      const alt = size || !alias ? baseName(target) : alias;
      return `<img src="${vaultUrl(path)}" alt="${escapeHtml(alt)}"${width}${height}>`;
    }
    if (path && VIDEO.test(path)) {
      // `#t=0.1` makes the player show the first frame, not a black box.
      return `<video class="media" controls preload="metadata" src="${vaultUrl(path)}#t=0.1"></video>`;
    }
    if (path && AUDIO.test(path)) {
      return `<div class="media-audio"><span class="media-name">${escapeHtml(alias ?? target)}</span><audio controls preload="metadata" src="${vaultUrl(path)}"></audio></div>`;
    }
    if (path && /\.pdf$/i.test(path)) {
      return `<iframe class="media-pdf" src="${vaultUrl(path)}${anchor ? `#${escapeHtml(anchor)}` : ""}" title="${escapeHtml(target)}"></iframe>`;
    }
    if (path && /\.base$/i.test(path)) {
      // The app runs the base; `![[x.base#View]]` opens on that view.
      return `<div class="base-query" data-path="${escapeHtml(path)}" data-view="${escapeHtml(anchor ?? "")}"></div>`;
    }
    if (path && /\.md$/i.test(path)) {
      return (
        `<div class="embed" data-path="${escapeHtml(path)}" data-anchor="${escapeHtml(anchor ?? "")}">` +
        `<div class="embed-title">${escapeHtml(target)}</div><div class="embed-body"></div></div>`
      );
    }
    if (path) {
      return `<a class="internal-link attachment" href="#" data-path="${escapeHtml(path)}">${escapeHtml(alias ?? target)}</a>`;
    }
    return `<span class="internal-link is-unresolved" role="link" aria-disabled="true" aria-label="${escapeHtml(alias ?? target)}, not in the vault">${escapeHtml(alias ?? target)}</span>`;
  };
}

/** Obsidian's coloured highlights: `==🔴 text==` is red, and the circle is not shown. */
const HIGHLIGHT_COLORS = { "🔴": "red", "🟠": "orange", "🟡": "yellow", "🟢": "green", "🔵": "blue", "🟣": "purple" };

function highlights(md) {
  md.inline.ruler.before("emphasis", "mark", (state, silent) => {
    const src = state.src;
    const start = state.pos;
    if (src.charCodeAt(start) !== 0x3d || src.charCodeAt(start + 1) !== 0x3d) return false;
    const end = src.indexOf("==", start + 2);
    if (end < 0 || end === start + 2) return false;
    const inner = src.slice(start + 2, end);
    if (/^\s|\s$/.test(inner) || inner.includes("\n")) return false;
    if (!silent) {
      const open = state.push("mark_open", "mark", 1);
      const swatch = /^(🔴|🟠|🟡|🟢|🔵|🟣)\s?/u.exec(inner);
      if (swatch) open.attrSet("data-highlight", HIGHLIGHT_COLORS[swatch[1]]);
      const oldMax = state.posMax;
      state.pos = start + 2 + (swatch ? swatch[0].length : 0);
      state.posMax = end;
      state.md.inline.tokenize(state);
      state.posMax = oldMax;
      state.push("mark_close", "mark", -1);
    }
    state.pos = end + 2;
    return true;
  });
}

function tags(md) {
  md.inline.ruler.push("tag", (state, silent) => {
    const src = state.src;
    const pos = state.pos;
    if (src.charCodeAt(pos) !== 0x23) return false;
    if (pos > 0 && !/\s/.test(src[pos - 1])) return false;
    const match = TAG.exec(src.slice(pos));
    // `#123` is a number, not a tag, to Obsidian as well.
    if (!match || /^[\d\/-]+$/.test(match[1])) return false;
    if (!silent) {
      const token = state.push("tag", "", 0);
      token.content = match[1];
    }
    state.pos += match[0].length;
    return true;
  });
  md.renderer.rules.tag = (tokens, idx) => {
    const tag = escapeHtml(tokens[idx].content);
    return `<a class="tag" href="#" data-tag="${tag}">#${tag}</a>`;
  };
}

function math(md) {
  const render = (tex, displayMode) => {
    try {
      return katex.renderToString(tex, { displayMode, throwOnError: false, output: "htmlAndMathml" });
    } catch (error) {
      return `<code class="math-error">${escapeHtml(tex)}</code>`;
    }
  };

  md.block.ruler.before("fence", "math_block", (state, startLine, endLine, silent) => {
    const start = state.bMarks[startLine] + state.tShift[startLine];
    const first = state.src.slice(start, state.eMarks[startLine]);
    if (!first.startsWith("$$")) return false;
    let body = first.slice(2);
    let line = startLine;
    let closed = false;
    if (body.trimEnd().endsWith("$$") && body.trim().length >= 2) {
      body = body.trimEnd().slice(0, -2);
      closed = true;
    }
    const parts = [body];
    while (!closed && ++line < endLine) {
      const text = state.src.slice(state.bMarks[line] + state.tShift[line], state.eMarks[line]);
      if (text.trimEnd().endsWith("$$")) {
        parts.push(text.trimEnd().slice(0, -2));
        closed = true;
      } else {
        parts.push(text);
      }
    }
    if (!closed) return false;
    if (silent) return true;
    const token = state.push("math_block", "div", 0);
    token.content = parts.join("\n").trim();
    token.map = [startLine, line + 1];
    state.line = line + 1;
    return true;
  });

  md.inline.ruler.after("escape", "math_inline", (state, silent) => {
    const src = state.src;
    const start = state.pos;
    if (src.charCodeAt(start) !== 0x24 || src.charCodeAt(start + 1) === 0x24) return false;
    if (/\s/.test(src[start + 1] ?? " ")) return false;
    let end = start + 1;
    while ((end = src.indexOf("$", end)) >= 0) {
      if (src[end - 1] !== "\\") break;
      end++;
    }
    if (end < 0 || /\s/.test(src[end - 1]) || /\d/.test(src[end + 1] ?? "")) return false;
    if (!silent) {
      const token = state.push("math_inline", "span", 0);
      token.content = src.slice(start + 1, end);
    }
    state.pos = end + 1;
    return true;
  });

  md.renderer.rules.math_block = (tokens, idx) => `<div class="math math-block">${render(tokens[idx].content, true)}</div>\n`;
  md.renderer.rules.math_inline = (tokens, idx) => `<span class="math">${render(tokens[idx].content, false)}</span>`;
}

/**
 * `> [!type] Title` blockquotes as callouts, `+`/`-` making them foldable.
 * The first line of the quote becomes the title, and the rest of the quote is
 * the body.
 */
function callouts(md) {
  md.core.ruler.after("inline", "callout", (state) => {
    const tokens = state.tokens;
    for (let i = 0; i < tokens.length; i++) {
      if (tokens[i].type !== "blockquote_open") continue;
      const para = tokens[i + 1];
      const inline = tokens[i + 2];
      if (para?.type !== "paragraph_open" || inline?.type !== "inline") continue;
      const firstLine = inline.content.split("\n")[0];
      const match = CALLOUT.exec(firstLine);
      if (!match) continue;
      const [, type, fold, title] = match;

      // The title's inline children are the ones before the first break; the
      // marker itself is the text the first child starts with.
      const children = inline.children;
      const breakAt = children.findIndex((c) => c.type === "softbreak" || c.type === "hardbreak");
      const titleChildren = breakAt < 0 ? children : children.slice(0, breakAt);
      const rest = breakAt < 0 ? [] : children.slice(breakAt + 1);
      if (titleChildren[0]?.type === "text") {
        titleChildren[0] = Object.assign(Object.create(Object.getPrototypeOf(titleChildren[0])), titleChildren[0], {
          content: titleChildren[0].content.replace(/^\[![\w-]+\][+-]?[ \t]*/, ""),
        });
      }
      tokens[i].meta = { callout: type.toLowerCase(), fold, hasTitle: title.trim().length > 0, title: type };
      tokens[i].type = "callout_open";

      const titleToken = new state.Token("callout_title", "", 0);
      titleToken.children = titleChildren;
      titleToken.meta = tokens[i].meta;
      if (rest.length === 0) {
        // The quote's first paragraph was the title alone.
        tokens.splice(i + 1, 3, titleToken, new state.Token("callout_body_open", "div", 1));
      } else {
        inline.children = rest;
        inline.content = inline.content.split("\n").slice(1).join("\n");
        tokens.splice(i + 1, 0, titleToken, new state.Token("callout_body_open", "div", 1));
      }
      // The matching close, at the same nesting.
      let depth = 0;
      for (let j = i + 1; j < tokens.length; j++) {
        if (tokens[j].type === "blockquote_open" || tokens[j].type === "callout_open") depth++;
        if (tokens[j].type === "blockquote_close") {
          if (depth === 0) {
            tokens[j].type = "callout_close";
            tokens[j].meta = tokens[i].meta;
            tokens.splice(j, 0, new state.Token("callout_body_close", "div", -1));
            break;
          }
          depth--;
        }
        if (tokens[j].type === "callout_close") depth--;
      }
    }
  });

  md.renderer.rules.callout_open = (tokens, idx) => {
    const { callout, fold } = tokens[idx].meta;
    const tag = fold ? "details" : "div";
    const open = fold === "+" ? " open" : "";
    // A plain callout is a note to a screen reader; a folding one is a details already.
    return `<${tag} class="callout" data-callout="${escapeHtml(callout)}" dir="auto"${fold ? "" : ' role="note"'}${open}>`;
  };
  md.renderer.rules.callout_close = (tokens, idx) => (tokens[idx].meta.fold ? "</details>\n" : "</div>\n");
  md.renderer.rules.callout_title = (tokens, idx, options, env, self) => {
    const { fold, hasTitle, title, callout } = tokens[idx].meta;
    const tag = fold ? "summary" : "div";
    const text = hasTitle ? self.renderInline(tokens[idx].children, options, env) : escapeHtml(title[0].toUpperCase() + title.slice(1));
    // The icon is the callout's kind, which a custom title does not say.
    const kind = (callout ?? title ?? "").replace(/^./, (c) => c.toUpperCase());
    return `<${tag} class="callout-title"><span class="callout-icon" role="img" aria-label="${escapeHtml(kind)}"></span><span class="callout-title-inner">${text}</span></${tag}>\n`;
  };
  md.renderer.rules.callout_body_open = () => `<div class="callout-content">\n`;
  md.renderer.rules.callout_body_close = () => `</div>\n`;
}

/**
 * `- [ ] text` as a checkbox that knows the file line it came from, so a
 * click can ask the app to tick exactly that line.
 */
function tasks(md) {
  md.core.ruler.after("inline", "task", (state) => {
    const tokens = state.tokens;
    const offset = state.env.lineOffset ?? 0;
    for (let i = 2; i < tokens.length; i++) {
      const inline = tokens[i];
      if (inline.type !== "inline" || tokens[i - 1].type !== "paragraph_open" || tokens[i - 2].type !== "list_item_open") continue;
      const first = inline.children[0];
      if (first?.type !== "text") continue;
      const match = TASK.exec(first.content);
      if (!match) continue;
      const status = match[1];
      first.content = first.content.slice(match[0].length);
      const box = new state.Token("task_checkbox", "", 0);
      box.meta = { status, line: tokens[i - 2].map[0] + offset, label: taskLabel(inline.content.replace(TASK, ""), status) };
      inline.children.unshift(box);
      tokens[i - 2].attrJoin("class", "task-list-item");
      tokens[i - 2].attrSet("data-task", status);
      if (isClosed(status)) tokens[i - 2].attrJoin("class", "is-checked");
    }
  });
  md.renderer.rules.task_checkbox = (tokens, idx) => {
    const { status, line, label } = tokens[idx].meta;
    return `<input type="checkbox" class="task-list-item-checkbox" data-line="${line}" data-status="${escapeHtml(status)}"${isClosed(status) ? " checked" : ""} aria-label="${escapeHtml(label)}">`;
  };
}

/** A trailing `^block-id` is hidden, and names the paragraph or item it ends. */
function blockIds(md) {
  md.core.ruler.after("inline", "block_id", (state) => {
    const tokens = state.tokens;
    for (let i = 1; i < tokens.length; i++) {
      const inline = tokens[i];
      if (inline.type !== "inline") continue;
      const last = inline.children[inline.children.length - 1];
      if (last?.type !== "text") continue;
      const match = BLOCK_ID.exec(last.content);
      if (!match) continue;
      last.content = last.content.slice(0, match.index);
      tokens[i - 1].attrSet("id", "^" + match[1]);
    }
  });
}

/**
 * `dir="auto"` on every block, so each paragraph takes the direction of its
 * first strong letter: a Persian paragraph reads right to left beside an
 * English one, with no plugin and no front matter.
 */
function direction(md) {
  const blocks = new Set(["paragraph_open", "heading_open", "bullet_list_open", "ordered_list_open", "list_item_open", "blockquote_open", "th_open", "td_open"]);
  md.core.ruler.push("direction", (state) => {
    for (const token of state.tokens) if (blocks.has(token.type)) token.attrSet("dir", "auto");
  });
}

/**
 * Where a link goes. The table is the app's, keyed by the note's own text:
 * `[[constructor]]` must not find a property every object has.
 */
function linkOf(env, target) {
  return env.links && Object.hasOwn(env.links, target) ? env.links[target] : undefined;
}

/** Headings carry their text, which is what `[[Note#Heading]]` names. */
function headings(md) {
  md.core.ruler.push("heading_ids", (state) => {
    const tokens = state.tokens;
    const offset = state.env.lineOffset ?? 0;
    for (let i = 0; i < tokens.length; i++) {
      if (tokens[i].type !== "heading_open") continue;
      tokens[i].attrSet("data-heading", tokens[i + 1].content.trim());
      tokens[i].attrSet("data-line", String(tokens[i].map[0] + offset));
    }
  });
}

/** Source lines on blocks, so the reader can scroll to a line. */
function sourceLines(md) {
  md.core.ruler.push("source_lines", (state) => {
    const offset = state.env.lineOffset ?? 0;
    for (const token of state.tokens) {
      // Top-level blocks, and every list item: the reading cursor steps
      // through a list one item at a time.
      const block = token.level === 0 || token.type === "list_item_open";
      if (token.map && token.nesting === 1 && block && !token.attrGet("data-line")) {
        token.attrSet("data-line", String(token.map[0] + offset));
      }
    }
  });
}

function fences(md) {
  md.options.highlight = (code, lang) => {
    if (lang && hljs.getLanguage(lang)) {
      try {
        return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
      } catch (error) {
        return "";
      }
    }
    return "";
  };
  const base = md.renderer.rules.fence;
  md.renderer.rules.fence = (tokens, idx, options, env, self) => {
    const token = tokens[idx];
    const lang = token.info.trim().split(/\s+/)[0].toLowerCase();
    if (lang === "mermaid") return `<pre class="mermaid">${escapeHtml(token.content)}</pre>\n`;
    // The app runs the query on its index; the page fills the result in.
    if (lang === "tasks") return `<div class="tasks-query" data-source="${escapeHtml(token.content)}"></div>\n`;
    if (lang === "base") return `<div class="base-query" data-source="${escapeHtml(token.content)}"></div>\n`;
    if (lang === "math" || lang === "latex") {
      return `<div class="math math-block">${katex.renderToString(token.content, { displayMode: true, throwOnError: false })}</div>\n`;
    }
    return base(tokens, idx, options, env, self);
  };
}

export function createRenderer() {
  const md = new MarkdownIt({ html: true, linkify: true, breaks: false, typographer: false });
  md.use(footnote).use(wikilinks).use(highlights).use(tags).use(math).use(callouts).use(tasks).use(blockIds).use(direction).use(headings).use(sourceLines).use(fences);
  return md;
}

const shared = createRenderer();

/** One line of markdown -- a task's text in a query result -- as inline HTML. */
export function renderInline(text, { path = "", links = {} } = {}) {
  return shared.renderInline(text, { path, links, lineOffset: 0 });
}

/**
 * The note's body as HTML. [links] maps each wikilink target written in the
 * note to the vault path it resolves to; one that is missing is drawn as a
 * broken link.
 */
export function render(text, { path = "", links = {} } = {}) {
  const lineOffset = frontMatterLines(text);
  const body = text.split("\n").slice(lineOffset).join("\n");
  return shared.render(stripComments(body), { path, links, lineOffset });
}
