#!/bin/bash
#
# NXCC Development Container Setup Script
# This script sets up and starts the NXCC development container on a TDX VM.

set -e

echo '=== Setting up NXCC Development Container ==='

# Configuration
CONTAINER_IMAGE="${NXCC_DEV_IMAGE:-ghcr.io/nxcc-bridge/dev:latest}"
CONTAINER_NAME="nxcc-dev-container"

echo "📋 Container Configuration:"
echo "   Image: $CONTAINER_IMAGE"
echo "   Name: $CONTAINER_NAME"
echo "   TDX Device: /dev/tdx_guest"
echo "   Workspace: /home/ubuntu/nxcc -> /workspace"
echo ""

# Pull the development container image
echo '📦 Pulling NXCC development container...'
echo "    Pulling: $CONTAINER_IMAGE"
if docker pull "$CONTAINER_IMAGE"; then
	echo '✅ Container image pulled successfully'
else
	echo '⚠️  Warning: Could not pull latest container image (using cached version)'
fi
echo ""

# Stop existing container if running
echo '🛑 Cleaning up existing container...'
if docker ps -q -f name="$CONTAINER_NAME" | grep -q .; then
	echo "    Stopping running container: $CONTAINER_NAME"
	docker stop "$CONTAINER_NAME"
else
	echo "    No running container found"
fi

if docker ps -aq -f name="$CONTAINER_NAME" | grep -q .; then
	echo "    Removing existing container: $CONTAINER_NAME"
	docker rm "$CONTAINER_NAME"
else
	echo "    No existing container found"
fi
echo ""

# Start the development container
echo '🚀 Starting development container...'
echo "    Creating container with TDX support..."
CONTAINER_ID=$(docker run -d \
	--name "$CONTAINER_NAME" \
	--privileged \
	--device /dev/tdx_guest:/dev/tdx_guest \
	-v /home/ubuntu/nxcc:/workspace \
	-v /sys/kernel/config:/sys/kernel/config:ro \
	-w /workspace \
	"$CONTAINER_IMAGE" \
	sleep infinity)

echo "    Container ID: ${CONTAINER_ID:0:12}"
echo ""

# Verify container is running
echo '🔍 Verifying container status...'
if docker ps | grep -q "$CONTAINER_NAME"; then
	echo '✅ Development container started successfully'
	echo '📁 Code mounted at: /workspace'
	echo '🔒 TDX device available in container'
	echo '🌐 Container name: '"$CONTAINER_NAME"
	echo ""
	echo "📊 Container Details:"
	docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" | grep -E "(NAMES|$CONTAINER_NAME)"
	echo ""
	echo "🎉 Container setup complete!"
	exit 0
else
	echo '❌ Failed to start development container'
	echo '🔍 Container status:'
	docker ps -a --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}" | grep -E "(NAMES|$CONTAINER_NAME)" || echo 'Container not found'
	echo ""
	echo "🔍 Recent container logs:"
	docker logs "$CONTAINER_NAME" 2>&1 || echo "No logs available"
	exit 1
fi
