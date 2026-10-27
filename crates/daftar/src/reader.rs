//! The reading view: the note rendered by `web/src` in a WebKitGTK view,
//! the same renderer Daftar for macOS uses.
//!
//! The page is served from two URI schemes and nothing else: `daftar:` for
//! the reader's own files, built into the binary, and `vault:` for the
//! vault's files, which is how an embedded image is loaded. The page cannot
//! reach the network or the rest of the disk.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Once;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use include_dir::{Dir, include_dir};
use serde_json::{Value, json};
use webkit6::prelude::*;
use webkit6::{NavigationPolicyDecision, PolicyDecisionType, URISchemeRequest, URISchemeResponse, WebView};

use daftar_core::{
    Base, BaseCell, BaseEngine, BaseImage, BaseLayout, Canvas, Day, Detected, NoteParser, QueryTask, TasksQuery, Vault, VaultIndex,
};

use crate::settings::AppSettings;
use crate::store::{VaultStore, file_in};

/// `web/src`, bundled by `just web`.
static READER: Dir = include_dir!("$CARGO_MANIFEST_DIR/reader");

/// Front matter the reader does not print: the plugins' own keys.
const HIDDEN: [&str; 5] = ["cssclasses", "cssclass", "icon", "iconColor", "position"];

/// The schemes a link may leave the app by: the web, mail, and the phone's
/// call, message and map. Never `file:`, `smb:` or an app's own scheme.
const OUTSIDE: [&str; 6] = ["http", "https", "mailto", "tel", "sms", "geo"];

/// What a detected address or phone number opens: OpenStreetMap's search for
/// the address, `tel:` with the number's digits.
fn detected_url(kind: &str, value: &str) -> Option<String> {
    match kind {
        "address" if !value.trim().is_empty() => {
            Some(format!("https://www.openstreetmap.org/search?query={}", glib::Uri::escape_string(value.trim(), None, false)))
        }
        "phone" => {
            let digits: String = value.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
            (digits.chars().filter(char::is_ascii_digit).count() >= 3).then(|| format!("tel:{digits}"))
        }
        _ => None,
    }
}

thread_local! {
    /// The open vault's folder, for the `vault:` scheme. The schemes belong
    /// to the web context, which every window shares.
    static VAULT_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// What the reader should show: a note, and where in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub path: String,
    pub heading: Option<String>,
    pub line: Option<usize>,
}

/// Where a link the page posts opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenIn {
    Here,
    NewTab,
    OtherPane,
}

impl OpenIn {
    /// A Ctrl or ⌘ click (or a middle click) is a new tab, an Alt click the
    /// other pane of a split.
    pub fn from_click(new_tab: bool, other_pane: bool) -> OpenIn {
        if new_tab {
            OpenIn::NewTab
        } else if other_pane {
            OpenIn::OtherPane
        } else {
            OpenIn::Here
        }
    }
}

// Shared, so a handler is taken out of the state before it runs: one that
// shows a note borrows the state again.
type OpenHandler = Rc<dyn Fn(&str, Option<&str>, OpenIn)>;
type NoticeHandler = Rc<dyn Fn(&str)>;
type EditTaskHandler = Rc<dyn Fn(&str, usize)>;
type CursorHandler = Rc<dyn Fn(&str, usize)>;

struct State {
    ready: bool,
    shown: Option<Request>,
    /// How many times a note was asked for: a base run for an older ask is dropped.
    shows: u64,
    store: Option<Rc<VaultStore>>,
    on_open: Option<OpenHandler>,
    on_notice: Option<NoticeHandler>,
    on_edit_task: Option<EditTaskHandler>,
    on_cursor: Option<CursorHandler>,
}

#[derive(Clone)]
pub struct Reader {
    pub view: WebView,
    settings: Rc<RefCell<AppSettings>>,
    state: Rc<RefCell<State>>,
}

