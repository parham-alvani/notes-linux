//! The editor in Neovim mode (`editor.vim_mode = true`): Neovim owns the
//! buffer and the file, and the editor shows what Neovim reports. Keys go to
//! Neovim; the view never edits its own text. The model the Mac's
//! `NeovimSession` and vscode-neovim use, which keeps what GTK does well --
//! a direction per paragraph, Persian, the colors -- while the editing is
//! the person's own Neovim with their config.
//!
//! `NeovimClient::request` blocks, so a worker thread owns the client and
//! the window only sends it messages; a second thread turns Neovim's
//! notifications into [Event]s on a channel the main loop reads.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use daftar_core::{MsgValue, NeovimClient, NeovimFailure, NeovimNotification, VimKeys, VimModifiers};

/// What Neovim said, for the main loop.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Neovim runs and has the file: its lines, and whether it ends with a newline.
    Ready {
        lines: Vec<String>,
        eol: bool,
    },
    /// `nvim_buf_lines_event`: lines [first, last) became [lines]; -1 is the end.
    Lines {
        first: usize,
        last: i64,
        lines: Vec<String>,
    },
    /// The mode, the cursor (row from 1, byte column) and the visual start.
    Cursor {
        mode: String,
        row: usize,
        column: usize,
        visual: Option<(usize, usize)>,
    },
    /// The first line in Neovim's window, from 1.
    Scroll(usize),
    /// The command line, or the last message; empty clears it. A message
    /// of more than one line (`:ls`, an error's trace) has them all.
    Status(String),
    /// Neovim's completion menu: its items, the one selected, and whether
    /// it completes the command line rather than the text.
    Menu {
        items: Vec<MenuItem>,
        selected: Option<usize>,
        cmdline: bool,
    },
    /// Another item of the menu selected, or none.
    MenuSelect(Option<usize>),
    MenuHide,
    Failed(String),
    Ended,
}

/// An item of Neovim's completion menu: the word, its kind and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub word: String,
    pub kind: String,
    pub menu: String,
}

enum Command {
    Notify(String, Vec<MsgValue>),
    Request(String, Vec<MsgValue>, mpsc::Sender<Result<MsgValue, NeovimFailure>>),
}

pub struct NeovimSession {
    commands: mpsc::Sender<Command>,
}

/// The autocommands that tell the view where the cursor is, what mode Neovim
/// is in, and where its window starts. The Mac's `NeovimSession.setup`.
const SETUP: &str = r#"
local chan = ...
local group = vim.api.nvim_create_augroup('daftar', { clear = true })
local function send()
  local mode = vim.api.nvim_get_mode().mode
  local cursor = vim.api.nvim_win_get_cursor(0)
  local v = vim.fn.getpos('v')
  vim.rpcnotify(chan, 'daftar_cursor', mode, cursor[1], cursor[2], v[2], v[3] - 1)
end
vim.api.nvim_create_autocmd({ 'CursorMoved', 'CursorMovedI', 'ModeChanged', 'BufEnter' }, { group = group, callback = send })
vim.api.nvim_create_autocmd('WinScrolled', { group = group, callback = function()
  vim.rpcnotify(chan, 'daftar_scroll', vim.fn.line('w0'))
end })
vim.o.autoread = true
send()
"#;

/// Scrolls the window to start at the line given.
const SCROLL_TO: &str = r#"
local by = ... - vim.fn.line('w0')
if by > 0 then
  vim.cmd('normal! ' .. by .. '\5')
elseif by < 0 then
  vim.cmd('normal! ' .. -by .. '\25')
end
"#;

