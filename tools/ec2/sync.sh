#!/usr/bin/env bash
# Runs ON the EC2 instance: summarizes the run and pushes results (not the large solution files)
# to the private data repository. run.sh starts a loop that calls this every 15 minutes.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${OUT:-data/m4b}"
TRAINING="${TRAINING:-}"; TRAINING="${TRAINING:-120}"
VALIDATION="${VALIDATION:-}"; VALIDATION="${VALIDATION:-40}"
DATA="$HOME/ReysonGTO-data"
NAME="$(basename "$OUT")"
DEST="$DATA/results/$NAME"
[ -f "$OUT/manifest.json" ] || { echo "no run in $OUT yet"; exit 0; }
mkdir -p "$DEST/logs"

python3 tools/m4_trial.py --out "$OUT" --training "$TRAINING" --validation "$VALIDATION" --limit "$TRAINING" \
  --stage2-target 0.5 --summarize-only --report "$DEST/report.md" || echo "summary failed (kept going)"
# The first matching rule wins: exclude the multi-GB solution files before including JSON.
rsync -a --prune-empty-dirs --exclude='solution.json' --include='*/' --include='*.json' --include='*.jsonl' \
  --exclude='*' "$OUT/" "$DEST/"
cp "$OUT"-shard-*.log "$DEST/logs/" 2>/dev/null || true
bash tools/ec2/status.sh > "$DEST/status.txt" 2>&1 || true

cd "$DATA"
git pull -q --rebase origin main || true
git add -A "results/$NAME"
if git diff --cached --quiet; then
  echo "no new results"
else
  # A distinct message makes completion visible in the GitHub commit list.
  if head -n 1 "$DEST/status.txt" | grep -q '完了'; then
    MESSAGE="results/$NAME 完了 $(date -u +%Y-%m-%dT%H:%MZ)"
  else
    MESSAGE="results/$NAME $(date -u +%Y-%m-%dT%H:%MZ)"
  fi
  git commit -q -m "$MESSAGE"
  git push -q origin HEAD:main
  echo "pushed results/$NAME"
fi
