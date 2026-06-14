# Build stage: compile a release binary.
FROM rust:1.96.0-slim-bookworm AS builder
WORKDIR /app
# The slim base omits curl, which utoipa-swagger-ui's build script uses to fetch
# the Swagger UI assets at build time. ca-certificates is needed for the HTTPS
# fetch.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# Cache dependencies separately from source for faster rebuilds: prebuild with
# stub sources, then drop them so the real sources recompile only the crate.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release \
    && rm -rf src
COPY . .
# Touch so cargo notices the real sources are newer than the stub build.
RUN touch src/main.rs src/lib.rs && cargo build --release --bin erbridge-geodesic

# Runtime stage: a slim image with just the binary and CA certs (needed for
# HTTPS to CCP's SDE endpoint and EVE-Scout).
FROM debian:bookworm-slim AS runtime

# Version metadata, passed by CI (the build context has no .git/). Surfaced as
# OCI image labels.
ARG APP_VERSION=0.0.0-dev
ARG GIT_COMMIT_SHA=unknown
LABEL org.opencontainers.image.title="erbridge-geodesic" \
      org.opencontainers.image.description="EVE Online gate-routing REST API" \
      org.opencontainers.image.version="${APP_VERSION}" \
      org.opencontainers.image.revision="${GIT_COMMIT_SHA}" \
      org.opencontainers.image.source="https://github.com/erbridge-foundation/erbridge-geodesic"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/erbridge-geodesic /usr/local/bin/erbridge-geodesic
# Persisted SDE cache lives here; mount a volume to survive restarts.
ENV GEODESIC_CACHE_DIR=/var/cache/erbridge-geodesic
EXPOSE 5001
ENTRYPOINT ["erbridge-geodesic"]
