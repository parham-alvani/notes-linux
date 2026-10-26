//! Daftar's rules for an Obsidian vault in a Git repository, with no window:
//! the note parser, the link resolver, the task lines, the index, search,
//! git and the config file.
//!
//! The rules are the ones Daftar keeps on macOS (`notes-mac`, `DaftarCore`)
//! and on Android (`notes-droid`). The three apps share no code; when a rule
//! changes in one, change it in the others.

pub mod config;
pub mod day;
pub mod fuzzy;
pub mod git;
pub mod link_resolver;
pub mod note_parser;
pub mod quick_switcher;
pub mod search;
pub mod tag_tree;
pub mod task_line;
pub mod text;
pub mod vault;
pub mod vault_edits;

pub use config::{ConfigFile, Value};
pub use day::Day;
pub use fuzzy::{Fuzzy, Match};
pub use git::{Git, GitFailure, GitStatus, Synced};
pub use link_resolver::LinkResolver;
pub use note_parser::{FrontMatter, Heading, Link, NoteParser, ParsedNote, ParsedTask, Wikilink};
pub use quick_switcher::{Candidate, QuickSwitcher};
pub use search::{Hit, SearchIndex};
pub use tag_tree::TagNode;
pub use task_line::{Meta, Recurrence, TaskFields, TaskLine, TaskMetadata};
pub use vault::{Backlink, ExcludedFiles, NoteEntry, TreeNode, Vault, VaultIndex, VaultTask};
pub use vault_edits::{Failure, VaultEdits};
