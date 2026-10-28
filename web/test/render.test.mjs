import { test } from "node:test";
import assert from "node:assert/strict";
import { render, stripComments, frontMatterLines, parseWikilink } from "../src/render.js";

const links = { Inbox: "Notes/Inbox.md", "logo.png": "uploads/logo.png", Other: "Other.md" };

test("a wikilink resolves to its path, and a broken one says so", () => {
  const html = render("See [[Inbox|the inbox]] and [[Missing]].", { links });
  assert.match(html, /data-path="Notes\/Inbox.md"[^>]*>the inbox<\/a>/);
  assert.match(html, /class="internal-link is-unresolved"[^>]*>Missing<\/a>/);
});

test("a heading link keeps its anchor", () => {
  assert.deepEqual(parseWikilink("Note#Part two|label"), { target: "Note", anchor: "Part two", alias: "label" });
  const html = render("[[#Local]]", { path: "A.md", links });
  assert.match(html, /data-path="A.md"[^>]*data-anchor="Local">Local</);
});

test("an image embed loads from the vault and takes a width", () => {
  const html = render("![[logo.png|300]]", { links });
  assert.match(html, /<img src="vault:\/\/\/uploads\/logo.png" alt="logo" width="300">/);
});

test("a note embed is a placeholder the page fills", () => {
  assert.match(render("![[Other#Part]]", { links }), /class="embed" data-path="Other.md" data-anchor="Part"/);
});

test("a task knows the file line it is on, front matter counted", () => {
  const text = "---\ntags: [a]\n---\n# Title\n\n- [ ] first\n- [x] second\n";
  assert.equal(frontMatterLines(text), 3);
  const html = render(text);
  assert.match(html, /data-line="5" data-status=" " aria-label="first">first/);
  assert.match(html, /data-line="6" data-status="x" checked aria-label="second">second/);
  assert.match(render("- [/] half"), /data-status="\/" aria-label="half, in progress">half/);
});

test("a callout has a title and a body, and a fold marker makes it a details", () => {
  const html = render("> [!warning]- Careful now\n> The body.\n");
  assert.match(html, /<details class="callout" data-callout="warning" dir="auto">/);
  assert.match(html, /<summary class="callout-title">.*Careful now/);
  assert.match(html, /<div class="callout-content">\n<p dir="ltr"[^>]*>The body.<\/p>/);
});

