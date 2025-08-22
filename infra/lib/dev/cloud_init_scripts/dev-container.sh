#!/bin/bash

# Default container name and image
CONTAINER_NAME="nxcc-dev-container"
IMAGE_TAG="ghcr.io/nxcc-bridge/nxcc/dev:latest"
CODE_PATH="/home/ubuntu/nxcc"

# Parse arguments
DETACHED=false
while [[ $# -gt 0 ]]; do
	case $1 in
	--name)
		CONTAINER_NAME="$2"
		shift 2
		;;
	--image)
		IMAGE_TAG="$2"
		shift 2
		;;
	--detached | -d)
		DETACHED=true
		shift
		;;
	*) break ;;
	esac
done

if [ ! -d "$CODE_PATH" ]; then
	echo "⚠️  NXCC code not found at $CODE_PATH. Sync code first."
	mkdir -p "$CODE_PATH"
fi

echo "=== Starting NXCC Development Container ==="
echo "Container name: $CONTAINER_NAME"
echo "Image: $IMAGE_TAG"
echo "Code path: $CODE_PATH"

if docker ps -q -f name="$CONTAINER_NAME" | grep -q .; then
	echo "🔄 Stopping existing container: $CONTAINER_NAME"
	docker stop "$CONTAINER_NAME" >/dev/null 2>&1
fi
if docker ps -aq -f name="$CONTAINER_NAME" | grep -q .; then
	echo "🗑️  Removing existing container: $CONTAINER_NAME"
	docker rm "$CONTAINER_NAME" >/dev/null 2>&1
fi

TDX_MOUNT=""
if [ -c /dev/tdx_guest ]; then
	TDX_MOUNT="--device=/dev/tdx_guest:/dev/tdx_guest"
	echo "✅ Mounting TDX device for hardware testing"
else
	echo "⚠️  TDX device not available - tests will use simulation mode"
fi

ADDITIONAL_MOUNTS="-v /dev:/dev -v /sys:/sys"

if [ "$DETACHED" = true ]; then
	echo "🚀 Starting container in detached mode..."
	docker run -d --name "$CONTAINER_NAME" --privileged -v "$CODE_PATH:/workspace" -w /workspace $TDX_MOUNT "$ADDITIONAL_MOUNTS" "$IMAGE_TAG" tail -f /dev/null
	echo "✅ Container started: $CONTAINER_NAME. Attach with: docker exec -it $CONTAINER_NAME bash"
else
	echo "🚀 Starting interactive container..."
	docker run -it --rm --name "$CONTAINER_NAME" --privileged -v "$CODE_PATH:/workspace" -w /workspace $TDX_MOUNT "$ADDITIONAL_MOUNTS" "$IMAGE_TAG" bash
fi
