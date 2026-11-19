# syntax=docker/dockerfile:1
# The runner image (`ax-api runner`): takes the AI's jobs from the server and
# runs them with Claude Code on a clone of the project. Build from the
# repository root:
#   docker build -f deploy/runner.Dockerfile -t ax-runner .
#
# The agent runs the project's own tests, so the image needs the project's
# tool chain. This one has git, Node.js 22 and Python 3; for anything else,
# build an image FROM this one and add it.

FROM rust:1.100-bookworm AS build
ENV RUSTUP_TOOLCHAIN=1.100.0
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p ax-api \
 && cp target/release/ax-api /ax-api

FROM node:22-bookworm-slim
ARG CLAUDE_CODE_VERSION=latest
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git python3 python3-pip python3-venv \
 && rm -rf /var/lib/apt/lists/* \
 && npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
 && npm cache clean --force
COPY --from=build /ax-api /usr/local/bin/ax-api
# The agent edits and runs commands in its clone: never as root.
USER node
WORKDIR /home/node
ENV AX_RUNNER_WORKDIR=/home/node/work \
    DISABLE_AUTOUPDATER=1
ENTRYPOINT ["ax-api", "runner"]
