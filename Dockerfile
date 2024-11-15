FROM rust:1.82-bookworm as builder
WORKDIR /usr/src/app

COPY . .
RUN cargo install --path .

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y procps ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/owui-rag-sync /usr/local/bin/owui-rag-sync

ENTRYPOINT ["/usr/local/bin/owui-rag-sync"]
