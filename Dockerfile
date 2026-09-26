FROM rust:1.83-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/aegis-core/Cargo.toml crates/aegis-core/
COPY crates/aegis-proxy/Cargo.toml crates/aegis-proxy/
COPY crates/aegis-bench/Cargo.toml crates/aegis-bench/

# Cache deps layer
RUN mkdir -p crates/aegis-core/src crates/aegis-proxy/src crates/aegis-bench/src \
    && echo "pub fn placeholder() {}" > crates/aegis-core/src/lib.rs \
    && echo "fn main() {}" > crates/aegis-proxy/src/main.rs \
    && echo "pub mod config; pub mod handler; pub mod metrics;" > crates/aegis-proxy/src/lib.rs \
    && echo "fn main() {}" > crates/aegis-bench/src/main.rs \
    && mkdir -p crates/aegis-bench/benches \
    && echo "fn main() {}" > crates/aegis-bench/benches/pipeline.rs \
    && cargo build --release -p aegis-proxy 2>/dev/null; true

COPY crates/aegis-core/src crates/aegis-core/src/
COPY crates/aegis-proxy/src crates/aegis-proxy/src/

RUN touch crates/aegis-core/src/lib.rs crates/aegis-proxy/src/main.rs \
    && cargo build --release -p aegis-proxy

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/aegis-proxy /usr/local/bin/aegis-proxy
COPY blocklist.txt ./blocklist.txt

ENV LISTEN_ADDR=0.0.0.0:8899 \
    DOWNSTREAM_URL=http://validator:8900 \
    BLOCKLIST_PATH=/app/blocklist.txt

EXPOSE 8899

ENTRYPOINT ["aegis-proxy"]
