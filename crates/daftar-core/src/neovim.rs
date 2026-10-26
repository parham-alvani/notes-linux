//! A Neovim process started with `--embed`, spoken to over msgpack-RPC on its
//! stdin and stdout; the mirror of its buffer the editor shows; and key
//! presses in Neovim's notation.
//!
//! The Mac's client is async. This one is threads and channels, so the core
//! needs no runtime: a reader thread answers each `request`, which blocks
//! its caller, and sends notifications -- buffer changes, the cursor,
//! redraws -- down [NeovimClient::notifications]. The GTK app calls
//! `request` off the main thread and reads notifications with a timeout or
//! from a thread of its own. Offsets into the view's text are characters,
//! as GtkTextBuffer counts them; the Mac counts UTF-16 units.

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::message_pack::{MessagePack, MsgStream, MsgValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeovimFailure {
    pub message: String,
}

impl fmt::Display for NeovimFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for NeovimFailure {}

fn failure(message: impl Into<String>) -> NeovimFailure {
    NeovimFailure { message: message.into() }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeovimNotification {
    pub method: String,
    pub params: Vec<MsgValue>,
}

type Waiters = Arc<Mutex<HashMap<u64, Sender<Result<MsgValue, NeovimFailure>>>>>;

pub struct NeovimClient {
    child: Child,
    input: Arc<Mutex<ChildStdin>>,
    next_id: Mutex<u64>,
    pending: Waiters,
    /// Every notification Neovim sends, in order. It ends when Neovim does.
    pub notifications: Receiver<NeovimNotification>,
}

impl NeovimClient {
    /// The first `nvim` that runs: [configured], else `$PATH`, else the usual places.
    pub fn locate(configured: &str) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if !configured.is_empty() {
            let expanded = match configured.strip_prefix("~/") {
                Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| configured.into()),
                None => configured.into(),
            };
            candidates.push(expanded);
        }
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|dir| dir.join("nvim")));
        }
        candidates.extend(["/usr/bin/nvim", "/usr/local/bin/nvim", "/opt/homebrew/bin/nvim"].map(PathBuf::from));
        candidates.into_iter().find(|p| is_executable(p))
    }

    /// Starts Neovim with `--embed` and [arguments], in [directory].
    pub fn start(
        executable: &Path,
        arguments: &[&str],
        directory: Option<&Path>,
        environment: &[(&str, &str)],
    ) -> Result<NeovimClient, NeovimFailure> {
        let mut command = Command::new(executable);
        command.arg("--embed").args(arguments).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        command.envs(environment.iter().copied());
        let mut child = command.spawn().map_err(|e| failure(format!("Neovim did not start: {e}")))?;
        let input = Arc::new(Mutex::new(child.stdin.take().ok_or_else(|| failure("no stdin"))?));
        let mut output = child.stdout.take().ok_or_else(|| failure("no stdout"))?;
        let pending: Waiters = Arc::default();
        let (notify, notifications) = mpsc::channel();
        {
            let (pending, input) = (pending.clone(), input.clone());
            std::thread::spawn(move || {
                let mut stream = MsgStream::new();
                let mut buffer = [0u8; 64 * 1024];
                while let Ok(count) = output.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    for message in stream.feed(&buffer[..count]) {
                        received(message, &pending, &input, &notify);
                    }
                }
                // Neovim exited: every request still waiting hears it.
                for (_, waiter) in pending.lock().unwrap().drain() {
                    let _ = waiter.send(Err(failure("Neovim exited")));
                }
            });
        }
        Ok(NeovimClient { child, input, next_id: Mutex::new(1), pending, notifications })
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Calls an API function and waits for its result.
    pub fn request(&self, method: &str, params: Vec<MsgValue>) -> Result<MsgValue, NeovimFailure> {
        let (sender, receiver) = mpsc::channel();
        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };
        self.pending.lock().unwrap().insert(id, sender);
        let message = MsgValue::Array(vec![MsgValue::Uint(0), MsgValue::Uint(id), method.into(), MsgValue::Array(params)]);
        if let Err(error) = send(&self.input, &message) {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        receiver.recv().unwrap_or_else(|_| Err(failure("Neovim exited")))
    }

    /// Calls an API function without waiting.
    pub fn notify(&self, method: &str, params: Vec<MsgValue>) -> Result<(), NeovimFailure> {
        send(&self.input, &MsgValue::Array(vec![MsgValue::Uint(2), method.into(), MsgValue::Array(params)]))
    }
}

