# syntax=docker/dockerfile:1
# Builds the native pieces of the standalone releases: the rules engine addon and the launcher, for Linux
# (glibc 2.36 baseline, Debian bookworm) and Windows x64 (MinGW cross-compile).
FROM rust:1-slim-bookworm AS toolchain
# APT::Sandbox::User=root: rootless Podman can't switch apt to its unprivileged download user
RUN apt-get -o APT::Sandbox::User=root update \
 && apt-get -o APT::Sandbox::User=root install -y --no-install-recommends gcc-mingw-w64-x86-64 && rm -rf /var/lib/apt/lists/* \
 && rustup target add x86_64-pc-windows-gnu

FROM toolchain AS build
# The Windows addon links against node.exe through Node's own import library (napi-build looks for it as libnode).
ARG NODE_VERSION
ADD https://nodejs.org/dist/${NODE_VERSION}/win-x64/node.lib /libnode/libnode.dll.a
RUN touch /libnode/libnode.dll
WORKDIR /src
COPY engine/Cargo.toml engine/Cargo.lock engine/
COPY engine/.cargo engine/.cargo
COPY engine/crates engine/crates
COPY packaging/launcher packaging/launcher
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/engine/target \
    --mount=type=cache,target=/src/packaging/launcher/target \
    set -e; mkdir -p /out/linux /out/windows; \
    cd /src/engine; \
    cargo build --release --locked -p mtg-node; \
    cp target/release/libmtg_node.so /out/linux/; \
    LIBNODE_PATH=/libnode cargo build --release --locked -p mtg-node --target x86_64-pc-windows-gnu; \
    cp target/x86_64-pc-windows-gnu/release/mtg_node.dll /out/windows/; \
    cd /src/packaging/launcher; \
    cargo build --release; \
    cp target/release/aetherhub /out/linux/; \
    cargo build --release --target x86_64-pc-windows-gnu; \
    cp target/x86_64-pc-windows-gnu/release/aetherhub.exe /out/windows/

FROM scratch
COPY --from=build /out /
