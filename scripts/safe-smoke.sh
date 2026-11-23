#!/usr/bin/env bash
# usage: scripts/safe-smoke.sh "<exact card name>"   (90 s, 6 GB resident cap, one at a time)
cd "$(dirname "$0")/.." || exit 1
export AETHERHUB_ROOT="$PWD"
exec bash scripts/safe-run.sh 90 engine/target/release/smoke "$@"
