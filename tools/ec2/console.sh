#!/usr/bin/env bash
# Runs ON ANY PC with the AWS CLI logged in: shows the instance's serial console output, where the
# user data prints its setup progress ([setup] lines). Useful before status.txt appears (no SSH needed).
# Usage: tools/ec2/console.sh [lines]   Env: REGION (ap-northeast-1), NAME (pg-yota-poc)
set -euo pipefail
REGION="${REGION:-ap-northeast-1}"
NAME="${NAME:-pg-yota-poc}"
LINES="${1:-60}"
export AWS_REGION="$REGION" AWS_PAGER=""
ID=$(aws ec2 describe-instances \
  --filters "Name=tag:Name,Values=$NAME" "Name=instance-state-name,Values=pending,running,stopping,stopped" \
  --query 'Reservations[0].Instances[0].InstanceId' --output text)
[ "$ID" != "None" ] || { echo "no live instance named $NAME in $REGION" >&2; exit 1; }
STATE=$(aws ec2 describe-instances --instance-ids "$ID" --query 'Reservations[0].Instances[0].State.Name' --output text)
echo "instance $ID ($STATE)"
OUTPUT=$(aws ec2 get-console-output --instance-id "$ID" --latest --query Output --output text 2>/dev/null || true)
if [ -z "$OUTPUT" ] || [ "$OUTPUT" = "None" ]; then
  echo "no console output yet (it appears a few minutes after boot)"
  exit 0
fi
echo "--- setup steps"
grep -E '\[setup\]' <<<"$OUTPUT" || echo "(no [setup] lines yet)"
echo "--- last $LINES lines"
tail -n "$LINES" <<<"$OUTPUT"
