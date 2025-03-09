FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Copy application
COPY . .
COPY development.p8 /app/development.p8
RUN cargo build --release --bin EchoScopeBackend

# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y libssl3 && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/EchoScopeBackend /usr/local/bin/EchoScopeBackend
COPY --from=builder /app/development.p8 /app/development.p8

# Create a symbolic link
#RUN ln -s /lib/x86_64-linux-gnu/libssl.so.3 /usr/lib/libssl.so.3
#RUN ln -s /lib/x86_64-linux-gnu/libcrypto.so.3 /usr/lib/libcrypto.so.3

ENTRYPOINT ["/usr/local/bin/EchoScopeBackend"]