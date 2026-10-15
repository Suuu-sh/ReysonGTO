#!/usr/bin/env bash
# Runs ON the EC2 instance: progress of the shards.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${OUT:-data/m4b}"
python3 - "$OUT" <<'PY'
import json, sys, pathlib
out = pathlib.Path(sys.argv[1])
manifest = json.loads((out / 'manifest.json').read_text())
sel = manifest['selection']
total = len(sel['training']) + len(sel.get('validation', []))
done = 0
for path in (out / 'flops').glob('*.json'):
    done += bool(json.loads(path.read_text()).get('complete'))
print(f'boards complete: {done} / {total}')
PY
echo "running shards: $(pgrep -fc "tools/m4_trial.py --out $OUT" || true)"
echo "running solves: $(pgrep -fc "target/release/cli solve" || true)"
for f in "$OUT"-shard-*.log; do
  [ -e "$f" ] && echo "$f: $(tail -n 1 "$f")"
done
uptime
free -g | awk '/Mem:/{print "memory used/total GB: "$3" / "$2}'
