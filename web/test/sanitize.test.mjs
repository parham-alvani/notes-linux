import { test } from "node:test";
import assert from "node:assert/strict";
import { render, renderInline } from "../src/render.js";
import { sanitizeHtml, safeStyle, decodeEntities, safeLink } from "../src/sanitize.js";

test("the HTML notes write today is drawn as before", () => {
  assert.match(render("one<br>two<br/>three"), /one<br>two<br>three/);
  assert.match(render("<details>\n<summary>More</summary>\n\nBody **bold**\n\n</details>"), /<details>\n<summary>More<\/summary>\n<p[^>]*>Body <strong>bold<\/strong><\/p>\n<\/details>/);
  assert.match(render("<details open><summary>S</summary>x</details>"), /<details open="">/);
  assert.match(render("Press <kbd>⌘</kbd>, H<sub>2</sub>O, x<sup>2</sup>, <u>u</u> <mark>m</mark>"), /<kbd>⌘<\/kbd>, H<sub>2<\/sub>O, x<sup>2<\/sup>, <u>u<\/u> <mark>m<\/mark>/);
  assert.match(render('<img src="vault:///a.png" width="300" alt="A &amp; B">'), /<img src="vault:\/\/\/a.png" alt="A &amp; B" width="300">/);
  assert.match(render('<table><tr><td colspan="2" align="center">x</td></tr></table>'), /<td colspan="2" align="center">x<\/td>/);
  assert.match(render('<a href="https://example.org/?a=1&amp;b=2" title="t">x</a>'), /<a href="https:\/\/example.org\/\?a=1&amp;b=2" title="t">x<\/a>/);
  assert.match(render('<span style="color: red; position: fixed">x</span>'), /<span style="color: red">x<\/span>/);
});

test("a name in angle brackets that is no element is text, and shown", () => {
  assert.match(render("Dear <first name>, from <name>"), /Dear &lt;first name&gt;, from &lt;name&gt;/);
  assert.match(render("a < b and c > d, 1 <2"), /a &lt; b and c &gt; d, 1 &lt;2/);
});

test("what runs, loads or takes the page over is left out", () => {
  for (const [source, gone] of [
    ["<script>alert(1)</script>", /script|alert/],
    ["<style>#note { background: url(https://x.org/a.png) }</style>", /style|url/],
    ['<meta http-equiv="refresh" content="0;url=daftar://reader/reader.css">', /meta|refresh/],
    ['<base href="https://x.org/">', /base|x\.org/],
    ['<link rel="preload" as="image" href="https://x.org/a.png">', /link|x\.org/],
    ['<form action="daftar://reader/reader.css"><button>Go</button></form>', /form|button|action/],
    ['<iframe src="vault:///uploads/feed.rss"></iframe>', /iframe|feed/],
    ['<object data="https://x.org/a.swf"></object>', /object|x\.org/],
    ['<embed src="https://x.org/a.swf">', /embed|x\.org/],
    ['<svg><image href="https://x.org/a.png"/></svg>', /svg|image|x\.org/],
    ['<image src="https://x.org/a.png">', /image|x\.org/],
    ['<input type="image" src="https://x.org/a.png">', /input|x\.org/],
    ['<video poster="https://x.org/a.png"></video>', /video|x\.org/],
    ['<picture><source srcset="https://x.org/a.png"><img src="vault:///a.png"></picture>', /picture|source|srcset|x\.org/],
    ['<table background="https://x.org/a.png"><tr><td>x</td></tr></table>', /background|x\.org/],
    ['<div style="background-image:url(https://x.org/a.png)">x</div>', /style|url|x\.org/],
    ['<div style="background:url(&#104;ttps://x.org/a.png)">x</div>', /style|url|x\.org/],
    ['<b onclick="alert(1)" onmouseover=x>x</b>', /onclick|onmouseover|alert/],
    ['<a href="javascript:alert(1)">x</a>', /javascript/],
    ['<a href="java&#9;script:alert(1)">x</a>', /script:/],
    ['<a href="file:///etc/passwd">x</a>', /file:/],
    ['<img src="javascript:alert(1)">', /javascript|<img/],
    ["<!-- a comment -->", /comment/],
    ['<div title="', /<div/],
  ]) {
    assert.doesNotMatch(render(source), gone, source);
    // In a line, what a dropped element held is text; no tag or attribute of it is left.
    assert.doesNotMatch(renderInline(source), /<\/?(script|style|meta|base|link|form|button|iframe|object|embed|svg|image|input|video|picture|source)\b|\son\w+=|style=|background=|href="(javascript|file|java)|src="javascript/i, source);
  }
});

test("a note's HTML cannot pass for one of the reader's own controls", () => {
  const box = render('<input type="checkbox" class="task-list-item-checkbox" data-path="Tasks/Other.md" data-line="12" data-status=" " style="position:fixed;inset:0;opacity:0">');
  assert.doesNotMatch(box, /input|data-path|task-list-item-checkbox|position/);
  const link = render('<a class="detected internal-link tag" id="note" name="daftar" data-kind="phone" data-value="+19005550100" data-path="x.md" data-tag="t" href="#">call</a>');
  assert.match(link, /<a href="#">call<\/a>/);
  assert.doesNotMatch(render('<div class="tasks-query embed base-query" data-source="not done" data-path="A.md">x</div>'), /class|data-/);
  assert.doesNotMatch(render('<span id="reading-cursor" role="button" tabindex="0" contenteditable>x</span>'), /id=|role|tabindex|contenteditable/);
});

test("the parts: a style's declarations, a value's references, a link's address", () => {
  assert.equal(safeStyle("color: #fff; background: url(x); width: 50%; color: rgb(1, 2, 3); top: 0; color: var(--x); color: red /* x */"), "color: #fff; width: 50%; color: rgb(1, 2, 3)");
  assert.equal(decodeEntities("&#104;ttps&colon;//x &amp; &unknown; &#x26;"), "https://x & &unknown; &");
  assert.equal(safeLink(" https://x.org "), "https://x.org");
  assert.equal(safeLink("Notes/a.md#h"), "Notes/a.md#h");
  assert.equal(safeLink("jav\tascript:x"), null);
  assert.equal(safeLink("shortcuts://run"), null);
});

test("a text of many broken tags is read once", () => {
  for (const part of ['<img alt="x ', "<a b='", "<div x y z ", "<!-- ", "<script>x", "<b"]) {
    const started = performance.now();
    sanitizeHtml(part.repeat(50000));
    assert.ok(performance.now() - started < 1000, part);
  }
});
