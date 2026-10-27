// A view of a base as HTML: a table, a grid of cards, or a list. The app
// runs the base (`BaseEngine`); this only draws what it sends. Kept apart
// from the page runtime so node can test it.

import { escapeHtml as escape, vaultUrl } from "./render.js";

/** A cell's value; a checkbox is named after its column, [name]. */
export function renderCell(cell, name = "") {
  if (cell.check !== undefined) return `<input type="checkbox" disabled${cell.check ? " checked" : ""}${name ? ` aria-label="${escape(name)}"` : ""}>`;
  return cell.parts
    .map((part) =>
      part.path
        ? `<a class="internal-link" href="#" data-path="${escape(part.path)}">${escape(part.text)}</a>`
        : escape(part.text),
    )
    .join("");
}

const empty = (cell) => cell.check === undefined && cell.parts.every((part) => part.text.trim() === "");

/** The note's own name link, which a card or a list item has as its title. */
const isSelf = (cell, row) => cell.parts.length === 1 && cell.parts[0].path === row.path;

function title(path) {
  return (path.split("/").pop() ?? path).replace(/\.md$/i, "");
}

const titleLink = (row) => `<a class="internal-link" href="#" data-path="${escape(row.path)}">${escape(title(row.path))}</a>`;

function groupHead(group, tag) {
  return group.name == null ? "" : `<${tag} class="base-group-head" dir="auto">${escape(group.name)} <span class="base-group-count">${group.rows.length}</span></${tag}>`;
}

function renderTable(view) {
  const head = view.columns.map((name) => `<th scope="col" dir="auto">${escape(name)}</th>`).join("");
  const body = view.groups
    .map((group) => {
      const title = group.name == null ? "" : `<tr class="base-group"><th colspan="${view.columns.length}" dir="auto">${escape(group.name)} <span class="base-group-count">${group.rows.length}</span></th></tr>`;
      const rows = group.rows
        .map((row) => `<tr>${row.cells.map((cell, at) => `<td dir="auto"${cell.numeric ? ' class="numeric"' : ""}>${renderCell(cell, view.columns[at])}</td>`).join("")}</tr>`)
        .join("");
      return title + rows;
    })
    .join("");
  return `<div class="base-scroll"><table class="base-table"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
}

/** A card's cover: a picture in the vault or on the web, or a colour. */
function cover(image, layout) {
  const ratio = Number(layout.imageAspectRatio) > 0 ? Number(layout.imageAspectRatio) : 1;
  const style = `aspect-ratio: 1 / ${ratio}`;
  if (!image) return "";
  if (image.color && /^#[0-9a-f]{3,8}$/i.test(image.color)) {
    return `<div class="base-card-cover" style="${style}; background: ${image.color}"></div>`;
  }
  const src = image.file ? vaultUrl(image.file) : image.url && /^https?:\/\//i.test(image.url) ? image.url : null;
  if (!src) return "";
  const fit = layout.imageFit === "contain" ? "contain" : "cover";
  return `<div class="base-card-cover" style="${style}"><img src="${escape(src)}" alt="" loading="lazy" style="object-fit: ${fit}"></div>`;
}

function renderCards(view) {
  const layout = view.layout ?? {};
  const size = Number(layout.cardSize) > 0 ? Number(layout.cardSize) : 200;
  const card = (row) => {
    const lines = row.cells
      .map((cell, at) => (isSelf(cell, row) || empty(cell) ? "" : `<div class="base-card-line"><span class="base-card-label" dir="auto">${escape(view.columns[at] ?? "")}</span><span class="base-card-value${cell.numeric ? " numeric" : ""}" dir="auto">${renderCell(cell, view.columns[at])}</span></div>`))
      .join("");
    // A card is a group named by its note, so VoiceOver says whose values these are.
    return `<div class="base-card" role="group" aria-label="${escape(title(row.path))}">${cover(row.image, layout)}<div class="base-card-body"><div class="base-card-title" dir="auto">${titleLink(row)}</div>${lines}</div></div>`;
  };
  return view.groups
    .map((group) => `${groupHead(group, "div")}<div class="base-cards" style="grid-template-columns: repeat(auto-fill, minmax(${size}px, 1fr))">${group.rows.map(card).join("")}</div>`)
    .join("");
}

function renderList(view) {
  const layout = view.layout ?? {};
  const mark = String(layout.separator ?? ",");
  const between = mark === "," || mark === ";" ? `${mark} ` : ` ${mark} `;
  const tag = layout.markers === "number" ? "ol" : "ul";
  const plain = layout.markers === "none" ? " is-plain" : "";
  const item = (row) => {
    const values = row.cells
      .map((cell, at) => ({ cell, at }))
      .filter(({ cell }) => !isSelf(cell, row) && !empty(cell));
    if (layout.indentProperties) {
      const inner = values.map(({ cell, at }) => `<li dir="auto"><span class="base-list-label">${escape(view.columns[at] ?? "")}</span> ${renderCell(cell)}</li>`).join("");
      return `<li dir="auto">${titleLink(row)}${inner ? `<ul>${inner}</ul>` : ""}</li>`;
    }
    const inline = values.map(({ cell, at }) => `<span class="base-list-value" title="${escape(view.columns[at] ?? "")}">${renderCell(cell)}</span>`).join(`<span class="base-list-separator">${escape(between)}</span>`);
    return `<li dir="auto">${titleLink(row)}${inline ? `<span class="base-list-values"> ${inline}</span>` : ""}</li>`;
  };
  return view.groups.map((group) => `${groupHead(group, "div")}<${tag} class="base-list${plain}">${group.rows.map(item).join("")}</${tag}>`).join("");
}

/** One view: `kind` is `table`, `cards` or `list`; anything else is a table. */
export function renderView(view) {
  const kind = view.kind ?? "table";
  const body = kind === "cards" ? renderCards(view) : kind === "list" ? renderList(view) : renderTable(view);
  const none = view.count === 0 ? `<div class="query-empty">No files match.</div>` : "";
  const note = kind === "table" && view.type && view.type !== "table" ? ` · shown as a table, not ${escape(view.type)}` : "";
  return `${body}${none}<div class="query-count">${view.count} ${view.count === 1 ? "result" : "results"}${note}</div>`;
}