impl Reader {
    pub fn new(settings: Rc<RefCell<AppSettings>>) -> Reader {
        register_schemes();
        let view = WebView::new();
        view.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
        view.set_vexpand(true);
        view.set_hexpand(true);
        if let Some(web) = WebViewExt::settings(&view) {
            web.set_enable_developer_extras(cfg!(debug_assertions));
            web.set_enable_back_forward_navigation_gestures(false);
        }
        let reader = Reader {
            view,
            settings,
            state: Rc::new(RefCell::new(State {
                ready: false,
                shown: None,
                shows: 0,
                store: None,
                on_open: None,
                on_notice: None,
                on_edit_task: None,
                on_cursor: None,
            })),
        };
        reader.connect_messages();
        reader.connect_policy();
        reader.view.load_uri("daftar://reader/index.html");
        reader
    }

    pub fn set_store(&self, store: Rc<VaultStore>) {
        VAULT_ROOT.with(|root| *root.borrow_mut() = Some(store.root.clone()));
        self.state.borrow_mut().store = Some(store);
    }

    /// A link followed: the path, the heading it names, and where it asked to open.
    pub fn connect_open(&self, handler: impl Fn(&str, Option<&str>, OpenIn) + 'static) {
        self.state.borrow_mut().on_open = Some(Rc::new(handler));
    }

    pub fn connect_notice(&self, handler: impl Fn(&str) + 'static) {
        self.state.borrow_mut().on_notice = Some(Rc::new(handler));
    }

    /// A right click on a task: the note and the line to edit.
    pub fn connect_edit_task(&self, handler: impl Fn(&str, usize) + 'static) {
        self.state.borrow_mut().on_edit_task = Some(Rc::new(handler));
    }

    /// The reading cursor moved: the note and its source line.
    pub fn connect_cursor(&self, handler: impl Fn(&str, usize) + 'static) {
        self.state.borrow_mut().on_cursor = Some(Rc::new(handler));
    }

    pub fn shown(&self) -> Option<Request> {
        self.state.borrow().shown.clone()
    }

    /// Shows [request]: at its heading or line when it is a new note, where
    /// the person was reading with [keep_scroll].
    pub fn show(&self, request: Request, keep_scroll: bool) {
        let mut state = self.state.borrow_mut();
        state.shown = Some(request.clone());
        state.shows += 1;
        if !state.ready {
            return;
        }
        let Some(store) = state.store.clone() else { return };
        let shows = state.shows;
        drop(state);
        let Some(text) = store.text(&request.path) else { return };
        if Canvas::is_canvas(&request.path) {
            self.draw_canvas(&store, &request, keep_scroll, &text);
            return;
        }
        if !Vault::is_base(&request.path) {
            self.draw(&store, &request, keep_scroll, &text, None);
            return;
        }
        // A `.base` file: its title at once, and its views when they are run,
        // away from the window, unless another note came on screen meanwhile.
        self.draw(&store, &request, keep_scroll, "", Some(json!({ "views": [], "problems": [] })));
        let job = BaseJob::new(&store, text, request.path.clone());
        let reader = self.clone();
        glib::spawn_future_local(async move {
            let Ok(base) = gio::spawn_blocking(move || job.run()).await else { return };
            let state = reader.state.borrow();
            let Some(store) = state.store.clone().filter(|_| state.shows == shows) else { return };
            drop(state);
            reader.draw(&store, &request, true, "", Some(base));
        });
    }

    /// Hands [request]'s note to the page: [text] as markdown, or a base's
    /// views when [base] has them.
    fn draw(&self, store: &VaultStore, request: &Request, keep_scroll: bool, text: &str, base: Option<Value>) {
        let note = if let Some(base) = base {
            json!({
                "path": request.path, "title": Vault::title(&request.path), "text": "", "links": {}, "properties": [],
                "base": base,
            })
        } else {
            let parsed = NoteParser::parse(text);
            let properties: Vec<Value> = parsed
                .properties
                .iter()
                .filter(|(key, _)| !HIDDEN.contains(&key.as_str()))
                .map(|(key, values)| json!([key, values.join(", ")]))
                .collect();
            let detected: Vec<Value> = Detected::find(text)
                .iter()
                .map(|item| json!({ "kind": item.kind.name(), "text": item.text, "value": item.value }))
                .collect();
            json!({
                "path": request.path,
                "title": Vault::title(&request.path),
                "text": text,
                "links": store.index().resolved_links_in(&parsed, &request.path),
                "properties": properties,
                "folds": store.places.folds(&request.path),
                "detected": detected,
            })
        };
        self.send(note, self.settings.borrow(), request, keep_scroll);
    }

