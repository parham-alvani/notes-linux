//! The statuses a task can have: the four every vault has, and the ones
//! the vault adds, such as `[r]` "In Review", each a `[tasks.statuses.<id>]`
//! section of its `.daftar.toml` (`docs/specs/no-obsidian-folder.md`), or,
//! in a vault with none, the Tasks plugin's settings
//! (`.obsidian/plugins/obsidian-tasks-plugin/data.json`). The rules are
//! Daftar's on macOS (`TaskStatuses.swift`) and Android (`TaskStatuses.kt`).

use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::Value;

/// What a status means, the plugin's `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskType {
    Todo,
    InProgress,
    Done,
    Cancelled,
    NonTask,
}

impl TaskType {
    fn parse(name: &str) -> Option<TaskType> {
        Some(match name {
            "TODO" => TaskType::Todo,
            "IN_PROGRESS" => TaskType::InProgress,
            "DONE" => TaskType::Done,
            "CANCELLED" => TaskType::Cancelled,
            "NON_TASK" => TaskType::NonTask,
            _ => return None,
        })
    }

    /// Done, cancelled, or not a task at all: nothing left to do.
    pub fn is_closed(self) -> bool {
        matches!(self, TaskType::Done | TaskType::Cancelled | TaskType::NonTask)
    }

