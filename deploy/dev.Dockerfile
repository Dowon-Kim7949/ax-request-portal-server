# Toolchain for scripts/cargo.sh. Keep the tag in step with rust-toolchain.toml.
FROM rust:1.100-bookworm
RUN rustup component add rustfmt clippy