    /// A `.canvas` file: its board in place of a note's text.
    fn draw_canvas(&self, store: &VaultStore, request: &Request, keep_scroll: bool, text: &str) {
        let canvas = Canvas::parse(text);
        let index = store.index();
        let files = canvas.files(&index, &request.path);
        let nodes: Vec<Value> = canvas
            .nodes
            .iter()
            .map(|node| {
                json!({
                    "id": node.id, "type": node.kind, "x": node.x, "y": node.y, "width": node.width, "height": node.height,
                    "color": Canvas::color(node.color.as_deref()), "text": node.text, "file": node.file,
                    "path": files.get(&node.id), "subpath": node.subpath, "url": node.url, "label": node.label,
                })
            })
            .collect();
        let edges: Vec<Value> = canvas
            .edges
            .iter()
            .map(|edge| {
                json!({
                    "id": edge.id, "fromNode": edge.from_node, "toNode": edge.to_node, "fromEnd": edge.from_end,
                    "toEnd": edge.to_end, "fromSide": edge.from_side, "toSide": edge.to_side,
                    "color": Canvas::color(edge.color.as_deref()), "label": edge.label,
                })
            })
            .collect();
        let note = json!({
            "path": request.path, "title": Vault::title(&request.path), "text": "",
            "links": canvas.links(&index, &request.path), "properties": [],
            "canvas": { "nodes": nodes, "edges": edges },
        });
        drop(index);
        self.send(note, self.settings.borrow(), request, keep_scroll);
    }

    /// Hands [note] to the page with the settings it draws with.
    fn send(&self, mut note: Value, settings: std::cell::Ref<AppSettings>, request: &Request, keep_scroll: bool) {
        let (font_text, font_code) = settings.font_css();
        let object = note.as_object_mut().unwrap();
        object.insert("keepScroll".into(), json!(keep_scroll));
        object.insert("textSize".into(), json!(settings.text_size()));
        object.insert("fontText".into(), json!(font_text));
        object.insert("fontCode".into(), json!(font_code));
        if !keep_scroll {
            if let Some(heading) = &request.heading {
                object.insert("heading".into(), json!(heading));
            }
            if let Some(line) = request.line {
                object.insert("line".into(), json!(line));
            }
        }
        // `show` returns a promise, which WebKit cannot hand back; the page
        // reports its own failures.
        self.run(&format!("daftar.show({note}); undefined"));
    }

    /// Shows the note on screen again, after its file changed.
    pub fn reload(&self) {
        if let Some(request) = self.shown() {
            self.show(request, true);
        }
    }

    pub fn find(&self, action: &str) {
        self.run(&format!("daftar.findCommand({})", json!(action)));
    }

    fn run(&self, script: &str) {
        self.view.evaluate_javascript(script, None, None, gio::Cancellable::NONE, |result| {
            if let Err(error) = result {
                eprintln!("daftar: the reader failed: {error}");
            }
        });
    }

    fn launch(&self, url: &str) {
        let window = self.view.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new(url).launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
    }

    fn notice(&self, text: &str) {
        let handler = self.state.borrow().on_notice.clone();
        if let Some(handler) = handler {
            handler(text);
        }
    }

    fn connect_messages(&self) {
        let Some(manager) = self.view.user_content_manager() else { return };
        manager.register_script_message_handler("daftar", None);
        let reader = self.clone();
        manager.connect_script_message_received(Some("daftar"), move |_, value| {
            if let Some(body) = value.to_json(0).and_then(|text| serde_json::from_str::<Value>(&text).ok()) {
                reader.message(&body);
            }
        });
        for name in ["embed", "tasks", "base"] {
            manager.register_script_message_handler_with_reply(name, None);
            let reader = self.clone();
            manager.connect_script_message_with_reply_received(Some(name), move |_, value, reply| {
                let body = value.to_json(0).and_then(|text| serde_json::from_str::<Value>(&text).ok()).unwrap_or(Value::Null);
                if name == "base" {
                    reader.answer_base(&body, value.context(), reply.clone());
                    return true;
                }
                let answer = reader.answer(name, &body);
                if let Some(context) = value.context() {
                    reply.return_value(&webkit6::javascriptcore::Value::from_json(&context, &answer.to_string()));
                }
                true
            });
        }
    }

