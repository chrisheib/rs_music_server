FROM rust:latest AS chef
RUN rustup default nightly
RUN cargo install cargo-chef
WORKDIR /music-srv


FROM chef AS planner
COPY ./src ./src
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json


FROM chef AS builder
COPY --from=planner /music-srv/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY ./src ./src
COPY ./Cargo.toml ./Cargo.toml
COPY ./Cargo.lock ./Cargo.lock
RUN cargo build --release --bin music-srv
RUN objcopy --compress-debug-sections ./target/release/music-srv ./target/release/music-srv-small


FROM debian:stable-slim AS runtime
RUN apt-get update && apt-get install -y mp3gain ffmpeg python3-pip && rm -rf /var/lib/apt/lists/*
ENV DENO_INSTALL=/usr/local
RUN curl -fsSL https://deno.land | sh
RUN pip3 install --root-user-action=ignore --break-system-packages --upgrade "yt-dlp[default]"
WORKDIR /music-srv
COPY --from=builder /music-srv/target/release/music-srv-small ./music-srv
COPY ./templates ./templates
ARG BUILD_TIMESTAMP=dev

LABEL org.opencontainers.image.source="https://github.com/chrisheib/rs_music_server"
RUN printf '%s\n' "$BUILD_TIMESTAMP" > ./build-timestamp.txt

CMD ["./music-srv"]

