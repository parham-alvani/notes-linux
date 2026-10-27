//! A web page as a note, as Obsidian's Web Clipper makes one: the page's
//! main text (its `<article>` or `<main>`, else the largest block of
//! paragraphs, without the menus, scripts, sidebars, headers and footers)
//! as Markdown, its pictures saved in the vault's attachment folder and
//! embedded, and front matter that says where and when it came from. The
//! fetching is the caller's: the command line brings an HTTP client, and the
//! tests a table of made-up pages.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::LazyLock;

use ego_tree::NodeRef;
use regex::Regex;
use scraper::{Html, Node};

use crate::day::Day;
use crate::html_markdown::HtmlMarkdown;
use crate::vault::Vault;
use crate::vault_moves::VaultMoves;

/// Elements that are never the page's text.
const DROPPED: [&str; 18] = [
    "nav", "script", "style", "aside", "footer", "header", "form", "noscript", "iframe", "button", "svg", "template", "select", "textarea",
    "canvas", "dialog", "object", "embed",
];

/// Elements written with no end tag.
const VOID: [&str; 5] = ["br", "hr", "img", "input", "wbr"];

/// The attributes the Markdown converter reads; the rest are left behind.
const KEPT: [&str; 5] = ["class", "start", "type", "checked", "alt"];

