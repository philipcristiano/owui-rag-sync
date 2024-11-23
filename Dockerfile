FROM lukemathwalker/cargo-chef:latest-rust-1.82-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin owui-rag-sync

# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install openssl ca-certificates -y && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/owui-rag-sync /usr/local/bin

ENTRYPOINT ["/usr/local/bin/owui-rag-sync"]
