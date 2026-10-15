#!/usr/bin/env bash
# Runs ON ANY PC with the AWS CLI logged in: cancels the persistent spot requests and terminates the
# ReysonGTO instances. Uses .local/ec2.env if this PC launched them; otherwise finds them by the Name tag.
# The final results are already pushed to ReysonGTO-data when status.txt says 完了; the disk is deleted.
# Usage: tools/ec2/terminate.sh [--cleanup] [--yes]
#   --cleanup  also delete the security group and key pair
#   --yes      do not ask for confirmation
# Env (when there is no .local/ec2.env): REGION (ap-northeast-1), NAME (reysongto-m4),
#   SG_NAME (reysongto-ssh), KEY_NAME (reysongto), KEY_PATH (~/.ssh/reysongto.pem)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
CLEANUP=0; YES=0
for arg in "$@"; do
  case "$arg" in
    --cleanup) CLEANUP=1 ;;
    --yes) YES=1 ;;
    *) echo "usage: terminate.sh [--cleanup] [--yes]" >&2; exit 1 ;;
  esac
done

if [ -f "$STATE" ]; then
  # shellcheck disable=SC1090
  source "$STATE"
fi
REGION="${REGION:-ap-northeast-1}"
NAME="${NAME:-reysongto-m4}"
SG_NAME="${SG_NAME:-reysongto-ssh}"
KEY_NAME="${KEY_NAME:-reysongto}"
KEY_PATH="${KEY_PATH:-$HOME/.ssh/reysongto.pem}"
export AWS_REGION="$REGION" AWS_PAGER=""
command -v aws >/dev/null || { echo "install the AWS CLI first (see docs/ec2-runbook.md)" >&2; exit 1; }

# Every live instance with the Name tag (covers a replacement started by a spot request, too).
IDS=$(aws ec2 describe-instances \
  --filters "Name=tag:Name,Values=$NAME" "Name=instance-state-name,Values=pending,running,stopping,stopped" \
  --query 'Reservations[].Instances[].InstanceId' --output text)
if [ -n "${INSTANCE_ID:-}" ] && ! grep -qw "$INSTANCE_ID" <<<"$IDS"; then
  IDS="$IDS $INSTANCE_ID"
fi
IDS=$(echo "$IDS" | xargs)
# Spot requests for this name that are still open/active (so none can start a new instance).
SIRS=$(aws ec2 describe-spot-instance-requests \
  --filters "Name=state,Values=open,active,disabled" \
  --query "SpotInstanceRequests[?InstanceId!=null] | [].[SpotInstanceRequestId,InstanceId]" --output text |
  awk -v ids=" $IDS " 'index(ids, " "$2" ")>0 {print $1}' | xargs)

if [ -z "$IDS" ]; then
  echo "no live instance named $NAME in $REGION"
else
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
fi

if [ "$CLEANUP" = "1" ]; then
  SG_ID="${SG_ID:-$(aws ec2 describe-security-groups --filters "Name=group-name,Values=$SG_NAME" \
    --query 'SecurityGroups[0].GroupId' --output text)}"
  if [ -n "$SG_ID" ] && [ "$SG_ID" != "None" ]; then
    aws ec2 delete-security-group --group-id "$SG_ID" && echo "deleted security group $SG_ID"
  fi
  if aws ec2 describe-key-pairs --key-names "$KEY_NAME" >/dev/null 2>&1; then
    aws ec2 delete-key-pair --key-name "$KEY_NAME" && echo "deleted key pair $KEY_NAME"
  fi
  rm -f "$KEY_PATH"
fi
rm -f "$STATE"
