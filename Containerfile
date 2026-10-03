# The sync server as a container image. Build from the repository root:
#   podman build -t hab-server -f Containerfile .      (or docker build)
# See docs/running-the-server.md.

FROM docker.io/library/rust:1-bookworm AS build
WORKDIR /src
COPY . .
# Only the server is built; the desktop app and its UI are not needed.
RUN cargo build --release --locked -p hab-server \
 && mkdir -p /out/data /out/backups

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/hab-server /usr/local/bin/hab-server
# The image runs as an unprivileged user, so it listens on 8443; publish it as
# 443 with `-p 443:8443`.
COPY --from=build --chown=nonroot:nonroot /out/data /data
COPY --from=build --chown=nonroot:nonroot /out/backups /backups
ENV HAB_SERVER_DATA_DIR=/data \
    HAB_SERVER_LISTEN=0.0.0.0:8443 \
    HAB_SERVER_BACKUP_DIR=/backups
VOLUME ["/data", "/backups"]
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/hab-server"]
