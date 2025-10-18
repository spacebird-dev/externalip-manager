FROM rust:1.90-trixie AS builder

COPY . .
RUN cargo build --release

FROM debian:trixie-slim

COPY --from=builder target/release/externalip-manager /usr/local/bin/
RUN chmod +x /usr/local/bin/externalip-manager

# run unprivileged
USER 1001

CMD ["externalip-manager"]
