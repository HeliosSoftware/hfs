# Pinned by digest; Dependabot bumps it.
FROM debian:13.7-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

ARG BINARY_NAME
ARG EXPOSE_PORT=8080
# Optional: directory inside the image that `hts run` will auto-import on
# first boot when the target DB is empty. Empty string = no bootstrap. Used
# by the hts image to point at the baked-in terminology-data/ directory.
ARG BOOTSTRAP_DIR=""
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

# Install runtime dependencies; upgrade picks up security fixes newer than the base
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

# Write state under /data: /app is root-owned (HFS_DATA_DIR stays, it holds shipped search params)
ENV HTS_DATABASE_URL=/data/hts.db
ENV HFS_BULK_EXPORT_OUTPUT_DIR=/data/bulk-export
ENV HFS_BULK_SUBMIT_OUTPUT_DIR=/data/bulk-submit
ENV HFS_EXPORT_DIR=/data/exports

# Numeric, so runAsNonRoot can verify it
USER 1000:1000

EXPOSE ${EXPOSE_PORT}

# No curl/wget in the image: probe GET /health over bash /dev/tcp
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
    CMD ["bash", "-c", "case \"$BINARY_NAME\" in hfs) p=${HFS_SERVER_PORT:-8080};; sof-server) p=${SOF_SERVER_PORT:-8080};; fhirpath-server) p=${FHIRPATH_SERVER_PORT:-3000};; hts) p=${HTS_SERVER_PORT:-8090};; *) exit 0;; esac; exec 3<>/dev/tcp/127.0.0.1/$p || exit 1; printf 'GET /health HTTP/1.0\\r\\nHost: localhost\\r\\n\\r\\n' >&3; read -r _ code _ <&3; [ \"$code\" = 200 ]"]

ENTRYPOINT ["sh", "-c", "exec /app/${BINARY_NAME} \"$@\"", "--"]
