FROM rust:1.85-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y git ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/devnpc /usr/local/bin/
ENTRYPOINT ["devnpc"]