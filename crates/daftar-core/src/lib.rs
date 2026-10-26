//! Daftar's rules for an Obsidian vault in a Git repository, with no window:
//! the note parser, the link resolver, the task lines, the index, search,
//! git and the config file.
//!
//! The rules are the ones Daftar keeps on macOS (`notes-mac`, `DaftarCore`)
//! and on Android (`notes-droid`). The three apps share no code; when a rule
//! changes in one, change it in the others.

pub mod bookmarks;
pub mod calendar_week;
pub mod config;
pub mod day;
pub mod editor_commands;
pub mod fuzzy;
pub mod git;
pub mod link_resolver;
pub mod markdown_highlighter;
pub mod markdown_table;
pub mod note_graph;
pub mod note_parser;
pub mod quick_switcher;
pub mod reading_places;
pub mod search;
pub mod tag_tree;
pub mod task_board;
pub mod task_expression;
pub mod task_line;
pub mod tasks_query;
pub mod text;
pub mod unlinked_mentions;
pub mod vault;
pub mod vault_edits;
pub mod vault_moves;

pub use bookmarks::{BookmarkEntry, Bookmarks, BookmarksFailure};
pub use calendar_week::{CalendarDay, CalendarWeek, Meeting};
pub use config::{ConfigFile, Value};
pub use day::Day;
pub use editor_commands::{EditorCommands, Return, Snippet, TextEdit};
pub use fuzzy::{Fuzzy, Match};
pub use git::{Git, GitFailure, GitStatus, Synced};
pub use link_resolver::LinkResolver;
pub use markdown_highlighter::{BlockState, Kind as HighlightKind, MarkdownHighlighter, Span as HighlightSpan};
pub use markdown_table::{MarkdownTable, TableTab};
pub use note_graph::{GraphEdge, GraphLayout, GraphNode, NoteGraph};
pub use note_parser::{FrontMatter, Heading, Link, NoteParser, ParsedNote, ParsedTask, Wikilink};
pub use quick_switcher::{Candidate, QuickSwitcher};
pub use reading_places::ReadingPlaces;
pub use search::{Hit, SearchIndex};
pub use tag_tree::TagNode;
pub use task_board::{Column, Drop as BoardDrop, Grouping, TaskBoard};
pub use task_expression::TaskExpression;
pub use task_line::{Meta, Recurrence, TaskFields, TaskLine, TaskMetadata};
pub use tasks_query::{DateRange, QueryTask, TaskState, TasksQuery};
pub use unlinked_mentions::{Mention, UnlinkedMentions};
pub use vault::{Backlink, ExcludedFiles, NoteEntry, TreeNode, Vault, VaultIndex, VaultTask};
pub use vault_edits::{Failure, VaultEdits};
pub use vault_moves::{MoveFailure, VaultMoves};