/// A class or an id that marks a block beside the text, not the text.
static BESIDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|[\s_-])(sidebar|share|sharing|social|comments?|related|newsletter|cookies?|breadcrumbs?|advert|promo)([\s_-]|$)")
        .unwrap()
});
static PICTURE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[([^\]\n]*)\]\((https?://[^)\s]+)\)").unwrap());
static LINKED_PICTURE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[!\[([^\]\n]*)\]\((https?://[^)\s]+)\)\]\([^)\s]*\)").unwrap());
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// What a page gives a note: its title and its main text as Markdown, the
/// pictures and links in it made absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clipping {
    pub title: String,
    pub markdown: String,
}

pub struct WebClip;

type Ref<'a> = NodeRef<'a, Node>;

fn name(node: Ref) -> String {
    match node.value() {
        Node::Element(element) => element.name().to_lowercase(),
        _ => String::new(),
    }
}

fn attr<'a>(node: Ref<'a>, key: &str) -> Option<&'a str> {
    match node.value() {
        Node::Element(element) => element.attr(key),
        _ => None,
    }
}

/// Whether [node] is a block beside the text: a menu, a script, a sidebar.
fn dropped(node: Ref) -> bool {
    let tag = name(node);
    if DROPPED.contains(&tag.as_str()) {
        return true;
    }
    if !matches!(node.value(), Node::Element(_)) {
        return false;
    }
    if attr(node, "hidden").is_some() || attr(node, "aria-hidden") == Some("true") {
        return true;
    }
    if matches!(attr(node, "role"), Some("navigation" | "complementary" | "banner" | "contentinfo")) {
        return true;
    }
    let marks = format!("{} {}", attr(node, "class").unwrap_or(""), attr(node, "id").unwrap_or(""));
    // Never the body or a whole article, whatever its class says.
    !matches!(tag.as_str(), "body" | "html" | "article" | "main") && BESIDE.is_match(&marks)
}

/// The length of the text under [node], without what is dropped.
fn text_length(node: Ref) -> usize {
    match node.value() {
        Node::Text(text) => text.trim().chars().count(),
        Node::Element(_) if dropped(node) => 0,
        _ => node.children().map(text_length).sum(),
    }
}

/// The element that holds the page's text.
fn main_text(document: &Html) -> Ref<'_> {
    let root = document.tree.root();
    let body = root.descendants().find(|n| name(*n) == "body").unwrap_or(root);
    let longest = |tag: &str| {
        root.descendants()
            .filter(|n| name(*n) == tag || (tag == "main" && attr(*n, "role") == Some("main")))
            .map(|n| (text_length(n), n))
            .filter(|(length, _)| *length > 0)
            .max_by_key(|(length, _)| *length)
            .map(|(_, n)| n)
    };
    if let Some(found) = longest("article").or_else(|| longest("main")) {
        return found;
    }
    // Readability's way: each paragraph counts for the block it is in, and
    // half for the block around that one.
    let mut scores: HashMap<ego_tree::NodeId, (usize, Ref)> = HashMap::new();
    for paragraph in body.descendants().filter(|n| matches!(name(*n).as_str(), "p" | "pre" | "blockquote")) {
        if paragraph.ancestors().any(dropped) {
            continue;
        }
        let length = text_length(paragraph);
        if length < 25 {
            continue;
        }
        if let Some(parent) = paragraph.parent() {
            scores.entry(parent.id()).or_insert((0, parent)).0 += length;
            if let Some(grand) = parent.parent().filter(|g| matches!(g.value(), Node::Element(_))) {
                scores.entry(grand.id()).or_insert((0, grand)).0 += length / 2;
            }
        }
    }
    scores.into_values().max_by_key(|(score, _)| *score).map(|(_, n)| n).unwrap_or(body)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

/// A picture's address: its `src`, else the lazy loaders' `data-src`, else
/// the first of its `srcset`.
fn picture_source(node: Ref) -> Option<String> {
    let usable = |s: &&str| !s.trim().is_empty() && !s.trim().starts_with("data:");
    ["src", "data-src", "data-original", "data-lazy-src"]
        .iter()
        .find_map(|key| attr(node, key).filter(usable).map(|s| s.trim().to_string()))
        .or_else(|| {
            ["srcset", "data-srcset"].iter().find_map(|key| {
                let first = attr(node, key)?.split(',').next()?.split_whitespace().next()?;
                Some(first.to_string()).filter(|s| usable(&s.as_str()))
            })
        })
}

/// [node]'s children as HTML again, without what is dropped, with every
/// link and picture made absolute against [base].
fn write(node: Ref, base: &str, out: &mut String) {
    for child in node.children() {
        match child.value() {
            Node::Text(text) => out.push_str(&escape(text)),
            Node::Element(element) => {
                if dropped(child) {
                    continue;
                }
                let tag = name(child);
                match tag.as_str() {
                    "img" => {
                        let Some(source) = picture_source(child).and_then(|s| WebClip::resolve(base, &s)) else { continue };
                        // Spaces and brackets would end the Markdown link.
                        let source = source.replace(' ', "%20").replace('(', "%28").replace(')', "%29");
                        let alt = attr(child, "alt").unwrap_or("").replace(['[', ']', '\n'], " ");
                        out.push_str(&format!("<img src=\"{}\" alt=\"{}\">", escape_attr(&source), escape_attr(alt.trim())));
                        continue;
                    }
                    "source" => continue,
                    "picture" => {
                        write(child, base, out);
                        continue;
                    }
                    _ => {}
                }
                // A caption is a paragraph under its picture.
                let tag = if tag == "figcaption" { "p".to_string() } else { tag };
                out.push('<');
                out.push_str(&tag);
                for (key, value) in element.attrs() {
                    if KEPT.contains(&key) {
                        out.push_str(&format!(" {key}=\"{}\"", escape_attr(value)));
                    }
                }
                if tag == "a"
                    && let Some(href) = attr(child, "href").filter(|h| !h.trim().starts_with("javascript:"))
                    && let Some(href) = WebClip::resolve(base, href)
                {
                    out.push_str(&format!(" href=\"{}\"", escape_attr(&href)));
                }
                out.push('>');
                if VOID.contains(&tag.as_str()) {
                    continue;
                }
                write(child, base, out);
                out.push_str(&format!("</{tag}>"));
            }
            _ => {}
        }
    }
}

/// The text under [node], each run of spaces as one.
fn plain(node: Ref) -> String {
    let text: String = node
        .descendants()
        .filter_map(|n| match n.value() {
            Node::Text(text) => Some(&**text),
            _ => None,
        })
        .collect();
    SPACES.replace_all(&text, " ").trim().to_string()
}

impl WebClip {
    /// The title and the main text of the page [html], read from [url].
    pub fn extract(html: &str, url: &str) -> Clipping {
        let document = Html::parse_document(html);
        let root = document.tree.root();
        let base = root
            .descendants()
            .find(|n| name(*n) == "base")
            .and_then(|n| attr(n, "href"))
            .and_then(|href| Self::resolve(url, href))
            .unwrap_or_else(|| url.to_string());
        let meta = |property: &str| {
            root.descendants()
                .find(|n| name(*n) == "meta" && (attr(*n, "property") == Some(property) || attr(*n, "name") == Some(property)))
                .and_then(|n| attr(n, "content"))
                .map(|t| SPACES.replace_all(t, " ").trim().to_string())
                .filter(|t| !t.is_empty())
        };
        let title = meta("og:title")
            .or_else(|| root.descendants().find(|n| name(*n) == "title").map(plain).filter(|t| !t.is_empty()))
            .or_else(|| root.descendants().find(|n| name(*n) == "h1").map(plain).filter(|t| !t.is_empty()))
            .unwrap_or_default();
        let main = main_text(&document);
        let mut inner = String::new();
        write(main, &base, &mut inner);
        let markdown = HtmlMarkdown::convert(&format!("<html><body>{inner}</body></html>")).unwrap_or_else(|| plain(main));
        Clipping { title, markdown }
    }

    /// [href] as an absolute address, read on the page at [base].
    pub fn resolve(base: &str, href: &str) -> Option<String> {
        let href = href.trim();
        if href.is_empty() {
            return None;
        }
        if SCHEME.is_match(href) {
            return Some(href.to_string());
        }
        let (scheme, rest) = base.split_once("://")?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let tail = tail.split('#').next().unwrap_or("");
        let (path, query) = match tail.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (tail, None),
        };
        let origin = format!("{scheme}://{authority}");
        if let Some(rest) = href.strip_prefix("//") {
            return Some(format!("{scheme}://{rest}"));
        }
        if href.starts_with('#') {
            let query = query.map(|q| format!("?{q}")).unwrap_or_default();
            return Some(format!("{origin}{}{query}{href}", if path.is_empty() { "/" } else { path }));
        }
        if href.starts_with('?') {
            return Some(format!("{origin}{}{href}", if path.is_empty() { "/" } else { path }));
        }
        let joined = if href.starts_with('/') {
            href.to_string()
        } else {
            let folder = path.rfind('/').map(|at| &path[..=at]).unwrap_or("/");
            format!("{folder}{href}")
        };
        // `.` and `..` in the path, not in its query or fragment.
        let split = joined.find(['?', '#']).unwrap_or(joined.len());
        let (joined_path, after) = joined.split_at(split);
        let mut parts: Vec<&str> = Vec::new();
        let pieces: Vec<&str> = joined_path.split('/').skip(1).collect();
        for (at, piece) in pieces.iter().enumerate() {
            let last = at + 1 == pieces.len();
            match *piece {
                "." => {
                    if last {
                        parts.push("");
                    }
                }
                ".." => {
                    parts.pop();
                    if last {
                        parts.push("");
                    }
                }
                piece => parts.push(piece),
            }
        }
        Some(format!("{origin}/{}{after}", parts.join("/")))
    }

    /// The web pictures [markdown] embeds, each once, in order.
    pub fn pictures(markdown: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        PICTURE.captures_iter(markdown).map(|c| c[2].to_string()).filter(|url| seen.insert(url.clone())).collect()
    }

    /// [markdown] with each picture in [saved] (its address to its vault
    /// path) embedded from the vault, as `![[path]]`; the others stay links.
    pub fn embed(markdown: &str, saved: &HashMap<String, String>) -> String {
        // A saved picture that was a link (to its larger self, most often) is
        // the picture alone: Obsidian draws no embed inside a link.
        let unlinked = LINKED_PICTURE.replace_all(markdown, |c: &regex::Captures| match saved.get(&c[2]) {
            Some(path) => format!("![[{path}]]"),
            None => c[0].to_string(),
        });
        PICTURE
            .replace_all(&unlinked, |c: &regex::Captures| match saved.get(&c[2]) {
                Some(path) => format!("![[{path}]]"),
                None => c[0].to_string(),
            })
            .into_owned()
    }

    /// A file name from a page's [title]: no character a vault path, a link
    /// or Windows refuses, at most 120 characters.
    pub fn note_name(title: &str) -> String {
        let cleaned: String = title.chars().map(|c| if c.is_control() || "/\\:#^[]|?*\"<>".contains(c) { ' ' } else { c }).collect();
        let cleaned = SPACES.replace_all(&cleaned, " ");
        let cleaned: String = cleaned.trim().trim_start_matches('.').trim().chars().take(120).collect();
        let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
        if VaultMoves::is_valid_name(&cleaned) { cleaned } else { "Clipping".to_string() }
    }

    /// The extension a picture is saved with, from its media type, else its
    /// address; `None` for what is not a picture.
    pub fn picture_extension(url: &str, content_type: Option<&str>) -> Option<String> {
        let media = content_type.map(|t| t.split(';').next().unwrap_or("").trim().to_lowercase()).unwrap_or_default();
        let known = ["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp"];
        let from_type = match media.as_str() {
            "image/png" => Some("png"),
            "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("jpg"),
            "image/gif" => Some("gif"),
            "image/webp" => Some("webp"),
            "image/svg+xml" => Some("svg"),
            "image/avif" => Some("avif"),
            "image/bmp" => Some("bmp"),
            _ => None,
        };
        if let Some(extension) = from_type {
            return Some(extension.to_string());
        }
        if !media.is_empty() && !media.starts_with("image/") && media != "application/octet-stream" {
            return None;
        }
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let extension = path.rsplit('/').next()?.rsplit_once('.')?.1.to_lowercase();
        known.contains(&extension.as_str()).then_some(extension)
    }

    /// The note's text: front matter with the page's address and the day
    /// it was clipped, then the page.
    pub fn note(url: &str, clipped: Day, body: &str) -> String {
        let quoted = url.replace('\\', "\\\\").replace('"', "\\\"");
        format!("---\nsource: \"{quoted}\"\nclipped: {clipped}\n---\n\n{}\n", body.trim_end())
    }

    /// Writes [bytes] to a new file `base.extension` in [folder] of the vault
    /// at [root], as `base 1.extension` and on when the name is taken in
    /// [files] or on disk: never over a file. Its vault path, added to [files].
    pub fn create(
        root: &Path,
        folder: &str,
        base: &str,
        extension: &str,
        bytes: &[u8],
        files: &mut BTreeSet<String>,
    ) -> io::Result<String> {
        let folder = folder.trim().trim_matches('/');
        if folder.split('/').any(|part| part == ".." || part.starts_with('.') || part.contains('\\'))
            || Path::new(folder).is_absolute()
            || folder.contains(':')
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{folder} is not a folder in the vault")));
        }
        loop {
            let path = VaultMoves::free_name(base, extension, folder, files);
            let file = root.join(&path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            match fs::OpenOptions::new().write(true).create_new(true).open(&file) {
                Ok(mut new) => {
                    files.insert(path.clone());
                    if let Err(error) = io::Write::write_all(&mut new, bytes) {
                        let _ = fs::remove_file(&file);
                        files.remove(&path);
                        return Err(error);
                    }
                    return Ok(path);
                }
                // Made meanwhile by another app: the next number.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    files.insert(path);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// The page [html], read from [url], as a new note in [folder] of the
    /// vault at [root], named [title] or the page's title: [save_clipping].
    pub fn save(
        root: &Path,
        url: &str,
        html: &str,
        folder: &str,
        title: Option<&str>,
        clipped: Day,
        fetch: impl Fn(&str) -> Result<(Vec<u8>, Option<String>), String>,
    ) -> Result<Saved, String> {
        let mut clipping = Self::extract(html, url);
        if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
            clipping.title = title.to_string();
        }
        Self::save_clipping(root, url, clipping, folder, clipped, fetch)
    }

    /// [clipping], from the page at [url], as a new note in [folder] of
    /// the vault at [root], named after its title (else the page's host).
    /// Each picture is fetched with [fetch] (its bytes and media type) into
    /// the attachment folder and embedded; one that fails stays a web link.
    pub fn save_clipping(
        root: &Path,
        url: &str,
        clipping: Clipping,
        folder: &str,
        clipped: Day,
        fetch: impl Fn(&str) -> Result<(Vec<u8>, Option<String>), String>,
    ) -> Result<Saved, String> {
        let mut skipped = Vec::new();
        let title = clipping.title.trim();
        let host = url.split_once("://").map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest)).unwrap_or(url);
        let name = Self::note_name(if title.is_empty() { host } else { title });
        let mut files = Vault::scan(root);
        let attachments = Vault::attachment_folder(root);
        let mut saved = HashMap::new();
        for picture in Self::pictures(&clipping.markdown) {
            let result = fetch(&picture).and_then(|(bytes, media)| {
                let extension = Self::picture_extension(&picture, media.as_deref())
                    .ok_or_else(|| format!("not a picture ({})", media.unwrap_or_default()))?;
                Self::create(root, &attachments, &name, &extension, &bytes, &mut files).map_err(|e| e.to_string())
            });
            match result {
                Ok(path) => {
                    saved.insert(picture, path);
                }
                Err(why) => skipped.push((picture, why)),
            }
        }
        let text = Self::note(url, clipped, &Self::embed(&clipping.markdown, &saved));
        let path = Self::create(root, folder, &name, "md", text.as_bytes(), &mut files).map_err(|e| format!("{folder}: {e}"))?;
        Ok(Saved { path, pictures: saved.into_values().collect(), skipped })
    }
}