test("a callout with no title is named after its type", () => {
  assert.match(render("> [!tip]\n> Text\n"), /callout-title-inner">Tip</);
});

test("a plain quote stays a quote", () => {
  assert.match(render("> just a quote\n"), /<blockquote dir="ltr"/);
});

test("comments are hidden and the lines after them do not move", () => {
  assert.equal(stripComments("a %%hidden%% b\n%%\nblock\n%%\nc"), "a  b\n\n\n\nc");
  assert.equal(stripComments("```\n%% code %%\n```"), "```\n%% code %%\n```");
  assert.match(render("%%\nx\n%%\n- [ ] task"), /data-line="3"/);
});

test("highlights, tags and math", () => {
  const html = render("==marked== #project/one not#tag #123 $x^2$");
  assert.match(html, /<mark>marked<\/mark>/);
  assert.match(html, /data-tag="project\/one"/);
  assert.doesNotMatch(html, /data-tag="tag"/);
  assert.doesNotMatch(html, /data-tag="123"/);
  assert.match(html, /class="math"><span class="katex">/);
  assert.match(render("$$\na+b\n$$"), /class="math math-block"/);
});

test("a price is not math", () => {
  assert.doesNotMatch(render("It costs $5 and $10 later."), /katex/);
});

test("every block takes its own direction", () => {
  assert.match(render("سلام دنیا"), /<p dir="rtl"/);
  assert.match(render("Hello سلام"), /<p dir="ltr"/);
  assert.match(render("- [x] سلام\n- [[Start|خانه]]\n"), /<ul dir="rtl"[^>]*>\n<li class="task-list-item is-checked" data-task="x" dir="rtl"[^]*<li dir="rtl"/);
  assert.match(render("12:30"), /<p dir="auto"/);
});

test("a block id is hidden and names its paragraph", () => {
  assert.match(render("Some text ^abc-1"), /<p id="\^abc-1"[^>]*>Some text<\/p>/);
});

test("mermaid is left for the page to draw", () => {
  assert.match(render("```mermaid\ngraph TD; A-->B\n```"), /<pre class="mermaid">graph TD; A--&gt;B/);
});

test("a tasks query is left for the app to answer", () => {
  assert.match(render("```tasks\nnot done\n```"), /<div class="tasks-query" data-source="not done\n"><\/div>/);
});

test("code is coloured", () => {
  assert.match(render("```yaml\nkey: value\n```"), /hljs-attr/);
});

test("a base, embedded or written in a block, is left for the app to run", () => {
  const html = render("![[Shops.base#Open]]", { links: { "Shops.base": "Shops/Shops.base" } });
  assert.match(html, /<div class="base-query" data-path="Shops\/Shops.base" data-view="Open"><\/div>/);
  assert.match(render("```base\nfilters: file.ext == \"md\"\n```"), /<div class="base-query" data-source="filters: file.ext == &quot;md&quot;\n"><\/div>/);
});

test("a video, a sound and a PDF are embedded, each in its own player", () => {
  const links = { "walk.mp4": "uploads/walk.mp4", "talk.m4a": "uploads/talk.m4a", "paper.pdf": "uploads/paper.pdf" };
  const html = render("![[walk.mp4]]\n\n![[talk.m4a]]\n\n![[paper.pdf#page=2]]", { links });
  assert.match(html, /<video class="media" controls preload="metadata" src="vault:\/\/\/uploads\/walk.mp4#t=0.1"><\/video>/);
  assert.match(html, /<audio controls preload="metadata" src="vault:\/\/\/uploads\/talk.m4a"><\/audio>/);
  assert.match(html, /<iframe class="media-pdf" src="vault:\/\/\/uploads\/paper.pdf#page=2"/);
});

test("an emoji at the start of a highlight sets its colour and is not shown", () => {
  assert.match(render("==🔴 Never this== and ==🟢Fixed== and ==plain=="), /<mark data-highlight="red">Never this<\/mark> and <mark data-highlight="green">Fixed<\/mark> and <mark>plain<\/mark>/);
});

test("a note named as an object's own property is a link like any other", () => {
  const html = render("[[constructor]], [[toString]] and ![[__proto__]]", { links });
  assert.match(html, /<a class="internal-link is-unresolved" href="#" data-path="" data-target="constructor"/);
  assert.match(html, /data-target="toString"/);
  assert.match(render("[[constructor]]", { links: { constructor: "constructor.md" } }), /data-path="constructor.md"/);
});

test("every list item has its source line, for the reading cursor", () => {
  const html = render("Intro\n\n- one\n- two\n    - nested\n");
  assert.match(html, /<li dir="ltr" data-line="2">one<\/li>/);
  assert.match(html, /<li dir="ltr" data-line="3">two/);
  assert.match(html, /<li dir="ltr" data-line="4">nested<\/li>/);
});

import { renderView } from "../src/bases.js";

const cell = (text, path) => ({ parts: [{ text, path }], numeric: false });
const baseView = (kind, layout = {}, image) => ({
  name: "V", type: kind, kind: ["cards", "list"].includes(kind) ? kind : "table", layout, columns: ["file name", "stage"], count: 2,
  groups: [
    { name: "Open", rows: [{ path: "Shops/Bakery.md", cells: [cell("Bakery", "Shops/Bakery.md"), cell("open")], image }] },
    { name: "Closed", rows: [{ path: "Shops/Cafe.md", cells: [cell("Cafe", "Shops/Cafe.md"), { parts: [], numeric: false }] }] },
  ],
});

test("a table view draws a table, and an unknown view type falls back to it", () => {
  assert.match(renderView(baseView("table")), /<table class="base-table">.*<th scope="col" dir="auto">stage<\/th>/s);
  const kanban = renderView(baseView("kanban"));
  assert.match(kanban, /<table class="base-table">/);
  assert.match(kanban, /shown as a table, not kanban/);
});

test("a cards view draws a card per note with its cover and its properties", () => {
  const html = renderView(baseView("cards", { cardSize: 260, imageFit: "contain", imageAspectRatio: 0.5 }, { file: "uploads/a b.png" }));
  assert.match(html, /grid-template-columns: repeat\(auto-fill, minmax\(260px, 1fr\)\)/);
  assert.match(html, /aspect-ratio: 1 \/ 0.5"><img src="vault:\/\/\/uploads\/a%20b.png"[^>]*object-fit: contain/);
  assert.match(html, /base-card-title[^>]*><a class="internal-link" href="#" data-path="Shops\/Bakery.md">Bakery<\/a>/);
  assert.match(html, /base-card-label" dir="auto">stage<\/span><span class="base-card-value" dir="auto">open/);
  assert.match(html, /class="base-group-head" dir="auto">Open <span class="base-group-count">1/);
  // The name is the title, not a line; an empty value is no line.
  assert.equal(html.match(/base-card-line/g).length, 1);
  assert.match(renderView(baseView("cards", {}, { color: "#ff5070" })), /background: #ff5070/);
  assert.doesNotMatch(renderView(baseView("cards", {}, { color: "red;}" })), /red;/);
  assert.doesNotMatch(renderView(baseView("cards", {}, { url: "javascript:x" })), /javascript/);
});

test("a list view draws a line per note, its properties inline or indented", () => {
  const html = renderView(baseView("list", { markers: "number", separator: "," }));
  assert.match(html, /<ol class="base-list"><li dir="auto"><a class="internal-link" href="#" data-path="Shops\/Bakery.md">Bakery<\/a><span class="base-list-values"> <span class="base-list-value" title="stage">open<\/span>/);
  assert.match(renderView(baseView("list", { markers: "none" })), /<ul class="base-list is-plain">/);
  assert.match(renderView(baseView("list", { indentProperties: true })), /<ul><li dir="auto"><span class="base-list-label">stage<\/span> open<\/li><\/ul>/);
});

test("a date keeps its order in a right-to-left line", () => {
  assert.match(render("جلسه 2026-10-30 ساعت 10:30"), /جلسه <bdi dir="ltr">2026-10-30<\/bdi> ساعت 10:30/);
  // A task's dates are the page's chips.
  assert.doesNotMatch(render("- [ ] کار 📅 2026-10-30"), /<bdi/);
  assert.doesNotMatch(render("On 2026-10-30"), /<bdi/);
});

test("a markdown image of the vault is found as its links are, and sized with a bar", () => {
  const links = { "uploads/a b.svg": "uploads/a b.svg", "cat.gif": "pics/cat.gif" };
  const html = render("![one](uploads/a%20b.svg) ![two|240](cat.gif) ![web](https://example.org/x.png)", { links });
  assert.match(html, /<img src="vault:\/\/\/uploads\/a%20b\.svg" alt="one">/);
  assert.match(html, /<img src="vault:\/\/\/pics\/cat\.gif" alt="two" width="240">/);
  assert.match(html, /src="https:\/\/example\.org\/x\.png"/);
});

test("a task's reason leaves its text and follows it, muted", () => {
  const html = render("- [-] Deploy Qdrant [reason:: waiting on [[Inbox]]] ➕ 2025-05-17 ❌ 2025-11-16\n", { links });
  assert.match(html, /Deploy Qdrant <span class="task-reason"><span class="task-reason-mark" aria-hidden="true">💬<\/span> waiting on <a [^>]*data-path="Notes\/Inbox.md"[^>]*>Inbox<\/a><\/span> ➕/);
  assert.doesNotMatch(html, /reason::/);
  assert.doesNotMatch(html, //);
  // Only a task's first field is its reason; elsewhere it is text.
  assert.match(render("- [ ] a [reason:: x] b [reason:: y]\n"), /b \[reason:: y\]/);
  assert.match(render("Not a task [reason:: x]\n"), /\[reason:: x\]/);
});

test("findReason balances brackets and takes the first field", async () => {
  const { findReason } = await import("../src/render.js");
  assert.deepEqual(findReason("Ship [reason:: waiting on [[Amir]]] x"), { start: 5, end: 35, value: "waiting on [[Amir]]" });
  assert.equal(findReason("Ship [REASON ::  busy ]").value, "busy");
  assert.equal(findReason("Ship [reason:: ]").value, null);
  assert.equal(findReason("Ship [reason:: open"), null);
  assert.equal(findReason("Ship [reasons:: x]"), null);
});
