# Hawkeye

Detect images in a video stream and execute automated actions.

## Use case

Triggering Ad insertion on an AWS MediaLive Channel when a slate image is present in the
video feed.

![Diagram showing usage of Hawkeye](resources/HawkeyeDesign.jpg)

## Running locally

The full Hawkeye application consists of:

1. a REST API that manages Workers
2. Workers that watch a stream for slates of interest, with each slate match firing
   events unique to it.

> Data is persisted via utilizing Kubernetes metadata to store and retrieve relational
> data.

There are multiple ways to run these components locally.

- Run a local API to accept `POST /v1/watchers` payloads. This request will also spawn
  a Kubernetes deployment and a "Watcher service" that runs a worker that digests
  a stream source to look for the configured slates (and fire the associated actions
  for that slate...).
- You may forgo the API abstraction and call a Watcher with a hardcoded config directly:
  ```shell
  cargo build --bin hawkeye-worker
  ./target/debug/hawkeye-worker fixtures/watcher-basic.json
  # Then, stream video to that port. See below for how to do this via `ffmpeg`.
  ```

> `hawkeye-api` currently has a hard dependency on being executed within a Kubernetes
> environment, so it's helpful to run Docker with Kubernetes enabled if you use
> something like Docker Desktop.

### Local Docker Setup

To run a Worker service:

```bash
docker build -f worker.Dockerfile -t hawkeye-worker .

docker run -it \
  -p 5000:5000/udp \
  -p 3030:3030 \
  -v $(pwd)/fixtures:/local \
  hawkeye-worker /local/watcher-basic.json
```

Where:
- `5000:5000` exposes the API on port `5000`
- `3030:3030`

### Local OS X Setup

Install `gstreamer` and dependencies so Cargo dependencies are able to be compiled.

```shell
brew install gstreamer gst-libav gst-plugins-base \
             gst-plugins-good gst-plugins-bad gst-plugins-ugly
```

### Local Linux (Debian) Setup

```shell
apt-get install -y --no-install-recommends \
  pkg-config libssl-dev libglib2.0-dev \
  libgstreamer1.0-dev gstreamer1.0-libav libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### Streaming Video

You can easily stream prepared media via `ffmpeg` to a Watcher to test functionality.

For example, you might have a set of slates configured on a Watcher and a custom video
clip that alternates playing a video with the slates spliced in. If you wanted to test
end-to-end, you'd want to stream this video to a Watcher and verify the configured
actions take place for each transition.

```shell
brew install ffmpeg
```

Save the following script to file to easily stream video files or run the `ffmpeg`
command yourself. Don't forget to `chmod +x <your-script-name.sh>` to make life easy.

```bash
#!/usr/bin/env bash

file=$1
port=$2
if [[ -z $file ]] || [[ -z $port ]]; then
    me=$(basename $0)
    echo "USAGE: ${me} <path-to-video-file> <port>"
    exit 1
fi

ffmpeg \
    -re \
    -y \
    -i "${file}" \
    -an \
    -c:v copy \
    -f rtp_mpegts \
    udp://0.0.0.0:${port}
```

## Prometheus Metrics

The Worker exposes metrics in the standard `/metrics` path for Prometheus to harvest.

```bash
curl http://localhost:3030/metrics
```