impl NeovimSession {
    /// `nvim_config`: "user" is your init.lua, "clean" is none, and any other
    /// value is a Lua or Vim file to start with instead, as `-u`.
    pub fn arguments(config: &str) -> Vec<String> {
        let mut arguments = vec!["--cmd".to_string(), "let g:daftar = 1".to_string()];
        match config.trim() {
            "clean" => arguments.push("--clean".into()),
            "user" | "" => {}
            file => {
                let expanded = match file.strip_prefix("~/") {
                    Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest).to_string_lossy().into_owned()),
                    None => None,
                };
                arguments.push("-u".into());
                arguments.push(expanded.unwrap_or_else(|| file.to_string()));
            }
        }
        arguments
    }

    /// Starts Neovim on [path] in the vault at [root], [rows] by [columns];
    /// what it says comes on the returned channel. `None` when there is no
    /// `nvim` to run.
    pub fn start(
        configured: &str,
        config: &str,
        root: &Path,
        path: &str,
        rows: i64,
        columns: i64,
    ) -> Option<(NeovimSession, async_channel::Receiver<Event>)> {
        let executable = NeovimClient::locate(configured)?;
        let arguments = Self::arguments(config);
        let (events, received) = async_channel::unbounded();
        let (commands, inbox) = mpsc::channel::<Command>();
        let (root, path) = (root.to_path_buf(), path.to_string());
        std::thread::spawn(move || {
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let mut client = match NeovimClient::start(&executable, &arguments, Some(&root), &[]) {
                Ok(client) => client,
                Err(error) => {
                    let _ = events.send_blocking(Event::Failed(error.to_string()));
                    return;
                }
            };
            let (_, none) = mpsc::channel();
            let notifications = std::mem::replace(&mut client.notifications, none);
            {
                let events = events.clone();
                std::thread::spawn(move || {
                    for note in notifications {
                        for event in translate(&note) {
                            if events.send_blocking(event).is_err() {
                                return;
                            }
                        }
                    }
                    let _ = events.send_blocking(Event::Ended);
                });
            }
            match open(&client, &path, rows, columns) {
                Ok((lines, eol)) => {
                    let _ = events.send_blocking(Event::Ready { lines, eol });
                }
                Err(error) => {
                    let _ = events.send_blocking(Event::Failed(error.to_string()));
                    return;
                }
            }
            for command in inbox {
                match command {
                    Command::Notify(method, params) => {
                        let _ = client.notify(&method, params);
                    }
                    Command::Request(method, params, reply) => {
                        let _ = reply.send(client.request(&method, params));
                    }
                }
            }
            // The window let go of the session: Neovim stops with the client.
        });
        Some((NeovimSession { commands }, received))
    }

    fn notify(&self, method: &str, params: Vec<MsgValue>) {
        let _ = self.commands.send(Command::Notify(method.into(), params));
    }

    /// Keys in Neovim's notation, as `nvim_input` takes them.
    pub fn input(&self, keys: &str) {
        self.notify("nvim_input", vec![keys.into()]);
    }

    pub fn paste(&self, text: &str) {
        self.notify("nvim_paste", vec![text.into(), true.into(), MsgValue::Int(-1)]);
    }

    /// The cursor to (row from 1, byte column).
    pub fn set_cursor(&self, row: usize, column: usize, center: bool) {
        self.notify(
            "nvim_win_set_cursor",
            vec![MsgValue::Int(0), MsgValue::Array(vec![MsgValue::Int(row as i64), MsgValue::Int(column as i64)])],
        );
        if center {
            self.notify("nvim_command", vec!["normal! zz".into()]);
        }
    }

    /// The menu's item [index] chosen, as a click on it chooses.
    pub fn choose(&self, index: usize) {
        self.notify("nvim_select_popupmenu_item", vec![MsgValue::Int(index as i64), true.into(), true.into(), MsgValue::Map(vec![])]);
    }

    /// The window to start at [top] (from 1), as the wheel left the page:
    /// scrolled with Ctrl-E or Ctrl-Y, which bring the cursor along, as
    /// Neovim's own wheel does.
    pub fn set_top(&self, top: usize) {
        self.notify("nvim_exec_lua", vec![SCROLL_TO.into(), MsgValue::Array(vec![MsgValue::Int(top as i64)])]);
    }

    pub fn resize(&self, rows: i64, columns: i64) {
        self.notify("nvim_ui_try_resize", vec![MsgValue::Int(columns), MsgValue::Int(rows)]);
    }

    /// Neovim writes the file when the buffer changed: `:update`.
    pub fn save(&self) {
        self.notify("nvim_command", vec!["silent! update".into()]);
    }

    /// The file changed on disk: Neovim reads it again when it has nothing of
    /// its own unsaved, and says so when it has.
    pub fn check_disk(&self) {
        self.notify("nvim_command", vec!["silent! checktime".into()]);
    }

    /// Writes the buffer and waits for Neovim to have done it, up to [limit],
    /// before the editor lets go of the note.
    pub fn save_and_wait(&self, limit: Duration) -> bool {
        let (reply, answer) = mpsc::channel();
        if self.commands.send(Command::Request("nvim_command".into(), vec!["silent! update".into()], reply)).is_err() {
            return false;
        }
        matches!(answer.recv_timeout(limit), Ok(Ok(_)))
    }
}

