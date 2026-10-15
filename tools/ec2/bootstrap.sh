#!/usr/bin/env bash
# Runs ON the EC2 instance (deploy.sh sends it over SSH): installs tools, clones and builds.
# Usage: bash bootstrap.sh [branch]
set -euo pipefail
BRANCH="${1:-main}"

if command -v apt-get >/dev/null; then
  sudo DEBIAN_FRONTEND=noninteractive apt-get update -y
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y build-essential git python3 curl
elif command -v dnf >/dev/null; then
  sudo dnf install -y gcc gcc-c++ make git python3 curl
else
  echo "unsupported OS: install gcc, git, python3, curl manually" >&2
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
echo "bootstrap done: $(git rev-parse --short HEAD) on $(uname -m), $(nproc) vCPU, $(free -g | awk '/Mem:/{print $2}') GB"
