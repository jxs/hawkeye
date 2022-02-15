# Hawkeye

Detect images in a video stream and execute automated actions.

## Use case

Triggering Ad insertion on an AWS MediaLive Channel when a slate image is present in the
video feed.

![Diagram showing usage of Hawkeye](resources/HawkeyeDesign.jpg)

## Components

### API

The REST API is responsible to manage Workers, serve healthchecks, and
provide Prometheus metrics.

When you create a Watcher by doing a `POST /v1/watchers`, the API will create some
Kubernetes artifacts: ConfigMap, Deployment, Service. These items are to support
starting the Watcher in Kubernetes, but they also contain metadata that is used to tie
these resources back to a specific Watcher config. When you do a `GET /v1/watchers`,
the endpoint just reads various Kubernetes metadata to build the listing. When you do a
`DELETE /v1/watchers/{uuid}`, then the Kubernetes artifacts are deleted. Kubernetes is
the current backend, basically.

After creating a Watcher, notice it is in `status=pending`. You have to "start" it using
`POST /v1/watchers/{uuid}/start`. The status should then update to `status=running`.

*The Kubernetes service will run a Docker image that is expected to be pre-built.*
The default value for this image name is `hawkeye-worker:latest` and may be overridden
by setting the environment variable `HAWKEYE_DOCKER_IMAGE` when starting the API.
To build:

```shell
docker build -f worker.Dockerfile -t hawkeye-worker .
```

The API currently only supports a hardcoded header token for authentication.
(there are open issues to make this better...).  Be sure to set the environment variable
`HAWKEYE_FIXED_TOKEN` and use it in all API requests by supplying the typical auth
header `Authorization: Bearer {token}`.

### Watcher

## Running locally

There are multiple ways to run these components locally.

- Run a local API to accept `POST /v1/watchers` payloads. This request will also spawn
  a Kubernetes deployment and a "Watcher service" that runs a worker that digests
  a stream source to look for the configured slates (and fire the associated actions
  for that slate...).
- You may forgo the API abstraction and create an on-the-fly Watch by using a hardcoded
  config directly:
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

### Streaming Prepared Video

You can use `ffmpeg` to send a RTP stream to a Watcher using a pre-recorded video file. This is useful to test functionality and to verify changes during development.

> `ffmpeg` is a popular app, so it'll be in whatever package manager you use.

The prepared video might be a custom video clip that plays a video clip of your
choosing, but with the slates that were configured against a Watcher spliced in to the
video clip.

```shell
# bs = black slate. optional, but tests more functionality.
| bs | ...content... | bs | slate1.jpg | bs | ...content... | bs | slate2.png | bs |
```
You can get notified of it "working" by reading `RUST_LOG=debug` output or setting up
the actions for a slate transition to call a URL to a web server that you control.

> You can even use something like https://github.com/LyleScott/blackhole which starts
> a basic Python webserver that will print out request details as they are made against
> the server, regardless of endpoint, HTTP verb, or payload.

To test API -> Watcher functionality end-to-end, you'll need to start the `hawkeye-api`
and do a `POST http://localhost:8080/v1/watchers` with something like:

```json
{
  "description": "test watcher",
  "source": {
    "ingest_port": 5000,
    "container": "mpeg-ts",
    "codec": "h264",
    "transport": {
      "protocol": "rtp"
    }
  },
  "transitions": [
    {
      "from": {
        "frame_type": "content",
      },
      "to": {
        "frame_type": "slate",
        "slate_context": {
          "url": "file://./resources/slate_fixtures/slate-0-cbsaa-213x120.jpg"
        }
      },
      "actions": [
        {
          "description": "Trigger AdBreak using API",
          "type": "http_call",
          "method": "POST",
          "retries": 3,
          "timeout": 10,
          "url": "http://non-existent.cbs.com/v1/organization/cbsa/channel/slate4/ad-break",
          "authorization": {
            "basic": {
              "username": "dev_user",
              "password": "something"
            }
          },
          "headers": {
            "Content-Type": "application/json"
          },
          "body": "{\"duration\":300}"
        }
      ]
    },
    {
      "from": {
        "frame_type": "slate",
        "slate_context": {
          "url": "file://./resources/slate_fixtures/slate-0-cbsaa-213x120.jpg"
        }
      },
      "to": {
        "frame_type": "content",
      },
      "actions": [
        {
          "description": "Use dump out of AdBreak API call",
          "type": "http_call",
          "method": "DELETE",
          "timeout": 10,
          "url": "http://non-existent.cbs.com/v1/organization/cbsa/channel/slate4/ad-break",
          "authorization": {
            "basic": {
              "username": "dev_user",
              "password": "something"
            }
          }
        }
      ]
    }
  ]
}
```

Save the following script to file to easily stream video files or run the `ffmpeg`
command yourself. Don't forget to `chmod +x <your-script-name.sh>` to make life easy.

```bash
#!/usr/bin/env bash

file=$1
host=$2
port=$3
if [[ -z $file ]] || [[ -z $host ]] || [[ -z $port ]]; then
    me=$(basename $0)
    echo "USAGE: ${me} <path-to-video-file> <host> <port>"
    exit 1
fi

ffmpeg \
    -re \
    -y \
    -i "${file}" \
    -an \
    -c:v copy \
    -f rtp_mpegts \
    udp://${host}:${port}
```

## Prometheus Metrics

The Worker exposes metrics in the standard `/metrics` path for Prometheus to harvest.

```bash
curl http://localhost:3030/metrics
```

## Environment Variables

| Environment Variable      | Default | Description                                    |
| ------------------------- | ------- | ---------------------------------------------- |
| `HAWKEYE_ENV`             | local   | `dev`/`prod`/whatever you want                 |
| `HAWKEYE_SENTRY_DSN    `  | <none>  | the DSN url to the Sentry project to use       |
| `HAWKEYE_SENTRY_ENABLED`  | `0`     | `"1"` or `0` will toggle Sentry initialization |
