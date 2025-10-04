#!/bin/bash
# Script to get peer ID from NXCC bootstrap worker HTTP API
# Used by Terraform external data source

set -euo pipefail

# Get worker IP from command line argument
if [[ $# -lt 1 ]]; then
	echo "Usage: $0 <worker_ip>" >&2
	exit 1
fi

WORKER_IP="$1"
HTTP_URL="http://$WORKER_IP:6922"

# Maximum wait time in seconds (5 minutes)
MAX_WAIT=300
WAIT_INTERVAL=10

# Function to check if HTTP API is ready and get peer ID
get_peer_id() {
	local url="$HTTP_URL/api/status"

	# Try to get the status with timeout
	if response=$(curl -sf --max-time 10 "$url" 2>/dev/null); then
		# Parse peer_id from JSON response using basic tools
		# Look for "peer_id":"..." pattern and extract the value
		peer_id=$(echo "$response" | grep -o '"peer_id":"[^"]*"' | cut -d'"' -f4 | head -1)

		if [[ -n "$peer_id" && "$peer_id" != "null" ]]; then
			echo "$peer_id"
			return 0
		fi
	fi
	return 1
}

# Wait for bootstrap worker to be ready and get peer ID
echo "Waiting for bootstrap worker HTTP API at $HTTP_URL..." >&2

elapsed=0
while [[ $elapsed -lt $MAX_WAIT ]]; do
	if peer_id=$(get_peer_id); then
		echo "✓ Bootstrap worker ready, peer ID: $peer_id" >&2

		# Output JSON for Terraform external data source
		echo "{\"peer_id\":\"$peer_id\"}"
		exit 0
	fi

	echo "  Still waiting... ($elapsed/${MAX_WAIT}s)" >&2
	sleep $WAIT_INTERVAL
	elapsed=$((elapsed + WAIT_INTERVAL))
done

echo "✗ Timeout waiting for bootstrap worker HTTP API" >&2
echo "  Tried: $HTTP_URL/api/status" >&2
echo "  Duration: ${MAX_WAIT}s" >&2

# Return empty peer_id on timeout - deployment can continue without full multiaddr
echo "{\"peer_id\":\"\"}"
exit 1
