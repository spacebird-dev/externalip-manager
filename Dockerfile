FROM rust:1.95-bookworm AS builder

COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt update \
    && apt -y install ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder target/release/externalip-manager /usr/local/bin/
RUN chmod +x /usr/local/bin/externalip-manager

# run unprivileged
USER 1001

CMD ["externalip-manager"]
