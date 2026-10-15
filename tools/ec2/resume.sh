#!/usr/bin/env bash
# Runs ON ANY PC with the AWS CLI logged in: restarts a stopped comparison without SSH by rebooting
# the instance; the @reboot cron entry (bootstrap.sh) restarts the shards (finished solves are skipped)
# and the result sync. Also starts the instance if it is stopped.
# Usage: tools/ec2/resume.sh   Env: AWS_PROFILE, REGION (ap-northeast-1), NAME (pg-yota-poc)
set -euo pipefail
REGION="${REGION:-ap-northeast-1}"
NAME="${NAME:-pg-yota-poc}"
export AWS_REGION="$REGION" AWS_PAGER=""
ID=$(aws ec2 describe-instances \
  --filters "Name=tag:Name,Values=$NAME" "Name=instance-state-name,Values=pending,running,stopping,stopped" \
  --query 'Reservations[0].Instances[0].InstanceId' --output text)
[ "$ID" != "None" ] || { echo "no live instance named $NAME in $REGION" >&2; exit 1; }
STATE=$(aws ec2 describe-instances --instance-ids "$ID" --query 'Reservations[0].Instances[0].State.Name' --output text)
case "$STATE" in
  running) aws ec2 reboot-instances --instance-ids "$ID"; echo "rebooting $ID" ;;
  stopped) aws ec2 start-instances --instance-ids "$ID" >/dev/null; echo "starting $ID" ;;
  *) echo "instance $ID is $STATE; try again in a minute" >&2; exit 1 ;;
esac
echo "the shards and the result sync restart about a minute after boot; check tools/ec2/progress.sh"
