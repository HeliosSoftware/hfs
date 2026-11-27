# Local-only image for benchmarking HFS next to WSO2.
# Build context = a clean `git archive` export of HFS main (see ../README-hfs.md).
# Same release build and linker flags as HFS's own fhir-benchmark workflow
# (clang + lld, 8 MB stack), FHIR R4 only, `ui` left out.
FROM rust:1-trixie AS build
RUN apt-get update && \
    apt-get install -y --no-install-recommends clang lld cmake pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN mkdir -p /root/.cargo && printf '[target.x86_64-unknown-linux-gnu]\nlinker = "clang"\nrustflags = ["-C", "link-arg=-fuse-ld=lld", "-C", "link-arg=-Wl,-zstack-size=8388608"]\n' > /root/.cargo/config.toml
ARG FEATURES="R4,sqlite,postgres"
RUN cargo build --release -p helios-hfs --no-default-features --features "${FEATURES}"

FROM debian:trixie-slim
RUN apt-get update && \
    apt-get install -y --no-install-recommends libssl3t64 ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN groupadd -g 1000 helios && useradd -u 1000 -g helios -m helios
WORKDIR /app
COPY --from=build /src/target/release/hfs /app/hfs
COPY --from=build /src/data /app/data
ENV HFS_SERVER_HOST=0.0.0.0 \
    HFS_SERVER_PORT=8080 \
    HFS_DATA_DIR=/app/data
USER helios
EXPOSE 8080
ENTRYPOINT ["/app/hfs"]
