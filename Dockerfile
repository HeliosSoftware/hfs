# Pinned by digest; Dependabot bumps them.
FROM debian:13.7-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2 AS builder

ARG BINARY_NAME

# Copy staged build context (binary + optional data files)
COPY . /app/

# Fixed binary path, so the exec-form ENTRYPOINT needs no shell
RUN test -n "$BINARY_NAME" || { echo "BINARY_NAME build arg is required"; exit 1; } && \
    mv "/app/$BINARY_NAME" /app/server && \
    chmod 755 /app/server && \
    mkdir /data

FROM gcr.io/distroless/cc-debian13@sha256:809d04afa1be033c441f76bb0051878c342fb76d0ea236bc6dc94b655fa0fbd6

ARG EXPOSE_PORT=8080
# Optional: directory inside the image that `hts run` will auto-import on
# first boot when the target DB is empty. Empty string = no bootstrap. Used
# by the hts image to point at the baked-in terminology-data/ directory.
ARG BOOTSTRAP_DIR=""

WORKDIR /app

COPY --from=builder /app /app
COPY --from=builder --chown=1000:1000 /data /data
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

ENTRYPOINT ["/app/server"]
