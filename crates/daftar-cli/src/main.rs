//! Daftar on the command line: the vault the app shows, for a shell, a
//! script or a launcher (`daftar-cli find … | fzf`). It reads the same
//! config file as the app and the same rules from `daftar-core`.

/// `println!` for what the command prints: a reader that stops early, as
/// `head` or `fzf` does, ends the command quietly instead of a panic.
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::emit(format_args!($($arg)*))
    };
}

mod agenda;
mod alerts;
mod check;
mod clip;
mod drill;
mod export;
mod figures;
mod fmt;
mod hook;
mod ics;
mod images;
mod inbox;
mod jobs;
mod lint;
mod lsp;
mod mcp;
mod meaning;
mod memory;
mod mentions;
mod okf;
mod report;
mod review;
mod similar;
mod tui;
mod vault_commands;
mod watching;

use std::collections::BTreeSet;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};

use daftar_core::properties::PropertyValue;

use daftar_core::{
    CalendarEvents, Candidate, Clock, ConfigFile, Day, Drill, DrillState, ExcludedFiles, Git, Place, QueryTask, QuickCapture,
    QuickSwitcher, RandomNote, ReplaceOptions, SearchIndex, Sections, Side, SyncFailure, TaskLine, TaskStatuses, TaskType, TasksQuery,
    Vault, VaultEdits, VaultIndex, VaultTask,
};

