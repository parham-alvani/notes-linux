//! `daftar-cli mcp --http`: the same server over MCP's Streamable HTTP
//! transport (`docs/specs/mcp-http.md`), for an agent on another computer,
//! such as a container that serves a read-only clone to Docker's MCP
//! gateway. `POST /mcp` takes one JSON-RPC message and answers it with one
//! JSON object (`Content-Type: application/json`), or with `202 Accepted`
//! and no body for a notification or a response; there is no SSE stream,
//! so `GET /mcp` is `405`. No session is kept: every message goes to the
//! one `Server` the stdio transport has, one at a time, so the tools and
//! their answers are the same. `GET /health` answers without a token.

use std::io::Read;
use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tiny_http::{Header, Method, Request, Response};

use super::{AGENTS_REFUSED, KNOWN, Server, failure};

/// The requests answered at once. A tool call waits for the one before it,
/// but `/health` and a refused request never wait.
const WORKERS: usize = 4;

/// The largest message taken; a tool's arguments are far smaller.
const MAX_BODY: u64 = 4 << 20;

/// How `--http` serves.
pub struct Options {
    pub address: SocketAddr,
    /// Every `/mcp` request must carry `Authorization: Bearer <token>`;
    /// with none, every request is let in.
    pub token: Option<String>,
    /// Pulls the vault this often, the first time at once.
    pub pull_every: Option<Duration>,
}

/// A bound socket, so a test can serve on port 0 and learn the port.
pub(super) struct Listener(tiny_http::Server);

impl Listener {
    pub(super) fn bind(address: SocketAddr) -> Result<Listener, String> {
        tiny_http::Server::http(address).map(Listener).map_err(|error| format!("cannot listen on {address}: {error}"))
    }

    pub(super) fn address(&self) -> Option<SocketAddr> {
        self.0.server_addr().to_ip()
    }

    /// Answers requests until the socket fails.
    pub(super) fn serve(self, server: Server, options: &Options) -> Result<(), String> {
        let root = server.root.clone();
        let reading = if server.read_only { " (read-only)" } else { "" };
        let at = self.address().map_or_else(|| options.address.to_string(), |address| address.to_string());
        eprintln!("daftar-cli: serving {}{reading} on http://{at}/mcp", root.display());
        let server = Arc::new(Mutex::new(server));
        if let Some(every) = options.pull_every {
            let server = Arc::clone(&server);
            thread::spawn(move || pull(&root, &server, every));
        }
        let listener = Arc::new(self.0);
        let token = Arc::new(options.token.clone());
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let (listener, server, token) = (Arc::clone(&listener), Arc::clone(&server), Arc::clone(&token));
                thread::spawn(move || {
                    loop {
                        match listener.recv() {
                            Ok(request) => respond(request, token.as_deref(), &server),
                            Err(error) => return error.to_string(),
                        }
                    }
                })
            })
            .collect();
        let mut failed = Vec::new();
        for worker in workers {
            failed.push(worker.join().unwrap_or_else(|_| "a worker panicked".into()));
        }
        Err(failed.join("; "))
    }
}

/// Pulls the vault every [every], logging a failure and going on, and opens
/// the door again as the clone and `.daftar.toml` say after the pull.
fn pull(root: &Path, server: &Mutex<Server>, every: Duration) {
    let git = daftar_core::Git::new(root);
    loop {
        if let Err(failure) = git.pull() {
            eprintln!("daftar-cli: the pull failed: {}", failure.to_string().trim());
        }
        let mut server = lock(server);
        if server.reopen() {
            let now = if server.closed {
                AGENTS_REFUSED
            } else if server.read_only {
                "the vault is served read-only"
            } else {
                "the vault is served with every tool"
            };
            eprintln!("daftar-cli: after the pull, {now}");
        }
        drop(server);
        thread::sleep(every);
    }
}

fn lock(server: &Mutex<Server>) -> MutexGuard<'_, Server> {
    // A tool that panicked left the server as it was between two calls.
    server.lock().unwrap_or_else(PoisonError::into_inner)
}

