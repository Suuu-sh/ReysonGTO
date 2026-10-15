#!/usr/bin/env bash
# Runs ON the EC2 instance: progress of the shards. The first line is the overall state:
#   状態：計算中 / 状態：完了 / 状態：停止（未完了あり。tools/ec2/remote.sh resume で再開）
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${OUT:-data/m4b}"
read -r DONE TOTAL < <(python3 - "$OUT" <<'PY'
import json, sys, pathlib
out = pathlib.Path(sys.argv[1])
manifest = json.loads((out / 'manifest.json').read_text())
sel = manifest['selection']
total = len(sel['training']) + len(sel.get('validation', []))
done = sum(bool(json.loads(p.read_text()).get('complete')) for p in (out / 'flops').glob('*.json'))
print(done, total)
PY
)
SHARDS=$(pgrep -fc "tools/m4_trial.py --out $OUT" || true)
if [ "${SHARDS:-0}" -gt 0 ]; then
  echo "状態：計算中"
elif [ "$DONE" -ge "$TOTAL" ]; then
  echo "状態：完了"
else
  echo "状態：停止（未完了あり。tools/ec2/remote.sh resume で再開）"
fi
echo "boards complete: $DONE / $TOTAL"
echo "running shards: ${SHARDS:-0}"
echo "running solves: $(pgrep -fc "target/release/cli solve" || true)"
echo "updated: $(date -u +%Y-%m-%dT%H:%MZ)"
for f in "$OUT"-shard-*.log; do
  [ -e "$f" ] && echo "$f: $(tail -n 1 "$f")"
done
uptime
free -g | awk '/Mem:/{print "memory used/total GB: "$3" / "$2}'
