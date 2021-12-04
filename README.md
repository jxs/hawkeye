# Hawkeye

Detect images in a video stream and execute automated actions.

## Use case

Triggering Ad insertion on an AWS MediaLive Channel when a slate image is present in the
video feed.

![Diagram showing usage of Hawkeye](resources/HawkeyeDesign.jpg)

## Running locally

The worker can be run independently with Docker or directly on the host machine.

### Running the full Hawkeye application in Minikube

The full Hawkeye application consists of a REST API that manages the Workers using the
Kubernetes API.

First we build the API docker image:

```bash
docker build -f api.Dockerfile -t hawkeye-api .
```

### Running the Worker directly with Docker

> `hawkeye-api` currently has a hard dependency on being executed within a Kubernetes
> environment. See the above section on running locally via Minikube which supports
> running _all_ services locally.

```bash
docker build -f worker.Dockerfile -t hawkeye-worker .
docker run -it \
  -p 5000:5000/udp \
  -p 3030:3030 \
  -v /home/user/dev/hawkeye/fixtures:/local \
  hawkeye-worker /local/watcher.json
```

## Prometheus Metrics

The Worker exposes metrics in the standard `/metrics` path for Prometheus to harvest.

```bash
curl http://localhost:3030/metrics
```

The payload looks something like:

```json

```
