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

### Docker

> `hawkeye-api` currently has a hard dependency on being executed within a Kubernetes
> environment, so it's helpful to run Docker with Kubernetes enabled.

To run a Worker service:

```bash
docker build -f worker.Dockerfile -t hawkeye-worker .

docker run -it \
  -p 5000:5000/udp \
  -p 3030:3030 \
  -v $(pwd)/fixtures:/local \
  hawkeye-worker /local/watcher.json
```

Where:
- `5000:5000` exposes the API on port `5000`
- `3030:3030`

### OS X

Install `gstreamer` and dependencies.

```shell
brew install gstreamer gst-libav gst-plugins-base \
             gst-plugins-good gst-plugins-bad gst-plugins-ugly
```

Then, use `ffmpeg` to stream a prepared media file that could contain black slates,
regular 'ol video frames, and some slates of our choosing.

```shell
brew install ffmpeg
```

```shell
#!/usr/bin/env bash

file=$1
if [ -z $file ]; then
    me=$(basename $0)
    echo "USAGE: ${me} <path-to-video-file>"
    exit 1
fi

ffmpeg \
    -stream_loop -1 \
    -re \
    -y \
    -i "$1" \
    -an \
    -c:v copy \
    -f rtp_mpegts \
    udp://localhost:5000
```

## Prometheus Metrics

The Worker exposes metrics in the standard `/metrics` path for Prometheus to harvest.

```bash
curl http://localhost:3030/metrics
```

The payload looks something like:

```json

```
