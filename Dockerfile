FROM rust:1.90-bookworm AS builder

COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

COPY --from=builder target/release/externalip-manager /usr/local/bin/
RUN chmod +x /usr/local/bin/externalip-manager

# run unprivileged
USER 1001

CMD ["externalip-manager"]
