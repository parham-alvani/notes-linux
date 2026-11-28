# The MCP server over HTTP: `daftar-cli mcp --http`

Shared by `daftar-cli` (`notes-linux`) and every app that runs it or starts its own MCP server: Daftar for macOS runs `daftar-cli`, and a server of another app must keep these rules, so an agent sees one Daftar.

`daftar-cli mcp` serves a vault to an agent on stdin and stdout. `--http` serves the same tools over MCP's [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#streamable-http), so a vault on a server can be read by agents elsewhere: a company's reports and knowledge base in a read-only clone, in a container behind Docker's MCP gateway (`docker/mcp-gateway`, a remote server with `transport_type: http`).

## 1. The command

```bash
DAFTAR_MCP_TOKEN=… daftar-cli mcp --vault /vault --http 0.0.0.0:8080 --pull-every 300
```

| Flag | What it does |
| --- | --- |
| `--http <addr:port>` | Serves over HTTP there instead of stdin and stdout. Without it, nothing changes. |
| `--token-env <NAME>` | The environment variable that holds the bearer token; `DAFTAR_MCP_TOKEN` by default. An empty or blank value is no token. |
| `--no-token` | Serves an address that is not loopback with no token, for a server behind something else that checks who asks. Refused when the variable holds a token. |
| `--pull-every <seconds>` | Pulls the vault with `Git::pull` (a read-only clone: `--ff-only`) at once and then every this many seconds; a failure goes to stderr and the next pull is tried. Refused in a folder that is no git repository. |
| `--read-only` | As on stdin: the writing tools are left out. |

- **An address that is not loopback needs a token.** With none, and no `--no-token`, the command refuses to start and says which variable to set. A loopback address may serve without one.
- The token is never printed or logged.

## 2. The endpoint

The MCP endpoint is `/mcp`; a query string is ignored and any other path is `404`.

1. **Origin.** A request with an `Origin` header whose host is not `localhost`, `127.0.0.1` or `[::1]` (any port, any scheme) is `403`: a page of another site must not reach a server on the computer through a name that resolves to it (DNS rebinding). `Origin: null` is refused. A request with no `Origin` (an agent, `curl`, the gateway) passes.
2. **Token.** When there is a token, a request without `Authorization: Bearer <token>` is `401` with `WWW-Authenticate: Bearer`. The scheme is read without case; the token is compared whole, in a time that hangs on its length alone.
3. **Method.** `POST` only. `GET` is `405` (the server opens no SSE stream), as is `DELETE` (no session to end); `Allow: POST`.
4. **Version.** An `MCP-Protocol-Version` header with a version the server does not know is `400`. None is fine.
5. **Body.** One JSON-RPC message, at most 4 MB (`413` above). Not JSON is `400` with a JSON-RPC error `-32700` and no id; an array (a batch) or another value that is no object is `400` with `-32600`.
6. **Answer.** A request (`method` and `id`) is answered `200` with `Content-Type: application/json` and one JSON-RPC response, the same the stdio server writes. A notification (no `id`) or a response from the client (no `method`) is `202` with no body.

The server keeps **no session**: it sends no `Mcp-Session-Id` and ignores one sent to it, so a client never has to begin again after a restart. Every message goes to the one server state the stdio transport has (the kept index, the client named by the last `initialize`), one message at a time; the transport only carries them.

`GET /health` (or `HEAD`) answers `200` with `ok` and needs no token, for a container's health check.

## 3. A read-only clone, and agents kept out

The door is the stdio server's ([read-only vault](read-only-vault.md), section 5):

- A clone with `daftar.readonly` is served with the reading tools alone, as with `--read-only`, whatever the flags.
- A read-only clone whose `.daftar.toml` says `[agents] read_only_clones = false` is not served: `--http` does not start, prints `this vault is not open to agents in a read-only clone (agents.read_only_clones in .daftar.toml)` and exits 1, before it listens.
- After each pull the door is worked out again, since a pull can bring a new `.daftar.toml`: once it is closed, every request is answered with that JSON-RPC error (`-32000`) and no note is read; once it is open again, the tools come back. A change of the door is written to stderr.

## 4. The container image

`ghcr.io/parham-alvani/daftar-cli`, tagged with the version and `latest`, for `linux/amd64`, published by each release's CI (`packaging/docker/Dockerfile`): `daftar-cli` built from the source on Debian, with `git`, `openssh-client`, `ca-certificates`, `tini` as PID 1, GitHub's host keys in `/etc/ssh/ssh_known_hosts` and `safe.directory = *`, as a mounted clone is often another user's. `DAFTAR_VAULT=/vault`; the entry point is `daftar-cli`, and the command `mcp --http 0.0.0.0:8080 --read-only`, which refuses to start without `DAFTAR_MCP_TOKEN`. The clone's deploy key is its `core.sshCommand` (`ssh -i /run/secrets/… -o IdentitiesOnly=yes`), or `GIT_SSH_COMMAND`.

## 5. Tests

1. `it_answers_over_http_as_on_stdin`: on `127.0.0.1:0` with a token, `initialize` is `200` with the server's name and the asked version, `notifications/initialized` is `202` with no body, `tools/list` has the reading tools and not the writing ones of a read-only server, a `search` call finds a note, a client's response is `202`, a batch and a body that is no JSON are `400`.
2. `a_request_without_the_token_or_from_a_page_elsewhere_is_refused`: no `Authorization`, a token with one letter changed, a longer token and the `Basic` scheme are `401` and list no tool; `Origin: https://evil.example` is `403` and `http://localhost:5173` passes; an unknown `MCP-Protocol-Version` is `400`; `GET /mcp` is `405`; `GET /health` is `200` with no token; another path is `404`.
3. `a_served_read_only_clone_follows_its_remote`: a read-only clone of a bare remote served with `--pull-every`: a note pushed to the remote is read through `/mcp` after a pull, and after `.daftar.toml` with `read_only_clones = false` is pushed, `read_note` is answered with the error.
4. `the_token_is_compared_whole` and `only_a_page_of_this_computer_is_a_local_origin`, on the two rules alone.