fn respond(mut request: Request, token: Option<&str>, server: &Mutex<Server>) {
    let path = request.url().split('?').next().unwrap_or_default().to_string();
    let response = match path.as_str() {
        "/health" if matches!(request.method(), Method::Get | Method::Head) => text(200, "ok\n"),
        "/health" => text(405, "GET only\n").with_header(header("Allow", "GET, HEAD")),
        "/mcp" => mcp(&mut request, token, server),
        _ => text(404, "not found: the MCP endpoint is /mcp\n"),
    };
    if let Err(error) = request.respond(response) {
        eprintln!("daftar-cli: an answer to {path} was not sent: {error}");
    }
}

type Answer = Response<std::io::Cursor<Vec<u8>>>;

fn mcp(request: &mut Request, token: Option<&str>, server: &Mutex<Server>) -> Answer {
    // A page in a browser must not reach a server on this computer through
    // a name that resolves to it (DNS rebinding): only a local page may.
    if let Some(origin) = header_of(request, "Origin")
        && !local_origin(origin)
    {
        return text(403, "this origin is not let in\n");
    }
    if !lets_in(token, header_of(request, "Authorization")) {
        return text(401, "a bearer token is needed\n").with_header(header("WWW-Authenticate", "Bearer"));
    }
    if *request.method() != Method::Post {
        return text(405, "POST only: this server opens no SSE stream\n").with_header(header("Allow", "POST"));
    }
    if let Some(version) = header_of(request, "MCP-Protocol-Version")
        && !KNOWN.contains(&version)
    {
        return text(400, &format!("unsupported MCP-Protocol-Version {version}\n"));
    }
    let mut body = Vec::new();
    if let Err(error) = request.as_reader().take(MAX_BODY + 1).read_to_end(&mut body) {
        return text(400, &format!("the body could not be read: {error}\n"));
    }
    if body.len() as u64 > MAX_BODY {
        return text(413, "the message is too large\n");
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(error) => return answer(400, &failure(&Value::Null, -32700, &format!("not JSON: {error}"))),
    };
    if !message.is_object() {
        return answer(400, &failure(&Value::Null, -32600, "one JSON-RPC message a request, not a batch"));
    }
    let mut server = lock(server);
    match catch_unwind(AssertUnwindSafe(|| server.answer(&message))) {
        Ok(Some(reply)) => answer(200, &reply),
        Ok(None) => Response::from_data(Vec::new()).with_status_code(202),
        Err(_) => answer(500, &failure(message.get("id").unwrap_or(&Value::Null), -32603, "the server failed on this message")),
    }
}

/// Whether a request with this `Authorization` header may come in.
fn lets_in(token: Option<&str>, authorization: Option<&str>) -> bool {
    let Some(token) = token else { return true };
    let Some((scheme, given)) = authorization.and_then(|value| value.trim().split_once(' ')) else { return false };
    scheme.eq_ignore_ascii_case("bearer") && same(given.trim().as_bytes(), token.as_bytes())
}

/// [given] is [token], in a time that hangs on the token's length alone.
fn same(given: &[u8], token: &[u8]) -> bool {
    let mut differ = u8::from(given.len() != token.len());
    for (i, byte) in token.iter().enumerate() {
        differ |= byte ^ given.get(i).copied().unwrap_or(!byte);
    }
    std::hint::black_box(differ) == 0
}

/// Whether an `Origin` is a page of this computer: `localhost`,
/// `127.0.0.1` or `[::1]`, at any port. `null` is not.
fn local_origin(origin: &str) -> bool {
    let Some((_, rest)) = origin.trim().split_once("://") else { return false };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    ["localhost", "127.0.0.1", "[::1]"].iter().any(|local| host.eq_ignore_ascii_case(local))
}

fn header_of<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request.headers().iter().find(|header| header.field.equiv(name)).map(|header| header.value.as_str())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a header of our own is valid")
}

