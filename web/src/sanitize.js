// The raw HTML of a note, made safe by a list of what is let through, not
// of what is kept out. A note holds text pasted from the web and from mail,
// so its HTML is read tag by tag and written again from the parts the list
// allows: nothing of the source reaches the page as it was written. Pure,
// with no DOM, so node and JavaScriptCore (Quick Look) run it as the page does.

function escapeHtml(text) {
  return text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

/** The elements a note may write. `img` is here too; its address is checked on its own. */
const ALLOWED = new Set(
  (
    "a abbr address article aside b bdi bdo big blockquote br caption center cite code col colgroup dd del details dfn div dl dt em " +
    "figcaption figure footer h1 h2 h3 h4 h5 h6 header hr i img ins kbd li main mark nav ol p pre q rp rt ruby s samp section small " +
    "span strike strong sub summary sup table tbody td tfoot th thead time tr tt u ul var wbr"
  ).split(" "),
);

/** Elements that go with all they hold: what is in them is not text to read. */
const DROPPED_WHOLE = new Set(
  (
    "script style title textarea select noscript template head iframe object embed svg math xmp noembed noframes plaintext frameset " +
    "applet audio video canvas"
  ).split(" "),
);

/**
 * Elements of HTML that are left out, tag only: a form's parts, the page's
 * own (`meta`, `base`, `link`), and the rest of HTML this list does not let
 * through. A name that is none of HTML's (`<first name>`) is text, and shown.
 */
const DROPPED_TAG = new Set(
  (
    "html body meta link base input button form label fieldset legend option optgroup datalist output progress meter source track " +
    "param area map picture font hgroup dialog menu menuitem slot portal frame marquee blink basefont nobr acronym dir data image " +
    "isindex keygen listing search bgsound command content element shadow spacer multicol nextid rb rtc"
  ).split(" "),
);

const NUMBER = /^\d{1,5}$/;
const LENGTH = /^\d{1,5}(?:\.\d{1,3})?%?$/;

/** What an attribute's value must be, by its name, on any element. */
const GLOBAL = {
  title: () => true,
  lang: (value) => /^[A-Za-z0-9-]{1,35}$/.test(value),
  dir: (value) => /^(ltr|rtl|auto)$/i.test(value),
  align: (value) => /^(left|right|center|justify)$/i.test(value),
};

/** And on one element only. */
const OWN = {
  a: { href: null },
  img: { src: null, alt: () => true, width: (value) => LENGTH.test(value), height: (value) => LENGTH.test(value) },
  details: { open: () => true },
  ol: { start: (value) => /^-?\d{1,9}$/.test(value), reversed: () => true, type: (value) => /^[1aAiI]$/.test(value) },
  li: { value: (value) => /^-?\d{1,9}$/.test(value) },
  td: { colspan: (v) => NUMBER.test(v), rowspan: (v) => NUMBER.test(v), valign: (v) => /^(top|middle|bottom|baseline)$/i.test(v), width: (v) => LENGTH.test(v) },
  th: {
    colspan: (v) => NUMBER.test(v), rowspan: (v) => NUMBER.test(v), valign: (v) => /^(top|middle|bottom|baseline)$/i.test(v), width: (v) => LENGTH.test(v),
    scope: (v) => /^(row|col|rowgroup|colgroup)$/i.test(v),
  },
  col: { span: (v) => NUMBER.test(v), width: (v) => LENGTH.test(v) },
  colgroup: { span: (v) => NUMBER.test(v) },
  table: { width: (v) => LENGTH.test(v) },
  time: { datetime: (value) => /^[\dTZ:.+ W-]{1,40}$/.test(value) },
  bdo: {},
};

/** The style properties a note may set: how its text looks, never where it is or what it loads. */
const STYLE = new Set(
  (
    "color background-color text-align font-weight font-style text-decoration text-decoration-line vertical-align white-space " +
    "width max-width min-width border border-top border-right border-bottom border-left border-color border-style border-width " +
    "border-collapse border-radius padding padding-top padding-right padding-bottom padding-left"
  ).split(" "),
);

/** The declarations of a `style` attribute that are on the list, with plain values; "" when none is. */
export function safeStyle(value) {
  const kept = [];
  for (const part of value.split(";")) {
    const colon = part.indexOf(":");
    if (colon < 0) continue;
    const name = part.slice(0, colon).trim().toLowerCase();
    const text = part.slice(colon + 1).trim();
    // No function but a colour's, so no `url()`, `var()`, `expression()` or `image-set()`; no escape, no comment.
    if (!STYLE.has(name) || text.length > 100 || !/^[\w\s.,%#()+-]+$/.test(text)) continue;
    if (/\(/.test(text) && !/^(rgba?|hsla?)\([\d\s.,%+-]+\)$/i.test(text)) continue;
    kept.push(`${name}: ${text}`);
  }
  return kept.join("; ");
}

const NAMED = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ", colon: ":", sol: "/", bsol: "\\", Tab: "\t", NewLine: "\n" };

/**
 * An attribute's value with its character references read. Only a few named
 * ones: the value is written back escaped, so a reference left unread is
 * shown as its letters and never read by the browser as something else.
 */
export function decodeEntities(text) {
  if (!text.includes("&")) return text;
  return text.replace(/&(?:#[xX]([0-9a-fA-F]{1,6})|#(\d{1,7})|([A-Za-z][A-Za-z0-9]{1,31}));?/g, (whole, hex, decimal, name) => {
    if (name) return Object.hasOwn(NAMED, name) ? NAMED[name] : whole;
    const code = hex ? parseInt(hex, 16) : Number(decimal);
    if (!(code > 0 && code <= 0x10ffff) || (code >= 0xd800 && code <= 0xdfff)) return "�";
    return String.fromCodePoint(code);
  });
}

/**
 * An address as the browser reads it before it looks for a scheme: the
 * spaces and control characters at its ends go, and every tab and line
 * break in it, so `ht\ttps:` is `https:` here as it is there.
 */
export function cleanAddress(value) {
  return value.replace(/[\t\n\r]/g, "").replace(/^[\u0000- ]+|[\u0000- ]+$/g, "");
}

/** The scheme of a cleaned address, in small letters, or null for a relative one. */
export function schemeOf(address) {
  return /^([A-Za-z][A-Za-z0-9+.-]*):/.exec(address)?.[1].toLowerCase() ?? null;
}

const LINK_SCHEMES = new Set(["http", "https", "mailto", "tel", "sms", "geo"]);

/** A link's address when it may be followed (the app's `Vault.outsideSchemes`, or a note of the vault), else null. */
export function safeLink(value) {
  const address = cleanAddress(value);
  const scheme = schemeOf(address);
  return scheme === null || LINK_SCHEMES.has(scheme) ? address : null;
}

/**
 * Reads a tag that starts at [at] (`<`): its name, its attributes in order
 * with their values as written, and where it ends. Null when it is not a
 * whole tag. [state] remembers that the rest of the text has no `>` or no
 * closing quote, so a text of a thousand broken tags is still read once.
 */
function readTag(html, at, state) {
  let pos = at + 1;
  const closing = html.charCodeAt(pos) === 0x2f;
  if (closing) pos++;
  const nameStart = pos;
  while (pos < html.length && /[A-Za-z0-9-]/.test(html[pos])) pos++;
  if (pos === nameStart || !/[A-Za-z]/.test(html[nameStart])) return null;
  const name = html.slice(nameStart, pos).toLowerCase();
  if (state.noEnd) return null;
  const attributes = [];
  for (;;) {
    const before = pos;
    while (pos < html.length && /[\s\/]/.test(html[pos])) pos++;
    if (pos >= html.length) {
      state.noEnd = true;
      return null;
    }
    if (html[pos] === ">") return { name, closing, attributes, end: pos + 1 };
    // An attribute comes after a space, and a tag's name ends at one.
    if (pos === before) return null;
    const keyStart = pos;
    while (pos < html.length && !/[\s"'<>\/=`]/.test(html[pos])) pos++;
    if (pos === keyStart) return null;
    const key = html.slice(keyStart, pos).toLowerCase();
    let value = "";
    let look = pos;
    while (look < html.length && /\s/.test(html[look])) look++;
    if (html[look] === "=") {
      look++;
      while (look < html.length && /\s/.test(html[look])) look++;
      const quote = html[look];
      if (quote === '"' || quote === "'") {
        const flag = quote === '"' ? "noDouble" : "noSingle";
        const end = state[flag] ? -1 : html.indexOf(quote, look + 1);
        if (end < 0) {
          state[flag] = true;
          return null;
        }
        value = html.slice(look + 1, end);
        pos = end + 1;
      } else {
        const valueStart = look;
        while (look < html.length && !/[\s"'=<>`]/.test(html[look])) look++;
        if (look === valueStart) return null;
        value = html.slice(valueStart, look);
        pos = look;
      }
    }
    attributes.push([key, value]);
  }
}

/** A tag of the list written again, with the attributes the list has for it. */
function writeTag(tag) {
  if (tag.closing) return `</${tag.name}>`;
  const own = OWN[tag.name] ?? {};
  const seen = new Set();
  let out = `<${tag.name}`;
  for (const [key, raw] of tag.attributes) {
    if (seen.has(key)) continue;
    seen.add(key);
    const value = decodeEntities(raw);
    if (key === "style") {
      const style = safeStyle(value);
      if (style) out += ` style="${escapeHtml(style)}"`;
      continue;
    }
    if (tag.name === "a" && key === "href") {
      const link = safeLink(value);
      if (link !== null) out += ` href="${escapeHtml(link)}"`;
      continue;
    }
    const check = Object.hasOwn(own, key) ? own[key] : Object.hasOwn(GLOBAL, key) ? GLOBAL[key] : null;
    if (check && check(value)) out += ` ${key}="${escapeHtml(value)}"`;
  }
  return out + ">";
}

/**
 * [html] with only what the list lets through. A tag of the list is
 * written again with its listed attributes: no `class`, `id`, `data-*`,
 * `on*` or any other the reader gives a meaning to. `script`, `style`,
 * `iframe` and their like go with what they hold; the rest of HTML's own
 * elements go, tag only; a comment goes; and anything else that starts
 * with `<` is text. [image] writes an `<img>` from its attributes (read
 * and checked here), so the caller decides what a picture from the web is.
 */
export function sanitizeHtml(html, { image } = {}) {
  let out = "";
  let at = 0;
  const state = { noEnd: false, noDouble: false, noSingle: false, noComment: false };
  const lower = { text: null };
  while (at < html.length) {
    const next = html.indexOf("<", at);
    if (next < 0) {
      out += html.slice(at);
      break;
    }
    out += html.slice(at, next);
    if (html.startsWith("<!--", next)) {
      const end = state.noComment ? -1 : html.indexOf("-->", next + 4);
      if (end < 0) {
        state.noComment = true;
        break;
      }
      at = end + 3;
      continue;
    }
    if (html[next + 1] === "!" || html[next + 1] === "?") {
      // A doctype, a CDATA section, a processing instruction: to its `>`.
      const end = state.noEnd ? -1 : html.indexOf(">", next);
      if (end < 0) {
        state.noEnd = true;
        break;
      }
      at = end + 1;
      continue;
    }
    const tag = readTag(html, next, state);
    if (!tag) {
      out += "&lt;";
      at = next + 1;
      continue;
    }
    at = tag.end;
    if (DROPPED_WHOLE.has(tag.name)) {
      if (tag.closing) continue;
      lower.text ??= html.toLowerCase();
      const close = lower.text.indexOf(`</${tag.name}`, at);
      if (close < 0) break;
      const end = html.indexOf(">", close);
      if (end < 0) break;
      at = end + 1;
      continue;
    }
    if (tag.name === "img") {
      if (tag.closing) continue;
      const attribute = {};
      for (const [key, raw] of tag.attributes) if (!Object.hasOwn(attribute, key)) attribute[key] = decodeEntities(raw);
      out += image ? image(attribute) : "";
      continue;
    }
    if (ALLOWED.has(tag.name)) out += writeTag(tag);
    else if (!DROPPED_TAG.has(tag.name)) out += escapeHtml(html.slice(next, tag.end));
  }
  return out;
}
