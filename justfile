# Daftar for Linux. `just` lists the recipes.

default:
    @just --list

# The reader bundle, from web/src into crates/daftar/reader.
web:
    cd web && ([ -d node_modules ] || npm ci) && node build.mjs

# The Rust tests and the renderer's tests.
test: web
    cargo test --workspace
    cd web && node --test test/*.test.mjs

# Formatting and clippy, as CI runs them.
lint: web
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings

# Starts the app; `just run Notes/Inbox.md` opens that note.
run *args: web
    cargo run -p daftar -- {{args}}

# The release binaries: the app, with the reader built into it, and the CLI.
app: web
    cargo build --release -p daftar -p daftar-cli

# The command line, on the vault in the config file: `just cli tasks --due today`.
cli *args:
    cargo run -q -p daftar-cli -- {{args}}

# Installs for this user: the binaries, the desktop entry and the icon.
install: app
    install -Dm755 target/release/daftar ~/.local/bin/daftar
    install -Dm755 target/release/daftar-cli ~/.local/bin/daftar-cli
    install -Dm644 data/me.parham1995.notes.desktop ~/.local/share/applications/me.parham1995.notes.desktop
    install -Dm644 data/me.parham1995.notes.png ~/.local/share/icons/hicolor/256x256/apps/me.parham1995.notes.png
