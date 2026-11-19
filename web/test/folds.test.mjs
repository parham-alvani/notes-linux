// What a folded heading says about the section under it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { foldHint } from "../src/controls.js";

test("a folded section says how its text begins and how much is under it", () => {
  assert.equal(foldHint("Coffee and tea are fine.", 240, 3), "Coffee and tea are fine. · 240 words · 3 sections");
  assert.equal(foldHint("One line only.", 3, 0), "One line only.");
  assert.equal(foldHint("One line only.", 3, 1), "One line only. · 1 section");
});

test("a long beginning is cut at a word, with no mark left before the dots", () => {
  const long = "Because it is a mail drop it cannot carry the empadronamiento, which needs the address we sleep at.";
  const hint = foldHint(long, 18, 0);
  assert.equal(hint, "Because it is a mail drop it cannot carry the empadronamiento… · 18 words");
  assert.ok(hint.split(" · ")[0].length <= 65);
  // One long word is cut where the room ends.
  assert.equal(foldHint("x".repeat(80), 1, 0), "x".repeat(64) + "… · 1 word");
});

test("a section of headings only says how many, and an empty one says nothing but the dots", () => {
  assert.equal(foldHint("", 0, 4), "4 sections");
  assert.equal(foldHint("", 0, 0), "…");
  assert.equal(foldHint("   ", 0, 0), "…");
});

test("Persian text is cut at a word too, and its comma is not left hanging", () => {
  const text = "امروز صبح زود بیدار شدم، قهوه خوردم و بعد از آن برای پیاده‌روی به کنار ساحل رفتم و برگشتم.";
  const hint = foldHint(text, 20, 0);
  assert.ok(hint.endsWith("… · 20 words"));
  assert.ok(!/[،\s]…/.test(hint));
});
