# Hawkeye

Detect images in a video stream and execute automated actions.

## Use case

Triggering Ad insertion on an AWS MediaLive Channel when a slate image is present in the
video feed.

![Diagram showing usage of Hawkeye](resources/HawkeyeDesign.jpg)

## Running locally

The full Hawkeye application consists of a REST API that manages Workers, Workers
that watch a stream for slates of interest to fire actions, and all the state services
needed for persistence of knowledge.

There are multiple ways to run these components locally.

### Minikube (full application)

To orchestrate all local services in a way similar to how they run in production, we
can use
[Minikube](https://minikube.sigs.k8s.io/docs/start/) to have a local Kubernetes
experience.

> TODO: We need to finish creating a helm chart to use this method.

### Docker

> `hawkeye-api` currently has a hard dependency on being executed within a Kubernetes
> environment.

```bash
export HAWKEYE_DOCKER_IMAGE=hawkeye-dev:latest
```

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

```shell
brew install ffmpeg
brew install gst-libav gst-plugins-base gst-plugins-good gst-plugins-bad gstreamer

ffmpeg -f avfoundation -list_devices true -i ""
ffmpeg \
  -f avfoundation \
  -pix_fmt yuyv422 \
  -video_size 640x480 \
  -framerate 15 \
  -i "1:1" -ac 2 \
  -vf format=yuyv422 \
  -vcodec h264 \
  -bufsize 2000k -acodec aac -ar 44100 -b:a 128k \
  -f rtp_mpegts udp://0.0.0.0:5000

ffmpeg \
    -re \
    -i Big_Buck_Bunny_360_10s_1MB.mp4 \
    -an \
    -c:v copy \
    -f rtp \
    -sdp_file video.sdp \
    "rtp://0.0.0.0:5000"

```

## Prometheus Metrics

The Worker exposes metrics in the standard `/metrics` path for Prometheus to harvest.

```bash
curl http://localhost:3030/metrics
```

The payload looks something like:

```json

```


-----

