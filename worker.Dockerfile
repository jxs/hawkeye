#
# Build the executable artifacts to include in a final image.
#
FROM rust:1.57-slim-buster as builder

RUN apt update -qq
RUN apt install -y \
    pkg-config \
    libglib2.0-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev
#              gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
#              gstreamer1.0-plugins-bad gstreamer1.0-libav libgstrtspserver-1.0-dev libges-1.0-dev
COPY Cargo.toml /Cargo.toml
COPY Cargo.lock /Cargo.lock
COPY hawkeye-api /hawkeye-api
COPY hawkeye-core /hawkeye-core
COPY hawkeye-worker /hawkeye-worker
COPY resources /resources
RUN cargo build --release --package hawkeye-worker

##
## Build the final image containing the built executables.
##
#FROM debian:buster-slim as app
#
## Make RUST_LOG configurable at buld time.
## This may be overridden with `-e RUST_LOG=debug` at `docker run` time.
#ARG RUST_LOG=info
#ENV RUST_LOG ${RUST_LOG}
#
###RUN apt update \
###    && apt install -y \
###           gstreamer1.0-plugins-base \
###           gstreamer1.0-plugins-good \
###           gstreamer1.0-plugins-bad \
###           gstreamer1.0-libav \
###    && apt-get clean
#
#COPY --from=builder /target/release/hawkeye-worker .
#ENTRYPOINT ["/hawkeye-worker"]
