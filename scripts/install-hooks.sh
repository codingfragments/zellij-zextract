#!/bin/sh
# Install git hooks for this repo. Run once after cloning.
set -e
HOOKS_DIR="$(git rev-parse --git-dir)/hooks"
cp scripts/pre-push.sh "$HOOKS_DIR/pre-push"
chmod +x "$HOOKS_DIR/pre-push"
echo "Git hooks installed."
