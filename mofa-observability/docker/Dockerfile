FROM rust:1.89-slim as builder

WORKDIR /usr/src/mofa
# Copy entire workspace since mofa-observability depends on workspace Cargo.toml
COPY . .

WORKDIR /usr/src/mofa/mofa-observability
RUN cargo build --example mock_harness --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/src/mofa/target/release/examples/mock_harness /usr/local/bin/mock_harness

EXPOSE 9092
CMD ["mock_harness"]