    /// What the page tells the app.
    fn message(&self, body: &Value) {
        let text = |key: &str| body.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        match body.get("kind").and_then(Value::as_str).unwrap_or("") {
            "ready" => {
                self.state.borrow_mut().ready = true;
                // The first note, asked for before the page was there: at
                // its heading or its line, not where a page never scrolled.
                if let Some(request) = self.shown() {
                    self.show(request, false);
                }
            }
            "toggleTask" => {
                let (Some(shown), Some(line), Some(status)) =
                    (self.shown(), body.get("line").and_then(Value::as_u64), text("status").chars().next())
                else {
                    return;
                };
                // A task in a query result is in its own note.
                let path = Some(text("path")).filter(|p| !p.is_empty()).unwrap_or(shown.path);
                let Some(store) = self.state.borrow().store.clone() else { return };
                match store.toggle_task(&path, line as usize, status) {
                    Ok(()) => self.reload(),
                    Err(error) => {
                        self.notice(&format!("The task was not changed: {error}."));
                        self.reload();
                    }
                }
            }
            "open" => {
                let (path, target) = (text("path"), text("target"));
                let anchor = Some(text("anchor")).filter(|a| !a.is_empty());
                let flag = |key: &str| body.get(key).and_then(Value::as_bool).unwrap_or(false);
                let place = OpenIn::from_click(flag("newTab"), flag("otherPane"));
                let Some(store) = self.state.borrow().store.clone() else { return };
                // A markdown link, `[x](Notes/a.md)`, names its note by path.
                let found = if path.is_empty() {
                    self.shown().and_then(|shown| store.index().resolve(&target, &shown.path))
                } else {
                    store.index().files.contains(&path).then_some(path)
                };
                match found {
                    Some(found) => {
                        let handler = self.state.borrow().on_open.clone();
                        if let Some(handler) = handler {
                            handler(&found, anchor.as_deref(), place);
                        }
                    }
                    None => self.notice(&format!("“{target}” is not in the vault yet.")),
                }
            }
            "editTask" => {
                let (Some(shown), Some(line)) = (self.shown(), body.get("line").and_then(Value::as_u64)) else { return };
                // A task in a query result is in its own note.
                let path = Some(text("path")).filter(|p| !p.is_empty()).unwrap_or(shown.path);
                let handler = self.state.borrow().on_edit_task.clone();
                if let Some(handler) = handler {
                    handler(&path, line as usize);
                }
            }
            "cursor" => {
                let (Some(shown), Some(line)) = (self.shown(), body.get("line").and_then(Value::as_u64)) else { return };
                let handler = self.state.borrow().on_cursor.clone();
                if let Some(handler) = handler {
                    handler(&shown.path, line as usize);
                }
            }
            "copy" => self.view.clipboard().set_text(&text("text")),
            // Escape on the page while the voice reads.
            "stopSpeaking" => crate::read_aloud::stop(),
            "external" => {
                // Web, mail and the phone only: a note's `file:` or `smb:`
                // link would start an app or a script.
                let url = text("url");
                let scheme = url.split(':').next().unwrap_or("").to_lowercase();
                if OUTSIDE.contains(&scheme.as_str()) {
                    self.launch(&url);
                } else {
                    self.notice(&format!("Daftar opens only web, mail and phone links, not “{scheme}:” links."));
                }
            }
            // An address or a phone number written as plain text: a map, or the phone.
            "detected" => {
                if let Some(url) = detected_url(&text("type"), &text("value")) {
                    self.launch(&url);
                }
            }
            // Sections folded shut, kept for the next time the note opens.
            "folds" => {
                let (Some(shown), Some(folds)) = (self.shown(), body.get("folds").and_then(Value::as_array)) else { return };
                let folds = folds.iter().filter_map(|f| f.as_str().map(String::from)).collect();
                let store = self.state.borrow().store.clone();
                if let Some(store) = store {
                    store.places.set_folds(&shown.path, folds);
                }
            }
            // `e` on a block: the editor, at that block's line.
            "edit" => {
                let Some(line) = body.get("line").and_then(Value::as_u64) else { return };
                if let Some(shown) = self.state.borrow_mut().shown.as_mut() {
                    shown.line = Some(line as usize);
                }
                let _ = self.view.activate_action("win.edit", None);
            }
            _ => {}
        }
    }

