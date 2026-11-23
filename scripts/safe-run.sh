#!/usr/bin/env bash
# Run a command under an RSS and wall-clock watchdog so a runaway card cannot hog the host.
# usage: scripts/safe-run.sh <seconds> <command...>   (kills it past 6 GB resident or the time limit;
# runs one at a time via a lock. The whole card DB is ~0.7 GB resident, so 6 GB leaves room
# for a cargo build next to it)
# (A virtual-memory ulimit does not work here: the engine binaries reserve large thread stacks.)
t="$1"; shift
mkdir -p /tmp/aetherhub-rounds
exec 9>/tmp/aetherhub-rounds/safe-run.lock
flock -w 600 9 || { echo "safe-run: lock wait timed out" >&2; exit 125; }
"$@" &
pid=$!
limit_kb=${SAFE_RUN_LIMIT_KB:-6000000}
start=$SECONDS
while kill -0 "$pid" 2>/dev/null; do
  rss=$(awk '/VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
  if [ "${rss:-0}" -gt "$limit_kb" ]; then echo "safe-run: killed $pid, resident ${rss} kB over the limit" >&2; kill -9 "$pid"; wait "$pid" 2>/dev/null; exit 137; fi
  if [ $((SECONDS - start)) -ge "$t" ]; then echo "safe-run: killed $pid, over ${t}s" >&2; kill -9 "$pid"; wait "$pid" 2>/dev/null; exit 124; fi
  sleep 1
done
wait "$pid"