impl Drop for NeovimClient {
    fn drop(&mut self) {
        self.stop();
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn send(input: &Mutex<ChildStdin>, message: &MsgValue) -> Result<(), NeovimFailure> {
    let mut input = input.lock().unwrap();
    input.write_all(&MessagePack::encode(message)).and_then(|_| input.flush()).map_err(|e| failure(format!("Neovim is gone: {e}")))
}

fn received(message: MsgValue, pending: &Waiters, input: &Mutex<ChildStdin>, notify: &Sender<NeovimNotification>) {
    let Some(parts) = message.as_array() else { return };
    match (parts.first().and_then(MsgValue::as_int), parts.len()) {
        (Some(1), 4) => {
            let Some(id) = parts[1].as_int() else { return };
            let Some(waiter) = pending.lock().unwrap().remove(&(id as u64)) else { return };
            let result = if parts[2] != MsgValue::Nil {
                let text = parts[2].as_array().and_then(|e| e.last()).and_then(MsgValue::as_str).map(String::from);
                Err(failure(text.unwrap_or_else(|| format!("{:?}", parts[2]))))
            } else {
                Ok(parts[3].clone())
            };
            let _ = waiter.send(result);
        }
        (Some(2), 3) => {
            let method = parts[1].as_str().unwrap_or("").to_string();
            let params = parts[2].as_array().map(<[MsgValue]>::to_vec).unwrap_or_default();
            let _ = notify.send(NeovimNotification { method, params });
        }
        (Some(0), 4) => {
            // Neovim asking us something (rpcrequest): there is nothing here
            // to answer with, so it gets an empty reply, not a hang.
            let _ = send(input, &MsgValue::Array(vec![MsgValue::Uint(1), parts[1].clone(), MsgValue::Nil, MsgValue::Nil]));
        }
        _ => {}
    }
}

/// The buffer's lines as Neovim last reported them, and the edit each report
/// makes to the text the view shows: the lines joined with newlines, and one
/// more newline at the end when the file has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferMirror {
    lines: Vec<String>,
    pub ends_with_newline: bool,
    /// Each line's length in characters, so a position is a sum.
    lengths: Vec<usize>,
}

/// An edit to the view's text: the characters in [range] become [text].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorEdit {
    pub range: Range<usize>,
    pub text: String,
}