#[derive(Parser)]
#[command(name = "daftar-cli", version, about = "Daftar on the command line: an Obsidian vault in a Git repository")]
struct Cli {
    /// The vault's folder; else the vault the current folder is in, else
    /// `vault` in ~/.config/daftar/config.toml. A folder, or the name of a
    /// vault of the list (`daftar-cli vaults`) when exactly one has it.
    #[arg(long, global = true, env = "DAFTAR_VAULT")]
    vault: Option<PathBuf>,
    /// Lists the hidden files too: Obsidian's excluded files and `files.hidden`
    /// in the config file (the files for AI agents), as `files.show_hidden` does.
    #[arg(long, global = true)]
    hidden: bool,
    /// With none, the TUI on a terminal, else this help.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// The vault in the terminal: the tree, the note, the quick switcher,
    /// search, tasks, backlinks and tags, in Daftar's colors.
    Tui,
    /// Notes and files by name, as the quick switcher ranks them.
    Find {
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// The text of every note, with SQLite's full-text search; with
    /// `--meaning`, by meaning too (a model run here, offline, Persian and
    /// English alike: see `semantic-index`).
    Search {
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        /// The words and the meaning together: the notes either search
        /// finds, ranked by both (reciprocal rank fusion).
        #[arg(long)]
        meaning: bool,
        /// By meaning alone: the notes whose sections are nearest the query.
        #[arg(long, conflicts_with = "meaning")]
        meaning_only: bool,
        /// One JSON array of `{"path", "line", "score", "snippet"}`: `line`
        /// from 1, `score` the cosine (--meaning-only), the fused rank
        /// (--meaning) or null (the words alone).
        #[arg(long)]
        json: bool,
    },
    /// The notes nearest to a note in meaning, that it does not link to yet.
    Related {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        /// One JSON array of `{"path", "line", "score", "snippet"}`, `score`
        /// the cosine.
        #[arg(long)]
        json: bool,
    },
    /// Pairs of notes that say nearly the same thing, to merge: `a.md`, `b.md`
    /// and the cosine of their vectors (each the mean of the note's
    /// sections), best first. Hidden notes, the week notes of `Calendar/`
    /// and the templates are left out. Needs the model, as `related` does.
    Similar {
        /// The cosine a pair needs, from -1 to 1.
        #[arg(long, default_value_t = similar::THRESHOLD)]
        threshold: f32,
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
        /// Only the pairs with this note, which comes first in each.
        #[arg(long, add = ArgValueCandidates::new(note_candidates))]
        note: Option<String>,
        /// Only the pairs with a note under this folder.
        #[arg(long)]
        folder: Option<String>,
        /// One JSON array of `{"a", "b", "score", "a_line", "b_line",
        /// "section"}`: the lines (from 1) of the two nearest sections and
        /// their cosine.
        #[arg(long)]
        json: bool,
    },
    /// Builds the index of search by meaning, or brings it up to the notes
    /// that changed: every section of a note embedded by a model run here,
    /// kept in ~/.cache/daftar/semantic/. The model is downloaded only with
    /// `--download-model` (531 MB, once, into ~/.cache/daftar/models/).
    /// Search by meaning and `related` bring the index up to the notes
    /// themselves, so this only does it ahead of them. Progress goes to
    /// stderr: `indexed N/M` lines when it is not a terminal. Without the
    /// model, these commands exit with 3.
    SemanticIndex {
        /// Downloads the model first when it is not there.
        #[arg(long)]
        download_model: bool,
    },
    /// Open tasks, the ones due first.
    Tasks {
        /// Only the ones due or scheduled by this day (YYYY-MM-DD, or `today`).
        #[arg(long)]
        due: Option<String>,
        /// Only the tasks in notes whose path starts with this.
        #[arg(long)]
        path: Option<String>,
        /// Closed tasks too.
        #[arg(long)]
        all: bool,
        /// One JSON array, each task with its path, line, status, dates and
        /// text (what `toggle --expect` takes).
        #[arg(long)]
        json: bool,
    },
    /// A Tasks plugin query, as a ` ```tasks ` block in a note runs it:
    /// `daftar-cli query 'not done' 'due before tomorrow' 'group by filename'`.
    Query {
        /// The query's lines; with none, the `tasks` blocks of `--note`, else
        /// the query read from stdin.
        lines: Vec<String>,
        /// The note the query is in, which `{{query.file.path}}` and the
        /// others stand for.
        #[arg(long)]
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: Option<String>,
        /// One JSON array, each task with the groups it is under.
        #[arg(long)]
        json: bool,
        /// Fail on a line the query language does not understand, as the Tasks
        /// plugin does, instead of running the query without it.
        #[arg(long)]
        strict: bool,
    },
    /// The days a `🔁` rule repeats on, as a tick works them out:
    /// `daftar-cli repeat 'every jalali month' --from 2026-12-31 --count 4`.
    Repeat {
        rule: String,
        /// The task's leading date (YYYY-MM-DD); today when not given.
        #[arg(long)]
        from: Option<String>,
        /// How many days to list.
        #[arg(long, default_value_t = 5)]
        count: usize,
    },
    /// Ticks or opens the task on a line of a note, as a click in the app does.
    Toggle {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The line, from 1, as `tasks` prints it.
        line: usize,
        /// The task's text, or its whole line, as `tasks` printed it: the
        /// tick is refused when the line holds another task now.
        #[arg(long)]
        expect: Option<String>,
    },
    /// Sets the status of the task on a line of a note, as the app's status
    /// menu does: `daftar-cli set-status Tasks/Work.md 12 r --reason 'waiting on review'`.
    SetStatus {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The line, from 1, as `tasks` prints it.
        line: usize,
        /// One character: ` ` to do, `/` in progress, `x` done, `-` cancelled,
        /// or one the Tasks plugin's settings add (see `statuses`).
        status: String,
        /// Why, kept on the task as `[reason:: …]`; empty takes it away.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Cancels the task on a line of a note, with the reason why:
    /// `daftar-cli cancel Tasks/Work.md 12 --reason 'no budget left'`.
    Cancel {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The line, from 1, as `tasks` prints it.
        line: usize,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Sorts the headings right under one by name, each with its section,
    /// as the outline's "Sort Headings Under This" does:
    /// `daftar-cli sort-headings Plants.md 'Plants'`.
    SortHeadings {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The heading's text, or a line, from 1, in its section.
        heading: String,
    },
    /// The task statuses of the vault: the four every vault has and the ones
    /// the Tasks plugin's settings add, with their symbol and type.
    Statuses,
    /// The notes that link to a note.
    Backlinks {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
    },
    /// Links that go nowhere.
    Broken,
    /// Every tag, with how many notes have it.
    Tags,
    /// Spaced-repetition drills from notes of questions: a card is a heading
    /// with its section as the answer (see `daftar_core::drill`), scheduled
    /// by SM-2 in Daftar's state folder; no note is written. In a terminal
    /// it asks each card due: Enter shows the answer, 1-4 rates it (Again,
    /// Hard, Good, Easy), q stops. Else it lists them, as --list.
    Drill {
        /// Notes or folders of the vault; `Learning/Interview-Preparation` when none.
        targets: Vec<String>,
        /// New cards at most.
        #[arg(long, default_value_t = 10)]
        new: usize,
        /// Cards at most, due and new.
        #[arg(short = 'n', long, default_value_t = 100)]
        limit: usize,
        /// Prints the cards due, a line each (`path:line`, a tab, the day it
        /// was due or `new`, a tab, the heading), and asks nothing.
        #[arg(long)]
        list: bool,
    },
    /// Triage of the capture inbox (`scratchpad.note` in the config file,
    /// else `Scratchpad.md`): each item, a line each (`path:line`, a tab,
    /// its text, then a tab before each note it likely belongs to: the
    /// notes it links to, then search by meaning when the model is there,
    /// else by words). On a terminal it walks them: 1-3 moves an item to
    /// that note, / names another, s skips, q stops.
    Inbox {
        #[command(subcommand)]
        action: Option<inbox::Action>,
        /// The inbox note, over the config file's.
        #[arg(long)]
        inbox: Option<String>,
        /// One array of `{path, line, heading, text, lines, suggestions: [{path, by}]}`.
        #[arg(long)]
        json: bool,
        /// Lists the items and asks nothing, on a terminal too.
        #[arg(long, conflicts_with = "interactive")]
        list: bool,
        /// Walks the items, reading the answers from stdin even when it is not a terminal.
        #[arg(short, long)]
        interactive: bool,
        /// Notes suggested for each item, at most.
        #[arg(short = 'n', long, default_value_t = 3)]
        limit: usize,
    },
    /// The path of a note picked at random, as Obsidian's "Open random note".
    Random {
        /// Only a note in this folder of the vault.
        #[arg(long)]
        folder: Option<String>,
        /// Only a note with this tag, or one under it.
        #[arg(long)]
        tag: Option<String>,
    },
    /// Commits every change, pulls with a rebase and pushes, as obsidian-git
    /// and the Mac app do. Uses your git config, credentials and SSH agent.
    /// A pull that stops on a conflict keeps its rebase open and lists the
    /// files; resolve them in the app, or `--abort`.
    Sync {
        /// The commit message; else `update notes @ <time> on <host>`.
        #[arg(short, long)]
        message: Option<String>,
        /// Ends a sync stopped on a conflict: the branch is as it was before
        /// the pull, and the next sync pulls again.
        #[arg(long, conflicts_with = "message")]
        abort: bool,
    },
    /// The commits that changed a note, newest first, across renames:
    /// the hash, the date, the author and the subject.
    History {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The note's text at this commit instead.
        #[arg(long)]
        show: Option<String>,
        #[arg(short = 'n', long, default_value_t = 300)]
        limit: usize,
    },
    /// Adds a line to the capture note (`[capture]` in the config file:
    /// today's daily note at first), as the app's quick capture does.
    Capture {
        /// The line; with none, read from stdin.
        text: Vec<String>,
        /// As a task, `- [ ] text`.
        #[arg(short, long)]
        task: bool,
    },
    /// Adds a line under a heading of a note (at its end with none), as
    /// the MCP's `append_to_note` does: `daftar-cli append Plants.md 'Water the basil' --heading Basil --task`.
    Append {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The line; with none, read from stdin.
        text: Vec<String>,
        /// The heading to add it under, made at the end when the note has none.
        #[arg(long)]
        heading: Option<String>,
        /// As a task, `- [ ] text`.
        #[arg(short, long)]
        task: bool,
    },
    /// A new note, never over one that is there: the text given, else stdin
    /// when it is not a terminal. `.md` is added to a path without it. Prints its path.
    New {
        path: String,
        /// Its text; `-` reads stdin.
        text: Vec<String>,
    },
    /// Moves or renames a note or a file, and the links to it and its
    /// bookmarks follow, as the app's Move does: `daftar-cli mv Inbox/Idea.md Projects/Idea`.
    Mv {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The new path in the vault; a name with no folder stays in the vault's root.
        to: String,
        /// Says what would move and which notes' links would change, and changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Sets a property of a note's front matter, keeping every other line as
    /// it was written: one value is text, several a list, none an empty
    /// value. `daftar-cli set-property Plants.md tags garden herbs`.
    SetProperty {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        key: String,
        values: Vec<String>,
        /// A list even with one value.
        #[arg(long, conflicts_with = "remove")]
        list: bool,
        /// Takes the property away.
        #[arg(long)]
        remove: bool,
    },
    /// A note's path, change time, aliases, tags, properties, outline, tasks,
    /// the links it makes and the notes that link to it.
    Info {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
    },
    /// The vault at a glance, as an agent's first call (`vault_brief`) reads
    /// it: tasks overdue, due and in progress, the job search's booked events,
    /// the drill cards due, the audit and yesterday's commits.
    Brief {
        /// The audit with the vault's `[audit]` profile.
        #[arg(long)]
        audit: bool,
    },
    /// The script that completes this command in [shell] (bash, zsh, fish,
    /// elvish or powershell), notes by their path in the vault included:
    /// `daftar-cli completions zsh > ~/.zfunc/_daftar-cli`, or
    /// `source <(daftar-cli completions zsh)` in .zshrc.
    Completions { shell: String },
    /// The vault's git hooks: `install` writes `pre-commit`, which formats,
    /// lints and audits the staged notes (`check --strict` on them) and the
    /// whole vault for errors, as CI does, and `post-merge` and
    /// `post-rewrite`, which after a pull audit and lint the notes it
    /// brought and refresh search by meaning in the background.
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// The branch, how far it is from its remote, and how many files changed.
    Status,
    /// What stands in a sync's way: an operation left open, conflicts, no
    /// upstream, commits not pushed or pulled, large untracked files. Fails
    /// when anything would stop a sync. The large files committed already
    /// are listed too, largest first, as information.
    Doctor {
        /// The size in MB over which a file is named.
        #[arg(long, default_value_t = 1)]
        large: u64,
    },
    /// Renames a tag and the tags under it (`#old/sub`) in every note, in the
    /// text and in the `tags` property; code and links are left alone.
    RenameTag {
        old: String,
        new: String,
        /// Lists the notes that would change, and changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Finds and replaces in the text of every note, as Replace in Vault does:
    /// prints each match's line before and after, then writes them.
    Replace {
        find: String,
        replacement: String,
        /// A regular expression, whose replacement names its groups as `$1`.
        #[arg(short = 'e', long)]
        regex: bool,
        #[arg(short = 'c', long)]
        match_case: bool,
        #[arg(short = 'w', long)]
        whole_word: bool,
        /// Code blocks and inline code too, which are left alone otherwise.
        #[arg(long)]
        in_code: bool,
        /// Only the notes in this folder of the vault.
        #[arg(long)]
        folder: Option<String>,
        /// Prints the matches, and changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Done and cancelled tasks first in each section of a note, as the Task
    /// Sorter plugin sorts them.
    SortTasks {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Moves a task's due date (else its scheduled, else its start date) on,
    /// as the Tasks plugin's Postpone does: by day, 2days, 3days, week, 2weeks or month.
    Postpone {
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// The line, from 1, as `tasks` prints it.
        line: usize,
        #[arg(default_value = "day")]
        by: String,
    },
    /// The path of today's periodic note (the Daily notes plugin's folder
    /// and format: a day, a week or a month), made from its template when
    /// it is not there.
    Daily {
        /// Another day than today (YYYY-MM-DD).
        #[arg(long)]
        day: Option<String>,
        /// Prints the path and makes nothing.
        #[arg(long)]
        no_create: bool,
    },
    /// The holidays and feasts of `calendar.place` (spain, catalonia or
    /// barcelona), a line each: the day, `holiday` or `event`, its name, its
    /// name there, and its note when the vault has one.
    Events {
        /// The first day (YYYY-MM-DD); else today.
        #[arg(long)]
        from: Option<String>,
        /// How many days from it.
        #[arg(long, default_value_t = 30)]
        days: i64,
        /// A whole year, in place of --from and --days.
        #[arg(long, conflicts_with_all = ["from", "days"])]
        year: Option<i32>,
        /// The place, in place of calendar.place.
        #[arg(long)]
        place: Option<String>,
    },
    /// A note as a PDF on A4: export.logo at its head, and the logo of the
    /// folder it is in (a company's logo.svg or logo.png) on the other side;
    /// its pictures drawn, its wiki links as their text. Prints the PDF's path.
    Export {
        /// The note: its path in the vault, or its name.
        #[arg(add = ArgValueCandidates::new(note_candidates))]
        note: String,
        /// Where the PDF goes; else the note's name with .pdf, here.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// The logo at the head, in place of export.logo.
        #[arg(long)]
        logo: Option<PathBuf>,
        /// Leaves the folder's logo out.
        #[arg(long)]
        no_folder_logo: bool,
        /// Only the section of this heading, with the headings under it;
        /// again for more, set in the note's order.
        #[arg(long = "section", value_name = "HEADING")]
        sections: Vec<String>,
        /// A table of the headings, each a link to its page, before the note.
        #[arg(long)]
        contents: bool,
        /// The page on its side, for a screen or a projector.
        #[arg(long)]
        landscape: bool,
    },
    /// A web page as a new note: its main text as Markdown, its pictures in
    /// the attachment folder, and `source` and `clipped` in its front matter.
    /// Prints the note's path.
    Clip {
        url: String,
        /// The folder of the vault the note goes in.
        #[arg(long, default_value = "Clippings")]
        folder: String,
        /// The note's name; else the page's title.
        #[arg(long)]
        title: Option<String>,
    },
    /// Lints the notes against the vault's .markdownlint.jsonc, as
    /// markdownlint-cli2 does, with Obsidian's syntax known; the whole vault,
    /// or the files and folders given (from the current folder, else from
    /// the vault's). Fails when a problem is left.
    Lint {
        paths: Vec<PathBuf>,
        /// Prints each problem as a JSON line, and nothing else on stdout:
        /// `{"tool", "level", "path", "line", "column", "code", "message",
        /// "fixable"}`.
        #[arg(long)]
        json: bool,
        /// Mends first what can be mended: spaces, blank lines, list marks,
        /// heading marks, fences, emphasis marks, bare URLs, the last newline.
        #[arg(long)]
        fix: bool,
        /// A folder or file of the vault to leave out (as .markdownlintignore).
        #[arg(long)]
        ignore: Vec<String>,
        /// Lints, then lints again the notes that change, under a line with
        /// the time and the count, until Ctrl+C.
        #[arg(long, conflicts_with_all = ["json", "fix"])]
        watch: bool,
    },
    /// Formats the notes as Prettier does with the vault's .prettierrc (its
    /// Markdown, and YAML front matter): the whole vault, or the files and
    /// folders given (from the current folder, else from the vault's), but
    /// what .prettierignore names. Lists what it changed.
    Fmt {
        #[arg(conflicts_with = "stdin")]
        paths: Vec<PathBuf>,
        /// Writes nothing: lists the notes that are not formatted, and fails
        /// when there is one.
        #[arg(long)]
        check: bool,
        /// With --check, shows what formatting would change in each note.
        #[arg(long, requires = "check")]
        diff: bool,
        /// With --check, prints each note that is not formatted as a JSON
        /// line, at its first line formatting changes, and nothing else on
        /// stdout (as `lint --json`, with the code `fmt`).
        #[arg(long, requires = "check", conflicts_with = "diff")]
        json: bool,
        /// Reads one note from stdin and prints it formatted, as fmt would
        /// write it; writes nothing.
        #[arg(long, conflicts_with = "check")]
        stdin: bool,
        /// With --stdin, the note's path in the vault: a file
        /// .prettierignore names is printed unchanged.
        #[arg(long, requires = "stdin")]
        path: Option<String>,
        /// A folder or file of the vault to leave out (as .prettierignore).
        #[arg(long)]
        ignore: Vec<String>,
    },
    /// Checks what lint cannot see: embeds of files that are not there,
    /// links to no note or to no heading or block, links a code span stops,
    /// callouts that fall apart, task dates that are not days, front matter
    /// never closed, block ids written twice, pictures no note shows, and
    /// web addresses written wrong, with --urls web addresses that are
    /// gone, and with --figures SVG text that overlaps, leaves the canvas
    /// or spills out of its box. Fails on an error; on a warning too with
    /// --strict. Given notes (from the current folder, else from the
    /// vault's), reports on and mends those only.
    Check {
        paths: Vec<PathBuf>,
        /// Prints each finding as a JSON line, and nothing else on stdout:
        /// `{"tool", "level", "path", "line", "column", "code", "message",
        /// "fixable"}`; what --fix mends goes to stderr.
        #[arg(long)]
        json: bool,
        /// Fails on warnings too.
        #[arg(long)]
        strict: bool,
        /// A folder whose notes must not have an H1.
        #[arg(long = "no-h1")]
        no_h1: Vec<String>,
        /// A callout type beyond Obsidian's and Daftar's `figure` and `read`.
        #[arg(long)]
        callout: Vec<String>,
        /// Asks every web address the notes link to whether it is still
        /// there: one that answers 404 or 410, or does not answer, is a
        /// warning. Addresses on the local network are left out. Off unless
        /// asked: how an address is written is checked every time.
        #[arg(long, env = "DAFTAR_CHECK_URLS")]
        urls: bool,
        /// With --urls, lists the addresses nothing answered at too: a name
        /// no server has, a refused connection, a bad certificate. A host
        /// on a VPN is one of them.
        #[arg(long, requires = "urls")]
        unreachable: bool,
        /// A host (with its subdomains) or the start of an address --urls
        /// does not ask.
        #[arg(long = "skip-url")]
        skip_url: Vec<String>,
        /// With --urls, asks the Wayback Machine for a copy of each dead
        /// address and names it; --fix puts the copy in its place.
        #[arg(long, requires = "urls")]
        archive: bool,
        /// Lays out the text of every SVG a note shows or links to, with the
        /// system's fonts and the built-in ones, and warns of text over
        /// other text (svg-text-overlap), past the canvas
        /// (svg-text-outside) or out of the box it starts in
        /// (svg-text-spill). Off unless asked.
        #[arg(long)]
        figures: bool,
        /// Mends what has one answer: a bare link whose name more than one
        /// note has gets the shortest path to the note it goes to now, and
        /// shows the text it showed. Prints each change.
        #[arg(long)]
        fix: bool,
        /// With --fix, prints the changes and writes nothing.
        #[arg(long, requires = "fix")]
        dry_run: bool,
        /// Warns of what makes a note stale as an agent's memory: a
        /// `**Where it stands (YYYY-MM-DD):**` paragraph older than the
        /// note's newest `**YYYY-MM-DD` history entry
        /// (memory-stale-status), "today", "next week" and the like in such
        /// a note (memory-relative-date), an --event-property that has
        /// passed (memory-past-event), and a dated call in a --prep-note
        /// with no `> [!done]` callout (memory-prep-held).
        #[arg(long)]
        memory: bool,
        /// With --memory, a property that holds a booked event's date or
        /// time, as `job_next`.
        #[arg(long = "event-property", requires = "memory")]
        event_property: Vec<String>,
        /// With --memory, the name of a preparation note, as `Interview
        /// Preparation`.
        #[arg(long = "prep-note", requires = "memory")]
        prep_note: Vec<String>,
        /// With --memory, a `key=value` (as `job_stage=applied`) that marks
        /// a note as still going, for --quiet-days.
        #[arg(long, requires = "memory")]
        active: Vec<String>,
        /// With --memory and --active, warns of a going note with nothing in
        /// it dated within this many days (memory-quiet).
        #[arg(long = "quiet-days", requires = "active")]
        quiet_days: Option<i64>,
        /// Warns of notes nothing links to that link to no note (orphan).
        /// `orphans = true` in `.daftar.toml` does the same.
        #[arg(long)]
        orphans: bool,
        /// With --orphans, a folder whose notes are left out (the week
        /// notes, the daily notes); `orphan_skip` in `.daftar.toml` adds to it.
        #[arg(long = "orphan-skip", requires = "orphans")]
        orphan_skip: Vec<String>,
        /// The Open Knowledge Format's warnings on the notes with a `type`
        /// (concepts): a stale one (okf-stale), a time that is none
        /// (okf-time), a status that is none of draft, stable and
        /// deprecated (okf-status), an event with no actor (okf-event), and
        /// a link to a deprecated concept that names its replacement
        /// (okf-superseded, which --fix rewrites). `okf = true` in
        /// `.daftar.toml` does the same.
        #[arg(long)]
        okf: bool,
        /// A section of the vault's `.daftar.toml` to add to its `[check]`,
        /// as `audit`. The file's options are read first and the flags add
        /// to them.
        #[arg(long)]
        profile: Option<String>,
        /// Audits, then audits the vault again whenever a note changes,
        /// under a line with the time and the counts, until Ctrl+C.
        #[arg(long, conflicts_with_all = ["json", "fix"])]
        watch: bool,
    },
    /// Unlinked mentions: the places where a note's title or alias is
    /// written in another note without a link, `path:line`, the link and
    /// the line around it. A name that is short, has no letter or names
    /// more than one note is left out, as are the hidden files, the week
    /// notes and the notes that link to the note already.
    Mentions {
        /// Only the mentions of this note.
        #[arg(long, add = ArgValueCandidates::new(note_candidates))]
        note: Option<String>,
        /// Only the mentions in these notes (from the current folder, else
        /// from the vault's).
        #[arg(long = "in")]
        within: Vec<PathBuf>,
        /// The fewest characters a title or alias has to be looked for.
        #[arg(long, default_value_t = mentions::Options::MIN_LENGTH)]
        min_length: usize,
        /// A name more notes than this write is a common word and is left
        /// out; 0 is no limit.
        #[arg(long, default_value_t = mentions::Options::MAX_NOTES)]
        max_notes: usize,
        /// A one-word name with a capital counts written in small letters
        /// too ("code" for Code), as Obsidian lists it.
        #[arg(long)]
        any_case: bool,
        /// Every mention, not only the first of each note in each other note.
        #[arg(long)]
        every: bool,
        /// Only the mentions of notes no note links to that link to no note.
        #[arg(long)]
        orphans: bool,
        /// One JSON array of `{"path", "line", "column", "target", "link",
        /// "text", "excerpt"}`, line and column from 1.
        #[arg(long)]
        json: bool,
        /// Makes the first mention of each note in each other note a link,
        /// showing the text as written; a note that changed since it was
        /// read is left as it is. Needs --note, --in or --all.
        #[arg(long)]
        fix: bool,
        /// With --fix, every note of the vault.
        #[arg(long, requires = "fix")]
        all: bool,
        /// With --fix, prints the links and writes nothing.
        #[arg(long, requires = "fix")]
        dry_run: bool,
    },
    /// The week before a day as a note: tasks done and cancelled, overdue,
    /// due in the week ahead and in progress, the meetings of the week
    /// notes, the notes git saw change, and the tasks to mend. Prints it,
    /// or with --write makes it in `Reviews/` (never over a note).
    Review {
        /// The day it is written (YYYY-MM-DD); it covers the seven days before. Today at first.
        #[arg(long)]
        day: Option<String>,
        /// Makes the note and prints its path.
        #[arg(long)]
        write: bool,
        /// The folder of the vault the note goes in.
        #[arg(long, default_value = "Reviews")]
        folder: String,
    },
    /// The job search: every note with a `job_stage` property is an
    /// application. Prints each stage with its count, then the booked events
    /// ahead (`job_next`): `path:line`, the time, the company, its stage,
    /// `job_next_step` and its `Interview Preparation.md`.
    Jobs {
        /// Each channel (`job_channel`, the text before ` (`) instead: its
        /// applications, how many got past applied, and the rate. A closed
        /// one (rejected, withdrawn) got past applied when its folder has a
        /// preparation note.
        #[arg(long)]
        channels: bool,
        /// Only the events within this many days.
        #[arg(long)]
        upcoming: Option<i64>,
    },
    /// The day at a glance: its timeline (the week notes' meetings, the
    /// tasks whose text starts with a time, the free time and now), the
    /// tasks overdue, due and in progress, and the job search's events of
    /// the day with their prep notes.
    Agenda {
        /// The day (YYYY-MM-DD); today at first.
        #[arg(long)]
        day: Option<String>,
        /// One JSON object: `day`, `now`, `timeline`, `free`, `overdue`,
        /// `today`, `in_progress` and `jobs`.
        #[arg(long)]
        json: bool,
        /// One line for a Waybar custom module (`"return-type": "json"`):
        /// the next meeting and how long until it, else how many tasks are
        /// due; the class `soon`, `overdue` or `clear`; the rest in the tooltip.
        #[arg(long, conflicts_with = "json")]
        waybar: bool,
        /// Sends a desktop notification for each meeting of the week notes
        /// and each booked job event that starts within --before minutes,
        /// once (what was sent is kept in Daftar's state folder), with its
        /// prep note; for a timer that runs it every minute.
        #[arg(long, conflicts_with_all = ["json", "waybar", "day"])]
        notify: bool,
        /// With --notify or --install-timer, the minutes of notice; else
        /// `notifications.meeting_lead` in the config file, else 5.
        #[arg(long)]
        before: Option<i64>,
        /// Writes the systemd user units daftar-agenda-notify.service and
        /// .timer, which run `agenda --notify` every minute (Linux), never
        /// over units of your own without --force.
        #[arg(long, conflicts_with_all = ["json", "waybar", "day", "notify"])]
        install_timer: bool,
        #[arg(long, requires = "install_timer")]
        force: bool,
    },
    /// An iCalendar file of the open tasks with a 📅 or ⏳ day ahead (all
    /// day, or at the time their text starts with) and the job search's
    /// booked events, for a calendar app to import. The week notes'
    /// meetings are left out: they came from a calendar.
    Ics {
        /// The days from today it covers.
        #[arg(long, default_value_t = 60)]
        days: i64,
        /// Writes the file here instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// The vault as an MCP server on stdin and stdout, for Claude Code and
    /// other agents: search, notes, links, tags, tasks and their queries,
    /// git history, the audit, and the app's safe writes (capture, add to a
    /// note, a new note, tick, set a status, postpone). `claude mcp add
    /// --scope user daftar -- daftar-cli mcp --vault ~/org`.
    Mcp {
        /// Leaves the tools that write out.
        #[arg(long)]
        read_only: bool,
    },
    /// The vault as a bundle of the Open Knowledge Format (OKF 0.3):
    /// `export` writes one from the vault, and `check` says whether a
    /// folder is one.
    Okf {
        #[command(subcommand)]
        action: OkfAction,
    },
    /// The vaults this computer knows (`vaults` in the config file), a line
    /// each: the name, the folder, and `in use`, `missing` or nothing.
    /// `--vault` takes the name of one when exactly one has it. The app
    /// changes the vault in use; this command never does.
    Vaults {
        /// One JSON array of `{"name", "path", "in_use", "missing"}`.
        #[arg(long)]
        json: bool,
    },
    /// Every setting with its value and where it came from, a line each
    /// (`key`, a tab, the value as the file writes it, a tab, `vault`,
    /// `vault (linux)` for the vault's table of this platform, `computer`
    /// or `default`): the vault's `.daftar.toml` may set the
    /// app's settings for every computer that opens it, and
    /// `vault_settings` in the config file declines them. A key the vault
    /// may not set is listed last as `refused`, with the value it wrote.
    Config {
        /// One JSON array of `{"key", "value", "from"}`.
        #[arg(long)]
        json: bool,
    },
    /// The vault as a language server on stdin and stdout, for Neovim, Helix
    /// and other editors: the lint and the audit as diagnostics, formatting,
    /// `[[` completion, a link's note, hover and references, and renaming a
    /// note with every link to it. In Neovim: `vim.lsp.start({ name =
    /// 'daftar', cmd = { 'daftar-cli', 'lsp', '--vault', vim.fn.expand('~/org') } })`.
    Lsp,
    /// Removes pictures no note names, makes the others JPEGs under 1 MB and
    /// renames them in the notes, as notes-cli does.
    Images {
        /// The folder of pictures, in the vault.
        #[arg(default_value = "uploads")]
        folder: PathBuf,
        /// Shows what would change, and changes nothing.
        #[arg(long)]
        dry_run: bool,
        /// Keeps the pictures that were converted.
        #[arg(long)]
        keep_originals: bool,
        /// Moves what it would remove into the vault's `.trash/`, as Obsidian's
        /// own deletes do, instead of removing it.
        #[arg(long)]
        trash: bool,
    },
}

#[derive(Subcommand)]
enum OkfAction {
    /// Writes an OKF 0.3 bundle from the vault into a folder outside it:
    /// each note in scope at its own path with a `type` (its own, else
    /// `[okf.types]` by its folder, else `Note`), its body as CommonMark
    /// with `/` links, the pictures it shows, an `index.md` in every folder
    /// and a `log.md` from git. `[okf]` in the vault's `.daftar.toml` says
    /// what goes in (`include`, `exclude`, `title`, `description`,
    /// `license`). Hidden notes, the week notes, the scratchpad and the
    /// templates never do. The folder must be empty or an earlier export,
    /// whose files this run does not write are removed. Nothing is written
    /// in the vault.
    Export {
        /// The folder the bundle is written in.
        #[arg(short, long)]
        output: PathBuf,
        /// A folder to export, with all under it, beside `include` of `[okf]`.
        #[arg(long)]
        include: Vec<String>,
        /// A folder or a note never exported, beside `exclude` of `[okf]`.
        #[arg(long)]
        exclude: Vec<String>,
        /// How many days back `log.md` goes.
        #[arg(long, default_value_t = 90)]
        log_days: i64,
        /// Lists what it would write and remove, and writes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Whether a folder is a conformant OKF bundle: every front matter is
    /// a YAML mapping, and each `index.md` and `log.md` has its shape.
    /// Then, as warnings, what the audit says of its concepts (okf-stale,
    /// okf-time, okf-status, okf-event).
    Check { folder: PathBuf },
}

#[derive(Subcommand)]
enum HookAction {
    /// Writes the hooks where git looks for them (`core.hooksPath`, else
    /// `.git/hooks`), each never over a hook of your own without --force:
    /// that one is skipped and named, and the others are written.
    Install {
        #[arg(long)]
        force: bool,
        /// Only this hook; given again, these hooks.
        #[arg(long, value_parser = hook::HOOKS)]
        only: Vec<String>,
    },
    /// Removes the hooks `install` wrote, and leaves any of your own.
    Uninstall,
    /// What the pre-commit hook runs: the staged notes formatted and staged
    /// again, linted and audited strictly, and the vault audited for errors.
    PreCommit,
    /// What the post-merge hook runs after a merge or a fast-forward: the
    /// notes between ORIG_HEAD and HEAD audited and linted, their findings
    /// printed, and search by meaning refreshed in the background when its
    /// model is downloaded. It never fails.
    PostMerge {
        /// 1 after `git merge --squash`, which brought nothing yet.
        squash: Option<String>,
    },
    /// What the post-rewrite hook runs: after `rebase` (a `git pull
    /// --rebase`), what post-merge does; after `amend`, nothing.
    PostRewrite { kind: String },
    /// The background refresh of search by meaning a hook after a pull
    /// starts: one at a time, again when a pull asked meanwhile.
    #[command(hide = true)]
    Refresh,
}

/// The exit status of a command that searches by meaning when the model
/// is not downloaded, so that a program running it can offer the download.
const NO_MODEL: u8 = 3;

fn main() -> ExitCode {
    // A shell asking for completions (`COMPLETE=zsh daftar-cli -- …`) is answered here.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
    let cli = Cli::parse();
    if cli.command.as_ref().is_some_and(needs_model) && !daftar_core::semantic_model::is_downloaded() {
        eprintln!("daftar-cli: {}", daftar_core::semantic_model::missing());
        return ExitCode::from(NO_MODEL);
    }
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("daftar-cli: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Whether [command] cannot run without the model of search by meaning.
fn needs_model(command: &Command) -> bool {
    match command {
        Command::Search { meaning, meaning_only, .. } => *meaning || *meaning_only,
        Command::Related { .. } | Command::Similar { .. } => true,
        Command::SemanticIndex { download_model } => !download_model,
        _ => false,
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let Some(command) = cli.command else {
        if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
            return tui::run(vault(cli.vault)?, cli.hidden);
        }
        return Cli::command().print_help().map_err(|e| e.to_string());
    };
    if let Command::Completions { shell } = &command {
        let shells = clap_complete::env::Shells::builtins();
        let completer = shells
            .completer(shell)
            .ok_or_else(|| format!("no completions for {shell}: {}", shells.names().collect::<Vec<_>>().join(", ")))?;
        let name = Cli::command().get_name().to_string();
        return completer
            .write_registration("COMPLETE", &name, &name, &name, &mut std::io::stdout())
            .or_else(|e| if e.kind() == std::io::ErrorKind::BrokenPipe { Ok(()) } else { Err(e.to_string()) });
    }
    // The list of vaults is the computer's: it needs no vault to be there.
    if let Command::Vaults { json } = &command {
        for line in vault_lines(&daftar_core::Vaults::read(), *json) {
            out!("{line}");
        }
        return Ok(());
    }
    let root = vault(cli.vault)?;
    // A read-only clone pulls and never writes (docs/specs/read-only-vault.md).
    if writes(&command) && Git::read_only(&root) {
        return Err(VaultEdits::READ_ONLY.into());
    }
    // What the lists leave out, as in the app: links to them still resolve.
    let hidden = ExcludedFiles::from_config(&root, &config(&root), cli.hidden);
    match command {
        Command::Tui => tui::run(root, cli.hidden)?,
        Command::Find { query, limit } => {
            let index = Vault::index(&root, None);
            for path in find(&index, &hidden, &query.join(" "), limit) {
                out!("{path}");
            }
        }
        Command::Search { query, limit, meaning, meaning_only, json } if meaning || meaning_only => {
            let search = SearchIndex::new();
            let index = Vault::index(&root, Some(&search));
            let mut semantic = meaning::Meaning::open(&root)?;
            semantic.update(&root, &index, true)?;
            let terminal = std::io::stdout().is_terminal();
            let query = query.join(" ");
            if json {
                let rows: Vec<serde_json::Value> = if meaning_only {
                    semantic.search(&hidden, &query, limit).iter().map(meaning::json).collect()
                } else {
                    semantic
                        .fused(&search, &hidden, &query, limit)
                        .into_iter()
                        .map(|f| {
                            let line = f.line.map_or_else(|| words_line(&root, &f.hit, &query), |line| line + 1);
                            hit_json(&f.hit.path, line, Some(f.score), &f.hit.snippet)
                        })
                        .collect()
                };
                out!("{}", serde_json::Value::Array(rows));
                return Ok(());
            }
            let hits = if meaning_only {
                semantic
                    .search(&hidden, &query, limit)
                    .into_iter()
                    .map(|n| daftar_core::Hit { path: n.path.clone(), snippet: n.shown() })
                    .collect()
            } else {
                semantic.hybrid(&search, &hidden, &query, limit)
            };
            for hit in hits {
                out!("{}\t{}", hit.path, snippet(&hit.snippet, terminal));
            }
        }
        Command::Related { note, limit, json } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let mut semantic = meaning::Meaning::open(&root)?;
            semantic.update(&root, &index, true)?;
            if json {
                let rows: Vec<serde_json::Value> = semantic.related(&index, &hidden, &path, limit).iter().map(meaning::json).collect();
                out!("{}", serde_json::Value::Array(rows));
                return Ok(());
            }
            for near in semantic.related(&index, &hidden, &path, limit) {
                out!("{}", meaning::line(&near));
            }
        }
        Command::Similar { threshold, limit, note, folder, json } => {
            let index = Vault::index(&root, None);
            let note = note.map(|note| note_path(&index, &note)).transpose()?;
            let mut semantic = meaning::Meaning::open(&root)?;
            semantic.update(&root, &index, true)?;
            let options = similar::Options { threshold, limit, note: note.as_deref(), folder: folder.as_deref() };
            let found = similar::pairs(&semantic, &index, &root, &hidden, &options);
            if json {
                out!("{}", serde_json::Value::Array(found.iter().map(similar::json).collect()));
                return Ok(());
            }
            for pair in &found {
                out!("{}", similar::line(pair));
            }
        }
        Command::SemanticIndex { download_model } => {
            let index = Vault::index(&root, None);
            meaning::build(&root, &index, download_model)?;
        }
        Command::Search { query, limit, json, .. } => {
            let search = SearchIndex::new();
            Vault::index(&root, Some(&search));
            let terminal = std::io::stdout().is_terminal();
            let query = query.join(" ");
            // More asked for, so that [limit] are left once the hidden ones are out.
            let hits = search.search(&query, limit.saturating_add(100));
            let hits = hits.into_iter().filter(|hit| !hidden.contains(&hit.path)).take(limit);
            if json {
                let rows: Vec<serde_json::Value> =
                    hits.map(|hit| hit_json(&hit.path, words_line(&root, &hit, &query), None, &hit.snippet)).collect();
                out!("{}", serde_json::Value::Array(rows));
                return Ok(());
            }
            for hit in hits {
                out!("{}\t{}", hit.path, snippet(&hit.snippet, terminal));
            }
        }
        Command::Tasks { due, path, all, json } => {
            let index = Vault::index(&root, None);
            let by = match due.as_deref() {
                None => None,
                Some("today") => Some(Day::today().to_string()),
                Some(text) => Some(Day::parse(text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?.to_string()),
            };
            let mut tasks: Vec<_> = hidden
                .tasks(&index)
                .into_iter()
                .filter(|t| all || !TaskLine::is_closed(t.task.status))
                .filter(|t| path.as_deref().is_none_or(|p| t.path.starts_with(p)))
                .filter(|t| by.as_deref().is_none_or(|by| t.task.due.as_deref().or(t.task.scheduled.as_deref()).is_some_and(|d| d <= by)))
                .collect();
            // Due first, the earliest on top; then by note and line.
            tasks.sort_by(|a, b| {
                let day = |t: &VaultTask| t.task.due.clone().or_else(|| t.task.scheduled.clone()).unwrap_or_else(|| "9999".into());
                day(a).cmp(&day(b)).then(a.path.cmp(&b.path)).then(a.task.line.cmp(&b.task.line))
            });
            if json {
                let rows: Vec<serde_json::Value> = tasks.iter().map(|task| task_json(task, None)).collect();
                out!("{}", serde_json::Value::Array(rows));
                return Ok(());
            }
            for task in tasks {
                out!("{}", task_line(&task));
            }
        }
        Command::Query { lines, note, json, strict } => {
            let index = Vault::index(&root, None);
            let file = note.as_deref().map(|n| note_path(&index, n)).transpose()?;
            let sources = if !lines.is_empty() {
                vec![lines.join("\n")]
            } else if let Some(file) = &file {
                let text = fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
                let blocks = query_blocks(&text);
                if blocks.is_empty() {
                    return Err(format!("{file} has no tasks query"));
                }
                blocks
            } else {
                vec![std::io::read_to_string(std::io::stdin()).map_err(|e| e.to_string())?]
            };
            let mut tasks = hidden.tasks(&index);
            tasks.sort_by(|a, b| (&a.path, a.task.line).cmp(&(&b.path, b.task.line)));
            let mut rows = Vec::new();
            for source in sources {
                let query = TasksQuery::parse(&source, file.as_deref());
                for line in &query.not_understood {
                    eprintln!("daftar-cli: not understood, so not applied: {line}");
                }
                if strict && !query.not_understood.is_empty() {
                    return Err(format!("{} lines of the query not understood", query.not_understood.len()));
                }
                if json {
                    for task in query.run(tasks.clone(), Day::today(), |t| QueryTask::from(t)) {
                        rows.push(task_json(&task, Some(query.groups(&QueryTask::from(&task)))));
                    }
                    continue;
                }
                // A group's name on its own line, `#` once per level, when it changes.
                let mut open: Vec<String> = Vec::new();
                for task in query.run(tasks.clone(), Day::today(), |t| QueryTask::from(t)) {
                    let groups = query.groups(&QueryTask::from(&task));
                    for (level, name) in groups.iter().enumerate() {
                        if open.get(level) == Some(name) {
                            continue;
                        }
                        open.truncate(level);
                        open.push(name.clone());
                        if !name.is_empty() {
                            out!("{} {name}", "#".repeat(level + 1));
                        }
                    }
                    out!("{}", task_line(&task));
                }
            }
            if json {
                out!("{}", serde_json::Value::Array(rows));
            }
        }
        Command::Repeat { rule, from, count } => {
            let recurrence = daftar_core::Recurrence::parse(&rule).ok_or_else(|| {
                format!("“{rule}” is not repeated by Daftar: a tick completes the task with no next one (rules such as every 2 weeks or every jalali month are)")
            })?;
            let mut day = match from.as_deref() {
                None => Day::today(),
                Some(text) => Day::parse(text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?,
            };
            for _ in 0..count {
                // Each step from the day the last reached, as ticking it each time would.
                let Some(next) = recurrence.next(day, day) else {
                    out!("(no next day: the rule runs off the calendar)");
                    break;
                };
                let jalali = daftar_core::jalali::Jalali::from_day(next);
                out!("{next}\t{}-{:02}-{:02}", jalali.year, jalali.month, jalali.day);
                day = next;
            }
        }
        Command::Toggle { note, line, expect } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let at = line.checked_sub(1).ok_or("lines count from 1")?;
            let text = fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            let raw = text.split('\n').nth(at).ok_or_else(|| format!("{path} has no line {line}"))?;
            let task = TaskLine::parse(raw.trim_end_matches('\r'), at, "").ok_or_else(|| format!("{path}:{line} is not a task"))?;
            if let Some(expected) = &expect
                && !TaskLine::holds(raw, expected)
            {
                return Err(format!("{path}:{line} is now “{}”, not “{expected}”", raw.trim()));
            }
            VaultEdits::apply(&path, &root, |text| VaultEdits::toggle_task(text, at, task.status, Some(&task.text), Day::today()))
                .map_err(|e| e.to_string())?;
            let after = fs::read_to_string(root.join(&path)).unwrap_or_default();
            out!("{}", after.split('\n').nth(at).unwrap_or_default());
        }
        Command::SortHeadings { note, heading } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let text = fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            let headings = Sections::headings(&text);
            let (at, line) = match heading.parse::<usize>() {
                Ok(line) => (None, line.checked_sub(1).ok_or("lines count from 1")?),
                Err(_) => {
                    (Some(headings.iter().position(|h| h.text == heading).ok_or_else(|| format!("{path} has no heading “{heading}”"))?), 0)
                }
            };
            let sorted = Sections::sort_headings(&text, at, line)?;
            VaultEdits::apply(&path, &root, |_| Ok(sorted)).map_err(|e| e.to_string())?;
        }
        Command::Backlinks { note } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            for link in index.backlinks(&path).iter().filter(|link| !hidden.contains(&link.source)) {
                out!("{}:{}", link.source, link.line + 1);
            }
        }
        Command::Broken => {
            let index = Vault::index(&root, None);
            let mut broken = BTreeSet::new();
            for note in index.notes.values().filter(|note| !hidden.contains(&note.path)) {
                for link in &note.parsed.links {
                    if !link.target.is_empty() && !link.target.contains("://") && index.resolve(&link.target, &note.path).is_none() {
                        broken.insert((note.path.clone(), link.line + 1, link.target.clone()));
                    }
                }
            }
            for (path, line, target) in broken {
                out!("{path}:{line}\t{target}");
            }
        }
        Command::Tags => {
            let index = Vault::index(&root, None);
            let counts = hidden.tags(&index);
            let mut tags: Vec<_> = counts.iter().collect();
            tags.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (tag, count) in tags {
                out!("{count}\t#{tag}");
            }
        }
        Command::Drill { targets, new, limit, list } => {
            let index = Vault::index(&root, None);
            let deck = Drill::deck(&root, &drill::notes(&index, &targets, |p| hidden.contains(p))?);
            let file = DrillState::file(&root);
            let mut state = DrillState::load(&file);
            let today = Day::today();
            let due = Drill::due(&deck, &state, today, new, limit);
            let asking = !list && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
            if !asking {
                if !list {
                    eprintln!("daftar-cli: not a terminal, so the cards due are listed; drill in a terminal to be asked");
                }
                for card in &due {
                    out!("{}", drill::line(card, &state));
                }
                return Ok(());
            }
            if due.is_empty() {
                eprintln!("daftar-cli: nothing due of {} cards", deck.len());
                return Ok(());
            }
            let outcome = drill::session(&due, &mut state, &file, today, &mut std::io::stdin().lock(), &mut std::io::stdout())?;
            eprintln!("\ndaftar-cli: {} rated, {} forgotten, {} left", outcome.rated, outcome.again, outcome.left);
        }
        Command::Inbox { action, inbox: named, json, list, interactive, limit } => {
            let setting = named.or_else(|| config(&root).string("scratchpad.note"));
            inbox::run(&root, &hidden, setting.as_deref(), action, &inbox::Options { json, list, interactive, limit })?;
        }
        Command::Random { folder, tag } => {
            let index = Vault::index(&root, None);
            let candidates = RandomNote::candidates(&index, folder.as_deref(), tag.as_deref(), |p| hidden.contains(p));
            out!("{}", RandomNote::pick(&candidates, RandomNote::number).ok_or("no note to pick")?);
        }
        Command::Sync { message: _, abort: true } => {
            let git = repository(&root)?;
            if !git.rebase_in_progress().map_err(|e| e.to_string())? {
                return Err("no sync is stopped".into());
            }
            git.abort_rebase().map_err(|e| e.to_string())?;
            out!("aborted: your commits are as they were before the pull");
        }
        Command::Sync { message, abort: false } => {
            let git = repository(&root)?;
            let message = message.unwrap_or_else(|| Git::message(chrono::Local::now(), &Git::host()));
            let done = match git.sync(&message) {
                Ok(done) => done,
                Err(SyncFailure::Stopped(stopped)) => {
                    for file in &stopped.conflicts {
                        out!("{}\t{} on the remote, {} here", file.path, side(file.remote), side(file.mine));
                    }
                    return Err(format!("{stopped} Resolve it in Daftar, or run `daftar-cli sync --abort`."));
                }
                Err(failure @ SyncFailure::PushRefusedReadOnly(_)) => {
                    return Err(format!("{failure} To make this clone read-only: `git config --local daftar.readonly true`."));
                }
                Err(failure) => return Err(failure.to_string()),
            };
            let status = git.status().map_err(|e| e.to_string())?;
            let mut steps = Vec::new();
            // A read-only clone only pulls, and says so.
            if git.is_read_only() {
                steps.push("read-only".to_string());
            }
            if done.committed {
                steps.push(format!("committed \"{message}\""));
            }
            if done.pulled {
                steps.push("pulled".to_string());
            }
            if done.pushed {
                steps.push("pushed".to_string());
            }
            if !done.committed && !done.pulled && !done.pushed {
                steps.push("up to date".to_string());
            }
            out!("{}: {}", status.branch, steps.join(", "));
        }
        Command::History { note, show, limit } => {
            let git = repository(&root)?;
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let history = git.history(&path, limit).map_err(|e| e.to_string())?;
            match show {
                Some(hash) => {
                    // A commit before a rename knows the note by its old path.
                    let revision =
                        history.iter().find(|r| r.hash.starts_with(&hash)).ok_or_else(|| format!("no commit {hash} changed {path}"))?;
                    let text = git.content(&revision.path, &revision.hash).map_err(|e| e.to_string())?;
                    print!("{text}");
                }
                None => {
                    for revision in history {
                        let moved = if revision.path == path { String::new() } else { format!("\t{}", revision.path) };
                        out!(
                            "{}\t{}\t{}\t{}{moved}",
                            &revision.hash[..12.min(revision.hash.len())],
                            revision.date.format("%Y-%m-%d %H:%M"),
                            revision.author,
                            revision.subject
                        );
                    }
                }
            }
        }
        Command::Capture { text, task } => {
            let typed = if text.is_empty() {
                let mut input = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).map_err(|e| e.to_string())?;
                input
            } else {
                text.join(" ")
            };
            let line = QuickCapture::line(&typed, task).ok_or("nothing to capture")?;
            let path = capture(&root, &config(&root), &line)?;
            out!("{path}\t{line}");
        }
        Command::Append { note, text, heading, task } => {
            let index = Vault::index(&root, None);
            let line = QuickCapture::line(&words_or_stdin(&text)?, task).ok_or("nothing to add")?;
            let target = note_path(&index, &note).unwrap_or(note);
            let path = QuickCapture::capture(&root, &target, heading.as_deref().unwrap_or_default(), &line, Day::today(), Clock::now())?;
            out!("{path}\t{line}");
        }
        Command::New { path, text } => {
            let text = if text.is_empty() && std::io::stdin().is_terminal() { String::new() } else { words_or_stdin(&text)? };
            out!("{}", mcp::create_note(&root, &path, &text)?);
        }
        Command::Mv { note, to, dry_run } => {
            let index = Vault::index(&root, None);
            for line in mcp::move_note(&root, &index, &note, &to, dry_run)? {
                out!("{line}");
            }
        }
        Command::SetProperty { note, key, values, list, remove } => {
            let index = Vault::index(&root, None);
            out!("{}", mcp::property(&root, &index, &note, &key, property_value(values, list, remove))?);
        }
        Command::Info { note } => {
            let index = Vault::index(&root, None);
            for line in mcp::note_info(&index, &hidden, &note)? {
                out!("{line}");
            }
        }
        Command::Brief { audit } => {
            let index = Vault::index(&root, None);
            TaskStatuses::set_current(TaskStatuses::load(&root));
            for line in mcp::vault_brief(&root, &DrillState::file(&root), &index, &hidden, audit)? {
                out!("{line}");
            }
        }
        Command::Completions { .. } => unreachable!("answered before the vault is read"),
        Command::Hook { action: HookAction::Install { force, only } } => {
            let done = hook::install(&repository(&root)?, &root, force, &only)?;
            for file in &done.written {
                out!("{}", file.display());
            }
            if let Some(refusal) = done.refusal() {
                return Err(refusal);
            }
        }
        Command::Hook { action: HookAction::Uninstall } => {
            let (removed, left) = hook::uninstall(&repository(&root)?, &root)?;
            for file in &removed {
                out!("{}", file.display());
            }
            for file in &left {
                eprintln!("daftar-cli: {} is a hook of your own; left as it is", file.display());
            }
        }
        Command::Hook { action: HookAction::PreCommit } => hook::pre_commit(&root, &repository(&root)?)?,
        Command::Hook { action: HookAction::PostMerge { squash } } => {
            hook::post_merge(&root, &repository(&root)?, squash.as_deref() == Some("1"));
        }
        Command::Hook { action: HookAction::PostRewrite { kind } } => hook::post_rewrite(&root, &repository(&root)?, &kind),
        Command::Hook { action: HookAction::Refresh } => hook::refresh(&root)?,
        Command::Status => {
            let status = repository(&root)?.status().map_err(|e| e.to_string())?;
            out!("{}\tahead {}\tbehind {}\tchanged {}", status.branch, status.ahead, status.behind, status.changed);
            for line in read_only_lines(&root) {
                out!("{line}");
            }
        }
        Command::Doctor { large } => {
            let found = repository(&root)?.doctor(large * 1_048_576).map_err(|e| e.to_string())?;
            for diagnosis in &found {
                out!("{}\t{}\t{}", diagnosis.level, diagnosis.code, diagnosis.message);
            }
            if found.is_empty() {
                out!("info\tclean\tnothing in a sync's way");
            }
            if found.iter().any(|d| d.level == "error") {
                return Err("the vault's repository needs a hand before it syncs".into());
            }
        }
        Command::RenameTag { old, new, dry_run } => {
            let index = Vault::index(&root, None);
            for path in vault_commands::rename_tag(&root, &index, &old, &new, dry_run)? {
                out!("{path}");
            }
        }
        Command::Replace { find, replacement, regex, match_case, whole_word, in_code, folder, dry_run } => {
            let index = Vault::index(&root, None);
            let options = ReplaceOptions { match_case, whole_word, regular_expression: regex, skip_code: !in_code };
            for line in vault_commands::replace(&root, &index, &find, &replacement, options, folder.as_deref(), dry_run)? {
                out!("{line}");
            }
        }
        Command::SortTasks { note, dry_run } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let moved = vault_commands::sort_tasks(&root, &path, dry_run)?;
            out!("{path}\t{}", if moved { "sorted" } else { "in order already" });
        }
        Command::SetStatus { note, line, status, reason } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let at = line.checked_sub(1).ok_or("lines count from 1")?;
            out!("{}", vault_commands::set_status(&root, &path, at, &status, reason.as_deref(), Day::today())?);
        }
        Command::Cancel { note, line, reason } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let at = line.checked_sub(1).ok_or("lines count from 1")?;
            // The vault's own cancelled status, `-` unless its settings say otherwise.
            let statuses = TaskStatuses::current();
            let symbol = statuses.all.iter().find(|s| s.kind == TaskType::Cancelled).map_or('-', |s| s.symbol);
            out!("{}", vault_commands::set_status(&root, &path, at, &symbol.to_string(), reason.as_deref(), Day::today())?);
        }
        Command::Statuses => {
            for status in &TaskStatuses::load(&root).all {
                out!("[{}]\t{}\t{}\tnext [{}]", status.symbol, status.name, status.kind.type_name(), status.next);
            }
        }
        Command::Postpone { note, line, by } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let at = line.checked_sub(1).ok_or("lines count from 1")?;
            out!("{}", vault_commands::postpone(&root, &path, at, &by, Day::today())?);
        }
        Command::Events { from, days, year, place } => {
            let named = place.or_else(|| config(&root).string("calendar.place")).unwrap_or_default();
            let place = Place::parse(&named).ok_or_else(|| match named.trim() {
                "" => "no place: set calendar.place in the config file, or pass --place (spain, catalonia or barcelona)".to_string(),
                _ => format!("{named} is not a place it knows: spain, catalonia or barcelona"),
            })?;
            let (first, last) = if let Some(year) = year {
                (Day::new(year, 1, 1).ok_or("no such year")?, Day::new(year, 12, 31).ok_or("no such year")?)
            } else {
                let first = match from {
                    Some(text) => Day::parse(&text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?,
                    None => Day::today(),
                };
                (first, first.adding_days(days.max(0)))
            };
            let index = Vault::index(&root, None);
            for happening in CalendarEvents::between(place, first, last) {
                let event = happening.event;
                let note = index.resolve(event.title, "").filter(|p| Vault::is_note(p)).unwrap_or_default();
                let kind = if event.holiday { "holiday" } else { "event" };
                out!("{}\t{kind}\t{}\t{}\t{note}", happening.day, event.title, event.local);
            }
        }
        Command::Daily { day, no_create } => {
            let index = Vault::index(&root, None);
            let day = match day {
                Some(text) => Day::parse(&text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?,
                None => Day::today(),
            };
            out!("{}", vault_commands::daily(&root, &index.files, day, !no_create, Clock::now())?);
        }
        Command::Lint { paths, ignore, watch: true, .. } => {
            let mut watched = lint::Watched::new(&root, &paths, &ignore, report::Mode::Plain)?;
            watching::run(&root, |changed| {
                if !changed.is_empty()
                    && let Err(error) = watched.update(changed)
                {
                    return watching::Pass { summary: error, lines: Vec::new() };
                }
                watching::Pass { summary: watched.summary(), lines: watched.lines() }
            })?;
        }
        Command::Lint { paths, json, fix, ignore, .. } => {
            let outcome = lint::run(&root, &paths, &ignore, fix, report::Mode::of(json), |line| out!("{line}"))?;
            if fix && outcome.fixed > 0 {
                eprintln!("daftar-cli: fixed {} of {} notes", outcome.fixed, outcome.checked);
            }
            if outcome.problems > 0 {
                return Err(format!("{} problems in {} of {} notes", outcome.problems, outcome.notes, outcome.checked));
            }
        }
        Command::Fmt { stdin: true, path, ignore, .. } => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| format!("stdin: {e}"))?;
            let formatted = fmt::formatted(&root, &text, path.as_deref(), &ignore)?;
            std::io::Write::write_all(&mut std::io::stdout().lock(), formatted.as_bytes()).map_err(|e| e.to_string())?;
        }
        Command::Fmt { paths, check, diff, json, ignore, .. } => {
            let options = fmt::Options { check, diff, mode: report::Mode::of(json) };
            let outcome = fmt::run(&root, &paths, &ignore, &options, |line| out!("{line}"))?;
            if check && outcome.changed > 0 {
                return Err(format!("{} of {} notes are not formatted; run daftar-cli fmt", outcome.changed, outcome.checked));
            }
        }
        Command::Check {
            paths,
            json,
            strict,
            no_h1,
            callout,
            urls,
            unreachable,
            skip_url,
            archive,
            figures,
            fix,
            dry_run,
            memory,
            event_property,
            prep_note,
            active,
            quiet_days,
            orphans,
            orphan_skip,
            okf,
            profile,
            watch,
        } => {
            let vault = check::VaultOptions::read(&root, profile.as_deref())?;
            let okf = okf || vault.okf;
            let memory = memory || vault.memory;
            let event_property = check::joined(vault.event_properties, event_property);
            let prep_note = check::joined(vault.prep_notes, prep_note);
            let active = check::joined(vault.active, active);
            let quiet_days = quiet_days.or(vault.quiet_days);
            let no_h1 = check::joined(vault.no_h1, no_h1);
            let callout = check::joined(vault.callouts, callout);
            let orphans = (orphans || vault.orphans).then(|| check::joined(vault.orphan_skip, orphan_skip));
            let github_token = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")).ok();
            // Given paths with no note of the vault in them (a file outside
            // it, a hidden folder, a picture) checked nothing: an error, not
            // a clean run, which a script would take for no problems.
            let only = if paths.is_empty() { None } else { Some(check::given_notes(&root, &paths)?) };
            let options = check::Options {
                strict,
                no_h1,
                callouts: callout,
                urls,
                unreachable,
                github_token,
                skip_urls: skip_url,
                archive,
                figures,
                mode: report::Mode::of(json),
                only,
                fix,
                dry_run,
                memory: memory.then_some(memory::Rules {
                    event_properties: event_property,
                    prep_notes: prep_note,
                    active: active.iter().filter_map(|a| memory::Rules::active_pair(a)).collect(),
                    quiet_days,
                }),
                orphans,
                okf,
            };
            if watch {
                return watching::run(&root, |_| check::pass(&root, |path| hidden.contains(path), &options));
            }
            let outcome = check::run(&root, |path| hidden.contains(path), &options, |line| out!("{line}"));
            if fix {
                let verb = if dry_run { "would fix" } else { "fixed" };
                eprintln!("daftar-cli: {verb} {} links", outcome.fixed);
            }
            eprintln!("daftar-cli: {} errors, {} warnings in {} notes", outcome.errors, outcome.warnings, outcome.notes);
            if outcome.errors > 0 || (options.strict && outcome.warnings > 0) {
                return Err("check failed".to_string());
            }
        }
        Command::Mentions { note, within, min_length, max_notes, any_case, every, orphans, json, fix, all, dry_run } => {
            if fix && note.is_none() && within.is_empty() && !all {
                return Err("--fix links in every note only with --all; give --note or --in to link fewer".to_string());
            }
            let index = Vault::index(&root, None);
            let note = note.map(|n| note_path(&index, &n)).transpose()?;
            let only = if within.is_empty() { None } else { Some(check::given_notes(&root, &within)?) };
            let options = mentions::Options { note, only, min_length, max_notes, orphans, any_case, every };
            let mut found = mentions::find(&root, &index, |path| hidden.contains(path), &options);
            found.sort_by(|a, b| (&a.path, a.mention.line, a.mention.column).cmp(&(&b.path, b.mention.line, b.mention.column)));
            if fix {
                let (made, refused) = mentions::fix(&root, &found, dry_run, &mut |line| out!("{line}"));
                let verb = if dry_run { "would link" } else { "linked" };
                eprintln!("daftar-cli: {verb} {made} mentions in {} notes", found.iter().map(|f| &f.path).collect::<BTreeSet<_>>().len());
                if !refused.is_empty() {
                    return Err(format!("{} notes changed since they were read; run it again", refused.len()));
                }
            } else if json {
                out!("{}", serde_json::Value::Array(found.iter().map(mentions::Found::json).collect()));
            } else {
                for item in &found {
                    out!("{}", item.line());
                }
            }
        }
        Command::Export { note, output, logo, no_folder_logo, sections, contents, landscape } => {
            let index = Vault::index(&root, None);
            let path = note_path(&index, &note)?;
            let mut look = export::Look::from_config(&root, &config(&root));
            if logo.is_some() {
                look.logo = logo;
            }
            look.folder_logo = !no_folder_logo;
            look.contents = contents;
            look.landscape = landscape;
            let output = output.unwrap_or_else(|| PathBuf::from(format!("{}.pdf", Vault::title(&path))));
            export::run(&root, &index, &path, &output, &look, &sections)?;
            out!("{}", output.display());
        }
        Command::Clip { url, folder, title } => {
            out!("{}", clip::run(&root, &url, &folder, title.as_deref())?);
        }
        Command::Review { day, write, folder } => {
            let day = match day {
                Some(text) => Day::parse(&text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?,
                None => Day::today(),
            };
            let index = Vault::index(&root, None);
            let week = review::Week::before(day);
            let text = review::markdown(&root, &index, |path| hidden.contains(path), week, day);
            if write {
                let path = week.path(&folder);
                review::write(&root, &path, &text)?;
                // As `fmt` would write it, so the vault's checks pass.
                fmt::run(&root, &[root.join(&path)], &[], &fmt::Options { check: false, diff: false, mode: report::Mode::Plain }, |_| {})?;
                out!("{path}");
            } else {
                print!("{text}");
            }
        }
        Command::Jobs { channels, upcoming } => {
            let index = Vault::index(&root, None);
            let keys = daftar_core::JobKeys::default();
            let search = daftar_core::JobSearch::new(&index, &keys, |path| !hidden.contains(path));
            let lines = if channels {
                jobs::channels(&search)
            } else {
                let mut lines = jobs::stages(&search);
                lines.extend(jobs::upcoming(&root, &search, &keys, chrono::Local::now().naive_local(), upcoming));
                lines
            };
            for line in lines {
                out!("{line}");
            }
        }
        Command::Agenda { install_timer: true, before, force, .. } => {
            if !cfg!(target_os = "linux") {
                return Err("--install-timer writes systemd units: on this system, run `daftar-cli agenda --notify` every minute from its own scheduler".into());
            }
            for file in alerts::install_timer(&alerts::units_folder(), &alerts::command(&root, before), force)? {
                out!("{}", file.display());
            }
            out!("Now run: systemctl --user enable --now {}.timer", alerts::UNIT);
        }
        Command::Agenda { notify: true, before, .. } => {
            let lead = before.or_else(|| config(&root).int("notifications.meeting_lead")).unwrap_or(alerts::LEAD).max(0);
            let index = Vault::index(&root, None);
            let search = daftar_core::JobSearch::new(&index, &daftar_core::JobKeys::default(), |path| !hidden.contains(path));
            alerts::run(&root, &search, lead, &chrono::Local, chrono::Utc::now())?;
        }
        Command::Agenda { day, json, waybar, .. } => {
            let (zone, now) = (chrono::Local, chrono::Utc::now());
            let day = match day {
                Some(text) => Day::parse(&text).ok_or_else(|| format!("{text} is not a day (YYYY-MM-DD)"))?,
                None => Day::today(),
            };
            let index = Vault::index(&root, None);
            let keys = daftar_core::JobKeys::default();
            let search = daftar_core::JobSearch::new(&index, &keys, |path| !hidden.contains(path));
            let agenda = agenda::Agenda::new(&root, &hidden.tasks(&index), &search, &keys, day, &zone, now);
            match (json, waybar) {
                (true, _) => out!("{}", agenda.json()),
                (_, true) => out!("{}", agenda.waybar()),
                _ => agenda.lines().iter().for_each(|line| out!("{line}")),
            }
        }
        Command::Ics { days, output } => {
            let index = Vault::index(&root, None);
            let search = daftar_core::JobSearch::new(&index, &daftar_core::JobKeys::default(), |path| !hidden.contains(path));
            let events = ics::events(&hidden.tasks(&index), &search, Day::today(), days);
            ics::put(&ics::calendar(&events, chrono::Utc::now()), output.as_deref())?;
        }
        Command::Mcp { read_only } => mcp::run(root, cli.hidden, read_only)?,
        Command::Okf { action: OkfAction::Export { output, include, exclude, log_days, dry_run } } => {
            let mut scope = okf::Scope::read(&root);
            scope.include = check::joined(scope.include, include);
            scope.exclude = check::joined(scope.exclude, exclude);
            let index = Vault::index(&root, None);
            // The hidden notes are never exported, whatever `--hidden` lists.
            let hidden = ExcludedFiles::from_config(&root, &config(&root), false);
            let done = okf::export(&root, &index, &hidden, &scope, &okf::Options { output: output.clone(), log_days, dry_run })?;
            for path in &done.written {
                out!("{}\t{path}", if dry_run { "would write" } else { "wrote" });
            }
            for path in &done.removed {
                out!("{}\t{path}", if dry_run { "would remove" } else { "removed" });
            }
            eprintln!(
                "daftar-cli: {} written, {} removed, {} unchanged in {}",
                done.written.len(),
                done.removed.len(),
                done.unchanged,
                output.display()
            );
        }
        Command::Okf { action: OkfAction::Check { folder } } => {
            let found = okf::check(&folder)?;
            for line in found.errors.iter().chain(&found.warnings) {
                out!("{line}");
            }
            eprintln!("daftar-cli: {} errors, {} warnings in {} files", found.errors.len(), found.warnings.len(), found.files);
            if !found.errors.is_empty() {
                return Err(format!("{} is not a conformant OKF {} bundle", folder.display(), daftar_core::Okf::VERSION));
            }
        }
        Command::Vaults { .. } => unreachable!("answered before the vault is read"),
        Command::Config { json } => {
            let entries = daftar_core::Settings::read(&root).entries();
            if json {
                let rows: Vec<serde_json::Value> = entries.iter().map(setting_json).collect();
                out!("{}", serde_json::Value::Array(rows));
            } else {
                for entry in &entries {
                    out!("{}\t{}\t{}", entry.key, entry.value.as_ref().map_or_default(daftar_core::Value::toml), entry.from.name());
                }
            }
        }
        Command::Lsp => lsp::run(&root, cli.hidden)?,
        Command::Images { folder, dry_run, keep_originals, trash } => {
            let folder = if folder.is_absolute() { folder } else { root.join(folder) };
            if !folder.is_dir() {
                return Err(format!("{} is not a folder", folder.display()));
            }
            images::run(&root, &folder, &images::Options { dry_run, keep_originals, trash }).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn emit(line: std::fmt::Arguments) {
    use std::io::Write;
    if let Err(error) = writeln!(std::io::stdout().lock(), "{line}") {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("daftar-cli: {error}");
        }
        std::process::exit(i32::from(error.kind() != std::io::ErrorKind::BrokenPipe));
    }
}

/// A hit of a search, as `search --json` and `related --json` print it:
/// `score` rounded to four places, the snippet on one line with no marks.
fn hit_json(path: &str, line: usize, score: Option<f32>, snippet: &str) -> serde_json::Value {
    let score = score.map(|s| (f64::from(s) * 10_000.0).round() / 10_000.0);
    serde_json::json!({ "path": path, "line": line, "score": score, "snippet": crate::snippet(snippet, false) })
}

/// The line (from 1) of [hit]'s note that its words are on.
fn words_line(root: &std::path::Path, hit: &daftar_core::Hit, query: &str) -> usize {
    let text = fs::read_to_string(root.join(&hit.path)).unwrap_or_default();
    SearchIndex::line_of(&text, &hit.snippet, query)
}

/// A search snippet on one line, its matches in bold on a terminal; the
/// index marks them with `\u{1}` and `\u{2}`.
fn snippet(text: &str, terminal: bool) -> String {
    let (open, close) = if terminal { ("\x1b[1m", "\x1b[0m") } else { ("", "") };
    text.replace('\n', " ").replace('\u{1}', open).replace('\u{2}', close)
}

fn side(side: Side) -> &'static str {
    match side {
        Side::Changed => "changed",
        Side::Added => "added",
        Side::Deleted => "deleted",
    }
}

/// The notes of the vault a completion is for (`$DAFTAR_VAULT`, else the
/// one the shell is in, else the app's), by their path, without the hidden
/// ones; none when there is no vault.
fn note_candidates() -> Vec<CompletionCandidate> {
    let Ok(root) = vault(std::env::var_os("DAFTAR_VAULT").map(PathBuf::from)) else { return Vec::new() };
    let hidden = ExcludedFiles::from_config(&root, &config(&root), false);
    Vault::scan(&root).into_iter().filter(|path| Vault::is_note(path) && !hidden.contains(path)).map(CompletionCandidate::new).collect()
}

/// What `set-property` sets: one value is text, several (or `--list`) a
/// list, none an empty value; `--remove` is none at all.
fn property_value(values: Vec<String>, list: bool, remove: bool) -> Option<PropertyValue> {
    match (remove, list, values.as_slice()) {
        (true, ..) => None,
        (false, false, []) => Some(PropertyValue::Empty),
        (false, false, [one]) => Some(PropertyValue::Text(one.clone())),
        (false, ..) => Some(PropertyValue::List(values)),
    }
}

/// The words given, joined by spaces; none, or `-` alone, reads stdin.
fn words_or_stdin(words: &[String]) -> Result<String, String> {
    if !words.is_empty() && words != ["-"] {
        return Ok(words.join(" "));
    }
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).map_err(|e| e.to_string())?;
    Ok(input)
}

/// Adds [line] to the capture note of [config], as the app's quick capture does.
fn capture(root: &std::path::Path, config: &ConfigFile, line: &str) -> Result<String, String> {
    let target = config.string("capture.target").unwrap_or_else(|| "daily".into());
    let heading = config.string("capture.heading").unwrap_or_default();
    QuickCapture::capture(root, &target, &heading, line, Day::today(), Clock::now())
}

/// The files named like [query], as the quick switcher ranks them, without the hidden ones.
fn find(index: &VaultIndex, hidden: &ExcludedFiles, query: &str, limit: usize) -> Vec<String> {
    let candidates: Vec<Candidate> = index
        .files
        .iter()
        .filter(|path| !hidden.contains(path))
        .map(|path| {
            let aliases = index.notes.get(path).map_or_default(|n| n.parsed.aliases.clone());
            Candidate::new(path, &Vault::title(path), &aliases)
        })
        .collect();
    QuickSwitcher::rank(query, &candidates, &[], limit)
}

/// The settings in use for the vault at [root]: the settings file,
/// `~/.config/daftar/config.toml`, with what the vault's `.daftar.toml` may
/// set on top (`docs/specs/vault-config.md`).
fn config(root: &std::path::Path) -> ConfigFile {
    daftar_core::Settings::read(root).file().clone()
}

/// Whether [command] writes a file of the vault, which a read-only clone
/// refuses; with `--dry-run` it writes nothing, and runs.
fn writes(command: &Command) -> bool {
    match command {
        Command::Append { .. }
        | Command::New { .. }
        | Command::SetProperty { .. }
        | Command::Toggle { .. }
        | Command::SetStatus { .. }
        | Command::Cancel { .. }
        | Command::Postpone { .. }
        | Command::Clip { .. }
        | Command::SortHeadings { .. }
        | Command::Capture { .. }
        | Command::Inbox { action: Some(inbox::Action::Move { .. }), .. } => true,
        Command::Mv { dry_run, .. }
        | Command::RenameTag { dry_run, .. }
        | Command::Replace { dry_run, .. }
        | Command::SortTasks { dry_run, .. }
        | Command::Images { dry_run, .. } => !dry_run,
        Command::Fmt { check, stdin, .. } => !check && !stdin,
        Command::Lint { fix, .. } => *fix,
        Command::Check { fix, dry_run, .. } | Command::Mentions { fix, dry_run, .. } => *fix && !dry_run,
        Command::Review { write, .. } => *write,
        Command::Daily { no_create, .. } => !no_create,
        _ => false,
    }
}

/// What `status` says of a read-only clone, after its own line: that it
/// is one, and that no agent is served there when the vault says so.
fn read_only_lines(root: &std::path::Path) -> Vec<&'static str> {
    if !Git::read_only(root) {
        return Vec::new();
    }
    let mut lines = vec!["read-only"];
    if !daftar_core::VaultSettings::read(root).agents_in_read_only_clones {
        lines.push("agents: refused in this read-only clone");
    }
    lines
}

/// A setting as `config --json` prints it: `{"key", "value", "from"}`, the
/// value as JSON has it, or null for one the file does not hold.
fn setting_json(entry: &daftar_core::SettingEntry) -> serde_json::Value {
    let value = match &entry.value {
        Some(daftar_core::Value::String(text)) => serde_json::json!(text),
        Some(daftar_core::Value::Int(number)) => serde_json::json!(number),
        Some(daftar_core::Value::Bool(flag)) => serde_json::json!(flag),
        Some(daftar_core::Value::Strings(list)) => serde_json::json!(list),
        None => serde_json::Value::Null,
    };
    serde_json::json!({ "key": entry.key, "value": value, "from": entry.from.name() })
}

fn repository(root: &std::path::Path) -> Result<Git, String> {
    let git = Git::new(root);
    if git.is_repository() { Ok(git) } else { Err(format!("{} is not a git repository", root.display())) }
}

/// The vault given, else the one the current folder is in (the nearest
/// folder up with `.obsidian`, as git finds its repository), else the app's.
fn vault(given: Option<PathBuf>) -> Result<PathBuf, String> {
    let root = match given {
        // A folder, as before, or the name of a vault of the list.
        Some(root) if root.is_dir() => root,
        Some(root) => named_vault(root, &daftar_core::Vaults::read())?,
        None => {
            if let Some(root) = std::env::current_dir().ok().as_deref().and_then(vault_around) {
                root
            } else {
                let text = fs::read_to_string(ConfigFile::default_path()).unwrap_or_default();
                ConfigFile::new(text)
                    .vault()
                    .ok_or("no vault: run it in one, pass --vault, or set `vault` in ~/.config/daftar/config.toml")?
            }
        }
    };
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    Ok(root)
}

/// What `--vault` names: a folder, or the name of a vault of the list
/// ([known], `vaults` in the config file) when exactly one has it.
fn named_vault(given: PathBuf, known: &daftar_core::Vaults) -> Result<PathBuf, String> {
    if given.is_dir() {
        return Ok(given);
    }
    let name = given.to_string_lossy();
    if name.contains(['/', '\\']) || name.starts_with('~') {
        return Ok(given);
    }
    match known.named(&name) {
        Ok(entry) => Ok(entry.path),
        Err(daftar_core::VaultsFailure::Unknown(_)) => Ok(given),
        Err(refused) => Err(refused.to_string()),
    }
}

/// The vaults this computer knows, as `vaults` prints them: the name, the
/// folder, and `in use`, `missing` or nothing.
fn vault_lines(known: &daftar_core::Vaults, json: bool) -> Vec<String> {
    let entries = known.entries();
    if json {
        let rows: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| serde_json::json!({ "name": e.name, "path": e.path.to_string_lossy(), "in_use": e.in_use, "missing": e.is_missing() }))
            .collect();
        return vec![serde_json::Value::Array(rows).to_string()];
    }
    entries
        .iter()
        .map(|e| {
            let state = if e.is_missing() {
                "missing"
            } else if e.in_use {
                "in use"
            } else {
                ""
            };
            format!("{}\t{}\t{state}", e.name, e.path.display())
        })
        .collect()
}

/// The vault [folder] is in: itself or the nearest folder up with `.obsidian`.
fn vault_around(folder: &std::path::Path) -> Option<PathBuf> {
    folder.ancestors().find(|f| f.join(".obsidian").is_dir()).map(PathBuf::from)
}

/// A note named as a path in the vault, or as a link would name it.
/// `path:line`, a tab, the status and the text, and the dates that matter.
fn task_line(task: &VaultTask) -> String {
    let t = &task.task;
    let mut line = format!("{}:{}\t[{}] {}", task.path, t.line + 1, t.status, t.text);
    for (mark, value) in [("⏳", &t.scheduled), ("📅", &t.due), ("✅", &t.done), ("❌", &t.cancelled), ("🔁", &t.recurring)] {
        if let Some(value) = value {
            line.push_str(&format!(" {mark} {value}"));
        }
    }
    if let Some(reason) = &t.reason {
        line.push_str(&format!(" 💬 {reason}"));
    }
    line
}

/// [task] for a script: `line` counts from 1, as `toggle` takes it, and
/// `text` is what `toggle --expect` and the MCP tools check the line against.
fn task_json(task: &VaultTask, groups: Option<Vec<String>>) -> serde_json::Value {
    let t = &task.task;
    let mut row = serde_json::json!({
        "path": task.path, "line": t.line + 1, "status": t.status.to_string(),
        "closed": TaskLine::is_closed(t.status), "text": t.text, "section": t.section,
        "created": t.created, "start": t.start, "scheduled": t.scheduled, "due": t.due,
        "done": t.done, "cancelled": t.cancelled, "recurring": t.recurring,
        "priority": t.priority, "reason": t.reason,
    });
    if let Some(groups) = groups {
        row["groups"] = groups.into();
    }
    row
}

/// The bodies of a note's ` ```tasks ` blocks, in a callout or a quote too.
fn query_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        let mut line = line.trim_start();
        while let Some(rest) = line.strip_prefix('>') {
            line = rest.strip_prefix(' ').unwrap_or(rest).trim_start_matches(' ');
        }
        let trimmed = line.trim();
        match &mut current {
            None if trimmed == "```tasks" => current = Some(Vec::new()),
            Some(body) if trimmed.starts_with("```") => {
                blocks.push(body.join("\n"));
                current = None;
            }
            Some(body) => body.push(line),
            None => {}
        }
    }
    blocks
}

fn note_path(index: &VaultIndex, note: &str) -> Result<String, String> {
    if index.notes.contains_key(note) {
        return Ok(note.to_string());
    }
    let target = note.strip_suffix(".md").unwrap_or(note);
    index.resolve(target, "").filter(|p| Vault::is_note(p)).ok_or_else(|| format!("no note {note}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault that is a repository with one commit: a note of tasks, a
    /// note with a tag, a picture no note shows.
    fn committed_vault() -> (tempfile::TempDir, Git) {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("uploads")).unwrap();
        fs::write(root.join("Tasks.md"), "# Tasks\n\n## Now\n\n- [ ] One #old\n- [ ] Two\n\n## Later\n\n* a  list\n").unwrap();
        fs::write(root.join("Note.md"), "# Note\n\nSee Tasks and <https://example.com>.\n").unwrap();
        fs::write(root.join("Scratchpad.md"), "# Scratchpad\n\n- an item\n").unwrap();
        fs::write(root.join("uploads/unused.png"), "not a picture").unwrap();
        let git = Git::new(root);
        for arguments in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"],
            &["config", "user.name", "Test"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
            &["add", "-A"],
            &["commit", "-qm", "start"],
        ] {
            git.run(arguments).unwrap();
        }
        (base, git)
    }

    fn run_in(root: &std::path::Path, arguments: &[&str]) -> Result<(), String> {
        let vault = root.to_string_lossy();
        run(Cli::try_parse_from([&["daftar-cli", "--vault", &vault], arguments].concat()).map_err(|e| e.to_string())?)
    }

    #[test]
    fn a_read_only_vault_refuses_every_writing_command() {
        let (base, git) = committed_vault();
        let root = base.path();
        git.set_read_only(true).unwrap();
        let files = || -> Vec<(String, Vec<u8>)> {
            let mut files: Vec<(String, Vec<u8>)> = walkdir::WalkDir::new(root)
                .into_iter()
                .filter_entry(|entry| entry.file_name() != ".git")
                .flatten()
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| (entry.path().to_string_lossy().into_owned(), fs::read(entry.path()).unwrap()))
                .collect();
            files.sort();
            files
        };
        let before = files();
        let writing: [&[&str]; 23] = [
            &["append", "Note", "a line"],
            &["new", "Fresh", "text"],
            &["mv", "Note", "Moved"],
            &["set-property", "Note", "status", "done"],
            &["toggle", "Tasks", "5"],
            &["set-status", "Tasks", "5", "/"],
            &["cancel", "Tasks", "5"],
            &["postpone", "Tasks", "5"],
            &["replace", "Tasks", "Jobs"],
            &["rename-tag", "old", "new"],
            &["fmt"],
            &["lint", "--fix"],
            &["check", "--fix"],
            &["mentions", "--fix", "--all"],
            &["inbox", "move", "3", "Note"],
            &["clip", "https://example.com/"],
            &["images"],
            &["review", "--write"],
            &["sort-headings", "Tasks", "Tasks"],
            &["sort-tasks", "Tasks"],
            &["capture", "a thought"],
            &["daily"],
            &["daily", "--day", "2026-10-05"],
        ];
        for arguments in writing {
            assert_eq!(run_in(root, arguments), Err(VaultEdits::READ_ONLY.to_string()), "{arguments:?}");
            assert_eq!(files(), before, "{arguments:?} changed a file");
        }
        // With --dry-run, or when it only looks, a command runs.
        let looking: [&[&str]; 10] = [
            &["mv", "Note", "Moved", "--dry-run"],
            &["rename-tag", "old", "new", "--dry-run"],
            &["replace", "Tasks", "Jobs", "--dry-run"],
            &["sort-tasks", "Tasks", "--dry-run"],
            &["check", "--fix", "--dry-run"],
            &["mentions", "--fix", "--all", "--dry-run"],
            &["images", "--dry-run"],
            &["daily", "--no-create"],
            &["tasks"],
            &["config"],
        ];
        for arguments in looking {
            assert_eq!(run_in(root, arguments), Ok(()), "{arguments:?}");
        }
        // What only checks may find something, and still is not refused.
        for arguments in [&["fmt", "--check"][..], &["lint"]] {
            assert_ne!(run_in(root, arguments), Err(VaultEdits::READ_ONLY.to_string()), "{arguments:?}");
        }
        assert_eq!(files(), before);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), "");
        assert_eq!(read_only_lines(root), ["read-only"]);
        // A commit is refused by the hook, and a sync only pulls.
        assert!(hook::pre_commit(root, &git).unwrap_err().contains(VaultEdits::READ_ONLY));
        // Turned off, the same commands write.
        git.set_read_only(false).unwrap();
        assert!(read_only_lines(root).is_empty());
        assert_eq!(run_in(root, &["toggle", "Tasks", "5"]), Ok(()));
        assert!(fs::read_to_string(root.join("Tasks.md")).unwrap().contains("- [x] One #old ✅"));
    }

    #[test]
    fn two_vaults_of_one_name_show_their_parents() {
        let home = tempfile::tempdir().unwrap();
        for folder in ["org", "work/org", "notes"] {
            fs::create_dir_all(home.path().join(folder)).unwrap();
        }
        let config = "vault = \"~/org\"\nvaults = [\"~/org\", \"~/work/org\", \"~/notes\", \"~/away\"]\n";
        let known = daftar_core::Vaults::new(config, home.path());
        let at = |folder: &str| home.path().join(folder).display().to_string();
        assert_eq!(
            vault_lines(&known, false),
            [
                format!("org — ~\t{}\tin use", at("org")),
                format!("org — ~/work\t{}\t", at("work/org")),
                format!("notes\t{}\t", at("notes")),
                format!("away\t{}\tmissing", at("away")),
            ]
        );
        let json: serde_json::Value = serde_json::from_str(&vault_lines(&known, true)[0]).unwrap();
        assert_eq!(json[0], serde_json::json!({ "name": "org — ~", "path": at("org"), "in_use": true, "missing": false }));
        assert_eq!(json[3]["missing"], true);
        // `--vault org` is refused as ambiguous and names both; `--vault notes` opens the one.
        let refused = named_vault(PathBuf::from("org"), &known).unwrap_err();
        assert!(refused.contains("org — ~, org — ~/work"), "{refused}");
        assert_eq!(named_vault(PathBuf::from("notes"), &known), Ok(home.path().join("notes")));
        // A folder is a folder, as before, whatever the list says.
        assert_eq!(named_vault(home.path().join("work/org"), &known), Ok(home.path().join("work/org")));
        assert_eq!(named_vault(PathBuf::from("nowhere"), &known), Ok(PathBuf::from("nowhere")));
    }

    #[test]
    fn the_settings_are_listed_with_where_each_came_from() {
        let vault = "[appearance]\ntheme = \"dark\"\n\n[terminal]\nshell = \"/tmp/x\"\n";
        let entries = daftar_core::Settings::resolved(vault, "[appearance]\ntext_size = 16\n").entries();
        let row = |key: &str, from: &str| {
            let entry = entries.iter().find(|e| e.key == key && e.from.name() == from).unwrap_or_else(|| panic!("{key} from {from}"));
            setting_json(entry)
        };
        assert_eq!(row("appearance.theme", "vault"), serde_json::json!({ "key": "appearance.theme", "value": "dark", "from": "vault" }));
        assert_eq!(row("appearance.text_size", "computer")["value"], 16);
        assert_eq!(row("git.pull_on_launch", "default")["value"], true);
        assert_eq!(row("files.hidden", "default")["value"][0], "CLAUDE.md");
        assert_eq!(row("git.name", "default")["value"], serde_json::Value::Null);
        assert_eq!(row("terminal.shell", "refused")["value"], "/tmp/x");
        // The vault's table for this platform is said as such.
        assert_eq!(row("terminal.shell", "default")["value"], "");
        assert_eq!(entries.last().unwrap().from.name(), "refused");
        // The vault's table for this platform is said as such.
        let tabled =
            daftar_core::Settings::resolved("[appearance]\ntheme = \"dark\"\n\n[linux.appearance]\nfont = \"Inter\"\n", "").entries();
        let from = |key: &str| tabled.iter().find(|e| e.key == key).unwrap().from.name();
        assert_eq!((from("appearance.font"), from("appearance.theme")), ("vault (linux)", "vault"));
    }

    /// The Mac's `CaptureTests`, through the command line's capture.
    #[test]
    fn a_capture_goes_to_todays_note_made_from_its_template() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join("Templates")).unwrap();
        fs::write(root.join(".obsidian/daily-notes.json"), r#"{"format":"YYYY-MM-DD","folder":"Journal","template":"Templates/Day"}"#)
            .unwrap();
        fs::write(root.join("Templates/Day.md"), "# {{title}}\n## Inbox\n\n## Log\n").unwrap();
        fs::write(root.join("Inbox.md"), "# Inbox\n- [ ] Old\n").unwrap();
        let today = Day::today().to_string();

        let config = ConfigFile::new("[capture]\ntarget = \"daily\"\nheading = \"Inbox\"\n".to_string());
        let path = capture(root, &config, "- [ ] Call Sara").unwrap();
        assert_eq!(path, format!("Journal/{today}.md"));
        assert_eq!(fs::read_to_string(root.join(&path)).unwrap(), format!("# {today}\n## Inbox\n\n- [ ] Call Sara\n\n## Log\n"));
        // The file is read again: a change made on disk meanwhile stays.
        fs::write(root.join(&path), "# Today\n## Inbox\n- [ ] Call Sara\n- [ ] Written elsewhere\n").unwrap();
        let config = ConfigFile::new("[capture]\ntarget = \"daily\"\n".to_string());
        capture(root, &config, "Idea").unwrap();
        assert_eq!(fs::read_to_string(root.join(&path)).unwrap(), "# Today\n## Inbox\n- [ ] Call Sara\n- [ ] Written elsewhere\n\nIdea\n");

        // A named note, found without case; one not there yet is made.
        let config = ConfigFile::new("[capture]\ntarget = \"inbox\"\n".to_string());
        assert_eq!(capture(root, &config, "- [ ] New").unwrap(), "Inbox.md");
        assert_eq!(fs::read_to_string(root.join("Inbox.md")).unwrap(), "# Inbox\n- [ ] Old\n- [ ] New\n");
        let config = ConfigFile::new("[capture]\ntarget = \"Notes/Later.md\"\nheading = \"Ideas\"\n".to_string());
        assert_eq!(capture(root, &config, "x").unwrap(), "Notes/Later.md");
        assert_eq!(fs::read_to_string(root.join("Notes/Later.md")).unwrap(), "## Ideas\n\nx\n");
        // Never outside the vault.
        let config = ConfigFile::new("[capture]\ntarget = \"../escaped\"\n".to_string());
        assert!(capture(root, &config, "x").is_err());
        assert!(!root.parent().unwrap().join("escaped.md").exists());
    }

    #[test]
    fn a_notes_tasks_blocks_are_its_queries() {
        let text = "# Week\n\n```tasks\nnot done\ndue this week\n```\n\n```js\nx\n```\n```tasks\ndone\n```\n";
        assert_eq!(query_blocks(text), ["not done\ndue this week", "done"]);
        let callout = "> [!todo] Now\n>\n> ```tasks\n> not done\n> group by filename\n> ```\n";
        assert_eq!(query_blocks(callout), ["not done\ngroup by filename"]);
    }

    #[test]
    fn a_hit_as_json() {
        assert_eq!(
            hit_json("A.md", 12, Some(0.834_567), "a \u{1}plan\u{2}\nnext").to_string(),
            r#"{"path":"A.md","line":12,"score":0.8346,"snippet":"a plan next"}"#
        );
        assert_eq!(hit_json("A.md", 1, None, "x").to_string(), r#"{"path":"A.md","line":1,"score":null,"snippet":"x"}"#);
    }

    #[test]
    fn only_search_by_meaning_needs_the_model() {
        let search = |meaning, meaning_only| Command::Search { query: vec![], limit: 1, meaning, meaning_only, json: true };
        assert!(!needs_model(&search(false, false)));
        assert!(needs_model(&search(true, false)));
        assert!(needs_model(&search(false, true)));
        assert!(needs_model(&Command::Related { note: "A".into(), limit: 1, json: false }));
        assert!(needs_model(&Command::Similar { threshold: 0.9, limit: 1, note: None, folder: None, json: false }));
        assert!(needs_model(&Command::SemanticIndex { download_model: false }));
        assert!(!needs_model(&Command::SemanticIndex { download_model: true }));
    }

    #[test]
    fn the_vault_is_found_from_a_folder_in_it() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::create_dir_all(root.path().join("Notes/Deep")).unwrap();
        assert_eq!(vault_around(&root.path().join("Notes/Deep")).as_deref(), Some(root.path()));
        assert_eq!(vault_around(root.path()).as_deref(), Some(root.path()));
    }

    #[test]
    fn a_snippet_is_bold_only_on_a_terminal() {
        assert_eq!(snippet("a \u{1}plan\u{2}\nnext", false), "a plan next");
        assert_eq!(snippet("a \u{1}plan\u{2}", true), "a \x1b[1mplan\x1b[0m");
    }

    #[test]
    fn find_leaves_the_hidden_files_out_unless_asked() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::create_dir_all(root.path().join("Work")).unwrap();
        fs::write(root.path().join("Work/CLAUDE.md"), "rules").unwrap();
        fs::write(root.path().join("Work/Claude notes.md"), "mine").unwrap();
        let index = Vault::index(root.path(), None);
        let hidden = ExcludedFiles::from_config(root.path(), &ConfigFile::new(""), false);
        assert_eq!(find(&index, &hidden, "claude", 10), ["Work/Claude notes.md"]);
        let all = ExcludedFiles::from_config(root.path(), &ConfigFile::new(""), true);
        assert_eq!(find(&index, &all, "claude", 10).len(), 2);
        // A hidden note still opens by its name.
        assert_eq!(note_path(&index, "CLAUDE").unwrap(), "Work/CLAUDE.md");
    }

    #[test]
    fn a_note_is_found_by_path_or_by_name() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("Notes")).unwrap();
        fs::write(root.path().join("Notes/Inbox.md"), "- [ ] One\n").unwrap();
        let index = Vault::index(root.path(), None);
        assert_eq!(note_path(&index, "Notes/Inbox.md").unwrap(), "Notes/Inbox.md");
        assert_eq!(note_path(&index, "Inbox").unwrap(), "Notes/Inbox.md");
        assert!(note_path(&index, "Missing").is_err());
    }

    #[test]
    fn set_property_reads_its_values_as_the_form_does() {
        let words = |w: &[&str]| w.iter().map(|w| (*w).to_string()).collect::<Vec<_>>();
        assert_eq!(property_value(words(&["growing"]), false, false), Some(PropertyValue::Text("growing".into())));
        assert_eq!(property_value(words(&["north", "south"]), false, false), Some(PropertyValue::List(words(&["north", "south"]))));
        assert_eq!(property_value(words(&["north"]), true, false), Some(PropertyValue::List(words(&["north"]))));
        assert_eq!(property_value(Vec::new(), false, false), Some(PropertyValue::Empty));
        assert_eq!(property_value(words(&["x"]), false, true), None);
        let parsed = Cli::try_parse_from(["daftar-cli", "mv", "Inbox/Idea.md", "Projects/Idea", "--dry-run"]).unwrap();
        assert!(matches!(parsed.command, Some(Command::Mv { dry_run: true, .. })));
    }
}
