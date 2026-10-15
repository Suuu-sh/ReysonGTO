#!/usr/bin/env bash
# Runs ON THE MAC: summarizes on the instance and copies the results (not the large solution files) back.
# Usage: tools/ec2/collect.sh <user@host> [key.pem]
set -euo pipefail
HOST="${1:?usage: collect.sh <user@host> [key.pem]}"
KEY="${2:-}"
OUT="${OUT:-data/m4b}"
TRAINING="${TRAINING:-120}"
VALIDATION="${VALIDATION:-40}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)
[ -n "$KEY" ] && OPTS+=(-i "$KEY")
ssh "${OPTS[@]}" "$HOST" "cd ReysonGTO && python3 tools/m4_trial.py --out $OUT --training $TRAINING --validation $VALIDATION --limit $TRAINING \
  --stage2-target 0.5 --summarize-only --report docs/benchmarks/m4-full.md"
mkdir -p "$ROOT/$OUT"
rsync -a -e "ssh ${OPTS[*]}" --include='*/' --include='*.json' --include='*.jsonl' --include='*.log' \
  --exclude='solution.json' --exclude='*' "$HOST:ReysonGTO/$OUT/" "$ROOT/$OUT/"
rsync -a -e "ssh ${OPTS[*]}" "$HOST:ReysonGTO/docs/benchmarks/m4-full.md" "$ROOT/docs/benchmarks/m4-full.md"
echo "collected into $ROOT/$OUT and docs/benchmarks/m4-full.md"
