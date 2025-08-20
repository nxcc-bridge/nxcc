#!/bin/bash
#
# NXCC Repository Setup Script
# Clones and sets up the NXCC repository if it doesn't exist

set -e

REPO_URL="https://github.com/nxcc-bridge/nxcc.git"
REPO_DIR="/home/ubuntu/nxcc"

echo "=== NXCC Repository Setup ==="

if [[ -d "$REPO_DIR/.git" ]]; then
	echo "✅ NXCC repository already exists at $REPO_DIR"
	cd "$REPO_DIR"
	echo "Current branch: $(git branch --show-current)"
	echo "Last commit: $(git log -1 --oneline)"
else
	echo "📦 Cloning NXCC repository..."
	git clone "$REPO_URL" "$REPO_DIR"
	echo "✅ Repository cloned to $REPO_DIR"
fi

echo ""
echo "Repository ready! Next steps:"
echo "  cd nxcc"
echo "  cargo build      # Development mode"
echo "  cargo test       # Run tests"
echo ""
echo "For TDX-specific development:"
echo "  cargo build --features tdx-hardware-required  # Production mode"
echo "  cargo test --features tdx-hardware-required   # TDX hardware tests"
