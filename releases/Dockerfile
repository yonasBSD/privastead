# Build arguments
ARG CRATE_NAME=""
ARG FEATURES=""
ARG RUST_HASH="5861091755a3298619c94a6cbf0534062cfeb9afcda9f698a13df3ef64e2dd8b" # Provided as default to quiet docker warning. This is never used (unless separately passed in).
ARG CARGO_TARGET=""
ARG BINARY_FILE_NAME=""

# Use a digest-locked Rust image as our builder
FROM docker.io/rust@sha256:${RUST_HASH} AS builder

ARG CRATE_NAME
ARG FEATURES
ARG CARGO_TARGET
ARG BINARY_FILE_NAME

WORKDIR /app

# Install build deps for crates that rely on system crypto libs (e.g., nettle-sys).
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      clang pkg-config nettle-dev; \
    if [ "$CRATE_NAME" = "motion_ai/cli" ]; then \
      apt-get install -y --no-install-recommends \
        libavutil-dev libavcodec-dev libavformat-dev libavfilter-dev \
        libavdevice-dev libswscale-dev libswresample-dev; \
    fi; \
    rm -rf /var/lib/apt/lists/*

# Copy the files from host into the builder
COPY --from=proj . .
WORKDIR /app/${CRATE_NAME}

# Build the intended crate
ENV CARGO_INCREMENTAL=0
ENV TZ=UTC
ENV RUSTFLAGS="-C link-arg=-Wl,--build-id=none --remap-path-prefix=/app=."
RUN rustup target add ${CARGO_TARGET}
RUN cargo build --release ${FEATURES} --locked --target ${CARGO_TARGET}

# Copy the binary artifact into the host directory
FROM scratch AS artifact
ARG CRATE_NAME
ARG CARGO_TARGET
ARG BINARY_FILE_NAME

COPY --from=builder /app/${CRATE_NAME}/target/${CARGO_TARGET}/release/${BINARY_FILE_NAME} /${BINARY_FILE_NAME}
