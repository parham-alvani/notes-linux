// An Obsidian canvas as HTML: the cards at their places in a world the page
// pans and zooms, and the edges as curves in an SVG under the cards. The app
// parses the file (`Canvas`), resolves its files and links, and sends it
// here. Kept apart from the page runtime so node can test it.

import { escapeHtml as escape, plainText, render, vaultUrl } from "./render.js";

/** Obsidian's six presets, in the palette's colours; `Canvas.color` names them. */
const PRESETS = { red: "var(--red)", orange: "var(--orange)", yellow: "var(--yellow)", green: "var(--spring-green)", cyan: "var(--aqua)", purple: "var(--purple)" };

/** A colour for CSS: a preset's palette colour, a `#hex`, or null. */
export function canvasColor(value) {
  if (!value) return null;
  if (PRESETS[value]) return PRESETS[value];
  return /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(value) ? value : null;
}

const IMAGE = /\.(png|jpe?g|gif|webp|svg|bmp|avif|heic|tiff?)$/i;

/** The box around every card, for fitting the board to the window. */
export function bounds(nodes) {
  if (nodes.length === 0) return { x: 0, y: 0, width: 0, height: 0 };
  const left = Math.min(...nodes.map((n) => n.x));
  const top = Math.min(...nodes.map((n) => n.y));
  const right = Math.max(...nodes.map((n) => n.x + n.width));
  const bottom = Math.max(...nodes.map((n) => n.y + n.height));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** The middle of a card's side, and the way out of it. */
function anchor(node, side) {
  const { x, y, width: w, height: h } = node;
  switch (side) {
    case "top": return { x: x + w / 2, y, dx: 0, dy: -1 };
    case "bottom": return { x: x + w / 2, y: y + h, dx: 0, dy: 1 };
    case "left": return { x, y: y + h / 2, dx: -1, dy: 0 };
    default: return { x: x + w, y: y + h / 2, dx: 1, dy: 0 };
  }
}

/** The side of [node] that faces [other], when the edge names none. */
function facing(node, other) {
  const dx = other.x + other.width / 2 - (node.x + node.width / 2);
  const dy = other.y + other.height / 2 - (node.y + node.height / 2);
  if (Math.abs(dx) * node.height >= Math.abs(dy) * node.width) return dx >= 0 ? "right" : "left";
  return dy >= 0 ? "bottom" : "top";
}

/** An arrowhead at [tip], pointing along (dx, dy), as SVG points. */
function arrowhead(tip, dx, dy) {
  const length = Math.hypot(dx, dy) || 1;
  const ux = dx / length, uy = dy / length;
  const size = 12, half = 6;
  const bx = tip.x - ux * size, by = tip.y - uy * size;
  const points = [[tip.x, tip.y], [bx - uy * half, by + ux * half], [bx + uy * half, by - ux * half]];
  return points.map(([px, py]) => `${round(px)},${round(py)}`).join(" ");
}

const round = (n) => Math.round(n * 10) / 10;

/**
 * An edge as a cubic curve between the two sides: `d` for the path, the
 * arrowheads' points, and where its label goes (the curve's middle).
 */
export function edgeGeometry(from, to, edge) {
  const start = anchor(from, edge.fromSide ?? facing(from, to));
  const end = anchor(to, edge.toSide ?? facing(to, from));
  const distance = Math.hypot(end.x - start.x, end.y - start.y);
  const pull = Math.min(Math.max(distance / 2, 40), 250);
  const c1 = { x: start.x + start.dx * pull, y: start.y + start.dy * pull };
  const c2 = { x: end.x + end.dx * pull, y: end.y + end.dy * pull };
  const d = `M ${round(start.x)} ${round(start.y)} C ${round(c1.x)} ${round(c1.y)}, ${round(c2.x)} ${round(c2.y)}, ${round(end.x)} ${round(end.y)}`;
  // The curve at t = 0.5.
  const label = { x: round((start.x + 3 * c1.x + 3 * c2.x + end.x) / 8), y: round((start.y + 3 * c1.y + 3 * c2.y + end.y) / 8) };
  const arrows = [];
  if (edge.toEnd !== "none") arrows.push(arrowhead(end, end.x - c2.x, end.y - c2.y));
  if (edge.fromEnd === "arrow") arrows.push(arrowhead(start, start.x - c1.x, start.y - c1.y));
  return { d, label, arrows };
}

const place = (node) => `left: ${node.x}px; top: ${node.y}px; width: ${node.width}px; height: ${node.height}px`;
const tint = (value) => (canvasColor(value) ? `; --node-color: ${canvasColor(value)}` : "");

function fileName(path) {
  return (path.split("/").pop() ?? path).replace(/\.md$/i, "");
}

/** What a file card holds: the note (filled by the page), the picture, or the file's name. */
function fileCard(node) {
  const anchor = (node.subpath ?? "").replace(/^#/, "");
  if (!node.path) {
    return `<div class="canvas-file-name is-missing" dir="auto">${escape(node.file ?? "")}</div><div class="canvas-node-body"><p class="canvas-missing">Not in the vault.</p></div>`;
  }
  const name = `<a class="internal-link canvas-file-name" href="#" data-path="${escape(node.path)}" data-anchor="${escape(anchor)}" dir="auto">${escape(fileName(node.path))}${anchor ? ` › ${escape(anchor)}` : ""}</a>`;
  if (IMAGE.test(node.path)) return `${name}<div class="canvas-node-body is-image"><img src="${vaultUrl(node.path)}" alt="${escape(fileName(node.path))}"></div>`;
  if (/\.md$/i.test(node.path)) {
    // The page fills it as it fills `![[note#part]]`.
    return `${name}<div class="canvas-node-body"><div class="embed canvas-embed" data-path="${escape(node.path)}" data-anchor="${escape(anchor)}"><div class="embed-body"></div></div></div>`;
  }
  if (/\.base$/i.test(node.path)) return `${name}<div class="canvas-node-body"><div class="base-query" data-path="${escape(node.path)}" data-view="${escape(anchor)}"></div></div>`;
  return `${name}<div class="canvas-node-body"></div>`;
}

function linkCard(node) {
  const url = node.url ?? "";
  if (!/^https?:\/\//i.test(url)) return `<div class="canvas-node-body"><p dir="auto">${escape(url)}</p></div>`;
  let host = url;
  try {
    host = new URL(url).host;
  } catch (error) {}
  return `<div class="canvas-node-body"><a class="canvas-link-card" href="${escape(url)}"><span class="canvas-link-host">${escape(host)}</span><span class="canvas-link-url">${escape(url)}</span></a></div>`;
}

/**
 * The cards in the order a person reads the board: in rows from the top,
 * each row from the left. A card whose top is within half a card of the
 * row's first card is in that row.
 */
export function readingOrder(nodes) {
  const cards = nodes.filter((n) => n.type !== "group").sort((a, b) => a.y - b.y || a.x - b.x);
  const rows = [];
  for (const card of cards) {
    const row = rows.at(-1);
    if (row && card.y < row[0].y + row[0].height / 2) row.push(card);
    else rows.push([card]);
  }
  return rows.flatMap((row) => row.sort((a, b) => a.x - b.x));
}

/** A card's name for a screen reader: its file, its link, or its text's first line. */
export function cardName(node) {
  if (node.type === "file") return node.path ? fileName(node.path) : `${node.file ?? "File"}, not in the vault`;
  if (node.type === "link") return node.url ?? "Link";
  const first = (node.text ?? "").split("\n").map((line) => plainText(line.replace(/^[#>\-*+\s]+(?:\[.\]\s*)?/, ""))).find(Boolean) ?? "";
  return first.length > 80 ? first.slice(0, 79) + "…" : first || "Empty card";
}

/**
 * The board in words, for `aria-describedby`: how many cards, the groups,
 * and each arrow as "From → To: label". The cards themselves follow in
 * reading order, each a stop for Tab.
 */
export function canvasSummary(canvas) {
  const nodes = canvas.nodes ?? [];
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const cards = nodes.filter((n) => n.type !== "group").length;
  const groups = nodes.filter((n) => n.type === "group" && n.label).map((n) => n.label);
  const arrows = (canvas.edges ?? [])
    .filter((edge) => byId.has(edge.fromNode) && byId.has(edge.toNode))
    .map((edge) => `${cardName(byId.get(edge.fromNode))} → ${cardName(byId.get(edge.toNode))}${edge.label ? `: ${edge.label}` : ""}`);
  const parts = [`${cards} ${cards === 1 ? "card" : "cards"}`];
  if (groups.length) parts.push(`Groups: ${groups.join(", ")}`);
  if (arrows.length) parts.push(`Arrows: ${arrows.join("; ")}`);
  return parts.join(". ") + ".";
}

function nodeHtml(node, links, path, order = 0) {
  const style = place(node) + tint(node.color);
  const tinted = canvasColor(node.color) ? " is-tinted" : "";
  if (node.type === "group") {
    const label = node.label ? `<div class="canvas-group-label" dir="auto">${escape(node.label)}</div>` : "";
    // A group's name is in the board's summary.
    return `<div class="canvas-group${tinted}" style="${style}" aria-hidden="true">${label}</div>`;
  }
  let inner = "";
  if (node.type === "text") inner = `<div class="canvas-node-body">${render(node.text ?? "", { path, links })}</div>`;
  else if (node.type === "file") inner = fileCard(node);
  else if (node.type === "link") inner = linkCard(node);
  else return "";
  const open = node.type === "file" && node.path ? ` data-open="${escape(node.path)}"` : "";
  // Each card is a stop for Tab, in reading order; its place in the file
  // keeps which card is drawn over which.
  return `<div class="canvas-node canvas-${node.type}${tinted}" data-id="${escape(node.id)}"${open} style="${style}; z-index: ${order}" tabindex="0" role="article" aria-label="${escape(cardName(node))}">${inner}</div>`;
}

/**
 * The whole board, in canvas coordinates: groups first (under everything),
 * then the edges, then the cards, then the edges' labels on top.
 */
export function renderCanvas(canvas, { path = "", links = {} } = {}) {
  const nodes = canvas.nodes ?? [];
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const box = bounds(nodes);
  const pad = 400;
  const view = `${box.x - pad} ${box.y - pad} ${box.width + 2 * pad} ${box.height + 2 * pad}`;
  const paths = [];
  const labels = [];
  for (const edge of canvas.edges ?? []) {
    const from = byId.get(edge.fromNode), to = byId.get(edge.toNode);
    if (!from || !to) continue;
    const shape = edgeGeometry(from, to, edge);
    const color = canvasColor(edge.color);
    const style = color ? ` style="--edge-color: ${color}"` : "";
    paths.push(`<g class="canvas-edge"${style} aria-hidden="true"><path d="${shape.d}"/>${shape.arrows.map((points) => `<polygon points="${points}"/>`).join("")}</g>`);
    if (edge.label) labels.push(`<div class="canvas-edge-label" dir="auto" aria-hidden="true" style="z-index: ${nodes.length + 1}; left: ${shape.label.x}px; top: ${shape.label.y}px${color ? `; --edge-color: ${color}` : ""}">${escape(edge.label)}</div>`);
  }
  const groups = nodes.filter((n) => n.type === "group").map((n) => nodeHtml(n, links, path)).join("");
  const stacking = new Map(nodes.map((node, at) => [node, at + 1]));
  const cards = readingOrder(nodes).map((n) => nodeHtml(n, links, path, stacking.get(n))).join("");
  const svg = `<svg class="canvas-edges" aria-hidden="true" style="left: ${box.x - pad}px; top: ${box.y - pad}px; width: ${box.width + 2 * pad}px; height: ${box.height + 2 * pad}px" viewBox="${view}">${paths.join("")}</svg>`;
  const empty = nodes.length === 0 ? `<div class="canvas-empty">The canvas is empty.</div>` : "";
  const summary = `<div id="canvas-summary" class="sr-only">${escape(canvasSummary(canvas))}</div>`;
  return `<div class="canvas-world">${groups}${svg}${cards}${labels.join("")}</div>${empty}${summary}`;
}

/** The zoom and offset that show the whole [box] in a [width] × [height] window. */
export function fit(box, width, height) {
  if (box.width <= 0 || box.height <= 0) return { scale: 1, x: width / 2, y: height / 2 };
  const margin = 40;
  const scale = Math.min(1, Math.max(0.05, Math.min((width - 2 * margin) / box.width, (height - 2 * margin) / box.height)));
  return { scale, x: (width - box.width * scale) / 2 - box.x * scale, y: (height - box.height * scale) / 2 - box.y * scale };
}
