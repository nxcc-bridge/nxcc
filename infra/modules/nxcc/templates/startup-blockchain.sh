#!/bin/bash
#
# Startup script for NXCC Blockchain node (Anvil)
# This script sets up an Anvil blockchain node for E2E testing
#

set -e

# Configuration
# shellcheck disable=SC2154
export ENVIRONMENT="${environment}"
# shellcheck disable=SC2154
export NAMESPACE="${namespace}"
DOCKER_IMAGE="ghcr.io/foundry-rs/foundry:latest"

# Logging setup
exec > >(tee -a /var/log/nxcc-blockchain.log)
exec 2>&1
echo "$(date): Starting NXCC Blockchain node setup..."

# Update system
apt-get update
apt-get install -y docker.io jq curl

# Enable and start Docker
systemctl enable docker
systemctl start docker

# Wait for Docker to be ready
sleep 5

# Pull Foundry image (contains anvil)
echo "$(date): Pulling Foundry Docker image..."
docker pull "$DOCKER_IMAGE"

# Create anvil data directory
mkdir -p /app/anvil-data

# Start Anvil blockchain node
echo "$(date): Starting Anvil blockchain node..."
docker run -d \
	--name anvil-blockchain \
	--restart unless-stopped \
	-p 8545:8545 \
	-v /app/anvil-data:/anvil-data \
	-e ANVIL_IP_ADDR=0.0.0.0 \
	"$DOCKER_IMAGE" \
	anvil \
	--chain-id 31337 \
	--accounts 10 \
	--balance 1000000 \
	--gas-limit 30000000

# Wait for Anvil to be ready
echo "$(date): Waiting for Anvil to be ready..."
for i in $(seq 1 30); do
	if curl -s -X POST \
		-H "Content-Type: application/json" \
		-d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
		http://localhost:8545 | grep -q "0x7a69"; then
		echo "$(date): Anvil is ready and responding"
		break
	fi
	echo "$(date): Waiting for Anvil... (attempt $i/30)"
	sleep 2
done

# Verify Anvil is running locally
if ! curl -s -X POST \
	-H "Content-Type: application/json" \
	-d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
	http://localhost:8545 | grep -q "0x7a69"; then
	echo "$(date): ERROR: Anvil failed to start properly"
	exit 1
fi

# Test external connectivity
EXTERNAL_IP=$(curl -s ifconfig.me)
echo "$(date): Testing external connectivity to Anvil..."
if ! curl -s -X POST \
	-H "Content-Type: application/json" \
	-d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
	"http://$EXTERNAL_IP:8545" | grep -q "0x7a69"; then
	echo "$(date): WARNING: Anvil not accessible externally - check firewall rules"
	echo "$(date): Anvil is running locally but may not be accessible from other nodes"
fi

echo "$(date): NXCC Blockchain node (Anvil) startup completed successfully"
echo "$(date): Blockchain RPC available at: http://$EXTERNAL_IP:8545"
echo "$(date): Chain ID: 31337"
echo "$(date): Default accounts available with 1M ETH each"
