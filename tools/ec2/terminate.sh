#!/usr/bin/env bash
# Runs ON YOUR PC: cancels the persistent spot request and terminates the instance recorded by launch.sh.
# Collect results first (tools/ec2/remote.sh collect): the disk is deleted with the instance.
# Usage: tools/ec2/terminate.sh [--cleanup]   (--cleanup also deletes the security group and key pair)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
[ -f "$STATE" ] || { echo "no recorded instance ($STATE)" >&2; exit 1; }
# shellcheck disable=SC1090
source "$STATE"
export AWS_REGION="$REGION" AWS_PAGER=""

read -r -p "terminate $INSTANCE_ID in $REGION (its disk and any uncollected results are deleted)? [y/N] " answer
[ "$answer" = "y" ] || { echo "cancelled"; exit 1; }

SIR=$(aws ec2 describe-instances --instance-ids "$INSTANCE_ID" \
  --query 'Reservations[0].Instances[0].SpotInstanceRequestId' --output text 2>/dev/null || echo None)
if [ "$SIR" != "None" ] && [ -n "$SIR" ]; then
  # Without this, a persistent spot request would start a replacement instance.
  aws ec2 cancel-spot-instance-requests --spot-instance-request-ids "$SIR" >/dev/null
  echo "cancelled spot request $SIR"
fi
aws ec2 terminate-instances --instance-ids "$INSTANCE_ID" >/dev/null
aws ec2 wait instance-terminated --instance-ids "$INSTANCE_ID"
echo "terminated $INSTANCE_ID"

if [ "${1:-}" = "--cleanup" ]; then
  aws ec2 delete-security-group --group-id "$SG_ID" && echo "deleted security group $SG_ID"
  aws ec2 delete-key-pair --key-name "$KEY_NAME" && echo "deleted key pair $KEY_NAME"
  rm -f "$KEY_PATH"
fi
rm -f "$STATE"
