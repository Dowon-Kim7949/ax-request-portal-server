# syntax=docker/dockerfile:1
# The API server image. Build from the repository root:
#   docker build -f deploy/api.Dockerfile -t ax-api .

FROM rust:1.100-bookworm AS build
# The image already has this toolchain; skip the components rust-toolchain.toml asks for.
ENV RUSTUP_TOOLCHAIN=1.100.0
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p ax-api \
 && cp target/release/ax-api /ax-api

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home ax  && mkdir -p /var/lib/ax/uploads  && chown ax /var/lib/ax/uploads
COPY --from=build /ax-api /usr/local/bin/ax-api
USER ax
ENV AX_BIND=0.0.0.0:8080
# A new volume mounted here starts with this directory's owner, so ax may write.
ENV AX_UPLOAD_DIR=/var/lib/ax/uploads
EXPOSE 8080
ENTRYPOINT ["ax-api"]
