#
# Build the executable artifacts to include in a final image.
#
FROM rust:1.59-slim-buster as builder

RUN apt update -qq
RUN apt install -yq --no-install-recommends \
    pkg-config \
    libssl-dev
COPY Cargo.toml /Cargo.toml
COPY Cargo.lock /Cargo.lock
COPY hawkeye-api /hawkeye-api
COPY hawkeye-core /hawkeye-core
COPY hawkeye-worker /hawkeye-worker
COPY resources /resources
RUN cargo build --release --package hawkeye-api

#
# Build the final image containing the built executables.
#
FROM debian:buster-slim as app

# Make RUST_LOG configurable at buld time.
# This may be overridden with `-e RUST_LOG=debug` at `docker run` time.
ARG RUST_LOG=info
ENV RUST_LOG ${RUST_LOG}

RUN apt-get update -qq \
    && apt install -y --no-install-recommends \
        libssl-dev

COPY --from=builder /target/release/hawkeye-api .
ENTRYPOINT ["/hawkeye-api"]
