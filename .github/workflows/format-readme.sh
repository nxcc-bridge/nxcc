#!/bin/sh
# Format or check README.md files using Prettier

set -e

usage() {
	echo "Usage: $0 [check|format]"
	echo "  check   - Check if README files are properly formatted (default)"
	echo "  format  - Format README files in-place"
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

# Find all README.md files
README_FILES=$(git ls-files '**/README.md')

if [ -z "$README_FILES" ]; then
	echo "No README.md files found"
	exit 0
fi

if [ "$MODE" = "check" ]; then
	echo "Checking README.md formatting..."
	echo "$README_FILES" | xargs prettier --check --config "$PRETTIER_CONFIG"
else
	echo "Formatting README.md files..."
	echo "$README_FILES" | xargs prettier --write --config "$PRETTIER_CONFIG"
fi
