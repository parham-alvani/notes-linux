# syntax=docker/dockerfile:1
# daftar-cli in a container, to serve a vault to agents over HTTP
# (`daftar-cli mcp --http`, docs/specs/mcp-http.md). Built from the
# repository's root: `docker build -f packaging/docker/Dockerfile .`
FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p daftar-cli \
    && cp target/release/daftar-cli /usr/local/bin/daftar-cli

FROM debian:trixie-slim
# git and ssh pull the clone with its deploy key; tini, as PID 1, passes
# Docker's SIGTERM on and reaps the processes git leaves.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git openssh-client tini \
    && rm -rf /var/lib/apt/lists/* \
    # A mounted clone is often owned by another user, which git refuses.
    && git config --system --add safe.directory '*'
# GitHub's host keys, as `gh api meta` gives them, so the first pull needs no prompt.
COPY packaging/docker/github_known_hosts /etc/ssh/ssh_known_hosts
COPY --from=build /usr/local/bin/daftar-cli /usr/local/bin/daftar-cli
ENV DAFTAR_VAULT=/vault
EXPOSE 8080
ENTRYPOINT ["tini", "--", "daftar-cli"]
# Read-only, and refused to start without a token in $DAFTAR_MCP_TOKEN.
CMD ["mcp", "--http", "0.0.0.0:8080", "--read-only"]
