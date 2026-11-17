# Runtime-only image: it compiles nothing and copies an already-built binary
# out of a *staged* build context (see .claude/skills/docker-and-release).
#
# Base: Debian 13 (trixie) slim, pinned by tag and by the digest of its
# multi-platform index, so amd64 and arm64 builds resolve the same release and
# a rebuild cannot silently pick up a different base. Dependabot
# (.github/dependabot.yml) proposes the next tag + digest.
FROM debian:13.7-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

ARG BINARY_NAME
ARG EXPOSE_PORT=8080
# Optional: directory inside the image that `hts run` will auto-import on
# first boot when the target DB is empty. Empty string = no bootstrap. Used
# by the hts image to point at the baked-in terminology-data/ directory.
ARG BOOTSTRAP_DIR=""
# OCI annotations. CI can pass the real values; the defaults keep a local
# build working.
ARG VERSION="dev"
ARG REVISION="unknown"
ARG CREATED=""

LABEL org.opencontainers.image.title="${BINARY_NAME}" \
      org.opencontainers.image.description="Helios FHIR Server (${BINARY_NAME})" \
      org.opencontainers.image.source="https://github.com/HeliosSoftware/hfs" \
      org.opencontainers.image.url="https://github.com/HeliosSoftware/hfs" \
      org.opencontainers.image.vendor="Helios Software" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}" \
      org.opencontainers.image.base.name="docker.io/library/debian:13.7-slim"

# Validate BINARY_NAME is set
RUN test -n "$BINARY_NAME" || { echo "BINARY_NAME build arg is required"; exit 1; }

# Install runtime dependencies. `upgrade` pulls in trixie-security fixes newer
# than the pinned base (e.g. zlib1g CVE-2026-85091 on 2026-11-17); the package
# versions themselves are left to the Debian release (see .hadolint.yaml).
RUN apt-get update && \
    apt-get upgrade -y --no-install-recommends && \
    apt-get install -y --no-install-recommends libssl3t64 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -g 1000 helios && useradd -u 1000 -g helios -m helios

WORKDIR /app

# Copy staged build context (binary + optional data files)
COPY . /app/

RUN chmod +x /app/${BINARY_NAME}

# Persist BINARY_NAME for the entrypoint
ENV BINARY_NAME=${BINARY_NAME}

# Create writable data directory for SQLite and other persistent data
RUN mkdir -p /data && chown 1000:1000 /data
VOLUME /data

# Default host binding for all servers (each binary reads only its own env var)
ENV HFS_SERVER_HOST=0.0.0.0
ENV HFS_DATABASE_URL=:memory:
ENV SOF_SERVER_HOST=0.0.0.0
ENV FHIRPATH_SERVER_HOST=0.0.0.0
ENV HTS_SERVER_HOST=0.0.0.0
ENV HTS_BOOTSTRAP_DIR=${BOOTSTRAP_DIR}

# Everything the servers write goes under the /data volume, the only directory
# the runtime user owns. The built-in defaults are relative to /app (root-owned),
# so without these `hts` exits at startup ("unable to open database file:
# ./data/hts.db") and an hfs `$export` fails with "Permission denied" creating
# ./data/exports. HFS_DATA_DIR is NOT moved: hfs reads the search parameters
# shipped in /app/data from it.
ENV HTS_DATABASE_URL=/data/hts.db
ENV HFS_BULK_EXPORT_OUTPUT_DIR=/data/bulk-export
ENV HFS_BULK_SUBMIT_OUTPUT_DIR=/data/bulk-submit
ENV HFS_EXPORT_DIR=/data/exports

# Numeric uid:gid (the `helios` user above), so runtimes that enforce
# runAsNonRoot can verify it without reading /etc/passwd.
USER 1000:1000

EXPOSE ${EXPOSE_PORT}

# The image has no curl or wget; bash's /dev/tcp sends a plain HTTP/1.0
# `GET /health` (every server binary has that route) to the port the binary
# itself reads from its own *_SERVER_PORT variable. An unknown BINARY_NAME
# reports healthy rather than guess a port. The start period covers the hts
# first-boot terminology import.
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
    CMD ["bash", "-c", "case \"$BINARY_NAME\" in hfs) p=${HFS_SERVER_PORT:-8080};; sof-server) p=${SOF_SERVER_PORT:-8080};; fhirpath-server) p=${FHIRPATH_SERVER_PORT:-3000};; hts) p=${HTS_SERVER_PORT:-8090};; *) exit 0;; esac; exec 3<>/dev/tcp/127.0.0.1/$p || exit 1; printf 'GET /health HTTP/1.0\\r\\nHost: localhost\\r\\n\\r\\n' >&3; read -r _ code _ <&3; [ \"$code\" = 200 ]"]

ENTRYPOINT ["sh", "-c", "exec /app/${BINARY_NAME} \"$@\"", "--"]
