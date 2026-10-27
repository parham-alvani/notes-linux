//! The size an embedded picture asks for: `![[a.png|300]]`,
//! `![[a.png|300x200]]` or `![alt|300](a.png)`, as Obsidian writes it. The
//! reader reads it in its own renderer; the editor draws its pictures at it.

use std::sync::LazyLock;

use regex::Regex;

/// The width, and the height when it is given.
pub type Hint = Option<(i32, Option<i32>)>;

/// Each embed of [line], its target as written and the size it asks for:
/// wiki embeds first, then markdown ones.
pub fn embeds(line: &str) -> Vec<(String, Hint)> {
    static WIKI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[\[([^\]\n]+?)\]\]").unwrap());
    static MARKDOWN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[([^\]\n]*)\]\(<?([^)>\s]+)>?\)").unwrap());
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d+)(?:x(\d+))?\s*$").unwrap());
    let size = |text: &str| SIZE.captures(text).and_then(|c| Some((c[1].parse().ok()?, c.get(2).and_then(|h| h.as_str().parse().ok()))));
    let mut out = Vec::new();
    for found in WIKI.captures_iter(line) {
        let inner = &found[1];
        let (target, alias) = inner.split_once('|').map_or((inner, None), |(t, a)| (t, Some(a)));
        let target = target.split('#').next().unwrap_or(target).trim();
        out.push((target.to_string(), alias.and_then(size)));
    }
    for found in MARKDOWN.captures_iter(line) {
        let alt = &found[1];
        let hint = size(alt.rsplit('|').next().unwrap_or(alt));
        out.push((percent_decoded(&found[2]), hint));
    }
    out
}

/// `%20` and the like as the characters they stand for.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(byte) = text.get(at + 1..at + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_a_number_after_the_bar() {
        assert_eq!(
            embeds("![[a.png|120x80]] ![[b.png#x|a beach]] ![Sky|250](sky%20blue.png) ![300](c.png) ![[d.png]]"),
            [
                ("a.png".to_string(), Some((120, Some(80)))),
                ("b.png".to_string(), None),
                ("d.png".to_string(), None),
                ("sky blue.png".to_string(), Some((250, None))),
                ("c.png".to_string(), Some((300, None))),
            ]
        );
    }
}
