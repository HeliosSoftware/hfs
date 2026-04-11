FROM debian:bookworm-slim

ARG BINARY_NAME
ARG EXPOSE_PORT=8080

# Validate BINARY_NAME is set
RUN test -n "$BINARY_NAME" || { echo "BINARY_NAME build arg is required"; exit 1; }

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends libssl3 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -g 1000 hfs && useradd -u 1000 -g hfs -m hfs

WORKDIR /app

# Copy staged build context (binary + optional data files)
COPY . /app/

RUN chmod +x /app/${BINARY_NAME}

# Persist BINARY_NAME for the entrypoint
ENV BINARY_NAME=${BINARY_NAME}

# Create writable data directory for SQLite and other persistent data
RUN mkdir -p /data && chown hfs:hfs /data
VOLUME /data

# Default host binding for all servers (each binary reads only its own env var)
ENV HFS_SERVER_HOST=0.0.0.0
ENV HFS_DATABASE_URL=:memory:
ENV SOF_SERVER_HOST=0.0.0.0
ENV FHIRPATH_SERVER_HOST=0.0.0.0
ENV HTS_SERVER_HOST=0.0.0.0

USER hfs

EXPOSE ${EXPOSE_PORT}

ENTRYPOINT ["sh", "-c", "exec /app/${BINARY_NAME} \"$@\"", "--"]
