//! `daftar-cli lsp`: the vault as a Language Server Protocol server on stdin
//! and stdout, so Neovim, Helix or any editor with a client gets Daftar's
//! rules in a note (`docs/specs/language-server.md`). The base protocol:
//! each message a `Content-Length` header and a JSON-RPC 2.0 body, answered
//! in order on one thread, as `mcp.rs` answers its lines.
//!
//! The editor sends the whole text of an open note at each change. The
//! server lints it there (`MarkdownLint`, the vault's `.markdownlint.jsonc`)
//! and runs `check` on the note as saved when it opens and at each save;
//! formats it as `fmt` does; completes `[[` with the notes as the quick
//! switcher ranks them and `[[Note#` with its headings; goes to a link's
//! note, heading or block; shows its first lines on hover; lists the links
//! to a note; and renames a note with every link to it, as a workspace edit
//! the editor applies (`VaultMoves::relinked`): the server writes no file.
//! The index (`Vault::index`) is built at `initialize` and read again for
//! the files a save or the editor's file watcher names.

mod links;
mod position;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde_json::{Value, json};

use daftar_core::text::slugs;
use daftar_core::{Bookmarks, Candidate, ExcludedFiles, LintConfig, MarkdownLint, QuickSwitcher, Sections, Vault, VaultIndex, VaultMoves};

use crate::{check, report};
use links::Typing;

/// What a `file:` URI's path keeps unencoded.
const IN_PATH: &AsciiSet = &NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'.').remove(b'_').remove(b'~');

/// A file's `file://` URI. On Windows the path's `\\` are `/`, a drive
/// is `/C:/…` (its colon kept, as Neovim and VS Code read it), and a
/// canonical path's `\\?\` is dropped.
fn uri_of_file(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).replace('\\', "/");
    let text = if text.starts_with('/') { text } else { format!("/{text}") };
    format!("file://{}", utf8_percent_encode(&text, IN_PATH).to_string().replacen("%3A", ":", usize::from(is_drive(&text))))
}

/// The file a `file://` URI names: `/C:/…` (or `/c%3A/…`) is `C:/…`.
fn file_of_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let path = percent_decode_str(rest).decode_utf8().ok()?.into_owned();
    Some(if is_drive(&path) { path[1..].to_string() } else { path })
}

/// `/C:/…` or `/C:`: a Windows drive written as a URI's path.
fn is_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' && bytes.get(3).is_none_or(|b| *b == b'/')
}

/// How many notes a completion offers; the editor asks again as more is typed.
const COMPLETIONS: usize = 50;
/// How many lines of a note a hover shows.
const HOVER_LINES: usize = 15;

/// An error answered: its code and message.
type Failure = (i64, String);

/// JSON-RPC's and the protocol's error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const NOT_INITIALIZED: i64 = -32002;
const REQUEST_FAILED: i64 = -32803;

/// An open note: the editor's text, which may not be saved.
struct Open {
    text: String,
    version: i64,
}

struct Server {
    /// The vault's folder, absolute as given: the editor's paths are
    /// written through it, symlinks and all.
    root: PathBuf,
    /// The same, with symlinks resolved, for a path the editor resolved.
    canonical: PathBuf,
    hidden: ExcludedFiles,
    index: VaultIndex,
    /// The open notes, by vault path.
    open: HashMap<String, Open>,
    /// The last `check` findings of each open note, as diagnostics.
    checked: HashMap<String, Vec<Value>>,
    lint: LintConfig,
    /// Whether the editor applies a rename of a file in a workspace edit.
    renames_files: bool,
    /// Whether the editor watches files for the server when asked.
    watches: bool,
    initialized: bool,
    shut_down: bool,
    exited: bool,
}

/// Serves until the editor says `exit` or closes stdin.
pub fn run(root: &Path, show_hidden: bool) -> Result<(), String> {
    let hidden = ExcludedFiles::from_config(root, &crate::config(), show_hidden);
    let mut server = Server::new(root, hidden);
    serve(&mut server, &mut std::io::stdin().lock(), &mut std::io::stdout().lock())
}

fn serve(server: &mut Server, input: &mut impl BufRead, output: &mut impl Write) -> Result<(), String> {
    while let Some(body) = read_message(input)? {
        for message in server.handle(&body) {
            let text = message.to_string();
            write!(output, "Content-Length: {}\r\n\r\n{text}", text.len()).and_then(|()| output.flush()).map_err(|e| e.to_string())?;
        }
        if server.exited {
            break;
        }
    }
    Ok(())
}

/// The body of the next message, or none at the end of the input.
fn read_message(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut length: Option<usize> = None;
    loop {
        let mut header = String::new();
        if input.read_line(&mut header).map_err(|e| e.to_string())? == 0 {
            return Ok(None);
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse().map_err(|_| format!("a bad header: {header}"))?);
        }
    }
    let mut body = vec![0; length.unwrap_or_default()];
    input.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Some(body))
}

fn response(id: &Value, result: Result<Value, Failure>) -> Value {
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    }
}

fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

fn failed(message: impl Into<String>) -> Failure {
    (REQUEST_FAILED, message.into())
}

/// A position's line and UTF-16 character.
fn position_of(params: &Value) -> Result<(usize, usize), Failure> {
    let at = &params["position"];
    match (at["line"].as_u64(), at["character"].as_u64()) {
        (Some(line), Some(character)) => {
            Ok((usize::try_from(line).unwrap_or(usize::MAX), usize::try_from(character).unwrap_or(usize::MAX)))
        }
        _ => Err((INVALID_PARAMS, "no position".into())),
    }
}

impl Server {
    fn new(root: &Path, hidden: ExcludedFiles) -> Server {
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        Server {
            root,
            canonical,
            hidden,
            index: VaultIndex::default(),
            open: HashMap::new(),
            checked: HashMap::new(),
            lint: LintConfig::all(),
            renames_files: false,
            watches: false,
            initialized: false,
            shut_down: false,
            exited: false,
        }
    }

