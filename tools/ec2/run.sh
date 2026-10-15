#!/usr/bin/env bash
# Runs ON the EC2 instance: selects the boards once, then starts parallel shards that survive SSH logout.
# Re-running resumes (finished solves are skipped). Override with env: OUT, THREADS, SHARDS, MEM_PER_SHARD_GB.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${OUT:-data/m4b}"
# Empty values (from deploy.sh) fall back to the defaults.
THREADS="${THREADS:-12}"
MEM_PER_SHARD_GB="${MEM_PER_SHARD_GB:-18}"

for f in data/ranges/btn_open.json data/ranges/bb_call.json target/release/cli target/release/sizing; do
  [ -e "$f" ] || { echo "missing $f (run deploy.sh from the Mac)" >&2; exit 1; }
done
if pgrep -f "tools/m4_trial.py --out $OUT" >/dev/null; then
  echo "shards for $OUT are already running; see tools/ec2/status.sh" >&2
  exit 1
fi

CORES=$(nproc)
MEM_GB=$(free -g | awk '/Mem:/{print $2}')
BY_CPU=$(( CORES / THREADS ))
BY_MEM=$(( MEM_GB / MEM_PER_SHARD_GB ))
AUTO=$(( BY_CPU < BY_MEM ? BY_CPU : BY_MEM ))
SHARDS="${SHARDS:-}"
SHARDS="${SHARDS:-$AUTO}"
[ "$SHARDS" -ge 1 ] || { echo "not enough CPU/memory for one shard" >&2; exit 1; }

COMMON=(--out "$OUT" --training 200 --validation 80 --limit 200 --stage2-target 0.5 --max-hours 48)
python3 tools/m4_trial.py "${COMMON[@]}" --prepare-only

echo "starting $SHARDS shards x $THREADS threads (${CORES} vCPU, ${MEM_GB} GB)"
for i in $(seq 0 $(( SHARDS - 1 ))); do
  nohup setsid python3 tools/m4_trial.py "${COMMON[@]}" --threads "$THREADS" \
    --memory-mb $(( MEM_PER_SHARD_GB * 1000 )) --shard "$i/$SHARDS" \
    > "$OUT-shard-$i.log" 2>&1 < /dev/null &
done
sleep 2
echo "started; logs: $OUT-shard-*.log"
