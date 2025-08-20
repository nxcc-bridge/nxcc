#!/bin/bash
set -e

echo "=== Cloning NXCC Repository ==="
if [ ! -d "/home/ubuntu/nxcc/.git" ]; then
	cd /home/ubuntu
	git clone https://github.com/nxcc-bridge/nxcc.git nxcc
	cd nxcc
else
	echo "Repository already exists, pulling latest changes..."
	cd /home/ubuntu/nxcc
	git pull origin main || echo "Could not pull, working with existing code"
fi

echo "=== NXCC Development Environment Ready ==="
echo "Quick Start Commands:"
echo "  cd /home/ubuntu/nxcc"
echo "  ./dev-container.sh"