    /// ` ```base ` in a note, or `![[file.base]]`, where `this` is the note:
    /// run away from the window, and answered with nothing when another note
    /// came on screen meanwhile, which the page no longer draws.
    fn answer_base(&self, body: &Value, context: Option<webkit6::javascriptcore::Context>, reply: webkit6::ScriptMessageReply) {
        let state = self.state.borrow();
        let shows = state.shows;
        let job = state.store.clone().and_then(|store| {
            let note = body.get("context").and_then(Value::as_str).unwrap_or("").to_string();
            let source = match body.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()) {
                Some(path) if store.index().files.contains(path) => store.text(path)?,
                Some(_) => return None,
                None => body.get("source").and_then(Value::as_str).unwrap_or("").to_string(),
            };
            Some(BaseJob::new(&store, source, note))
        });
        drop(state);
        let reader = self.clone();
        glib::spawn_future_local(async move {
            let answer = match job {
                Some(job) => gio::spawn_blocking(move || job.run()).await.unwrap_or(Value::Null),
                None => Value::Null,
            };
            let answer = if reader.state.borrow().shows == shows { answer } else { Value::Null };
            if let Some(context) = context {
                reply.return_value(&webkit6::javascriptcore::Value::from_json(&context, &answer.to_string()));
            }
        });
    }

    /// The page's other questions: an embedded note, a tasks query.
    fn answer(&self, name: &str, body: &Value) -> Value {
        match name {
            "tasks" => {
                let Some(store) = self.state.borrow().store.clone() else { return Value::Null };
                let source = body.get("source").and_then(Value::as_str).unwrap_or("");
                let path = body.get("path").and_then(Value::as_str);
                run_query(&store, source, path)
            }
            _ => {
                let Some(store) = self.state.borrow().store.clone() else { return Value::Null };
                let path = body.get("path").and_then(Value::as_str).unwrap_or("");
                if !store.index().files.contains(path) {
                    return Value::Null;
                }
                let Some(text) = store.text(path) else { return Value::Null };
                let parsed = NoteParser::parse(&text);
                json!({ "text": text, "links": store.index().resolved_links_in(&parsed, path) })
            }
        }
    }

    /// The page stays the reader: a note's `<meta http-equiv=refresh>` or
    /// `<form>` cannot take the view to a web page, which would then talk to
    /// the app. Links go out through the click handler instead.
    fn connect_policy(&self) {
        self.view.connect_decide_policy(|_, decision, kind| {
            if kind == PolicyDecisionType::NewWindowAction {
                decision.ignore();
                return true;
            }
            if kind != PolicyDecisionType::NavigationAction {
                return false;
            }
            let Some(navigation) = decision.downcast_ref::<NavigationPolicyDecision>() else { return false };
            let uri =
                navigation.navigation_action().and_then(|action| action.request()).and_then(|request| request.uri()).unwrap_or_default();
            let scheme = uri.split(':').next().unwrap_or("");
            if ["daftar", "vault", "about"].contains(&scheme) {
                return false;
            }
            decision.ignore();
            true
        });
        // A frame can go to `vault:`; the window itself only ever shows the reader.
        self.view.connect_load_changed(|view, event| {
            if event == webkit6::LoadEvent::Committed && !view.uri().is_some_and(|uri| uri.starts_with("daftar://reader/")) {
                view.load_uri("daftar://reader/index.html");
            }
        });
    }
}