/// Attaches as a UI (for the command line and messages), opens [path], and
/// reads its lines; the autocommands follow. Blocks: the worker runs it.
fn open(client: &NeovimClient, path: &str, rows: i64, columns: i64) -> Result<(Vec<String>, bool), NeovimFailure> {
    let options = MsgValue::Map(
        ["ext_linegrid", "ext_cmdline", "ext_messages", "ext_popupmenu", "rgb"]
            .iter()
            .map(|key| (MsgValue::from(*key), MsgValue::Bool(true)))
            .collect(),
    );
    client.request("nvim_ui_attach", vec![MsgValue::Int(columns), MsgValue::Int(rows), options])?;
    let channel = client.request("nvim_get_api_info", vec![])?.as_array().and_then(|a| a.first()).and_then(MsgValue::as_int).unwrap_or(0);
    client.request("nvim_exec_lua", vec!["vim.cmd.edit(vim.fn.fnameescape(...))".into(), MsgValue::Array(vec![path.into()])])?;
    let lines = client
        .request("nvim_buf_get_lines", vec![MsgValue::Int(0), MsgValue::Int(0), MsgValue::Int(-1), false.into()])?
        .as_array()
        .map(|items| items.iter().filter_map(|l| l.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let eol = client.request("nvim_get_option_value", vec!["eol".into(), MsgValue::Map(vec![])])?.as_bool().unwrap_or(true);
    client.request("nvim_buf_attach", vec![MsgValue::Int(0), false.into(), MsgValue::Map(vec![])])?;
    client.request("nvim_exec_lua", vec![SETUP.into(), MsgValue::Array(vec![MsgValue::Int(channel)])])?;
    Ok((lines, eol))
}

/// What a notification means for the view.
fn translate(note: &NeovimNotification) -> Vec<Event> {
    let int = |at: usize| note.params.get(at).and_then(MsgValue::as_int);
    match note.method.as_str() {
        "nvim_buf_lines_event" => {
            let (Some(first), Some(last)) = (int(2), int(3)) else { return vec![] };
            let lines = note
                .params
                .get(4)
                .and_then(MsgValue::as_array)
                .map(|items| items.iter().filter_map(|l| l.as_str().map(String::from)).collect())
                .unwrap_or_default();
            vec![Event::Lines { first: first.max(0) as usize, last, lines }]
        }
        "daftar_cursor" => {
            let (Some(row), Some(column)) = (int(1), int(2)) else { return vec![] };
            let mode = note.params.first().and_then(MsgValue::as_str).unwrap_or("n").to_string();
            let visual = int(3).zip(int(4)).map(|(r, c)| (r.max(1) as usize, c.max(0) as usize));
            vec![Event::Cursor { mode, row: row.max(1) as usize, column: column.max(0) as usize, visual }]
        }
        "daftar_scroll" => int(0).map(|top| vec![Event::Scroll(top.max(1) as usize)]).unwrap_or_default(),
        "redraw" => redraw(&note.params),
        _ => vec![],
    }
}

/// The command line's caret, drawn where Neovim says it is.
pub const CARET: char = '▏';

/// The command line, messages and the completion menu, from the UI events;
/// the grid itself is not drawn, the text view is the screen. The messages
/// of one redraw are said together, a line each.
fn redraw(batches: &[MsgValue]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut messages: Option<Vec<String>> = None;
    for batch in batches {
        let Some(items) = batch.as_array() else { continue };
        let Some(name) = items.first().and_then(MsgValue::as_str) else { continue };
        for args in items.iter().skip(1).filter_map(MsgValue::as_array) {
            let int = |at: usize| args.get(at).and_then(MsgValue::as_int);
            match name {
                "cmdline_show" => {
                    let content = chunks(args.first());
                    let firstc = args.get(2).and_then(MsgValue::as_str).unwrap_or("");
                    let prompt = args.get(3).and_then(MsgValue::as_str).unwrap_or("");
                    // The caret at its byte, or the end when that is not a character's start.
                    let at = int(1).map(|p| p.max(0) as usize).filter(|p| content.is_char_boundary(*p)).unwrap_or(content.len());
                    out.push(Event::Status(format!("{firstc}{prompt}{}{CARET}{}", &content[..at], &content[at..])));
                }
                "cmdline_pos" => {}
                "cmdline_hide" | "msg_clear" => {
                    messages = None;
                    out.push(Event::Status(String::new()));
                }
                "msg_show" => {
                    let text = chunks(args.get(1));
                    let replace = args.get(2).and_then(MsgValue::as_bool).unwrap_or(false);
                    let said = messages.get_or_insert_with(Vec::new);
                    if replace {
                        said.pop();
                    }
                    if !text.is_empty() {
                        said.push(text.trim_end_matches('\n').to_string());
                    }
                }
                "popupmenu_show" => {
                    let items = args
                        .first()
                        .and_then(MsgValue::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(MsgValue::as_array)
                                .map(|item| {
                                    let field = |at: usize| item.get(at).and_then(MsgValue::as_str).unwrap_or("").to_string();
                                    MenuItem { word: field(0), kind: field(1), menu: field(2) }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let selected = int(1).filter(|i| *i >= 0).map(|i| i as usize);
                    out.push(Event::Menu { items, selected, cmdline: int(4) == Some(-1) });
                }
                "popupmenu_select" => out.push(Event::MenuSelect(int(0).filter(|i| *i >= 0).map(|i| i as usize))),
                "popupmenu_hide" => out.push(Event::MenuHide),
                _ => {}
            }
        }
    }
    if let Some(said) = messages.filter(|said| !said.is_empty()) {
        out.push(Event::Status(said.join("\n")));
    }
    out
}

/// `[[attr, text, …], …]`, the chunks a message or a command line is made of.
fn chunks(value: Option<&MsgValue>) -> String {
    value
        .and_then(MsgValue::as_array)
        .map(|items| {
            items.iter().filter_map(|chunk| chunk.as_array().filter(|c| c.len() >= 2).and_then(|c| c[1].as_str())).collect::<String>()
        })
        .unwrap_or_default()
}

/// The mode as the bar names it.
pub fn mode_name(mode: &str) -> &'static str {
    match mode.chars().next() {
        Some('i') => "INSERT",
        Some('R') => "REPLACE",
        Some('v') => "VISUAL",
        Some('V') => "V-LINE",
        Some('\u{16}') => "V-BLOCK",
        Some('c') => "COMMAND",
        Some('t') => "TERMINAL",
        _ => "NORMAL",
    }
}

/// Normal mode: not inserting, replacing or selecting.
pub fn is_normal(mode: &str) -> bool {
    !(mode.starts_with('i') || mode.starts_with('R') || mode.starts_with('v') || mode.starts_with('V') || mode == "\u{16}")
}

/// A GTK key press in Neovim's notation, or `None` for a key that is only a
/// modifier. [name] is the key's GDK name (`Up`, `Page_Up`, `Return`), and
/// [character] what it types, Shift applied.
pub fn vim_key(name: &str, character: Option<char>, control: bool, alt: bool, super_: bool, shift: bool) -> Option<String> {
    let mut modifiers = VimModifiers::NONE;
    if control {
        modifiers = modifiers | VimModifiers::CONTROL;
    }
    if alt {
        modifiers = modifiers | VimModifiers::OPTION;
    }
    if super_ {
        modifiers = modifiers | VimModifiers::COMMAND;
    }
    if shift {
        modifiers = modifiers | VimModifiers::SHIFT;
    }
    let named = match name {
        "Up" | "KP_Up" => Some("Up"),
        "Down" | "KP_Down" => Some("Down"),
        "Left" | "KP_Left" => Some("Left"),
        "Right" | "KP_Right" => Some("Right"),
        "Home" | "KP_Home" => Some("Home"),
        "End" | "KP_End" => Some("End"),
        "Page_Up" | "KP_Page_Up" => Some("PageUp"),
        "Page_Down" | "KP_Page_Down" => Some("PageDown"),
        "Delete" | "KP_Delete" => Some("Del"),
        "Insert" | "KP_Insert" => Some("Insert"),
        "Return" | "KP_Enter" => Some("CR"),
        "Tab" | "ISO_Left_Tab" | "KP_Tab" => Some("Tab"),
        "Escape" => Some("Esc"),
        "BackSpace" => Some("BS"),
        f if f.len() <= 3 && f.starts_with('F') && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) => Some(f),
        _ => None,
    };
    if let Some(named) = named {
        return VimKeys::named(named, modifiers);
    }
    let character = character.filter(|c| !c.is_control())?;
    // What a printable key typed already has Shift in it.
    let without_shift = VimModifiers(modifiers.0 & !VimModifiers::SHIFT.0);
    VimKeys::notation(&character.to_string(), without_shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_in_neovims_notation() {
        assert_eq!(vim_key("a", Some('a'), false, false, false, false).as_deref(), Some("a"));
        assert_eq!(vim_key("A", Some('A'), false, false, false, true).as_deref(), Some("A"));
        assert_eq!(vim_key("w", Some('w'), true, false, false, false).as_deref(), Some("<C-w>"));
        assert_eq!(vim_key("less", Some('<'), false, false, false, true).as_deref(), Some("<LT>"));
        assert_eq!(vim_key("Escape", None, false, false, false, false).as_deref(), Some("<Esc>"));
        assert_eq!(vim_key("ISO_Left_Tab", None, false, false, false, true).as_deref(), Some("<S-Tab>"));
        assert_eq!(vim_key("Page_Down", None, false, false, false, false).as_deref(), Some("<PageDown>"));
        assert_eq!(vim_key("F5", None, false, false, false, false).as_deref(), Some("<F5>"));
        assert_eq!(vim_key("space", Some(' '), true, false, false, false).as_deref(), Some("<C-Space>"));
        // Persian letters reach Neovim as they are.
        assert_eq!(vim_key("Arabic_seen", Some('س'), false, false, false, false).as_deref(), Some("س"));
        assert_eq!(vim_key("Shift_L", None, false, false, false, true), None);
    }

    #[test]
    fn the_config_choice_is_neovims_arguments() {
        assert_eq!(NeovimSession::arguments("user"), ["--cmd", "let g:daftar = 1"]);
        assert_eq!(NeovimSession::arguments("clean"), ["--cmd", "let g:daftar = 1", "--clean"]);
        assert_eq!(NeovimSession::arguments("/x/init.lua"), ["--cmd", "let g:daftar = 1", "-u", "/x/init.lua"]);
    }

    #[test]
    fn modes_and_messages() {
        assert_eq!(mode_name("niI"), "NORMAL");
        assert_eq!(mode_name("i"), "INSERT");
        assert_eq!(mode_name("\u{16}"), "V-BLOCK");
        assert!(is_normal("n") && !is_normal("v") && !is_normal("i"));
        let note = |method: &str, params: Vec<MsgValue>| NeovimNotification { method: method.into(), params };
        let show = MsgValue::Array(vec![
            "cmdline_show".into(),
            MsgValue::Array(vec![
                MsgValue::Array(vec![MsgValue::Array(vec![MsgValue::Int(0), "w".into()])]),
                MsgValue::Int(1),
                ":".into(),
                "".into(),
            ]),
        ]);
        assert_eq!(translate(&note("redraw", vec![show])), [Event::Status(":w▏".into())]);
        let chunk = |text: &str| MsgValue::Array(vec![MsgValue::Array(vec![MsgValue::Int(0), text.into()])]);
        let message = |text: &str, replace: bool| MsgValue::Array(vec!["echo".into(), chunk(text), replace.into()]);
        let said = MsgValue::Array(vec!["msg_show".into(), message("one", false), message("two\n", false)]);
        let replaced = MsgValue::Array(vec!["msg_show".into(), message("three", true)]);
        assert_eq!(translate(&note("redraw", vec![said, replaced])), [Event::Status("one\nthree".into())]);
        let item = |word: &str| MsgValue::Array(vec![word.into(), "v".into(), "[LSP]".into(), "".into()]);
        let menu = MsgValue::Array(vec![
            "popupmenu_show".into(),
            MsgValue::Array(vec![
                MsgValue::Array(vec![item("alpha"), item("beta")]),
                MsgValue::Int(-1),
                MsgValue::Int(3),
                MsgValue::Int(0),
                MsgValue::Int(-1),
            ]),
        ]);
        let select = MsgValue::Array(vec!["popupmenu_select".into(), MsgValue::Array(vec![MsgValue::Int(1)])]);
        assert_eq!(
            translate(&note("redraw", vec![menu, select])),
            [
                Event::Menu {
                    items: ["alpha", "beta"].map(|w| MenuItem { word: w.into(), kind: "v".into(), menu: "[LSP]".into() }).to_vec(),
                    selected: None,
                    cmdline: true,
                },
                Event::MenuSelect(Some(1)),
            ]
        );
        let lines = note(
            "nvim_buf_lines_event",
            vec![MsgValue::Int(1), MsgValue::Int(4), MsgValue::Int(2), MsgValue::Int(3), MsgValue::Array(vec!["new".into()]), false.into()],
        );
        assert_eq!(translate(&lines), [Event::Lines { first: 2, last: 3, lines: vec!["new".into()] }]);
    }

    /// The real thing, when `nvim` is installed: a key typed through Neovim
    /// comes back as a buffer change, and `:update` writes the file.
    #[test]
    fn typing_goes_through_neovim_and_neovim_writes_the_file() {
        if NeovimClient::locate("").is_none() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Note.md"), "one\n").unwrap();
        let (session, events) = NeovimSession::start("", "clean", root.path(), "Note.md", 30, 80).unwrap();
        let next = || events.recv_blocking().unwrap();
        let ready = loop {
            match next() {
                event @ Event::Ready { .. } => break event,
                Event::Failed(error) => panic!("{error}"),
                _ => {}
            }
        };
        assert_eq!(ready, Event::Ready { lines: vec!["one".into()], eol: true });
        session.input("Atwo<Esc>");
        // Each key in insert mode is a change of its own; the last says it all.
        loop {
            if let Event::Lines { lines, .. } = next()
                && lines == ["onetwo"]
            {
                break;
            }
        }
        assert!(session.save_and_wait(Duration::from_secs(5)));
        assert_eq!(std::fs::read_to_string(root.path().join("Note.md")).unwrap(), "onetwo\n");
    }
}
