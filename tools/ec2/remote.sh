#!/usr/bin/env bash
# Runs ON THE MAC: shortcuts for the instance recorded by launch.sh.
# Usage: tools/ec2/remote.sh status | collect | resume | stop | ssh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
[ -f "$STATE" ] || { echo "no recorded instance ($STATE); run tools/ec2/launch.sh" >&2; exit 1; }
# shellcheck disable=SC1090
source "$STATE"
export AWS_REGION="$REGION" AWS_PAGER=""
# The public IP changes when a spot instance is stopped and restarted; look it up each time.
IP=$(aws ec2 describe-instances --instance-ids "$INSTANCE_ID" \
  --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
[ "$IP" != "None" ] || { echo "instance $INSTANCE_ID has no public IP (stopped or interrupted?)" >&2; exit 1; }
SSH=(ssh -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30 -i "$KEY_PATH" "ubuntu@$IP")
case "${1:-status}" in
  status)  "${SSH[@]}" 'bash ReysonGTO/tools/ec2/status.sh' ;;
  collect) "$HERE/collect.sh" "ubuntu@$IP" "$KEY_PATH" ;;
  resume)  "${SSH[@]}" 'cd ReysonGTO && bash tools/ec2/run.sh' ;;
  stop)    "${SSH[@]}" 'pkill -f tools/m4_trial.py; pkill -f "target/release/cli solve"; echo stopped' ;;
  ssh)     "${SSH[@]}" ;;
  *) echo "usage: remote.sh status | collect | resume | stop | ssh" >&2; exit 1 ;;
esac
