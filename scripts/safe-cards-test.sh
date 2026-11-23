#!/usr/bin/env bash
# usage: scripts/safe-cards-test.sh <Card_Name_Underscores>   (120 s, 6 GB resident cap, one at a time)
cd "$(dirname "$0")/.." || exit 1
export AETHERHUB_ROOT="$PWD"
exec bash scripts/safe-run.sh 120 engine/target/release/cards test "$@"
