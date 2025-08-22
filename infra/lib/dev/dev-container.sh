#!/bin/bash
#
# NXCC Development Container Management Script
#
# Usage: ./dev-container.sh [--detached]

set -e

CONTAINER_NAME="nxcc-dev-container"
CONTAINER_IMAGE="${NXCC_DEV_IMAGE:-ghcr.io/nxcc-bridge/nxcc/dev:latest}"
DETACHED=false

# Parse arguments
while [[ $# -gt 0 ]]; do
	case $1 in
	--detached | -d)
		DETACHED=true
		shift
		;;
	*)
		echo "Unknown option: $1"
		echo "Usage: $0 [--detached]"
		exit 1
		;;
	esac
done

echo "=== NXCC Development Container Management ==="
echo ""

# Stop existing container if running
if docker ps -q -f name="$CONTAINER_NAME" | grep -q .; then
	echo "Stopping existing container..."
	docker stop "$CONTAINER_NAME" >/dev/null
fi

# Remove existing container
if docker ps -a -q -f name="$CONTAINER_NAME" | grep -q .; then
	echo "Removing existing container..."
	docker rm "$CONTAINER_NAME" >/dev/null
fi

# Pull latest image
echo "Pulling development container image: $CONTAINER_IMAGE"
docker pull "$CONTAINER_IMAGE"

# Start container
echo "Starting development container..."
if [[ "$DETACHED" == "true" ]]; then
	docker run -d \
		--name "$CONTAINER_NAME" \
		--privileged \
		--device /dev/tdx_guest:/dev/tdx_guest \
		-v /home/ubuntu/nxcc:/workspace \
		-v /sys/kernel/config:/sys/kernel/config:ro \
		-w /workspace \
		"$CONTAINER_IMAGE" \
		sleep infinity

	echo "✅ Development container started in background"
	echo "Connect with: docker exec -it $CONTAINER_NAME bash"
else
	docker run -it --rm \
		--name "$CONTAINER_NAME" \
		--privileged \
		--device /dev/tdx_guest:/dev/tdx_guest \
		-v /home/ubuntu/nxcc:/workspace \
		-v /sys/kernel/config:/sys/kernel/config:ro \
		-w /workspace \
		"$CONTAINER_IMAGE"
fi