impl BufferMirror {
    pub fn new(lines: Vec<String>, ends_with_newline: bool) -> BufferMirror {
        let lines = if lines.is_empty() { vec![String::new()] } else { lines };
        let lengths = lines.iter().map(|l| l.chars().count()).collect();
        BufferMirror { lines, ends_with_newline, lengths }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn text(&self) -> String {
        let mut text = self.lines.join("\n");
        if self.ends_with_newline {
            text.push('\n');
        }
        text
    }

    /// Where line [index] (from 0) starts in the view's text.
    fn start(&self, index: usize) -> usize {
        self.lengths[..index.min(self.lengths.len())].iter().map(|l| l + 1).sum()
    }

    /// Lines [a, b) of [lines] as the view's text says them: with the newline
    /// after each, and after the last one only when more follow or the file
    /// ends with one.
    fn segment(&self, a: usize, b: usize) -> String {
        if a >= b {
            return String::new();
        }
        let mut out = self.lines[a..b].join("\n");
        if b < self.lines.len() || self.ends_with_newline {
            out.push('\n');
        }
        out
    }

    /// `nvim_buf_lines_event`: lines [first, last) became [replacement]; a
    /// [last] of -1 is the end of the buffer. Returns the smallest edit to
    /// the view's text, so it keeps its tags, its scroll and its pictures
    /// everywhere else.
    pub fn apply(&mut self, first: usize, last: i64, replacement: Vec<String>) -> MirrorEdit {
        let end = if last < 0 { self.lines.len() } else { (last as usize).min(self.lines.len()) };
        let begin = first.min(self.lines.len()).min(end);
        let from = begin.saturating_sub(1);
        let offset = self.start(from);
        let old: Vec<char> = self.segment(from, end).chars().collect();
        let count = replacement.len();
        self.lengths.splice(begin..end, replacement.iter().map(|l| l.chars().count()));
        self.lines.splice(begin..end, replacement);
        if self.lines.is_empty() {
            // No line left: the buffer is one empty line, as Neovim keeps it.
            self.lines = vec![String::new()];
            self.lengths = vec![0];
            return MirrorEdit { range: 0..offset + old.len(), text: self.text() };
        }
        let new: Vec<char> = self.segment(from, begin + count).chars().collect();
        let limit = old.len().min(new.len());
        let prefix = (0..limit).take_while(|&i| old[i] == new[i]).count();
        let suffix = (0..limit - prefix).take_while(|&i| old[old.len() - 1 - i] == new[new.len() - 1 - i]).count();
        MirrorEdit { range: offset + prefix..offset + old.len() - suffix, text: new[prefix..new.len() - suffix].iter().collect() }
    }

    /// A Neovim position (row from 1, column in UTF-8 bytes from 0) as a
    /// character offset into the view's text.
    pub fn offset(&self, row: usize, byte_column: usize) -> usize {
        let index = row.saturating_sub(1).min(self.lines.len() - 1);
        let line = &self.lines[index];
        let mut cut = byte_column.min(line.len());
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        self.start(index) + line[..cut].chars().count()
    }

    /// The other way: a character offset as a Neovim (row from 1, byte column).
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let mut remaining = offset;
        for (index, length) in self.lengths.iter().enumerate() {
            if remaining <= *length {
                let bytes = self.lines[index].chars().take(remaining).map(char::len_utf8).sum();
                return (index + 1, bytes);
            }
            remaining -= length + 1;
        }
        (self.lines.len(), self.lines.last().map_or(0, String::len))
    }
}

/// The modifier keys held, as bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VimModifiers(pub u8);

impl VimModifiers {
    pub const NONE: VimModifiers = VimModifiers(0);
    pub const SHIFT: VimModifiers = VimModifiers(1);
    pub const CONTROL: VimModifiers = VimModifiers(2);
    /// Alt.
    pub const OPTION: VimModifiers = VimModifiers(4);
    /// Super, which Neovim calls `D-`.
    pub const COMMAND: VimModifiers = VimModifiers(8);

    pub fn contains(self, other: VimModifiers) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for VimModifiers {
    type Output = VimModifiers;
    fn bitor(self, other: VimModifiers) -> VimModifiers {
        VimModifiers(self.0 | other.0)
    }
}

/// A key press as Neovim's `nvim_input` spells it: `a`, `<CR>`, `<C-w>`,
/// `<M-Left>`, `<LT>`.
pub struct VimKeys;

/// The control characters the keyboard sends, by their key's name.
const CONTROL_KEYS: [(char, &str); 7] =
    [('\r', "CR"), ('\u{3}', "CR"), ('\t', "Tab"), ('\u{19}', "Tab"), ('\u{1B}', "Esc"), ('\u{7F}', "BS"), ('\u{8}', "BS")];

/// Key names Neovim knows, as a key that types no text is named here: the
/// GTK app maps its key values (`Up`, `Page_Up`, `F5`) to these.
const NAMED_KEYS: [&str; 26] = [
    "Up", "Down", "Left", "Right", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Del", "Home", "End",
    "PageUp", "PageDown", "CR", "Tab", "Esc", "BS", "Insert",
];

impl VimKeys {
    fn prefix(modifiers: VimModifiers) -> String {
        let mut prefix = String::new();
        if modifiers.contains(VimModifiers::CONTROL) {
            prefix.push_str("C-");
        }
        if modifiers.contains(VimModifiers::OPTION) {
            prefix.push_str("M-");
        }
        if modifiers.contains(VimModifiers::COMMAND) {
            prefix.push_str("D-");
        }
        prefix
    }

