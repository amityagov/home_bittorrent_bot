FROM rust:1 AS build

RUN USER=root cargo new --bin home_bittorrent_bot
WORKDIR /home_bittorrent_bot

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

RUN cargo build --release
RUN rm src/*.rs

COPY ./src ./src

RUN rm -rf ./target/release/deps/home_bittorrent_bot*

RUN cargo build --release

# FROM rust:1-slim-buster
FROM ubuntu
RUN apt-get update && apt install -y openssl ca-certificates

WORKDIR /application
COPY ./config.toml ./config.toml
COPY --from=build /home_bittorrent_bot/target/release/home_bittorrent_bot .

CMD ["./home_bittorrent_bot"]