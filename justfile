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

# Installs the command line alone, which needs no GTK: ~/.local/bin/daftar-cli.
install-cli:
    cargo build --release -p daftar-cli
    install -Dm755 target/release/daftar-cli ~/.local/bin/daftar-cli

# Installs for this user: the binaries, the desktop entry and the icon.
install: app
    install -Dm755 target/release/daftar ~/.local/bin/daftar
    install -Dm755 target/release/daftar-cli ~/.local/bin/daftar-cli
    install -Dm644 data/me.parham1995.notes.desktop ~/.local/share/applications/me.parham1995.notes.desktop
    install -Dm644 data/me.parham1995.notes.png ~/.local/share/icons/hicolor/256x256/apps/me.parham1995.notes.png

# Cuts a release: `just release 0.2.0` sets the version everywhere, commits, tags and pushes. The tag's CI publishes the binaries, and the tap's daily bump moves the daftar-cli formula to them.
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    test -z "$(git status --porcelain)" || { echo "commit your changes first"; exit 1; }
    old=$(cat VERSION)
    echo "{{version}}" > VERSION
    sed -i -E "s/^version = \"$old\"/version = \"{{version}}\"/; s/daftar-core = \{ version = \"$old\"/daftar-core = { version = \"{{version}}\"/" crates/*/Cargo.toml
    cargo update -q -w
    git commit -qam "release v{{version}}"
    git tag -a "v{{version}}" -m "v{{version}}"
    git push -q origin HEAD "v{{version}}"