    /// A key that types no text, by its Neovim name (`Up`, `F5`, `PageUp`).
    /// Shift matters on such a key (`<S-Tab>`, `<S-Up>`).
    pub fn named(name: &str, modifiers: VimModifiers) -> Option<String> {
        if !NAMED_KEYS.contains(&name) {
            return None;
        }
        let shift = if modifiers.contains(VimModifiers::SHIFT) { "S-" } else { "" };
        Some(format!("<{shift}{}{name}>", Self::prefix(modifiers)))
    }

    /// [characters] is what the key typed with every modifier but Control
    /// (Shift applied); [modifiers] the ones held.
    pub fn notation(characters: &str, modifiers: VimModifiers) -> Option<String> {
        let first = characters.chars().next()?;
        if let Some((_, name)) = CONTROL_KEYS.iter().find(|(c, _)| *c == first) {
            return Self::named(name, modifiers);
        }
        let prefix = Self::prefix(modifiers);
        if characters == " " {
            return Some(if prefix.is_empty() { " ".into() } else { format!("<{prefix}Space>") });
        }
        let key = if characters == "<" {
            "LT"
        } else if characters == "\\" && !prefix.is_empty() {
            "Bslash"
        } else {
            characters
        };
        if prefix.is_empty() {
            return Some(if key == "LT" { "<LT>".into() } else { key.into() });
        }
        Some(format!("<{prefix}{key}>"))
    }

    /// Return, Escape, Tab and Backspace, as the characters they send.
    pub fn is_special(characters: &str) -> bool {
        characters.chars().next().is_some_and(|c| CONTROL_KEYS.iter().any(|(k, _)| *k == c))
    }

