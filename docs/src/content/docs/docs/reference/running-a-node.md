---
title: Running a Node
description: How to run a local nXCC node for development and debugging.
---

Running a local nXCC node is the best way to develop and debug your workers. It gives you direct access to logs and allows you to iterate quickly without deploying to a public network.

## Using Docker (Recommended)

The easiest way to run a node is with Docker. We provide a pre-built, multi-platform image on GitHub Container Registry.

### 1. Pull the Image

```bash
docker pull ghcr.io/nxcc-bridge/nxcc/node:latest
```

### 2. Run the Container

To run a node for local development, you'll typically want to connect it to a local blockchain like Anvil. First, create a Docker network, then run both containers on it.

```bash
# Create a network if you haven't already
docker network create nxcc-dev-net

# Run Anvil (in a separate terminal)
docker run --rm -it --name anvil -p 8545:8545 --network nxcc-dev-net --network-alias anvil \
  ghcr.io/foundry-rs/foundry:latest anvil --host 0.0.0.0

# Run the nXCC node
docker run -d --rm \
  --name nxcc-node \
  -p 6922:6922 \
  --network nxcc-dev-net \
  -e RUST_LOG=info,nxcc_daemon=debug \
  -e NXCC_HTTP_API_ENABLED=true \
  -e NXCC_HTTP_API_CORS_ALLOWED_ORIGINS='*' \
  ghcr.io/nxcc-bridge/nxcc/node:latest
```

### Key Configuration

-   `-p 6922:6922`: Exposes the node's HTTP API, which the `@nxcc/cli` uses for `worker deploy`.
-   `--network nxcc-dev-net`: Connects the node to the same network as your Anvil container.
-   `-e RUST_LOG=...`: Controls the log level. `nxcc_daemon=debug` is useful for development.
-   `-e NXCC_HTTP_API_ENABLED=true`: **Required** to enable the `/api/work-orders` endpoint for deployments.

### Viewing Logs

You can view the real-time output from your node and any running workers using the `docker logs` command. This is essential for debugging.

```bash
docker logs -f nxcc-node
```

## Building from Source (Advanced)

For advanced development, you can build and run the node directly from the source code in the `node/` directory of the [nXCC repository](https://github.com/nxcc-bridge/nxcc). The `node/entrypoint.sh` script provides a clear example of how the different services (daemon, enclave, vm) are started and configured to work together.
