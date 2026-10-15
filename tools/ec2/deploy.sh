#!/usr/bin/env bash
# Runs ON THE MAC: sets up a fresh EC2 instance and starts the size comparison.
# Usage: tools/ec2/deploy.sh <user@host> [path/to/key.pem]
#   e.g. tools/ec2/deploy.sh ubuntu@203.0.113.10 ~/.ssh/reysongto.pem
# Env: BRANCH (default main), THREADS, SHARDS, MEM_PER_SHARD_GB (passed to run.sh).
set -euo pipefail
HOST="${1:?usage: deploy.sh <user@host> [key.pem]}"
KEY="${2:-}"
BRANCH="${BRANCH:-main}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)
[ -n "$KEY" ] && OPTS+=(-i "$KEY")

[ -f "$ROOT/data/ranges/btn_open.json" ] && [ -f "$ROOT/data/ranges/bb_call.json" ] ||
  { echo "missing data/ranges on the Mac" >&2; exit 1; }

echo "== 1/3 install and build on $HOST (about 5-10 minutes)"
ssh "${OPTS[@]}" "$HOST" "bash -s -- '$BRANCH'" < "$HERE/bootstrap.sh"

echo "== 2/3 send the ranges (product data, not in Git)"
ssh "${OPTS[@]}" "$HOST" "mkdir -p ReysonGTO/data"
scp "${OPTS[@]}" -r "$ROOT/data/ranges" "$HOST:ReysonGTO/data/"

echo "== 3/3 start the shards"
ssh "${OPTS[@]}" "$HOST" "cd ReysonGTO && THREADS='${THREADS:-}' TRAINING='${TRAINING:-}' VALIDATION='${VALIDATION:-}' SHARDS='${SHARDS:-}' MEM_PER_SHARD_GB='${MEM_PER_SHARD_GB:-}' bash tools/ec2/run.sh"
echo "done. progress: ssh ${OPTS[*]} $HOST 'bash ReysonGTO/tools/ec2/status.sh'"
