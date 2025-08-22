#!/bin/sh
# Format or check YAML files using Prettier

set -e

usage() {
	echo "Usage: $0 [check|format]"
	echo "  check   - Check if YAML files are properly formatted (default)"
	echo "  format  - Format YAML files in-place"
	exit 1
}

MODE="${1:-check}"
SCRIPT_DIR="$(dirname "$0")"
PRETTIER_CONFIG="$SCRIPT_DIR/.prettierrc.json"

# Ensure we're in the repository root
cd "$(git rev-parse --show-toplevel)"

if [ "$MODE" != "check" ] && [ "$MODE" != "format" ]; then
	usage
fi

if [ "$MODE" = "check" ]; then
	echo "Checking YAML formatting..."
	prettier --check ".github/workflows/*.yml" --config "$PRETTIER_CONFIG"
else
	echo "Formatting YAML files..."
	prettier --write ".github/workflows/*.yml" --config "$PRETTIER_CONFIG"
fi