/// `daftar://reader/…` from the binary and `vault:///…` from the open vault.
fn register_schemes() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(context) = webkit6::WebContext::default() else { return };
        if let Some(security) = context.security_manager() {
            security.register_uri_scheme_as_secure("daftar");
            security.register_uri_scheme_as_secure("vault");
        }
        context.register_uri_scheme("daftar", |request| {
            let path = request.path().unwrap_or_default();
            let path = path.trim_start_matches('/');
            match READER.get_file(path) {
                Some(file) => respond(request, file.contents().to_vec(), path, false),
                None => fail(request),
            }
        });
        context.register_uri_scheme("vault", |request| {
            let path = request.path().unwrap_or_default();
            let path = glib::Uri::unescape_string(&path, None::<&str>).map(|p| p.to_string()).unwrap_or_default();
            let file = VAULT_ROOT.with(|root| root.borrow().as_ref().and_then(|root| file_in(path.trim_start_matches('/'), root)));
            match file.and_then(|file| std::fs::read(file).ok()) {
                Some(data) => respond(request, data, &path, true),
                None => fail(request),
            }
        });
    });
}

/// A video or a sound is read in parts, with `Range: bytes=start-end`, and
/// a page or a picture from the vault with script in it runs with no rights.
fn respond(request: &URISchemeRequest, data: Vec<u8>, path: &str, from_vault: bool) {
    let kind = mime_guess::from_path(path).first_or_octet_stream().essence_str().to_string();
    let headers = soup::MessageHeaders::new(soup::MessageHeadersType::Response);
    let sandboxed = ["text/html", "image/svg+xml", "application/xhtml+xml", "text/xml", "application/xml"].contains(&kind.as_str());
    if from_vault && sandboxed {
        headers.append("Content-Security-Policy", "sandbox; default-src 'none'; img-src vault: data:; style-src 'unsafe-inline'");
    }
    headers.append("Accept-Ranges", "bytes");
    let size = data.len();
    let range = request.http_headers().and_then(|h| h.one("Range")).map(|r| r.to_string());
    let (status, body) = match range {
        Some(range) => match bytes(&range, size) {
            Some((start, end)) => {
                headers.append("Content-Range", &format!("bytes {start}-{end}/{size}"));
                (206, data[start..=end].to_vec())
            }
            None => {
                headers.append("Content-Range", &format!("bytes */{size}"));
                (416, Vec::new())
            }
        },
        None => (200, data),
    };
    let length = body.len() as i64;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(body));
    let response = URISchemeResponse::new(&stream, length);
    response.set_status(status, None);
    response.set_content_type(&kind);
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}

fn fail(request: &URISchemeRequest) {
    let mut error = glib::Error::new(gio::IOErrorEnum::NotFound, "Not in the vault");
    request.finish_error(&mut error);
}

/// An open end is sent in parts: a whole film is not copied at once.
const CHUNK: usize = 4 << 20;

/// `bytes=0-`, `bytes=100-199` or `bytes=-500`, inside a file of [size].
fn bytes(header: &str, size: usize) -> Option<(usize, usize)> {
    let spec = header.strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    if size == 0 {
        return None;
    }
    let (first, last) = (first.parse::<usize>().ok(), last.parse::<usize>().ok());
    let (start, end) = match (first, last) {
        (Some(a), Some(b)) => (a, b.min(size - 1)),
        (Some(a), None) => (a, (size - 1).min(a + CHUNK - 1)),
        (None, Some(b)) => (size.saturating_sub(b), size - 1),
        (None, None) => return None,
    };
    (start <= end && start < size).then_some((start, end))
}

/// A query's result as the page draws it: each task with the groups it is
/// under, its note, its line, and the links its text needs.
fn run_query(store: &VaultStore, source: &str, path: Option<&str>) -> Value {
    let index = store.index();
    let query = TasksQuery::parse(source, path);
    let mut tasks = index.tasks();
    tasks.sort_by(|a, b| (&a.path, a.task.line).cmp(&(&b.path, b.task.line)));
    let found = query.run(tasks, Day::today(), |t| QueryTask::from(t));
    let mut links = serde_json::Map::new();
    let rows: Vec<Value> = found
        .iter()
        .map(|task| {
            for link in NoteParser::parse(&task.task.text).links {
                if !links.contains_key(&link.target)
                    && let Some(target) = index.resolve(&link.target, &task.path)
                {
                    links.insert(link.target, target.into());
                }
            }
            json!({
                "path": task.path, "line": task.task.line, "status": task.task.status.to_string(),
                "text": task.task.text, "title": Vault::title(&task.path), "groups": query.groups(&QueryTask::from(task)),
                "due": task.task.due, "scheduled": task.task.scheduled, "done": task.task.done,
            })
        })
        .collect();
    json!({ "tasks": rows, "notUnderstood": query.not_understood, "links": links })
}