    /// Typed text -- from the keyboard or an input method -- for `nvim_input`,
    /// where `<` starts a key name.
    pub fn escape(text: &str) -> String {
        text.replace('<', "<LT>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // BufferMirrorTests

    #[test]
    fn apply_returns_the_smallest_edit() {
        let mut mirror = BufferMirror::new(strings(&["one", "two", "three"]), true);
        let edit = mirror.apply(1, 2, strings(&["TWO"]));
        assert_eq!(mirror.text(), "one\nTWO\nthree\n");
        assert_eq!(edit, MirrorEdit { range: 4..7, text: "TWO".into() });
        let added = mirror.apply(3, 3, strings(&["four"]));
        assert_eq!(mirror.text(), "one\nTWO\nthree\nfour\n");
        assert_eq!(added.text, "four\n");
        mirror.apply(0, -1, vec![]);
        assert_eq!(mirror.text(), "\n");
    }

    #[test]
    fn positions_count_bytes_in_neovim_and_characters_in_the_view() {
        let mirror = BufferMirror::new(strings(&["abc", "سلام x", "😀b"]), true);
        assert_eq!(mirror.offset(1, 2), 2);
        // "سلام " is 4 letters of 2 bytes and a space: byte 9 is the "x".
        assert_eq!(mirror.offset(2, 9), 4 + 5);
        // The emoji is 4 bytes and one character.
        assert_eq!(mirror.offset(3, 4), 4 + 7 + 1);
        assert_eq!(mirror.position(4 + 5), (2, 9));
        assert_eq!(mirror.position(4 + 7 + 1), (3, 4));
    }

    // VimKeysTests

    #[test]
    fn notation() {
        let none = VimModifiers::NONE;
        assert_eq!(VimKeys::notation("a", none).as_deref(), Some("a"));
        assert_eq!(VimKeys::notation("A", VimModifiers::SHIFT).as_deref(), Some("A"));
        assert_eq!(VimKeys::notation("w", VimModifiers::CONTROL).as_deref(), Some("<C-w>"));
        assert_eq!(VimKeys::notation("<", VimModifiers::SHIFT).as_deref(), Some("<LT>"));
        assert_eq!(VimKeys::notation("\r", none).as_deref(), Some("<CR>"));
        assert_eq!(VimKeys::notation("\u{1B}", none).as_deref(), Some("<Esc>"));
        assert_eq!(VimKeys::named("Up", VimModifiers::SHIFT).as_deref(), Some("<S-Up>"));
        assert_eq!(VimKeys::notation(" ", VimModifiers::CONTROL).as_deref(), Some("<C-Space>"));
        assert_eq!(VimKeys::notation("ش", none).as_deref(), Some("ش"));
        assert_eq!(VimKeys::escape("a<b"), "a<LT>b");
    }

    // NeovimClientTests: against a real Neovim, when there is one; CI has none.

    #[test]
    fn edits_a_file_and_reports_the_buffer() {
        let Some(nvim) = NeovimClient::locate("") else { return };
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("note.md"), "one\ntwo\n").unwrap();
        let client = NeovimClient::start(&nvim, &["--clean"], Some(folder.path()), &[]).unwrap();
        let options = MsgValue::Map(vec![("ext_linegrid".into(), true.into()), ("ext_messages".into(), true.into())]);
        client.request("nvim_ui_attach", vec![MsgValue::Int(80), MsgValue::Int(24), options]).unwrap();
        client.request("nvim_command", vec!["edit note.md".into()]).unwrap();
        let lines = client.request("nvim_buf_get_lines", vec![0i64.into(), 0i64.into(), (-1i64).into(), false.into()]).unwrap();
        let texts: Vec<&str> = lines.as_array().unwrap().iter().filter_map(MsgValue::as_str).collect();
        assert_eq!(texts, ["one", "two"]);

        client.request("nvim_buf_attach", vec![0i64.into(), false.into(), MsgValue::Map(vec![])]).unwrap();
        client.request("nvim_input", vec!["ggAX<Esc>".into()]).unwrap();
        let mut changed = None;
        while let Ok(note) = client.notifications.recv_timeout(Duration::from_secs(10)) {
            if note.method == "nvim_buf_lines_event" {
                changed = note.params[4].as_array().map(|l| l.iter().filter_map(MsgValue::as_str).map(String::from).collect::<Vec<_>>());
                break;
            }
        }
        assert_eq!(changed, Some(strings(&["oneX"])));
        client.request("nvim_command", vec!["write".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(folder.path().join("note.md")).unwrap(), "oneX\ntwo\n");
    }

    // BufferMirrorFuzzTests: random edits; the mirror and a plain copy of the
    // text never differ.

    #[test]
    fn every_edit_the_mirror_returns_makes_its_text() {
        let mut state: u64 = 1995;
        let mut next = |n: usize| -> usize {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        let words = ["", "a", "سلام", "x y", "😀z", "# h", "- [ ] t"];
        for eol in [true, false] {
            let mut mirror = BufferMirror::new(strings(&["one", "دو", "three"]), eol);
            let mut shown: Vec<char> = mirror.text().chars().collect();
            for _ in 0..3000 {
                let count = mirror.lines().len();
                let first = next(count + 1);
                let last = if next(2) == 0 { -1 } else { (first + next(count - first + 1)) as i64 };
                let replacement: Vec<String> = (0..next(4)).map(|_| words[next(words.len())].to_string()).collect();
                let edit = mirror.apply(first, last, replacement.clone());
                shown.splice(edit.range.clone(), edit.text.chars());
                assert_eq!(shown.iter().collect::<String>(), mirror.text(), "first {first} last {last} {replacement:?}");
                // A row and column there and back again.
                let row = 1 + next(mirror.lines().len());
                assert_eq!(mirror.position(mirror.offset(row, 0)).0, row);
            }
        }
    }
}