    /// What the server sends for one message: a response to a request, and
    /// notifications (diagnostics) or requests of its own.
    fn handle(&mut self, body: &[u8]) -> Vec<Value> {
        let message: Value = match serde_json::from_slice(body) {
            Ok(message) => message,
            Err(error) => return vec![response(&Value::Null, Err((PARSE_ERROR, format!("not JSON: {error}"))))],
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // The editor's answer to a request of the server's.
            return Vec::new();
        };
        let Some(id) = message.get("id") else { return self.notified(method, &params) };
        let result = if method == "initialize" {
            Ok(self.initialize(&params))
        } else if !self.initialized {
            Err((NOT_INITIALIZED, "initialize first".to_string()))
        } else if self.shut_down {
            Err((INVALID_REQUEST, "the server was shut down".to_string()))
        } else {
            self.request(method, &params)
        };
        vec![response(id, result)]
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let capabilities = &params["capabilities"];
        let edit = &capabilities["workspace"]["workspaceEdit"];
        // Resource operations come only in `documentChanges`, so an editor
        // that has them takes those too (Neovim does not say so).
        self.renames_files = edit["resourceOperations"].as_array().is_some_and(|ops| ops.iter().any(|op| op == "rename"));
        self.watches = capabilities["workspace"]["didChangeWatchedFiles"]["dynamicRegistration"].as_bool() == Some(true);
        self.reload();
        self.initialized = true;
        json!({
            "capabilities": {
                "positionEncoding": "utf-16",
                "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
                "documentFormattingProvider": true,
                "completionProvider": { "triggerCharacters": ["[", "#"] },
                "definitionProvider": true,
                "referencesProvider": true,
                "hoverProvider": true,
                "renameProvider": { "prepareProvider": true },
            },
            "serverInfo": { "name": "daftar", "version": env!("CARGO_PKG_VERSION") },
        })
    }