/// One run of a base, with all it reads, so it can run on another thread:
/// the index as it is now, the vault's folder, the base's text and the file
/// `this` names (the base, or the note that embeds it).
struct BaseJob {
    index: VaultIndex,
    root: PathBuf,
    source: String,
    context: String,
}

impl BaseJob {
    fn new(store: &VaultStore, source: String, context: String) -> BaseJob {
        BaseJob { index: store.index().clone(), root: store.root.clone(), source, context }
    }

    fn run(&self) -> Value {
        run_base(&self.index, &self.root, &self.source, &self.context)
    }
}

/// Every view of a base, as the page draws it.
fn run_base(index: &VaultIndex, root: &Path, source: &str, context: &str) -> Value {
    let mut engine = BaseEngine::on_disk(index, root);
    let tables = engine.run(&Base::parse(source), context);
    let views: Vec<Value> = tables
        .iter()
        .map(|table| {
            let groups: Vec<Value> = table
                .groups
                .iter()
                .map(|group| {
                    let rows: Vec<Value> = group
                        .rows
                        .iter()
                        .map(|row| {
                            let mut out = json!({ "path": row.path, "cells": row.cells.iter().map(cell_json).collect::<Vec<_>>() });
                            match &row.image {
                                Some(BaseImage::File(path)) => out["image"] = json!({ "file": path }),
                                Some(BaseImage::Url(url)) => out["image"] = json!({ "url": url }),
                                Some(BaseImage::Color(color)) => out["image"] = json!({ "color": color }),
                                None => {}
                            }
                            out
                        })
                        .collect();
                    let mut out = json!({ "rows": rows });
                    if let Some(name) = &group.name {
                        out["name"] = json!(name);
                    }
                    out
                })
                .collect();
            json!({
                "name": table.name, "type": table.kind, "kind": table.draws_as, "layout": layout_json(&table.layout),
                "columns": table.columns, "count": table.count, "groups": groups,
            })
        })
        .collect();
    json!({ "views": views, "problems": engine.problems })
}

fn layout_json(layout: &BaseLayout) -> Value {
    json!({
        "imageFit": layout.image_fit, "imageAspectRatio": layout.image_aspect_ratio, "cardSize": layout.card_size,
        "markers": layout.markers, "indentProperties": layout.indent_properties, "separator": layout.separator,
    })
}

fn cell_json(cell: &BaseCell) -> Value {
    let parts: Vec<Value> = cell
        .parts
        .iter()
        .map(|part| match &part.path {
            Some(path) => json!({ "text": part.text, "path": path }),
            None => json!({ "text": part.text }),
        })
        .collect();
    let mut out = json!({ "parts": parts, "numeric": cell.numeric });
    if let Some(check) = cell.check {
        out["check"] = json!(check);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_ranges() {
        assert_eq!(bytes("bytes=0-", 10), Some((0, 9)));
        assert_eq!(bytes("bytes=2-4", 10), Some((2, 4)));
        assert_eq!(bytes("bytes=-3", 10), Some((7, 9)));
        assert_eq!(bytes("bytes=5-100", 10), Some((5, 9)));
        assert_eq!(bytes("bytes=10-", 10), None);
        assert_eq!(bytes("bytes=0-", CHUNK * 2), Some((0, CHUNK - 1)));
        assert_eq!(bytes("lines=0-", 10), None);
    }

    #[test]
    fn the_reader_is_built_in() {
        assert!(READER.get_file("index.html").is_some(), "run `just web` first");
    }

    #[test]
    fn a_click_opens_where_its_keys_say() {
        assert_eq!(OpenIn::from_click(false, false), OpenIn::Here);
        assert_eq!(OpenIn::from_click(true, false), OpenIn::NewTab);
        assert_eq!(OpenIn::from_click(false, true), OpenIn::OtherPane);
        // Ctrl with Alt is a new tab, as ⌘ with ⌥ is on the Mac.
        assert_eq!(OpenIn::from_click(true, true), OpenIn::NewTab);
    }
}
