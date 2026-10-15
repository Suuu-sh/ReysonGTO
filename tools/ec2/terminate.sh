#!/usr/bin/env bash
# Runs ON ANY PC with the AWS CLI logged in: cancels the persistent spot requests and terminates the
# instances named pg-yota-poc. Results are pushed to ReysonGTO-data every 15 minutes and when the run
# completes (status.txt says 完了); anything newer than the last push is lost with the disk.
# Usage: [AWS_PROFILE=<profile>] tools/ec2/terminate.sh [--yes]   Env: REGION (ap-northeast-1), NAME (pg-yota-poc)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
YES=0
for arg in "$@"; do
  case "$arg" in
    --yes) YES=1 ;;
    *) echo "usage: terminate.sh [--yes]" >&2; exit 1 ;;
  esac
done
if [ -f "$STATE" ]; then
  # shellcheck disable=SC1090
  source "$STATE"
  # Use the profile recorded at launch unless one is given now.
  [ -n "${AWS_PROFILE:-}" ] && export AWS_PROFILE
fi
REGION="${REGION:-ap-northeast-1}"
NAME="${NAME:-pg-yota-poc}"
export AWS_REGION="$REGION" AWS_PAGER=""
command -v aws >/dev/null || { echo "install the AWS CLI first (see docs/ec2-runbook.md)" >&2; exit 1; }
echo "AWS account $(aws sts get-caller-identity --query Account --output text), profile ${AWS_PROFILE:-default}, region $REGION"

# Every live instance with the Name tag (covers a replacement started by a spot request, too).
IDS=$(aws ec2 describe-instances \
  --filters "Name=tag:Name,Values=$NAME" "Name=instance-state-name,Values=pending,running,stopping,stopped" \
  --query 'Reservations[].Instances[].InstanceId' --output text | xargs)
# Spot requests still able to start an instance for these.
SIRS=$(aws ec2 describe-spot-instance-requests --filters "Name=state,Values=open,active,disabled" \
  --query "SpotInstanceRequests[?InstanceId!=null] | [].[SpotInstanceRequestId,InstanceId]" --output text |
  awk -v ids=" $IDS " 'index(ids, " "$2" ")>0 {print $1}' | xargs)

if [ -z "$IDS" ]; then
  echo "no live instance named $NAME in $REGION"
  rm -f "$STATE"
  exit 0
fi
echo "instances: $IDS"
echo "spot requests: ${SIRS:-none}"
if [ "$YES" != "1" ]; then
  read -r -p "terminate them in $REGION (their disks are deleted)? [y/N] " answer
  [ "$answer" = "y" ] || { echo "cancelled"; exit 1; }
fi
if [ -n "$SIRS" ]; then
  # Without this, a persistent spot request would start a replacement instance.
  # shellcheck disable=SC2086
  aws ec2 cancel-spot-instance-requests --spot-instance-request-ids $SIRS >/dev/null
  echo "cancelled spot requests: $SIRS"
fi
# shellcheck disable=SC2086
aws ec2 terminate-instances --instance-ids $IDS >/dev/null
# shellcheck disable=SC2086
aws ec2 wait instance-terminated --instance-ids $IDS
echo "terminated: $IDS"
rm -f "$STATE"
