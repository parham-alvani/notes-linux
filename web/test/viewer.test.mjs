// The math of the picture viewer: fit, the zoom limits, zoom around the
// pointer, and the pan that keeps the picture in the window.

import { test } from "node:test";
import assert from "node:assert/strict";
import { fitScale, clampScale, zoomAbout, clampPan, toggleScale, percent, svgSize, MAX_SCALE } from "../src/viewer.js";

test("a picture fits the box, and a small one stays at 100 %", () => {
  assert.equal(fitScale(2000, 1000, 1000, 1000), 0.5);
  assert.equal(fitScale(1000, 2000, 1000, 500), 0.25);
  assert.equal(fitScale(200, 100, 1000, 800), 1);
  // The padding is taken from each side.
  assert.equal(fitScale(1000, 100, 1048, 800, 24), 1);
  assert.equal(fitScale(2000, 100, 1048, 800, 24), 0.5);
  // A size not known yet is drawn at 100 %.
  assert.equal(fitScale(0, 0, 1000, 800), 1);
  assert.equal(fitScale(100, 100, 0, 0), 1);
});

test("the zoom stays between a quarter of fit and 1600 %", () => {
  assert.equal(clampScale(0.01, 0.5), 0.125);
  assert.equal(clampScale(100, 0.5), MAX_SCALE);
  assert.equal(clampScale(2, 0.5), 2);
  // A small picture's fit is 100 %: it can still go down to 25 %.
  assert.equal(clampScale(0.1, 1), 0.25);
});

test("a zoom keeps the point under the pointer where it is", () => {
  const view = { scale: 1, x: 10, y: 20 };
  const next = zoomAbout(view, 2, 110, 220);
  // The picture's point under (110, 220) was (100, 200); it is there still.
  assert.equal((110 - next.x) / next.scale, 100);
  assert.equal((220 - next.y) / next.scale, 200);
  assert.deepEqual(zoomAbout(view, 1, 50, 50), view);
});

test("a picture smaller than the box is in its middle, a larger one covers it", () => {
  assert.deepEqual(clampPan({ scale: 1, x: 99, y: -99 }, 200, 100, 1000, 500), { scale: 1, x: 400, y: 200 });
  // 2000 × 1000 at 100 % in a 1000 × 500 box: from 0 to -1000 and -500.
  assert.deepEqual(clampPan({ scale: 1, x: 50, y: -40 }, 2000, 1000, 1000, 500), { scale: 1, x: 0, y: -40 });
  assert.deepEqual(clampPan({ scale: 1, x: -5000, y: -5000 }, 2000, 1000, 1000, 500), { scale: 1, x: -1000, y: -500 });
  // Larger on one side only.
  assert.deepEqual(clampPan({ scale: 2, x: -300, y: 7 }, 1000, 100, 1000, 500), { scale: 2, x: -300, y: 150 });
});

test("a double click goes between fit and 100 %", () => {
  assert.equal(toggleScale(0.5, 0.5), 1);
  assert.equal(toggleScale(3, 0.5), 0.5);
  assert.equal(toggleScale(1, 0.5), 0.5);
  // A small picture is at 100 % when fitted: it goes to 200 %.
  assert.equal(toggleScale(1, 1), 2);
  assert.equal(toggleScale(2, 1), 1);
});

test("the zoom is said in whole percents", () => {
  assert.equal(percent(1), "100 %");
  assert.equal(percent(0.333), "33 %");
  assert.equal(percent(16), "1600 %");
});

test("an inline SVG's size is its viewBox, its own size, or the box it was drawn in", () => {
  assert.deepEqual(svgSize("0 0 640 480", "100%", null, { width: 10, height: 10 }), { width: 640, height: 480 });
  assert.deepEqual(svgSize("0,0,20,10"), { width: 20, height: 10 });
  assert.deepEqual(svgSize(null, "120", "80px"), { width: 120, height: 80 });
  assert.deepEqual(svgSize(null, "100%", "100%", { width: 300, height: 200 }), { width: 300, height: 200 });
  assert.deepEqual(svgSize("0 0 0 0", null, null, null), { width: 300, height: 150 });
});
