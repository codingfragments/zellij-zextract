#!/bin/sh
if [ "${SKIP_CHECKS}" = "1" ]; then
    echo "pre-push: checks skipped (SKIP_CHECKS=1)"
    exit 0
fi

echo "pre-push: running fmt + clippy + test + wasm build..."
just check
STATUS=$?

if [ $STATUS -ne 0 ]; then
    echo ""
    echo "pre-push: checks failed. Fix the errors above, then push again."
    echo "To skip (e.g. for WIP branches): SKIP_CHECKS=1 git push"
    exit 1
fi

echo "pre-push: all checks passed."