    pub fn type_name(self) -> &'static str {
        match self {
            TaskType::Todo => "TODO",
            TaskType::InProgress => "IN_PROGRESS",
            TaskType::Done => "DONE",
            TaskType::Cancelled => "CANCELLED",
            TaskType::NonTask => "NON_TASK",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatus {
    pub symbol: char,
    pub name: String,
    pub next: char,
    pub kind: TaskType,
}

impl TaskStatus {
    fn new(symbol: char, name: &str, next: char, kind: TaskType) -> TaskStatus {
        TaskStatus { symbol, name: name.to_string(), next, kind }
    }

    /// One of ` `, `x`, `/` and `-`, which every app has its own look for.
    pub fn is_default(&self) -> bool {
        matches!(self.symbol, ' ' | 'x' | 'X' | '/' | '-')
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatuses {
    /// In the settings' order: the core statuses, then the custom ones.
    pub all: Vec<TaskStatus>,
}

static CURRENT: LazyLock<RwLock<Arc<TaskStatuses>>> = LazyLock::new(|| RwLock::new(Arc::new(TaskStatuses::default())));

impl Default for TaskStatuses {
    fn default() -> Self {
        TaskStatuses::from_json("")
    }
}

impl TaskStatuses {
    pub const PATH: &'static str = ".obsidian/plugins/obsidian-tasks-plugin/data.json";

    /// The section each status of the vault's own is under, with its id.
    pub const SECTION: &'static str = "tasks.statuses";

    /// The vault's statuses: its `[tasks.statuses.*]` sections when it has
    /// any, and then the plugin's file is not read; else the plugin's; the
    /// four defaults when neither is there or can be read.
    pub fn load(root: &Path) -> TaskStatuses {
        let own = crate::OwnSettings::read(root);
        Self::from_settings(&own)
            .unwrap_or_else(|| TaskStatuses::from_json(&std::fs::read_to_string(root.join(Self::PATH)).unwrap_or_default()))
    }

    /// Whether the vault file at [path] names the statuses: read them again
    /// when it changes.
    pub fn reads(path: &str) -> bool {
        path == Self::PATH || path == crate::VaultSettings::FILE
    }

    /// The statuses `[tasks.statuses.<id>]` sections of [own] give, in the
    /// file's order after the four every vault has; None when it has none.
    /// A section with one of the four's symbols changes its name or next
    /// symbol; one with no symbol, or a symbol of a section before it, is
    /// left out, and one with no `type` is to do.
    pub fn from_settings(own: &crate::OwnSettings) -> Option<TaskStatuses> {
        let prefix = format!("{}.", Self::SECTION);
        let keys = own.keys_under(&prefix);
        if keys.is_empty() {
            return None;
        }
        let mut ids: Vec<String> = Vec::new();
        for (key, _) in &keys {
            if let Some((id, _)) = status_key(&format!("{prefix}{key}"))
                && !ids.iter().any(|known| known == id)
            {
                ids.push(id.to_string());
            }
        }
        let mut statuses = TaskStatuses::from_json("");
        let mut own_symbols: Vec<char> = Vec::new();
        for id in ids {
            let get = |name: &str| own.string(&format!("{prefix}{id}.{name}"));
            let Some(symbol) = get("symbol").and_then(|s| s.chars().next()) else { continue };
            if own_symbols.contains(&symbol) {
                continue;
            }
            own_symbols.push(symbol);
            let name = get("name");
            let next = get("next").and_then(|s| s.chars().next());
            let kind = get("type").and_then(|t| TaskType::parse(&t));
            if let Some(known) = statuses.all.iter_mut().find(|s| s.symbol == symbol) {
                if let Some(name) = name {
                    known.name = name;
                }
                known.next = next.unwrap_or(known.next);
                known.kind = kind.unwrap_or(known.kind);
            } else {
                let name = name.unwrap_or_else(|| format!("[{symbol}]"));
                statuses.all.push(TaskStatus { symbol, name, next: next.unwrap_or(' '), kind: kind.unwrap_or(TaskType::Todo) });
            }
        }
        Some(statuses)
    }

    /// The statuses in the plugin's settings. The four defaults are there
    /// even when the settings leave one out, and `X` is done unless they
    /// say otherwise; a symbol listed twice is the first one.
    pub fn from_json(text: &str) -> TaskStatuses {
        let mut all: Vec<TaskStatus> = Vec::new();
        let settings: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        for list in ["coreStatuses", "customStatuses"] {
            let Some(items) = settings["statusSettings"][list].as_array() else { continue };
            for item in items {
                let (Some(symbol), Some(kind)) = (one_char(&item["symbol"]), item["type"].as_str().and_then(TaskType::parse)) else {
                    continue;
                };
                if all.iter().any(|s| s.symbol == symbol) {
                    continue;
                }
                let name = item["name"].as_str().filter(|n| !n.trim().is_empty()).map_or_else(|| format!("[{symbol}]"), str::to_string);
                let next = one_char(&item["nextStatusSymbol"]).unwrap_or(' ');
                all.push(TaskStatus { symbol, name, next, kind });
            }
        }
        let defaults = [
            TaskStatus::new(' ', "Todo", 'x', TaskType::Todo),
            TaskStatus::new('x', "Done", ' ', TaskType::Done),
            TaskStatus::new('/', "In Progress", 'x', TaskType::InProgress),
            TaskStatus::new('-', "Cancelled", ' ', TaskType::Cancelled),
        ];
        for (at, status) in defaults.into_iter().enumerate() {
            if !all.iter().any(|s| s.symbol == status.symbol) {
                all.insert(at.min(all.len()), status);
            }
        }
        TaskStatuses { all }
    }

    /// The status a symbol stands for: one the plugin does not know is still
    /// to do, named by its symbol.
    pub fn status(&self, symbol: char) -> TaskStatus {
        if let Some(found) = self.all.iter().find(|s| s.symbol == symbol) {
            return found.clone();
        }
        if symbol == 'X' {
            return TaskStatus::new('X', "Done", ' ', TaskType::Done);
        }
        TaskStatus::new(symbol, &format!("[{symbol}]"), 'x', TaskType::Todo)
    }

    pub fn kind(&self, symbol: char) -> TaskType {
        if let Some(found) = self.all.iter().find(|s| s.symbol == symbol) {
            return found.kind;
        }
        if symbol == 'X' { TaskType::Done } else { TaskType::Todo }
    }

    /// The statuses of the vault open now, for the rules that are given only
    /// a symbol. The defaults until a vault sets its own.
    pub fn current() -> Arc<TaskStatuses> {
        CURRENT.read().unwrap().clone()
    }

    pub fn set_current(statuses: TaskStatuses) {
        *CURRENT.write().unwrap() = Arc::new(statuses);
    }
}

/// The id and the key of `tasks.statuses.<id>.<key>`: an id of letters,
/// digits, `_` and `-`, as a notification rule's.
pub fn status_key(key: &str) -> Option<(&str, &str)> {
    let (id, name) = key.strip_prefix(TaskStatuses::SECTION)?.strip_prefix('.')?.split_once('.')?;
    let fits = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (fits && !name.contains('.')).then_some((id, name))
}

fn one_char(value: &Value) -> Option<char> {
    let text = value.as_str()?;
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = r#"{"statusSettings":{"coreStatuses":[{"symbol":" ","name":"Todo","nextStatusSymbol":"x","availableAsCommand":true,"type":"TODO"},{"symbol":"x","name":"Done","nextStatusSymbol":" ","availableAsCommand":true,"type":"DONE"}],"customStatuses":[{"symbol":"/","name":"In Progress","nextStatusSymbol":"x","availableAsCommand":true,"type":"IN_PROGRESS"},{"symbol":"-","name":"Cancelled","nextStatusSymbol":" ","availableAsCommand":true,"type":"CANCELLED"},{"symbol":"r","name":"In Review","nextStatusSymbol":"x","availableAsCommand":false,"type":"IN_PROGRESS"},{"symbol":"r","name":"Again","nextStatusSymbol":"x","type":"DONE"},{"symbol":"ab","name":"Long","type":"DONE"}]}}"#;

    #[test]
    fn the_settings_add_their_statuses() {
        let statuses = TaskStatuses::from_json(SETTINGS);
        assert_eq!(statuses.all.iter().map(|s| s.symbol).collect::<String>(), " x/-r");
        let review = statuses.status('r');
        assert_eq!((review.name.as_str(), review.kind, review.next), ("In Review", TaskType::InProgress, 'x'));
        assert!(!review.kind.is_closed());
        assert!(statuses.kind('-').is_closed() && statuses.kind('X').is_closed());
    }

    #[test]
    fn statuses_from_the_settings() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".obsidian/plugins/obsidian-tasks-plugin")).unwrap();
        // The plugin's file says otherwise, and is not read.
        std::fs::write(root.path().join(TaskStatuses::PATH), SETTINGS.replace("In Review", "Plugin")).unwrap();
        let toml = "[tasks.statuses.in_review]\nsymbol = \"r\"\nname = \"In Review\"\nnext = \"x\"\ntype = \"IN_PROGRESS\"\n";
        std::fs::write(root.path().join(".daftar.toml"), toml).unwrap();
        let statuses = TaskStatuses::load(root.path());
        assert_eq!(statuses.all.iter().map(|s| s.symbol).collect::<String>(), " x/-r");
        let review = statuses.status('r');
        assert_eq!((review.name.as_str(), review.kind, review.next), ("In Review", TaskType::InProgress, 'x'));
        // The same statuses as the plugin's file gave.
        assert_eq!(statuses, TaskStatuses::from_json(SETTINGS));
        // A section with a symbol of the four renames it; one that does not fit is left out.
        let own = crate::OwnSettings::new(
            "[tasks.statuses.done]\nsymbol = \"x\"\nname = \"Finished\"\n[tasks.statuses.long]\nsymbol = \"ab\"\n[tasks.statuses.\"a b\"]\nsymbol = \"q\"\n",
            None,
        );
        let renamed = TaskStatuses::from_settings(&own).unwrap();
        assert_eq!(renamed.all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Todo", "Finished", "In Progress", "Cancelled"]);
        assert_eq!(TaskStatuses::from_settings(&crate::OwnSettings::new("[reader]\nfolded = true\n", None)), None);
    }

    #[test]
    fn no_settings_are_the_four_defaults() {
        let statuses = TaskStatuses::from_json("not json");
        assert_eq!(statuses.all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Todo", "Done", "In Progress", "Cancelled"]);
        let unknown = statuses.status('>');
        assert_eq!((unknown.name.as_str(), unknown.kind), ("[>]", TaskType::Todo));
    }
}
