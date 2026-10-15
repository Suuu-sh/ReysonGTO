#!/usr/bin/env bash
# Runs ON YOUR PC: sets up a fresh EC2 instance and starts the size comparison.
# Ranges come from, and results go to, the private data repository on GitHub.
# Usage: DATA_TOKEN=<github token> tools/ec2/deploy.sh <user@host> [path/to/key.pem]
# Env: DATA_TOKEN (required; fine-grained token with Contents read/write on DATA_REPO),
#      DATA_REPO (default Suuu-sh/ReysonGTO-data), BRANCH (default main),
#      TRAINING, VALIDATION, THREADS, SHARDS, MEM_PER_SHARD_GB (passed to run.sh).
set -euo pipefail
HOST="${1:?usage: DATA_TOKEN=... deploy.sh <user@host> [key.pem]}"
KEY="${2:-}"
BRANCH="${BRANCH:-main}"
DATA_REPO="${DATA_REPO:-Suuu-sh/ReysonGTO-data}"
: "${DATA_TOKEN:?set DATA_TOKEN to a GitHub token that can read and write $DATA_REPO}"
HERE="$(cd "$(dirname "$0")" && pwd)"
OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)
[ -n "$KEY" ] && OPTS+=(-i "$KEY")

echo "== 1/3 GitHub credentials for $DATA_REPO (sent over stdin, stored with mode 600)"
printf 'https://x-access-token:%s@github.com\n' "$DATA_TOKEN" |
  ssh "${OPTS[@]}" "$HOST" 'umask 077 && cat > ~/.git-credentials &&
    git config --global credential.helper store &&
    git config --global user.name "reysongto-ec2" &&
    git config --global user.email "reysongto-ec2@users.noreply.github.com"'

echo "== 2/3 install, build and fetch the ranges on $HOST (about 5-10 minutes)"
ssh "${OPTS[@]}" "$HOST" "bash -s -- '$BRANCH' '$DATA_REPO'" < "$HERE/bootstrap.sh"

echo "== 3/3 start the shards and the result sync"
ssh "${OPTS[@]}" "$HOST" "cd ReysonGTO && THREADS='${THREADS:-}' TRAINING='${TRAINING:-}' VALIDATION='${VALIDATION:-}' SHARDS='${SHARDS:-}' MEM_PER_SHARD_GB='${MEM_PER_SHARD_GB:-}' bash tools/ec2/run.sh"
echo "done. results appear in https://github.com/$DATA_REPO/tree/main/results every 15 minutes"
