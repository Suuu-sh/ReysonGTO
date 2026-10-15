#!/usr/bin/env bash
# Runs ON the EC2 instance (deploy.sh sends it over SSH): installs tools, clones and builds.
# Usage: bash bootstrap.sh [branch] [data-repo]
# The GitHub credentials for the private data repository are set up by deploy.sh beforehand.
set -euo pipefail
BRANCH="${1:-main}"
DATA_REPO="${2:-Suuu-sh/ReysonGTO-data}"

if command -v apt-get >/dev/null; then
  sudo DEBIAN_FRONTEND=noninteractive apt-get update -y
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential git python3 curl rsync
elif command -v dnf >/dev/null; then
  sudo dnf install -y gcc gcc-c++ make git python3 curl rsync
else
  echo "unsupported OS: install gcc, git, python3, curl, rsync manually" >&2
  exit 1
fi

if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

if [ ! -d "$HOME/ReysonGTO/.git" ]; then
  git clone https://github.com/Suuu-sh/ReysonGTO.git "$HOME/ReysonGTO"
fi
cd "$HOME/ReysonGTO"
git fetch origin
git checkout "$BRANCH"
git pull --ff-only origin "$BRANCH"
cargo build --release -p cli -p sizing
mkdir -p data

# Ranges (product data) come from the private data repository; results are pushed back there.
if [ ! -d "$HOME/ReysonGTO-data/.git" ]; then
  git clone "https://github.com/$DATA_REPO.git" "$HOME/ReysonGTO-data"
fi
git -C "$HOME/ReysonGTO-data" pull -q --ff-only origin main
ln -sfn "$HOME/ReysonGTO-data/ranges" data/ranges
# Resume the shards automatically after a spot interruption brings the instance back.
( crontab -l 2>/dev/null | grep -v 'tools/ec2/run.sh' ; \
  echo "@reboot sleep 60 && cd $HOME/ReysonGTO && bash tools/ec2/run.sh >> $HOME/ReysonGTO/data/reboot.log 2>&1" ) | crontab -
echo "bootstrap done: $(git rev-parse --short HEAD) on $(uname -m), $(nproc) vCPU, $(free -g | awk '/Mem:/{print $2}') GB"
