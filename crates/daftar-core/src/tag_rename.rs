//! A tag renamed in a note: `#old` in the text and `old` in the `tags`
//! property become the new name, and so do the tags under it, `#old/sub`.
//! Tags match without regard to case, as Obsidian counts them. Code, code
//! blocks and links are left as they are.

use std::sync::LazyLock;

use regex::Regex;

/// A fence line, also for `VaultReplace`'s "Skip code".
pub(crate) static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}_/-]+").unwrap());
static WHOLE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}_/-]+$").unwrap());
static BODY_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)#([\p{L}\p{N}_/-]+)").unwrap());
static PROPERTY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w-]*)\s*:(.*)$").unwrap());
static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*-\s+)(.*)$").unwrap());
static PROTECTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`[^`\n]*`|!?\[\[[^\]\n]*\]\]|!?\[[^\]\n]*\]\([^)\n]*\)|[a-zA-Z][a-zA-Z0-9+.-]*://\S+").unwrap());

pub struct TagRename;

impl TagRename {
    /// A name a tag can have: letters, digits, `_`, `-` and `/`, not only digits.
    pub fn is_valid(tag: &str) -> bool {
        let bare = tag.strip_prefix('#').unwrap_or(tag);
        WHOLE_NAME.is_match(bare)
            && !bare.chars().all(|c| c.is_numeric() || c == '/' || c == '-')
            && !bare.starts_with('/')
            && !bare.ends_with('/')
            && !bare.contains("//")
    }

    /// [text] with the tag [old] named [new], or the text as it was.
    pub fn rename(text: &str, old: &str, new: &str) -> String {
        let old = old.strip_prefix('#').unwrap_or(old);
        let new = new.strip_prefix('#').unwrap_or(new);
        if !Self::is_valid(old) || !Self::is_valid(new) || old == new {
            return text.to_string();
        }
        let from = old.to_lowercase();
        let renamed = |tag: &str| -> Option<String> {
            let folded = tag.to_lowercase();
            if folded == from {
                return Some(new.to_string());
            }
            if folded.starts_with(&format!("{from}/")) {
                return Some(format!("{new}{}", tag.chars().skip(old.chars().count()).collect::<String>()));
            }
            None
        };
        let in_value =
            |value: &str| NAME.replace_all(value, |c: &regex::Captures| renamed(&c[0]).unwrap_or_else(|| c[0].to_string())).into_owned();

        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let mut start = 0;
        if lines.first().is_some_and(|l| crate::text::trim(l) == "---")
            && let Some(end) = (1..lines.len()).find(|&i| crate::text::trim(&lines[i]) == "---")
        {
            let mut in_tags = false;
            for line in lines.iter_mut().take(end).skip(1) {
                if let Some(c) = PROPERTY.captures(line) {
                    in_tags = ["tags", "tag"].contains(&c[1].to_lowercase().as_str());
                    if in_tags {
                        *line = format!("{}:{}", &c[1], in_value(&c[2]));
                    }
                } else if in_tags && let Some(c) = ITEM.captures(line) {
                    *line = format!("{}{}", &c[1], in_value(&c[2]));
                } else if !line.starts_with(' ') && !line.starts_with('\t') {
                    in_tags = false;
                }
            }
            start = end + 1;
        }

        let mut fence: Option<String> = None;
        for line in lines.iter_mut().skip(start) {
            if let Some(c) = FENCE.captures(line) {
                let marker = c[1].to_string();
                match &fence {
                    None => fence = Some(marker),
                    Some(open) if marker.starts_with(&open[..1]) && marker.len() >= open.len() => fence = None,
                    _ => {}
                }
                continue;
            }
            if fence.is_some() || !line.contains('#') {
                continue;
            }
            // Code and links are looked for only on a line with the tag: a
            // heading has a `#` too.
            let tags: Vec<_> = BODY_TAG.captures_iter(line).filter_map(|c| c.get(1)).filter(|m| renamed(m.as_str()).is_some()).collect();
            if tags.is_empty() {
                continue;
            }
            let taken: Vec<_> = PROTECTED.find_iter(line).map(|m| m.range()).collect();
            let mut result = String::new();
            let mut last = 0;
            for tag in tags {
                // The whole match, `#` included, decides whether it is in code or a link.
                let hash = tag.start() - 1;
                if taken.iter().any(|r| r.start < tag.end() && hash < r.end) {
                    continue;
                }
                let Some(replacement) = renamed(tag.as_str()) else { continue };
                result.push_str(&line[last..tag.start()]);
                result.push_str(&replacement);
                last = tag.end();
            }
            if last != 0 {
                result.push_str(&line[last..]);
                *line = result;
            }
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_in_the_text_and_under_them_are_renamed() {
        let text = "#project/alpha and #Project too, #projects stays, #project/alpha/x\n(#project) [#project] are not tags in Obsidian, word#project neither";
        assert_eq!(
            TagRename::rename(text, "project", "work"),
            "#work/alpha and #work too, #projects stays, #work/alpha/x\n(#project) [#project] are not tags in Obsidian, word#project neither"
        );
    }

    #[test]
    fn code_links_and_addresses_are_left_alone() {
        let text = "Use `#project` and #project.\n[[Note#project]] [label](Note.md#project) https://example.com/#project\n```\n#project in a block\n```\n#project after it";
        assert_eq!(
            TagRename::rename(text, "#project", "#work"),
            "Use `#project` and #work.\n[[Note#project]] [label](Note.md#project) https://example.com/#project\n```\n#project in a block\n```\n#work after it"
        );
    }

    #[test]
    fn the_property_is_renamed_in_each_of_its_forms() {
        let text = "---\ntitle: project\ntags: [project, \"project/alpha\", other]\ntag: project\naliases:\n  - project\n---\n# project";
        assert_eq!(
            TagRename::rename(text, "project", "work"),
            "---\ntitle: project\ntags: [work, \"work/alpha\", other]\ntag: work\naliases:\n  - project\n---\n# project"
        );
        let list = "---\ntags:\n  - project\n  - '#project/x'\n  - other\nup: \"[[project]]\"\n---\n";
        assert_eq!(
            TagRename::rename(list, "project", "work"),
            "---\ntags:\n  - work\n  - '#work/x'\n  - other\nup: \"[[project]]\"\n---\n"
        );
    }

    #[test]
    fn a_name_that_cannot_be_a_tag_changes_nothing() {
        assert!(!TagRename::is_valid("2026"));
        assert!(!TagRename::is_valid("two words"));
        assert!(!TagRename::is_valid("a//b"));
        assert!(!TagRename::is_valid(""));
        assert!(TagRename::is_valid("#کار/پروژه"));
        assert_eq!(TagRename::rename("#a", "a", "b c"), "#a");
        assert_eq!(TagRename::rename("#a", "a", "a"), "#a");
        assert_eq!(TagRename::rename("#کار و #کار/x", "کار", "work"), "#work و #work/x");
    }
}
