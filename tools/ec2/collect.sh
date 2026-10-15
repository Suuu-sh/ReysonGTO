#!/usr/bin/env bash
# Runs ON YOUR PC: pushes the latest results from the instance to the private data repository now
# (the instance also does this every 15 minutes). Then read them on GitHub or `git pull` the data repo.
# Usage: tools/ec2/collect.sh <user@host> [key.pem]
set -euo pipefail
HOST="${1:?usage: collect.sh <user@host> [key.pem]}"
KEY="${2:-}"
OUT="${OUT:-data/m4b}"
OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)
[ -n "$KEY" ] && OPTS+=(-i "$KEY")
ssh "${OPTS[@]}" "$HOST" "cd ReysonGTO && OUT='$OUT' TRAINING='${TRAINING:-}' VALIDATION='${VALIDATION:-}' bash tools/ec2/sync.sh"
echo "results: https://github.com/${DATA_REPO:-Suuu-sh/ReysonGTO-data}/tree/main/results/$(basename "$OUT")"
