#!/usr/bin/env bash
# Runs ON ANY PC with the GitHub CLI logged in (no AWS needed): prints status.txt from the private
# ReysonGTO-data repository, which the instance updates every 15 minutes.
# Usage: tools/ec2/progress.sh [--watch]   (--watch refreshes every 5 minutes until 完了)
# Env: DATA_REPO (Suuu-sh/ReysonGTO-data), NAME (m4b)
set -euo pipefail
DATA_REPO="${DATA_REPO:-Suuu-sh/ReysonGTO-data}"
NAME="${NAME:-m4b}"
command -v gh >/dev/null || { echo "install the GitHub CLI (gh) and run gh auth login" >&2; exit 1; }

show() {
  local out
  # gh prints the error body on stdout when it fails, so only show the output on success.
  if out=$(gh api "repos/$DATA_REPO/contents/results/$NAME/status.txt" -H "Accept: application/vnd.github.raw" 2>/dev/null); then
    printf '%s\n' "$out"
  else
    echo "no status yet (the first push comes about 15 minutes after the shards start)"
  fi
}

if [ "${1:-}" = "--watch" ]; then
  while true; do
    clear || true
    OUTPUT="$(show)"
    echo "$OUTPUT"
    head -n 1 <<<"$OUTPUT" | grep -q '完了' && { echo; echo "完了しました。tools/ec2/terminate.sh で終了してください。"; break; }
    sleep 300
  done
else
  show
fi