    /// The index and the lint's config, read again.
    fn reload(&mut self) {
        self.index = Vault::index(&self.root, None);
        self.lint = crate::lint::config(&self.root).unwrap_or_else(|error| {
            eprintln!("daftar-cli lsp: {error}");
            LintConfig::all()
        });
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, Failure> {
        match method {
            "shutdown" => {
                self.shut_down = true;
                Ok(Value::Null)
            }
            "textDocument/formatting" => self.formatting(params),
            "textDocument/completion" => self.completion(params),
            "textDocument/definition" => self.definition(params),
            "textDocument/references" => self.references(params),
            "textDocument/hover" => self.hover(params),
            "textDocument/prepareRename" => self.prepare_rename(params),
            "textDocument/rename" => self.rename(params),
            _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
        }
    }

    fn notified(&mut self, method: &str, params: &Value) -> Vec<Value> {
        let document = &params["textDocument"];
        let path = document["uri"].as_str().and_then(|uri| self.path_of(uri));
        match (method, path) {
            ("exit", _) => {
                self.exited = true;
                Vec::new()
            }
            ("initialized", _) if self.watches => vec![json!({
                "jsonrpc": "2.0",
                "id": "daftar-watch",
                "method": "client/registerCapability",
                "params": { "registrations": [{
                    "id": "daftar-files",
                    "method": "workspace/didChangeWatchedFiles",
                    "registerOptions": { "watchers": [{ "globPattern": "**/*" }] },
                }] },
            })],
            ("textDocument/didOpen", Some(path)) if Vault::is_note(&path) => {
                let text = document["text"].as_str().unwrap_or_default().to_string();
                self.open.insert(path.clone(), Open { text, version: document["version"].as_i64().unwrap_or(0) });
                let found = self.check(&path);
                self.checked.insert(path.clone(), found);
                vec![self.diagnostics(&path)]
            }
            ("textDocument/didChange", Some(path)) => {
                let Some(open) = self.open.get_mut(&path) else { return Vec::new() };
                // Full sync: the last change holds the whole text.
                if let Some(text) = params["contentChanges"].as_array().and_then(|c| c.last()).and_then(|c| c["text"].as_str()) {
                    open.text = text.to_string();
                }
                open.version = document["version"].as_i64().unwrap_or(open.version);
                vec![self.diagnostics(&path)]
            }
            ("textDocument/didSave", Some(path)) if self.open.contains_key(&path) => {
                self.changed(&BTreeSet::from([path.clone()]));
                let found = self.check(&path);
                self.checked.insert(path.clone(), found);
                vec![self.diagnostics(&path)]
            }
            ("textDocument/didSave", Some(path)) => {
                self.changed(&BTreeSet::from([path]));
                Vec::new()
            }
            ("textDocument/didClose", Some(path)) if self.open.remove(&path).is_some() => {
                self.checked.remove(&path);
                vec![notification("textDocument/publishDiagnostics", json!({ "uri": self.uri_of(&path), "diagnostics": [] }))]
            }
            ("workspace/didChangeWatchedFiles", _) => {
                let changed: BTreeSet<String> = params["changes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["uri"].as_str().and_then(|u| self.path_of(u)))
                    .collect();
                self.changed(&changed);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The index after [changed] changed on disk; the lint's config too
    /// when it is one of them.
    fn changed(&mut self, changed: &BTreeSet<String>) {
        if changed.is_empty() {
            return;
        }
        if changed.iter().any(|p| p.starts_with(".markdownlint")) {
            self.reload();
        } else {
            Vault::update_in_place(&mut self.index, changed, &self.root, None);
        }
    }

    /// The vault path of a `file:` URI in the vault.
    fn path_of(&self, uri: &str) -> Option<String> {
        let path = PathBuf::from(file_of_uri(uri)?);
        let relative =
            path.strip_prefix(&self.root).or_else(|_| path.strip_prefix(&self.canonical)).map(Path::to_path_buf).ok().or_else(|| {
                // A file the editor reached through a symlink, or that is not there yet.
                let parent = path.parent()?.canonicalize().ok()?;
                Some(parent.strip_prefix(&self.canonical).ok()?.join(path.file_name()?))
            })?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        (!relative.is_empty() && !relative.split('/').any(|p| p == "..")).then_some(relative)
    }

    fn uri_of(&self, path: &str) -> String {
        uri_of_file(&self.root.join(path))
    }

    /// The note at [path]: the editor's text when it is open, else the file's.
    fn text(&self, path: &str) -> Option<String> {
        match self.open.get(path) {
            Some(open) => Some(open.text.clone()),
            None => fs::read_to_string(self.root.join(path)).ok(),
        }
    }

    /// The note a request is about, which must be open, and its text.
    fn document(&self, params: &Value) -> Result<(String, String), Failure> {
        let uri = params["textDocument"]["uri"].as_str().ok_or_else(|| (INVALID_PARAMS, "no document".to_string()))?;
        let path = self.path_of(uri).ok_or_else(|| failed(format!("{uri} is not in the vault")))?;
        let text = self.text(&path).ok_or_else(|| failed(format!("{path} cannot be read")))?;
        Ok((path, text))
    }

    /// The link at the request's position, and the note it goes to.
    fn link_at(&self, path: &str, text: &str, params: &Value) -> Result<Option<(usize, links::Found, Option<String>)>, Failure> {
        let (line, character) = position_of(params)?;
        let content = position::line(text, line);
        let Some(found) = links::at(content, position::byte_of_utf16(content, character)) else { return Ok(None) };
        let to = self.index.resolve(&found.target, path);
        Ok(Some((line, found, to)))
    }

    // Diagnostics.

    /// The lint's findings in the open note's text, and the last `check`
    /// of the note as saved.
    fn diagnostics(&self, path: &str) -> Value {
        let Some(open) = self.open.get(path) else { return Value::Null };
        let mut diagnostics = Vec::new();
        if !crate::lint::left_out(&self.root, &[], Some(".markdownlintignore"))(path) {
            for problem in MarkdownLint::lint(&open.text, &self.lint) {
                let line = problem.line.saturating_sub(1);
                let content = position::line(&open.text, line);
                let start = problem.column.map_or(0, |c| position::utf16_of_chars(content, c.saturating_sub(1)));
                let detail = if problem.detail.is_empty() { String::new() } else { format!(" {}", problem.detail) };
                diagnostics.push(json!({
                    "range": position::on_line(line, start, position::utf16_len(content).max(start)),
                    "severity": 1,
                    "source": "daftar lint",
                    "code": format!("{}/{}", problem.rule, problem.name()),
                    "message": format!("{}{detail}", problem.description()),
                }));
            }
        }
        diagnostics.extend(self.checked.get(path).into_iter().flatten().cloned());
        notification(
            "textDocument/publishDiagnostics",
            json!({ "uri": self.uri_of(path), "version": open.version, "diagnostics": diagnostics }),
        )
    }

    /// What `check` finds in the note at [path] as saved, with the vault's
    /// options (`.daftar.toml`), as diagnostics on the lines of the open text.
    fn check(&self, path: &str) -> Vec<Value> {
        if self.hidden.contains(path) || !self.index.notes.contains_key(path) {
            return Vec::new();
        }
        let vault = check::VaultOptions::read(&self.root, None).unwrap_or_default();
        let options = check::Options {
            strict: false,
            no_h1: vault.no_h1,
            callouts: vault.callouts,
            urls: false,
            unreachable: false,
            github_token: None,
            skip_urls: Vec::new(),
            archive: false,
            figures: false,
            mode: report::Mode::Json,
            only: Some(HashSet::from([path.to_string()])),
            fix: false,
            dry_run: false,
            memory: vault.memory.then(|| crate::memory::Rules {
                event_properties: vault.event_properties,
                prep_notes: vault.prep_notes,
                active: vault.active.iter().filter_map(|a| crate::memory::Rules::active_pair(a)).collect(),
                quiet_days: vault.quiet_days,
            }),
            orphans: vault.orphans.then(|| vault.orphan_skip.clone()),
        };
        let mut lines = Vec::new();
        check::run_on(&self.index, &self.root, |p| self.hidden.contains(p), &options, |line| lines.push(line));
        let text = self.text(path).unwrap_or_default();
        lines
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .map(|finding| {
                let line = usize::try_from(finding["line"].as_u64().unwrap_or(0)).unwrap_or(0).saturating_sub(1);
                let severity = if finding["level"] == "error" { 1 } else { 2 };
                json!({
                    "range": position::on_line(line, 0, position::utf16_len(position::line(&text, line))),
                    "severity": severity,
                    "source": "daftar check",
                    "code": finding["code"],
                    "message": finding["message"],
                })
            })
            .collect()
    }

    // Formatting.

    fn formatting(&self, params: &Value) -> Result<Value, Failure> {
        let (path, text) = self.document(params)?;
        let formatted = crate::fmt::formatted(&self.root, &text, Some(&path), &[]).map_err(failed)?;
        Ok(Value::Array(position::line_edit(&text, &formatted).into_iter().collect()))
    }

    // Completion.

    fn completion(&self, params: &Value) -> Result<Value, Failure> {
        let (path, text) = self.document(params)?;
        let (line, character) = position_of(params)?;
        let content = position::line(&text, line);
        let cursor = position::byte_of_utf16(content, character);
        let Some(typing) = links::typing(&content[..cursor]) else { return Ok(json!({ "isIncomplete": false, "items": [] })) };
        let range = |start: usize| position::on_line(line, position::utf16_of_byte(content, start), character);
        let items: Vec<Value> = match typing {
            Typing::Note { query, start } => self
                .notes_for(&query)
                .iter()
                .enumerate()
                .map(|(rank, (target, written))| {
                    json!({
                        "label": Vault::title(target),
                        "kind": 17,
                        "detail": target,
                        "filterText": query,
                        "sortText": format!("{rank:04}"),
                        "textEdit": { "range": range(start), "newText": written },
                    })
                })
                .collect(),
            Typing::Heading { note, query, start } => {
                let Some(to) = self.index.resolve(&note, &path) else { return Ok(json!({ "isIncomplete": false, "items": [] })) };
                let headings = if to == path { Sections::headings(&text) } else { Sections::headings(&self.text(&to).unwrap_or_default()) };
                let wanted = slugs::fold(&query);
                headings
                    .iter()
                    .filter(|h| slugs::fold(&h.text).contains(&wanted))
                    .enumerate()
                    .map(|(rank, h)| {
                        json!({
                            "label": h.text,
                            "kind": 18,
                            "detail": format!("{} {}", "#".repeat(h.level), to),
                            "filterText": query,
                            "sortText": format!("{rank:04}"),
                            "textEdit": { "range": range(start), "newText": h.text },
                        })
                    })
                    .collect()
            }
        };
        Ok(json!({ "isIncomplete": true, "items": items }))
    }

    /// The files [query] finds, as the quick switcher ranks them (the notes
    /// changed last when it is empty), each with how a link names it: its
    /// name when no other file has it, else its path, without `.md`.
    fn notes_for(&self, query: &str) -> Vec<(String, String)> {
        let files: Vec<&String> = self.index.files.iter().filter(|p| !self.hidden.contains(p)).collect();
        let candidates: Vec<Candidate> = files
            .iter()
            .map(|path| {
                let aliases = self.index.notes.get(*path).map_or_default(|n| n.parsed.aliases.clone());
                Candidate::new(path, &Vault::title(path), &aliases)
            })
            .collect();
        let mut recent: Vec<&daftar_core::NoteEntry> = self.index.notes.values().filter(|n| !self.hidden.contains(&n.path)).collect();
        recent.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
        let recent: Vec<String> = recent.into_iter().take(COMPLETIONS).map(|n| n.path.clone()).collect();
        let mut names: HashMap<String, usize> = HashMap::new();
        for path in &self.index.files {
            *names.entry(slugs::fold(&Vault::title(path))).or_default() += 1;
        }
        QuickSwitcher::rank(query, &candidates, &recent, COMPLETIONS)
            .into_iter()
            .map(|path| {
                let title = Vault::title(&path);
                let written =
                    if names.get(&slugs::fold(&title)) == Some(&1) { title } else { path.strip_suffix(".md").unwrap_or(&path).to_string() };
                (path, written)
            })
            .collect()
    }

    // Links.

    /// The line the heading or `^block` [anchor] is on in [text].
    fn anchor_line(text: &str, anchor: &str) -> Option<usize> {
        if let Some(id) = anchor.strip_prefix('^') {
            let mark = format!("^{id}");
            return text.split('\n').position(|l| {
                let l = l.trim_end();
                l == mark || l.ends_with(&format!(" {mark}"))
            });
        }
        // `[[Note#Part#Under it]]` is the last heading named.
        let anchor = anchor.rsplit('#').next().unwrap_or(anchor);
        let (wanted, slugged) = (check::plain(anchor), anchor.to_lowercase());
        Sections::headings(text).into_iter().find(|h| check::plain(&h.text) == wanted || check::slug(&h.text) == slugged).map(|h| h.line)
    }

    fn definition(&self, params: &Value) -> Result<Value, Failure> {
        let (path, text) = self.document(params)?;
        let Some((_, found, Some(to))) = self.link_at(&path, &text, params)? else { return Ok(Value::Null) };
        let line = found.anchor.as_deref().and_then(|a| Self::anchor_line(&self.text(&to).unwrap_or_default(), a)).unwrap_or(0);
        Ok(json!({ "uri": self.uri_of(&to), "range": position::on_line(line, 0, 0) }))
    }

    fn hover(&self, params: &Value) -> Result<Value, Failure> {
        let (path, text) = self.document(params)?;
        let Some((line, found, Some(to))) = self.link_at(&path, &text, params)? else { return Ok(Value::Null) };
        let content = position::line(&text, line);
        let range =
            position::on_line(line, position::utf16_of_byte(content, found.whole.start), position::utf16_of_byte(content, found.whole.end));
        if !Vault::is_note(&to) {
            return Ok(json!({ "contents": { "kind": "markdown", "value": format!("`{to}`") }, "range": range }));
        }
        let target = self.text(&to).unwrap_or_default();
        let (lines, _) = Sections::lines(&target);
        let anchored = found.anchor.as_deref().and_then(|a| Self::anchor_line(&target, a));
        let (start, end) = match anchored {
            Some(at) if !found.anchor.as_deref().is_some_and(|a| a.starts_with('^')) => {
                let headings = Sections::headings(&target);
                let index = headings.iter().position(|h| h.line == at).unwrap_or(0);
                (at, Sections::end(index, &headings, lines.len()))
            }
            Some(at) => (at, at + 1),
            // The note with no front matter.
            None => (daftar_core::FrontMatter::parse(&lines).line_count, lines.len()),
        };
        let shown: Vec<&str> = lines.get(start..end.min(start + HOVER_LINES)).unwrap_or_default().to_vec();
        let more = if end > start + HOVER_LINES { "\n\n…" } else { "" };
        Ok(json!({
            "contents": { "kind": "markdown", "value": format!("`{to}`\n\n---\n\n{}{more}", shown.join("\n").trim()) },
            "range": range,
        }))
    }

    /// The links to the note at the cursor's link, else to this note.
    fn references(&self, params: &Value) -> Result<Value, Failure> {
        let (path, text) = self.document(params)?;
        let to = match self.link_at(&path, &text, params)? {
            Some((_, _, Some(to))) => to,
            _ => path.clone(),
        };
        let mut locations = Vec::new();
        if params["context"]["includeDeclaration"].as_bool() == Some(true) {
            locations.push(json!({ "uri": self.uri_of(&to), "range": position::on_line(0, 0, 0) }));
        }
        let sources: BTreeSet<&str> =
            self.index.backlinks(&to).iter().map(|b| b.source.as_str()).filter(|s| !self.hidden.contains(s)).collect();
        for source in sources {
            let Some(text) = self.text(source) else { continue };
            for (line, found) in links::in_text(&text) {
                if self.index.resolve(&found.target, source).as_deref() == Some(to.as_str()) {
                    let content = position::line(&text, line);
                    let (start, end) =
                        (position::utf16_of_byte(content, found.whole.start), position::utf16_of_byte(content, found.whole.end));
                    locations.push(json!({ "uri": self.uri_of(source), "range": position::on_line(line, start, end) }));
                }
            }
        }
        Ok(Value::Array(locations))
    }

    // Rename.

    /// The note a rename at the cursor renames: the one its link goes to,
    /// else this one; and the link's target, which the editor marks.
    fn renamed(&self, params: &Value) -> Result<(String, Value), Failure> {
        let (path, text) = self.document(params)?;
        let (line, character) = position_of(params)?;
        let (to, range) = match self.link_at(&path, &text, params)? {
            Some((line, found, Some(to))) if !found.target.is_empty() => {
                let content = position::line(&text, line);
                let (start, end) =
                    (position::utf16_of_byte(content, found.target_range.start), position::utf16_of_byte(content, found.target_range.end));
                (to, position::on_line(line, start, end))
            }
            Some((_, _, None)) => return Err(failed("this link goes to no note")),
            _ => (path, position::on_line(line, character, character)),
        };
        if !Vault::is_note(&to) || !self.index.files.contains(&to) {
            return Err(failed(format!("{to} is not a saved note of the vault")));
        }
        Ok((to, range))
    }

    fn prepare_rename(&self, params: &Value) -> Result<Value, Failure> {
        let (to, range) = self.renamed(params)?;
        Ok(json!({ "range": range, "placeholder": Vault::title(&to) }))
    }

    /// The note moves to [newName] (a name in its folder, or a path from
    /// the vault's folder), and every link to it is rewritten, as the
    /// app's Rename does: one workspace edit, a file renamed and the notes
    /// changed, which the editor applies.
    fn rename(&self, params: &Value) -> Result<Value, Failure> {
        if !self.renames_files {
            return Err(failed("renaming a note needs an editor that renames files in a workspace edit (resourceOperations: rename)"));
        }
        let (from, _) = self.renamed(params)?;
        let name = params["newName"].as_str().unwrap_or_default();
        let to = destination(&from, name)?;
        let moves = VaultMoves::plan(&from, &to, &self.index.files).map_err(|e| failed(e.to_string()))?;
        if moves.is_empty() {
            return Ok(json!({ "documentChanges": [] }));
        }
        let relinked = VaultMoves::relinked(&self.index, &moves, |path| self.text(path));
        let mut changes = vec![json!({ "kind": "rename", "oldUri": self.uri_of(&from), "newUri": self.uri_of(&to) })];
        for (now, new) in &relinked {
            let was = moves.iter().find(|(_, n)| *n == now).map_or(now.as_str(), |(o, _)| o.as_str());
            let Some(old) = self.text(was) else { continue };
            let version = self.open.get(was).map_or(Value::Null, |o| json!(o.version));
            changes.push(json!({
                "textDocument": { "uri": self.uri_of(now), "version": version },
                "edits": position::line_edit(&old, new).into_iter().collect::<Vec<_>>(),
            }));
        }
        let mut bookmarks = Bookmarks::load(&self.root);
        let file = ".obsidian/bookmarks.json";
        if bookmarks.follow(&moves, None)
            && let (Ok(new), Ok(old)) = (bookmarks.text(), fs::read_to_string(self.root.join(file)))
            && let Some(edit) = position::line_edit(&old, &new)
        {
            changes.push(json!({ "textDocument": { "uri": self.uri_of(file), "version": null }, "edits": [edit] }));
        }
        Ok(json!({ "documentChanges": changes }))
    }
}

/// Where the note at [from] goes when it is renamed [name]: a name stays in
/// its folder, a path with `/` is from the vault's folder; `.md` is added.
fn destination(from: &str, name: &str) -> Result<String, Failure> {
    let name = name.trim().trim_start_matches("./").trim_start_matches('/');
    let (folder, last) = match name.rsplit_once('/') {
        Some((folder, last)) => (folder.to_string(), last),
        None => (from.rsplit_once('/').map_or_default(|(f, _)| f.to_string()), name),
    };
    let stem = last.strip_suffix(".md").unwrap_or(last);
    if !VaultMoves::is_valid_name(stem) || folder.split('/').any(|p| p == "." || p == ".." || p.starts_with('.')) {
        return Err((INVALID_PARAMS, format!("“{name}” cannot be a note's name: it is empty or has one of / \\ : # ^ [ ] |")));
    }
    let file = VaultMoves::note_name(stem);
    Ok(if folder.is_empty() { file } else { format!("{folder}/{file}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in [
            (".markdownlint.jsonc", "{ \"default\": true, \"MD013\": false }\n"),
            (
                ".obsidian/bookmarks.json",
                "{\n  \"items\": [\n    {\n      \"type\": \"file\",\n      \"path\": \"Garden/Plants.md\"\n    }\n  ]\n}",
            ),
            ("Garden/Plants.md", "# Plants\n\nSee [[Seeds]].\n\n## Basil\n\nNeeds sun. ^sun\n\n## Mint\n\nGrows fast.\n"),
            ("Seeds.md", "# Seeds\n\nTomato seeds from [[Plants#Basil]] and [[Garden/Plants|the garden]].\n"),
            ("Persian.md", "# یادداشت\n\nسلام 😀 [[Plants#Mint]] و [[Seeds]].\n"),
            ("Projects/Seeds.md", "# Other seeds\n"),
        ] {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        root
    }

    struct Client {
        server: Server,
        next: i64,
    }

    impl Client {
        fn new(root: &Path, renames_files: bool) -> Client {
            let mut client = Client { server: Server::new(root, ExcludedFiles::new::<&str>(&[])), next: 0 };
            let operations = if renames_files { json!(["create", "rename", "delete"]) } else { json!([]) };
            let capabilities = json!({ "workspace": { "workspaceEdit": { "resourceOperations": operations } } });
            let answer = client.request("initialize", json!({ "capabilities": capabilities }));
            assert_eq!(answer["result"]["capabilities"]["positionEncoding"], "utf-16");
            assert!(client.notify("initialized", json!({})).is_empty());
            client
        }

        fn request(&mut self, method: &str, params: Value) -> Value {
            self.next += 1;
            let body = json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params }).to_string();
            let mut out = self.server.handle(body.as_bytes());
            assert_eq!(out.len(), 1, "{out:?}");
            out.remove(0)
        }

        fn result(&mut self, method: &str, params: Value) -> Value {
            let answer = self.request(method, params);
            assert!(answer.get("error").is_none(), "{answer}");
            answer["result"].clone()
        }

        fn notify(&mut self, method: &str, params: Value) -> Vec<Value> {
            self.server.handle(json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string().as_bytes())
        }

        fn uri(&self, path: &str) -> String {
            self.server.uri_of(path)
        }

        /// Opens [path] with [text], else its file's, and returns the diagnostics.
        fn open(&mut self, path: &str, text: Option<&str>) -> Value {
            let text = text.map_or_else(|| fs::read_to_string(self.server.root.join(path)).unwrap(), String::from);
            let uri = self.uri(path);
            let mut out = self.notify(
                "textDocument/didOpen",
                json!({ "textDocument": { "uri": uri, "languageId": "markdown", "version": 1, "text": text } }),
            );
            assert_eq!(out.len(), 1);
            assert_eq!(out[0]["method"], "textDocument/publishDiagnostics");
            out.remove(0)["params"].clone()
        }

        fn at(&mut self, method: &str, path: &str, line: usize, character: usize) -> Value {
            let uri = self.uri(path);
            self.result(method, json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character }, "context": { "includeDeclaration": false } }))
        }
    }

    fn codes(diagnostics: &Value) -> Vec<(String, String, u64, u64)> {
        diagnostics["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["source"].as_str().unwrap().to_string(),
                    d["code"].as_str().unwrap().to_string(),
                    d["range"]["start"]["line"].as_u64().unwrap(),
                    d["range"]["start"]["character"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn it_frames_messages_and_stops_at_exit() {
        let dir = vault();
        let mut server = Server::new(dir.path(), ExcludedFiles::new::<&str>(&[]));
        let mut input = Vec::new();
        for message in [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }),
        ] {
            let body = message.to_string();
            input.extend(
                format!("Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}", body.len()).bytes(),
            );
        }
        let mut output = Vec::new();
        serve(&mut server, &mut input.as_slice(), &mut output).unwrap();
        let mut reader = output.as_slice();
        let first: Value = serde_json::from_slice(&read_message(&mut reader).unwrap().unwrap()).unwrap();
        assert_eq!(first["result"]["serverInfo"]["name"], "daftar");
        let second: Value = serde_json::from_slice(&read_message(&mut reader).unwrap().unwrap()).unwrap();
        assert_eq!(second, json!({ "jsonrpc": "2.0", "id": 2, "result": null }));
        // Nothing after exit is read.
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn a_request_before_initialize_or_after_shutdown_is_refused() {
        let dir = vault();
        let mut server = Server::new(dir.path(), ExcludedFiles::new::<&str>(&[]));
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "textDocument/hover", "params": {} }).to_string();
        assert_eq!(server.handle(body.as_bytes())[0]["error"]["code"], NOT_INITIALIZED);
        assert_eq!(server.handle(b"{")[0]["error"]["code"], PARSE_ERROR);
        let mut client = Client::new(dir.path(), true);
        assert_eq!(client.request("workspace/nothing", json!({}))["error"]["code"], METHOD_NOT_FOUND);
        client.result("shutdown", Value::Null);
        assert_eq!(client.request("textDocument/hover", json!({}))["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn it_asks_the_editor_to_watch_the_files_when_it_can() {
        let dir = vault();
        let mut server = Server::new(dir.path(), ExcludedFiles::new::<&str>(&[]));
        let capabilities = json!({ "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } } });
        server.handle(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": capabilities } }).to_string().as_bytes(),
        );
        let out = server.handle(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }).to_string().as_bytes());
        assert_eq!(out[0]["method"], "client/registerCapability");
        assert_eq!(out[0]["params"]["registrations"][0]["method"], "workspace/didChangeWatchedFiles");
        // The editor's answer is taken quietly.
        assert!(server.handle(json!({ "jsonrpc": "2.0", "id": "daftar-watch", "result": null }).to_string().as_bytes()).is_empty());
    }

    #[test]
    fn an_open_note_is_linted_as_typed_and_checked_as_saved() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        let text = "# Note\n\nسلام 😀 \n[[Nowhere]]\n";
        fs::write(dir.path().join("Note.md"), text).unwrap();
        client.notify("workspace/didChangeWatchedFiles", json!({ "changes": [{ "uri": client.uri("Note.md"), "type": 1 }] }));
        let found = client.open("Note.md", None);
        assert_eq!(found["uri"], client.uri("Note.md"));
        // The trailing space is after 4 letters, a space and the emoji's 2 units.
        assert_eq!(
            codes(&found),
            [
                ("daftar lint".to_string(), "MD009/no-trailing-spaces".to_string(), 2, 7),
                ("daftar check".to_string(), "dangling-link".to_string(), 3, 0),
            ]
        );
        let check = &found["diagnostics"][1];
        assert_eq!(check["severity"], 2);
        assert_eq!(check["message"], "[[Nowhere]] goes to no note");
        // A change is linted at once; the check stays until the save.
        let uri = client.uri("Note.md");
        let out = client.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "# Note\n\nسلام\n[[Nowhere]]\n" }] }),
        );
        assert_eq!(out[0]["params"]["version"], 2);
        assert_eq!(codes(&out[0]["params"]), [("daftar check".to_string(), "dangling-link".to_string(), 3, 0)]);
        fs::write(dir.path().join("Note.md"), "# Note\n\n[[Seeds]] [[Persian]]\n").unwrap();
        client.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": 3 }, "contentChanges": [{ "text": "# Note\n\n[[Seeds]] [[Persian]]\n" }] }),
        );
        let out = client.notify("textDocument/didSave", json!({ "textDocument": { "uri": uri } }));
        // Seeds names two notes.
        assert_eq!(codes(&out[0]["params"]), [("daftar check".to_string(), "ambiguous-link".to_string(), 2, 0)]);
        // The index learned the save: Note now links to Persian.
        assert!(client.server.index.backlinks("Persian.md").iter().any(|b| b.source == "Note.md"));
        let out = client.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri } }));
        assert_eq!(out[0]["params"]["diagnostics"], json!([]));
    }

    #[test]
    fn formatting_replaces_only_the_lines_that_change() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        client.open("Note.md", Some("# Note\n* one\n* two\n\nend\n"));
        let uri = client.uri("Note.md");
        let edits = client.result(
            "textDocument/formatting",
            json!({ "textDocument": { "uri": uri }, "options": { "tabSize": 2, "insertSpaces": true } }),
        );
        assert_eq!(edits, json!([{ "range": { "start": position::at(1, 0), "end": position::at(3, 0) }, "newText": "\n- one\n- two\n" }]));
        client.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "# Note\n\n- one\n" }] }),
        );
        assert_eq!(client.result("textDocument/formatting", json!({ "textDocument": { "uri": uri } })), json!([]));
    }

    #[test]
    fn completes_notes_as_the_switcher_ranks_them_and_headings() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        client.open("Note.md", Some("سلام 😀 [[pla\n[[Seeds#Ba\n[[Plants#\n[[#\n## Here\n"));
        let found = client.at("textDocument/completion", "Note.md", 0, 13);
        let items = found["items"].as_array().unwrap();
        assert_eq!(items[0]["label"], "Plants");
        assert_eq!(items[0]["textEdit"], json!({ "range": position::on_line(0, 10, 13), "newText": "Plants" }));
        // The name of two notes is written with its folder.
        let found = client.at("textDocument/completion", "Note.md", 1, 4);
        let written: Vec<&str> = found["items"].as_array().unwrap().iter().map(|i| i["textEdit"]["newText"].as_str().unwrap()).collect();
        assert!(written.contains(&"Projects/Seeds") && written.contains(&"Seeds"), "{written:?}");
        // [[Seeds#Ba: the headings of the note Seeds goes to, that hold Ba.
        let found = client.at("textDocument/completion", "Note.md", 1, 10);
        assert_eq!(found["items"], json!([]));
        let found = client.at("textDocument/completion", "Note.md", 2, 9);
        let labels: Vec<&str> = found["items"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["Plants", "Basil", "Mint"]);
        assert_eq!(found["items"][1]["textEdit"]["range"], position::on_line(2, 9, 9));
        // [[# is this note's, as typed.
        let found = client.at("textDocument/completion", "Note.md", 3, 3);
        assert_eq!(found["items"][0]["label"], "Here");
        // Not in a link.
        let found = client.at("textDocument/completion", "Note.md", 4, 3);
        assert_eq!(found["items"], json!([]));
    }

    #[test]
    fn a_link_goes_to_its_note_heading_or_block() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        client.open("Note.md", Some("سلام 😀 [[Plants#Mint]] [[Garden/Plants#^sun]] [[Plants]] [[Nowhere]] [x](Garden/Plants.md#basil)\n"));
        let plants = client.uri("Garden/Plants.md");
        // On the emoji's link, which starts at unit 8.
        assert_eq!(client.at("textDocument/definition", "Note.md", 0, 8), json!({ "uri": plants, "range": position::on_line(8, 0, 0) }));
        assert_eq!(client.at("textDocument/definition", "Note.md", 0, 30)["range"]["start"]["line"], 6);
        assert_eq!(client.at("textDocument/definition", "Note.md", 0, 50)["range"]["start"]["line"], 0);
        assert_eq!(client.at("textDocument/definition", "Note.md", 0, 62), Value::Null);
        assert_eq!(client.at("textDocument/definition", "Note.md", 0, 75)["range"]["start"]["line"], 4);
        assert_eq!(client.at("textDocument/definition", "Note.md", 0, 2), Value::Null);
    }

    #[test]
    fn hover_shows_the_note_or_its_section() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        client.open("Persian.md", None);
        let shown = client.at("textDocument/hover", "Persian.md", 2, 10);
        assert_eq!(shown["contents"]["value"], "`Garden/Plants.md`\n\n---\n\n## Mint\n\nGrows fast.");
        assert_eq!(shown["range"], position::on_line(2, 8, 23));
        let shown = client.at("textDocument/hover", "Persian.md", 2, 26);
        assert_eq!(
            shown["contents"]["value"],
            "`Seeds.md`\n\n---\n\n# Seeds\n\nTomato seeds from [[Plants#Basil]] and [[Garden/Plants|the garden]]."
        );
    }

    #[test]
    fn references_are_every_link_to_the_note() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        client.open("Garden/Plants.md", None);
        let found = client.at("textDocument/references", "Garden/Plants.md", 0, 0);
        let places: Vec<(String, Value)> =
            found.as_array().unwrap().iter().map(|l| (l["uri"].as_str().unwrap().to_string(), l["range"].clone())).collect();
        assert_eq!(
            places,
            [
                (client.uri("Persian.md"), position::on_line(2, 8, 23)),
                (client.uri("Seeds.md"), position::on_line(2, 18, 34)),
                (client.uri("Seeds.md"), position::on_line(2, 39, 67)),
            ]
        );
        // On a link: the links to its note.
        let found = client.at("textDocument/references", "Garden/Plants.md", 2, 6);
        assert_eq!(found.as_array().unwrap().len(), 2, "{found}");
    }

    #[test]
    fn rename_moves_the_note_and_rewrites_every_link_as_one_edit() {
        let dir = vault();
        let mut client = Client::new(dir.path(), true);
        client.open("Persian.md", None);
        let prepared = client.at("textDocument/prepareRename", "Persian.md", 2, 12);
        assert_eq!(prepared, json!({ "range": position::on_line(2, 10, 16), "placeholder": "Plants" }));
        let uri = client.uri("Persian.md");
        let edit = client.result(
            "textDocument/rename",
            json!({ "textDocument": { "uri": uri }, "position": { "line": 2, "character": 12 }, "newName": "Herbs" }),
        );
        let changes = edit["documentChanges"].as_array().unwrap();
        assert_eq!(
            changes[0],
            json!({ "kind": "rename", "oldUri": client.uri("Garden/Plants.md"), "newUri": client.uri("Garden/Herbs.md") })
        );
        let by_uri: HashMap<String, &Value> =
            changes[1..].iter().map(|c| (c["textDocument"]["uri"].as_str().unwrap().to_string(), c)).collect();
        // The open note's edit is on its version, its text the editor's.
        assert_eq!(by_uri[&uri]["textDocument"]["version"], 1);
        assert_eq!(by_uri[&uri]["edits"][0]["newText"], "سلام 😀 [[Herbs#Mint]] و [[Seeds]].\n");
        assert_eq!(
            by_uri[&client.uri("Seeds.md")]["edits"][0]["newText"],
            "Tomato seeds from [[Herbs#Basil]] and [[Garden/Herbs|the garden]].\n"
        );
        assert_eq!(by_uri[&client.uri("Seeds.md")]["textDocument"]["version"], Value::Null);
        assert!(by_uri[&client.uri(".obsidian/bookmarks.json")]["edits"][0]["newText"].as_str().unwrap().contains("Garden/Herbs.md"));
        // Nothing was written: the editor applies the edit.
        assert!(dir.path().join("Garden/Plants.md").exists() && !dir.path().join("Garden/Herbs.md").exists());
        // Anywhere in a note renames that note; a path moves it.
        let prepared = client.at("textDocument/prepareRename", "Persian.md", 0, 0);
        assert_eq!(prepared["placeholder"], "Persian");
        let edit = client.result(
            "textDocument/rename",
            json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 0 }, "newName": "Notes/Farsi" }),
        );
        assert_eq!(edit["documentChanges"][0]["newUri"], client.uri("Notes/Farsi.md"));
        // A name a link cannot hold, and a name taken, are refused.
        for name in ["Bad#Name", "Seeds"] {
            let answer = client.request(
                "textDocument/rename",
                json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 0 }, "newName": name }),
            );
            assert!(answer["error"]["message"].is_string(), "{answer}");
        }
    }

    #[test]
    fn rename_is_refused_by_an_editor_that_cannot_rename_files() {
        let dir = vault();
        let mut client = Client::new(dir.path(), false);
        client.open("Seeds.md", None);
        let uri = client.uri("Seeds.md");
        let answer = client.request(
            "textDocument/rename",
            json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 0 }, "newName": "Grains" }),
        );
        assert_eq!(answer["error"]["code"], REQUEST_FAILED);
        assert!(answer["error"]["message"].as_str().unwrap().contains("resourceOperations"));
    }

    #[test]
    fn a_uri_is_a_vault_path_only_inside_the_vault() {
        let dir = vault();
        let server = Server::new(dir.path(), ExcludedFiles::new::<&str>(&[]));
        let uri = server.uri_of("Garden/یادداشت تازه.md");
        assert!(uri.starts_with("file:///") && !uri.contains(' '), "{uri}");
        assert_eq!(server.path_of(&uri).as_deref(), Some("Garden/یادداشت تازه.md"));
        assert_eq!(server.path_of("file:///etc/passwd"), None);
        assert_eq!(server.path_of("untitled:1"), None);
        let canonical = uri_of_file(&dir.path().canonicalize().unwrap().join("Seeds.md"));
        assert_eq!(server.path_of(&canonical).as_deref(), Some("Seeds.md"));
    }

    /// A Windows path is `file:///C:/…`, whichever way an editor writes the colon.
    #[test]
    fn a_windows_path_is_a_uri_with_its_drive() {
        assert_eq!(uri_of_file(Path::new(r"C:\Users\a b\x.md")), "file:///C:/Users/a%20b/x.md");
        assert_eq!(uri_of_file(Path::new(r"\\?\C:\v\x.md")), "file:///C:/v/x.md");
        assert_eq!(file_of_uri("file:///C:/Users/a%20b/x.md").as_deref(), Some("C:/Users/a b/x.md"));
        assert_eq!(file_of_uri("file:///c%3A/v/x.md").as_deref(), Some("c:/v/x.md"));
        assert_eq!(uri_of_file(Path::new("/home/a:b/x.md")), "file:///home/a%3Ab/x.md");
        assert_eq!(file_of_uri("file:///home/a%3Ab/x.md").as_deref(), Some("/home/a:b/x.md"));
        assert_eq!(file_of_uri("untitled:1"), None);
    }

    #[test]
    fn a_new_name_is_in_the_notes_folder_or_a_path() {
        assert_eq!(destination("Garden/Plants.md", "Herbs").unwrap(), "Garden/Herbs.md");
        assert_eq!(destination("Garden/Plants.md", "Herbs.md").unwrap(), "Garden/Herbs.md");
        assert_eq!(destination("Plants.md", "Kitchen/Herbs").unwrap(), "Kitchen/Herbs.md");
        assert_eq!(destination("Plants.md", "گیاهان").unwrap(), "گیاهان.md");
        assert!(destination("Plants.md", "").is_err());
        assert!(destination("Plants.md", "../Out").is_err());
        assert!(destination("Plants.md", "a|b").is_err());
    }
}
