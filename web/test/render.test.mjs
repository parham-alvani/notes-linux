import { test } from "node:test";
import assert from "node:assert/strict";
import { render, renderInline, stripComments, frontMatterLines, parseWikilink, setRemoteImages, pictureAddress, hostOf, withoutRemotePictures } from "../src/render.js";

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

test("a figure callout is a numbered figure with its caption", () => {
  const env = { links: { "beach.png": "pictures/beach.png" } };
  const html = render("> [!figure]\n>\n> ![[beach.png|The beach at noon]]\n", env);
  assert.match(html, /^<figure class="figure" data-callout="figure" dir="auto">/);
  assert.doesNotMatch(html, /callout-title/);
  assert.match(html, /<img src="vault:\/\/\/pictures\/beach.png"[^>]*>[\s\S]*<figcaption><span class="figure-number">Figure 1.<\/span> The beach at noon<\/figcaption>\n<\/figure>/);
});

test("a figure's title is its caption, and a size is no caption", () => {
  const html = render("> [!figure] Low *tide*\n> ![[beach.png|300]]\n\n> [!figure]\n> ![[beach.png|300]]\n\n> [!figure]- Third\n> ![[beach.png]]\n");
  assert.match(html, /Figure 1.<\/span> Low <em>tide<\/em><\/figcaption>/);
  // The second has no caption, but is counted, as the PDF counts it.
  assert.equal((html.match(/<figcaption>/g) ?? []).length, 2);
  assert.match(html, /Figure 3.<\/span> Third<\/figcaption>/);
  assert.doesNotMatch(html, /<details/);
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

test("a block id names its list item, in a tight list too", () => {
  const html = render("- one ^item1\n- two\n    - nested ^deep");
  assert.match(html, /<li id="\^item1"[^>]*>one<\/li>/);
  assert.match(html, /<li id="\^deep"[^>]*>nested<\/li>/);
});

test("a block id on its own line names the table, quote or code above it", () => {
  const table = render("| a | b |\n|---|---|\n| 1 | 2 |\n\n^tbl\n\nAfter");
  assert.match(table, /<table id="\^tbl"/);
  assert.doesNotMatch(table, /\^tbl</);
  assert.match(table, /<p[^>]*>After<\/p>/);
  assert.match(render("> A quote\n\n^q1"), /<blockquote id="\^q1"/);
  assert.match(render("> [!tip] Title\n> Body\n\n^c1"), /<div class="callout"[^>]*id="\^c1"/);
  assert.match(render("```js\nx\n```\n\n^code1"), /<pre id="\^code1"><code class="language-js">/);
  // After a paragraph it is a paragraph of its own, as Obsidian shows it.
  assert.match(render("Text\n\n^p1"), /<p[^>]*>\^p1<\/p>/);
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

test("a picture from the web is a placeholder until clicked when remote images are off", () => {
  setRemoteImages(false);
  try {
    const remote = render("![a](https://x.org/a.png)", { links });
    assert.match(remote, /class="remote-image" data-src="https:\/\/x.org\/a.png"/);
    assert.match(remote, /remote-image-host">x.org</);
    assert.doesNotMatch(remote, /<img/);
    assert.match(render("![[logo.png]]", { links }), /<img src="vault:/);
    assert.match(render("![a](data:image/png;base64,AAAA)", { links }), /<img src="data:image\/png/);
    const html = render('<img src="HTTP://X.ORG/a.png" width="20">');
    assert.match(html, /data-width="20"/);
    assert.match(html, /remote-image-host">x.org</);
    assert.match(render("[link](https://x.org)", { links }), /<a href="https:\/\/x.org"/);
  } finally {
    setRemoteImages(true);
  }
  assert.match(render("![a](https://x.org/a.png)", { links }), /<img src="https:\/\/x.org\/a.png"/);
});

test("no picture from the web gets past the placeholder, however its address or its tag is written", () => {
  setRemoteImages(false);
  try {
    for (const source of [
      '<img src=" https://x.org/a.png">',
      '<img alt="a>b" src="https://x.org/a.png">',
      `<img alt="x src=vault:y" src='https://x.org/a.png'>`,
      "<IMG SRC=//x.org/a.png>",
      // Read as the browser reads it: one slash, none, backslashes, a tab, a character reference.
      "![a](https:/x.org/a.png)",
      "![a](https:x.org/a.png)",
      '<img src="https:\\\\x.org/a.png">',
      "![a](&#104;ttps://x.org/a.png)",
      '<img src="https:/x.org/a.png">',
      '<img src="&#104;ttps://x.org/a.png">',
      '<img src="https&colon;//x.org/a.png">',
      '<img src="ht\ttps://x.org/a.png">',
      '<img src="\u0001https://x.org/a.png">',
      '<img src="http://user@X.org:80/a.png">',
      "- [ ] task ![a](https:/x.org/a.png)",
      "> [!note] ![a](https:/x.org/a.png)\n> body ![a](https:x.org/b.png)",
      "| a |\n| --- |\n| ![a](https:/x.org/a.png) |",
      "![a][r]\n\n[r]: https:/x.org/a.png",
    ]) {
      const html = render(source);
      assert.doesNotMatch(html, /<img/i, source);
      assert.match(html, /remote-image-host">x\.org(:80)?</, source);
      assert.match(html, /data-src="https:\/\/(user@)?x\.org/i, source);
    }
    // An address of a `srcset` is not kept at all; the vault's picture is.
    const set = render('<img src="vault:///a.png" srcset="https://x.org/a.png 2x">');
    assert.match(set, /<img src="vault:\/\/\/a.png" alt="">/);
    assert.doesNotMatch(set, /x\.org/);
    // A task in a query and a property with a link are drawn by renderInline.
    assert.doesNotMatch(renderInline("Pay ![](https://x.org/p.png)", { links }), /<img/);
    assert.match(renderInline("Pay ![](https:/x.org/p.png)", { links }), /class="remote-image"/);
  } finally {
    setRemoteImages(true);
  }
  assert.match(render("![a](https://x.org/a.png)", { links }), /<img src="https:\/\/x.org\/a.png"/);
});

test("a picture's address is read as the browser reads it; plain http is asked for over https", () => {
  assert.deepEqual(pictureAddress("https://x.org/a.png"), { src: "https://x.org/a.png", remote: true });
  assert.deepEqual(pictureAddress(" HTTP:/x.org/a.png "), { src: "https://x.org/a.png", remote: true });
  assert.deepEqual(pictureAddress("//x.org/a.png"), { src: "https://x.org/a.png", remote: true });
  assert.deepEqual(pictureAddress("vault:///uploads/a.png"), { src: "vault:///uploads/a.png", remote: false });
  assert.deepEqual(pictureAddress("uploads/a.png"), { src: "uploads/a.png", remote: false });
  assert.equal(pictureAddress("javascript:alert(1)"), null);
  assert.equal(pictureAddress("file:///etc/passwd"), null);
  assert.equal(pictureAddress("data:text/html,<b>"), null);
  assert.equal(hostOf("https://User@X.org:8080/a/b?c#d"), "x.org:8080");
  assert.match(render("![a](http://x.org/a.png)"), /<img src="https:\/\/x.org\/a.png" alt="a">/);
});

test("a base card's cover from the web is a placeholder while pictures from the web are off", async () => {
  const { renderView } = await import("../src/bases.js");
  const view = { kind: "cards", columns: [], count: 1, layout: {}, groups: [{ name: null, rows: [{ path: "A.md", cells: [], image: { url: "https://x.org/c.png" } }] }] };
  assert.match(renderView(view), /<img src="https:\/\/x.org\/c.png" alt="" loading="lazy" style="object-fit: cover">/);
  setRemoteImages(false);
  try {
    assert.match(renderView(view), /class="remote-image" data-src="https:\/\/x.org\/c.png"/);
    assert.doesNotMatch(renderView(view), /<img/);
  } finally {
    setRemoteImages(true);
  }
});

test("a diagram's picture shape loses an address on the web, and keeps one in the vault", () => {
  assert.equal(withoutRemotePictures('A@{ img: "https://x.org/a.png", label: "x" }'), 'A@{ img: "", label: "x" }');
  assert.equal(withoutRemotePictures("A@{ img : 'http:/x.org/a.png' }"), "A@{ img : '' }");
  assert.equal(withoutRemotePictures('A@{ img: "vault:///a.png" }'), 'A@{ img: "vault:///a.png" }');
  assert.equal(withoutRemotePictures("A --> B"), "A --> B");
});

test("math KaTeX cannot draw is shown as its source, and the note is drawn", () => {
  const deep = "{".repeat(20000);
  for (const source of ["```math\n" + deep + "\n```\n\nafter", "```latex\n" + deep + "\n```\n\nafter", "$$\n" + deep + "\n$$\n\nafter", "$" + deep + "x$ after"]) {
    const html = render(source);
    assert.match(html, /<code class="math-error">\{\{/);
    assert.match(html, /after/);
  }
  assert.match(render("```math\n\\frac{1}{2}\n```"), /class="math math-block"><span class="katex-display">/);
});

test("a box with nothing after it is a task, and a box glued to its text is not", () => {
  assert.match(render("- [ ]\n"), /<li class="task-list-item" data-task=" "/);
  assert.match(render("- [x]\n"), /<li class="task-list-item is-checked" data-task="x"/);
  assert.doesNotMatch(render("- [x]done\n"), /task-list-item/);
  assert.doesNotMatch(render("- [x]**done**\n"), /task-list-item/);
});
