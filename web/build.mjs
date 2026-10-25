// Bundles the reading view into crates/daftar/reader/, which the app embeds
// in its binary. The output is generated: edit web/src, then run
// `just web`.

import { build } from "esbuild";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";

const out = "../crates/daftar/reader";
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

await build({
  entryPoints: ["src/reader.js"],
  bundle: true,
  minify: true,
  format: "iife",
  target: "safari17",
  outfile: `${out}/reader.js`,
  legalComments: "eof",
});

// Mermaid is most of the weight, and most notes have no diagram: the page
// loads it the first time a note needs it.
await build({
  stdin: { contents: `import mermaid from "mermaid"; window.mermaid = mermaid;`, resolveDir: "." },
  bundle: true,
  minify: true,
  format: "iife",
  target: "safari17",
  outfile: `${out}/mermaid.js`,
  legalComments: "eof",
});

// Task checkboxes as the phone draws them (`Blocks.kt`): a Lucide icon per
// status, in the colour of its role. The icons become CSS masks, so the
// colour is a CSS value and follows the theme.
const statuses = [
  [" ", "square", "--list-marker"],
  ["x", "square-check-big", "--done-color"],
  ["X", "square-check-big", "--done-color"],
  ["-", "square-x", "--done-color"],
  ["/", "square-dot", "--accent"],
  [">", "square-arrow-right", "--blue"],
  ["<", "calendar-clock", "--blue"],
  ["!", "circle-alert", "--red"],
  ["?", "circle-help", "--yellow"],
  ["*", "star", "--vivid-yellow"],
  ['"', "quote", "--warm-grey"],
  ["i", "info", "--blue"],
  ["b", "bookmark", "--orange"],
  ["l", "map-pin", "--red"],
  ["I", "lightbulb", "--vivid-yellow"],
  ["S", "piggy-bank", "--spring-green"],
  ["f", "flame", "--red"],
  ["k", "key", "--vivid-yellow"],
  ["w", "trophy", "--vivid-yellow"],
  ["u", "thumbs-up", "--spring-green"],
  ["p", "thumbs-up", "--spring-green"],
  ["d", "thumbs-down", "--red"],
  ["c", "thumbs-down", "--red"],
];
const icon = (name) => {
  const svg = readFileSync(`node_modules/lucide-static/icons/${name}.svg`, "utf8").replace(/<!--.*?-->/s, "").replace(/\s+/g, " ").trim();
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
};
const selector = (status) => `input.task-list-item-checkbox[data-status='${status === "'" ? "\\'" : status}']`;
const css = [
  `input.task-list-item-checkbox { --task-icon: ${icon("square-dashed")}; --task-color: var(--list-marker); }`,
  ...statuses.map(([status, name, color]) => `${selector(status)} { --task-icon: ${icon(name)}; --task-color: var(${color}); }`),
];
writeFileSync(`${out}/tasks.css`, css.join("\n") + "\n");

// Each callout kind's Lucide icon, as Obsidian draws them, with its aliases;
// and the icons of the page's own buttons.
const calloutIcons = [
  [["note"], "pencil"],
  [["info"], "info"],
  [["abstract", "summary", "tldr"], "clipboard-list"],
  [["todo"], "circle-check"],
  [["tip", "hint", "important"], "flame"],
  [["success", "check", "done"], "check"],
  [["question", "help", "faq"], "circle-help"],
  [["warning", "caution", "attention"], "triangle-alert"],
  [["failure", "fail", "missing"], "x"],
  [["danger", "error"], "zap"],
  [["bug"], "bug"],
  [["example"], "list"],
  [["quote", "cite"], "quote"],
  [["figure"], "image"],
  [["read"], "book-open"],
];
const calloutCss = [
  `.callout { --callout-icon: ${icon("pencil")}; }`,
  ...calloutIcons.map(([kinds, name]) => `${kinds.map((kind) => `.callout[data-callout="${kind}"]`).join(", ")} { --callout-icon: ${icon(name)}; }`),
  `:root { --icon-copy: ${icon("copy")}; --icon-copied: ${icon("check-check")}; --icon-search: ${icon("search")}; --icon-up: ${icon("chevron-up")}; --icon-down: ${icon("chevron-down")}; --icon-close: ${icon("x")}; }`,
];
writeFileSync(`${out}/callouts.css`, calloutCss.join("\n") + "\n");

cpSync("src/index.html", `${out}/index.html`);
cpSync("src/reader.css", `${out}/reader.css`);
cpSync("node_modules/katex/dist/katex.min.css", `${out}/katex.min.css`);
cpSync("node_modules/katex/dist/fonts", `${out}/fonts`, { recursive: true });
