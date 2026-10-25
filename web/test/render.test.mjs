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
  assert.match(html, /<img src="vault:\/\/\/uploads\/logo.png" alt="logo.png" width="300">/);
});

test("a note embed is a placeholder the page fills", () => {
  assert.match(render("![[Other#Part]]", { links }), /class="embed" data-path="Other.md" data-anchor="Part"/);
});

test("a task knows the file line it is on, front matter counted", () => {
  const text = "---\ntags: [a]\n---\n# Title\n\n- [ ] first\n- [x] second\n";
  assert.equal(frontMatterLines(text), 3);
  const html = render(text);
  assert.match(html, /data-line="5" data-status=" ">first/);
  assert.match(html, /data-line="6" data-status="x" checked>second/);
  assert.match(render("- [/] half"), /data-status="\/">half/);
});

test("a callout has a title and a body, and a fold marker makes it a details", () => {
  const html = render("> [!warning]- Careful now\n> The body.\n");
  assert.match(html, /<details class="callout" data-callout="warning" dir="auto">/);
  assert.match(html, /<summary class="callout-title">.*Careful now/);
  assert.match(html, /<div class="callout-content">\n<p dir="auto"[^>]*>The body.<\/p>/);
});

test("a callout with no title is named after its type", () => {
  assert.match(render("> [!tip]\n> Text\n"), /callout-title-inner">Tip</);
});

test("a plain quote stays a quote", () => {
  assert.match(render("> just a quote\n"), /<blockquote dir="auto"/);
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
  assert.match(render("سلام دنیا"), /<p dir="auto"/);
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
  assert.match(html, /<li dir="auto" data-line="2">one<\/li>/);
  assert.match(html, /<li dir="auto" data-line="3">two/);
  assert.match(html, /<li dir="auto" data-line="4">nested<\/li>/);
});