/// What a clip wrote: the note's vault path and its pictures', and each
/// picture left a web link, with why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: String,
    pub pictures: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every page here is made up.

    const PAGE: &str = r#"<!doctype html><html><head>
<title>Kites in Spring | The Example Gazette</title>
<meta property="og:title" content="Kites in   Spring">
<style>p { color: red }</style><script>track()</script>
</head><body>
<header><a href="/">Gazette</a><nav><a href="/news">News</a></nav></header>
<div class="sidebar"><p>Subscribe to our newsletter for more stories about everything under the sun.</p></div>
<article>
<header><h1>Kites in Spring</h1></header>
<p>The wind on the hill is <strong>steady</strong> in April, and a <a href="../guide/kites.html">light kite</a> flies best.</p>
<figure><img src="images/kite one.png" alt="A red kite"><figcaption>A red kite</figcaption></figure>
<p><img data-src="//cdn.example.org/kite2.jpg" src="data:image/gif;base64,R0lGOD"></p>
<aside>Related: <a href="/other">Other</a></aside>
<div class="share-buttons"><a href="https://social.example/share">Share</a></div>
<h2>Strings</h2>
<ul><li>Cotton</li><li>Nylon</li></ul>
<script>more()</script>
</article>
<footer>© The Example Gazette</footer>
</body></html>"#;

    #[test]
    fn an_article_is_the_text() {
        let clipping = WebClip::extract(PAGE, "https://news.example.com/2026/spring/kites.html?ref=home");
        assert_eq!(clipping.title, "Kites in Spring");
        assert_eq!(
            clipping.markdown,
            "The wind on the hill is **steady** in April, and a [light kite](https://news.example.com/2026/guide/kites.html) flies best.\n\n\
             ![A red kite](https://news.example.com/2026/spring/images/kite%20one.png)\n\nA red kite\n\n\
             ![](https://cdn.example.org/kite2.jpg)\n\n\
             ## Strings\n\n\
             - Cotton\n- Nylon"
        );
        for gone in ["Gazette", "News", "newsletter", "Related", "Share", "track", "more()", "©"] {
            assert!(!clipping.markdown.contains(gone), "{gone}: {}", clipping.markdown);
        }
    }

    #[test]
    fn without_an_article_the_largest_block_of_paragraphs() {
        let html = r#"<html><head><title>Notes on tea</title></head><body>
<div id="menu"><p>Home</p><p>About</p></div>
<div id="content"><div class="post">
<p>Green tea wants water well below the boil, about eighty degrees.</p>
<p>Black tea takes boiling water and four minutes, sometimes five.</p>
</div></div>
<div class="comments"><p>Great post! I always wondered about the water temperature for tea.</p></div>
</body></html>"#;
        let clipping = WebClip::extract(html, "https://tea.example.net/notes");
        assert_eq!(clipping.title, "Notes on tea");
        assert_eq!(
            clipping.markdown,
            "Green tea wants water well below the boil, about eighty degrees.\n\nBlack tea takes boiling water and four minutes, sometimes five."
        );
        // A main element, when there is no article.
        let main = r#"<html><body><nav><p>Menu</p></nav><main><h1>Title</h1><p>Body text.</p></main></body></html>"#;
        assert_eq!(WebClip::extract(main, "https://x.example/").markdown, "# Title\n\nBody text.");
        assert_eq!(WebClip::extract(main, "https://x.example/").title, "Title");
    }

    #[test]
    fn addresses_resolve_as_a_browser_does() {
        let base = "https://example.com/a/b/page.html?x=1#top";
        for (href, wanted) in [
            ("https://other.example/x", "https://other.example/x"),
            ("mailto:a@example.com", "mailto:a@example.com"),
            ("//cdn.example.com/p.png", "https://cdn.example.com/p.png"),
            ("/root.png", "https://example.com/root.png"),
            ("pic.png", "https://example.com/a/b/pic.png"),
            ("./pic.png", "https://example.com/a/b/pic.png"),
            ("../pic.png?s=2", "https://example.com/a/pic.png?s=2"),
            ("../../../../pic.png", "https://example.com/pic.png"),
            ("..", "https://example.com/a/"),
            ("?page=2", "https://example.com/a/b/page.html?page=2"),
            ("#notes", "https://example.com/a/b/page.html?x=1#notes"),
        ] {
            assert_eq!(WebClip::resolve(base, href).as_deref(), Some(wanted), "{href}");
        }
        assert_eq!(WebClip::resolve("https://example.com", "x.png").as_deref(), Some("https://example.com/x.png"));
        assert_eq!(WebClip::resolve(base, "  "), None);
        // A <base> changes where the page's addresses start.
        let html = r#"<html><head><base href="https://static.example.com/assets/"></head><body><article><p>See <img src="k.png" alt="k"></p></article></body></html>"#;
        assert_eq!(WebClip::extract(html, "https://example.com/post").markdown, "See ![k](https://static.example.com/assets/k.png)");
    }

    #[test]
    fn pictures_are_found_once_and_embedded() {
        let markdown =
            "![a](https://x.example/a.png) and ![b](https://x.example/b.jpg)\n\n![a again](https://x.example/a.png) ![local](a.png)";
        assert_eq!(WebClip::pictures(markdown), ["https://x.example/a.png", "https://x.example/b.jpg"]);
        let saved = HashMap::from([("https://x.example/a.png".to_string(), "uploads/Kites.png".to_string())]);
        assert_eq!(
            WebClip::embed(markdown, &saved),
            "![[uploads/Kites.png]] and ![b](https://x.example/b.jpg)\n\n![[uploads/Kites.png]] ![local](a.png)"
        );
        // A saved picture in a link is the picture alone; one not saved keeps its link.
        let linked = "[![a](https://x.example/a.png)](https://x.example/big) [![b](https://x.example/b.jpg)](https://x.example/b)";
        assert_eq!(WebClip::embed(linked, &saved), "![[uploads/Kites.png]] [![b](https://x.example/b.jpg)](https://x.example/b)");
    }

    #[test]
    fn names_and_extensions() {
        assert_eq!(WebClip::note_name("Kites: a guide [2026] | Gazette?"), "Kites a guide 2026 Gazette");
        assert_eq!(WebClip::note_name("  ...hidden "), "hidden");
        assert_eq!(WebClip::note_name("///"), "Clipping");
        assert_eq!(WebClip::note_name(&"x".repeat(300)).chars().count(), 120);
        assert_eq!(WebClip::picture_extension("https://x/a", Some("image/jpeg; q=1")).as_deref(), Some("jpg"));
        assert_eq!(WebClip::picture_extension("https://x/a.WEBP?w=2", None).as_deref(), Some("webp"));
        assert_eq!(WebClip::picture_extension("https://x/a.png", Some("text/html")), None);
        assert_eq!(WebClip::picture_extension("https://x/a", None), None);
        assert_eq!(
            WebClip::note("https://x.example/a?b=\"c\"", Day::new(2026, 10, 27).unwrap(), "Body\n\n"),
            "---\nsource: \"https://x.example/a?b=\\\"c\\\"\"\nclipped: 2026-10-27\n---\n\nBody\n"
        );
    }

    #[test]
    fn a_new_file_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("Clippings")).unwrap();
        fs::write(root.path().join("Clippings/Kites.md"), "mine").unwrap();
        // Not in [files]: found on disk all the same.
        let mut files = BTreeSet::new();
        let path = WebClip::create(root.path(), "Clippings", "Kites", "md", b"new", &mut files).unwrap();
        assert_eq!(path, "Clippings/Kites 1.md");
        assert_eq!(fs::read_to_string(root.path().join("Clippings/Kites.md")).unwrap(), "mine");
        assert_eq!(WebClip::create(root.path(), "Clippings", "Kites", "md", b"", &mut files).unwrap(), "Clippings/Kites 2.md");
        assert_eq!(WebClip::create(root.path(), "", "Top", "md", b"", &mut files).unwrap(), "Top.md");
        for folder in ["../out", "a/../../b", ".git", "C:/x", "a\\..\\b"] {
            assert!(WebClip::create(root.path(), folder, "x", "md", b"", &mut files).is_err(), "{folder}");
        }
    }

    #[test]
    fn a_page_is_saved_with_its_pictures() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::write(root.path().join(".obsidian/app.json"), r#"{"attachmentFolderPath":"uploads"}"#).unwrap();
        fs::create_dir_all(root.path().join("Clippings")).unwrap();
        fs::write(root.path().join("Clippings/Kites in Spring.md"), "clipped before").unwrap();
        let fetch = |url: &str| match url {
            "https://news.example.com/2026/spring/images/kite%20one.png" => Ok((b"png".to_vec(), Some("image/png".to_string()))),
            _ => Err("404".to_string()),
        };
        let day = Day::new(2026, 10, 27).unwrap();
        let url = "https://news.example.com/2026/spring/kites.html";
        let saved = WebClip::save(root.path(), url, PAGE, "Clippings", None, day, fetch).unwrap();
        let path = saved.path;
        assert_eq!(path, "Clippings/Kites in Spring 1.md");
        assert_eq!(saved.pictures, ["uploads/Kites in Spring.png"]);
        assert_eq!(fs::read_to_string(root.path().join("Clippings/Kites in Spring.md")).unwrap(), "clipped before");
        assert_eq!(fs::read(root.path().join("uploads/Kites in Spring.png")).unwrap(), b"png");
        let text = fs::read_to_string(root.path().join(&path)).unwrap();
        assert!(
            text.starts_with("---\nsource: \"https://news.example.com/2026/spring/kites.html\"\nclipped: 2026-10-27\n---\n\nThe wind"),
            "{text}"
        );
        assert!(text.contains("![[uploads/Kites in Spring.png]]\n\nA red kite"), "{text}");
        assert!(text.contains("![](https://cdn.example.org/kite2.jpg)"), "{text}");
        assert_eq!(saved.skipped, [("https://cdn.example.org/kite2.jpg".to_string(), "404".to_string())]);
        // A title given names the note, and the vault's root holds the pictures without a setting.
        fs::remove_file(root.path().join(".obsidian/app.json")).unwrap();
        let saved = WebClip::save(root.path(), url, PAGE, "", Some("My kites"), day, fetch).unwrap();
        assert_eq!(saved.path, "My kites.md");
        assert!(root.path().join("My kites.png").is_file());
    }
}
