//! The statuses a task can have, as the Tasks plugin's settings in the vault
//! name them (`.obsidian/plugins/obsidian-tasks-plugin/data.json`): the four
//! every vault has, and the ones added there, such as `[r]` "In Review". The
//! rules are Daftar's on macOS (`TaskStatuses.swift`) and Android
//! (`TaskStatuses.kt`).

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

    /// The vault's statuses; the four defaults when the file is missing or
    /// cannot be read.
    pub fn load(root: &Path) -> TaskStatuses {
        TaskStatuses::from_json(&std::fs::read_to_string(root.join(Self::PATH)).unwrap_or_default())
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
    fn no_settings_are_the_four_defaults() {
        let statuses = TaskStatuses::from_json("not json");
        assert_eq!(statuses.all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Todo", "Done", "In Progress", "Cancelled"]);
        let unknown = statuses.status('>');
        assert_eq!((unknown.name.as_str(), unknown.kind), ("[>]", TaskType::Todo));
    }
}