fn text(status: u16, body: &str) -> Answer {
    Response::from_string(body).with_status_code(status).with_header(header("Content-Type", "text/plain; charset=utf-8"))
}

fn answer(status: u16, body: &Value) -> Answer {
    Response::from_string(body.to_string()).with_status_code(status).with_header(header("Content-Type", "application/json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use std::net::TcpStream;

    /// The test vault of `mcp.rs`, served on a free port of 127.0.0.1.
    fn serve(token: Option<&str>) -> (tempfile::TempDir, SocketAddr) {
        let root = crate::mcp::tests::vault();
        let server = crate::mcp::tests::server(root.path(), true);
        let listener = Listener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.address().unwrap();
        let options = Options { address, token: token.map(String::from), pull_every: None };
        thread::spawn(move || listener.serve(server, &options));
        (root, address)
    }

    /// One request, as `curl` would send it: the status and the body.
    fn ask(address: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(address).unwrap();
        let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);
        stream.write_all(request.as_bytes()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        let status = reply.split(' ').nth(1).unwrap().parse().unwrap();
        let body = reply.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default();
        (status, body)
    }

    const JSON: (&str, &str) = ("Content-Type", "application/json");
    const ACCEPT: (&str, &str) = ("Accept", "application/json, text/event-stream");
    const BEARER: (&str, &str) = ("Authorization", "Bearer s3cret");

    #[test]
    fn it_answers_over_http_as_on_stdin() {
        let (_root, address) = serve(Some("s3cret"));
        let post = |body: &str| ask(address, "POST", "/mcp", &[JSON, ACCEPT, BEARER], body);
        let (status, body) = post(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"t","version":"1"}}}"#,
        );
        assert_eq!(status, 200, "{body}");
        let answer: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(answer["result"]["serverInfo"]["name"], "daftar");
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), (202, String::new()));
        let (status, body) = post(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert_eq!(status, 200);
        let tools: Value = serde_json::from_str(&body).unwrap();
        let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search") && !names.contains(&"capture"), "{names:?}");
        let call = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "search", "arguments": { "query": "tomato" } } });
        let (status, body) = post(&call.to_string());
        assert_eq!(status, 200);
        let result: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(result["result"]["isError"], false);
        assert!(result["result"]["content"][0]["text"].as_str().unwrap().contains("Seeds.md"), "{body}");
        // A response from the client is taken, as a notification is.
        assert_eq!(post(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#).0, 202);
        assert_eq!(post("[]").0, 400);
        assert_eq!(post("not json").0, 400);
    }

    #[test]
    fn a_request_without_the_token_or_from_a_page_elsewhere_is_refused() {
        let (_root, address) = serve(Some("s3cret"));
        let list = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let (status, body) = ask(address, "POST", "/mcp", &[JSON, ACCEPT], list);
        assert_eq!(status, 401);
        assert!(!body.contains("read_note"));
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, ("Authorization", "Bearer s3creT")], list).0, 401);
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, ("Authorization", "Bearer s3cret2")], list).0, 401);
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, ("Authorization", "Basic s3cret")], list).0, 401);
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, BEARER, ("Origin", "https://evil.example")], list).0, 403);
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, BEARER, ("Origin", "http://localhost:5173")], list).0, 200);
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, BEARER, ("MCP-Protocol-Version", "1999-01-01")], list).0, 400);
        assert_eq!(ask(address, "POST", "/mcp", &[JSON, BEARER, ("MCP-Protocol-Version", "2025-06-18")], list).0, 200);
        assert_eq!(ask(address, "GET", "/mcp", &[BEARER], "").0, 405);
        assert_eq!(ask(address, "GET", "/health", &[], ""), (200, "ok\n".into()));
        assert_eq!(ask(address, "GET", "/elsewhere", &[BEARER], "").0, 404);
    }

    #[test]
    fn a_served_read_only_clone_follows_its_remote() {
        let base = tempfile::tempdir().unwrap();
        let path = |name: &str| base.path().join(name).to_str().unwrap().to_string();
        let git = |dir: &str| daftar_core::Git::new(base.path().join(dir));
        let run = |dir: &str, args: &[&str]| git(dir).run(args).unwrap();
        let root = crate::mcp::tests::vault();
        let first = daftar_core::Git::new(root.path());
        for args in
            [&["init", "-q", "-b", "main"][..], &["add", "-A"], &["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "start"]]
        {
            first.run(args).unwrap();
        }
        daftar_core::Git::new(base.path()).run(&["clone", "-q", "--bare", root.path().to_str().unwrap(), &path("remote.git")]).unwrap();
        first.run(&["remote", "add", "origin", &path("remote.git")]).unwrap();
        first.run(&["fetch", "-q", "origin"]).unwrap();
        first.run(&["branch", "-q", "--set-upstream-to=origin/main"]).unwrap();
        daftar_core::Git::new(base.path()).run(&["clone", "-q", &path("remote.git"), &path("served")]).unwrap();
        run("served", &["config", "--local", "daftar.readonly", "true"]);
        let served = base.path().join("served");
        let mut server = crate::mcp::tests::server(&served, false);
        server.reopen();
        assert!(server.read_only && !server.closed);
        let listener = Listener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.address().unwrap();
        let options = Options { address, token: None, pull_every: Some(Duration::from_millis(200)) };
        thread::spawn(move || listener.serve(server, &options));
        let read = |note: &str| {
            let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "read_note", "arguments": { "note": note } } });
            serde_json::from_str::<Value>(&ask(address, "POST", "/mcp", &[JSON], &call.to_string()).1).unwrap()
        };
        let waits_for = |done: &dyn Fn() -> bool| {
            (0..100).any(|_| {
                done() || {
                    thread::sleep(Duration::from_millis(100));
                    false
                }
            })
        };
        fs::write(root.path().join("Pulled.md"), "# Pulled\n\nFrom the remote.\n").unwrap();
        first.run(&["add", "-A"]).unwrap();
        first.run(&["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "pulled"]).unwrap();
        first.run(&["push", "-q"]).unwrap();
        assert!(waits_for(&|| read("Pulled")["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("From the remote"))));
        // The vault then keeps agents out of its read-only clones: after the next pull, nothing is read.
        fs::write(root.path().join(".daftar.toml"), "[agents]\nread_only_clones = false\n").unwrap();
        first.run(&["add", "-A"]).unwrap();
        first.run(&["-c", "user.name=T", "-c", "user.email=t@t", "commit", "-qm", "closed"]).unwrap();
        first.run(&["push", "-q"]).unwrap();
        assert!(waits_for(&|| read("Pulled")["error"]["message"] == AGENTS_REFUSED));
    }

    #[test]
    fn the_token_is_compared_whole() {
        assert!(lets_in(None, None));
        assert!(lets_in(Some("abc"), Some("Bearer abc")));
        assert!(lets_in(Some("abc"), Some("bearer  abc ")));
        assert!(!lets_in(Some("abc"), None));
        assert!(!lets_in(Some("abc"), Some("Bearer ab")));
        assert!(!lets_in(Some("abc"), Some("Bearer abcd")));
        assert!(!lets_in(Some("abc"), Some("Bearer")));
        assert!(!lets_in(Some("\u{ff}"), Some("Bearer ")));
    }

    #[test]
    fn only_a_page_of_this_computer_is_a_local_origin() {
        for origin in ["http://localhost", "http://localhost:3000", "https://127.0.0.1:8443", "http://[::1]:80", "HTTP://LOCALHOST"] {
            assert!(local_origin(origin), "{origin}");
        }
        for origin in ["null", "https://example.com", "http://localhost.example.com", "http://127.0.0.1.nip.io", "http://[::2]"] {
            assert!(!local_origin(origin), "{origin}");
        }
    }
}
