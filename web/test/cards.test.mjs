// The card of a link's preview and of a footnote
// (`docs/specs/look.md` in notes-linux, "Link preview" and "Footnote popover").
import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "../src/render.js";
import { previewHead, footnoteLabel } from "../src/controls.js";

test("the card's head is the title, the heading the link names and the folder", () => {
  const head = previewHead({ title: "Plan", anchor: "Next", folder: "Work/2026" });
  assert.match(head, /<span class="preview-title" dir="auto">Plan<\/span>/);
  assert.match(head, /<span class="preview-anchor" dir="auto">› Next<\/span>/);
  assert.match(head, /<span class="preview-path">Work › 2026<\/span>$/);
  // A note at the vault's top has no folder, and a link to the note has no heading.
  assert.equal(previewHead({ title: "Plan" }), `<span class="preview-title" dir="auto">Plan</span>`);
  // A block id is shown without its mark.
  assert.match(previewHead({ title: "Plan", anchor: "^b1" }), /› b1</);
  assert.match(previewHead({ title: "a <b>" }), /a &lt;b&gt;/);
});

test("a link to no note has its name alone, marked as missing", () => {
  const head = previewHead({ title: "Nowhere", anchor: "Next", folder: "Work", missing: true });
  assert.equal(head, `<span class="preview-title is-missing" dir="auto">Nowhere</span>`);
});

test("a footnote shows where its reference is", () => {
  const html = render("a[^1]\n\n[^1]: The note.\n");
  // The number is a link to the footnote's item, which holds its text.
  const reference = /<sup class="footnote-ref"><a href="#(fn\d+)" id="fnref\d+">(\[\d+\])<\/a><\/sup>/.exec(html);
  assert.ok(reference);
  assert.equal(footnoteLabel(reference[2]), "Footnote 1");
  const item = new RegExp(`<li id="${reference[1]}" class="footnote-item"[^>]*><p[^>]*>The note\\.`).exec(html);
  assert.ok(item);
  // A reference with no footnote is no link: nothing opens.
  assert.doesNotMatch(render("a[^2]\n"), /footnote-ref/);
});
