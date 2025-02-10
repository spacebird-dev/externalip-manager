FROM debian:bookworm-slim

ARG TARGET_DIR=release

COPY target/${TARGET_DIR}/externalip-manager /usr/local/bin/
RUN chmod +x /usr/local/bin/externalip-manager

# run unprivileged
USER 1001

CMD ["externalip-manager"]
