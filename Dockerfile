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
WORKDIR /music-srv
COPY --from=builder /music-srv/target/release/music-srv-small ./music-srv
COPY ./templates ./templates
RUN apt-get update && apt-get install -y yt-dlp mp3gain

LABEL org.opencontainers.image.source="https://github.com/chrisheib/rs_music_server"

CMD ["./music-srv"]