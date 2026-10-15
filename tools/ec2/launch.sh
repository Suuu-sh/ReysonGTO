#!/usr/bin/env bash
# Runs ON YOUR PC (macOS, Linux or Windows WSL) with the AWS CLI logged in: buys one spot (or
# on-demand) instance named pg-yota-poc in the default VPC / subnet / security group. No key pair and
# no SSH: the user data (tools/ec2/user-data.sh) prepares the instance, starts the shards and pushes
# results and status to the private data repository every 15 minutes.
# Usage: DATA_TOKEN=<github token> tools/ec2/launch.sh
# Env: DATA_TOKEN (required; fine-grained token with Contents read/write on DATA_REPO). Optional:
#   REGION (ap-northeast-1)  TYPE (c8g.8xlarge)  MARKET (spot | ondemand)  DISK_GB (100)
#   NAME (pg-yota-poc; the instance Name tag)  VPC_ID (default VPC)  SUBNET_ID (default subnet of the VPC)
#   DATA_REPO (Suuu-sh/ReysonGTO-data)  BRANCH (main)
#   TRAINING (120)  VALIDATION (40)  THREADS (12)  SHARDS (auto)  MEM_PER_SHARD_GB (18)
set -euo pipefail
REGION="${REGION:-ap-northeast-1}"
TYPE="${TYPE:-c8g.8xlarge}"
MARKET="${MARKET:-spot}"
DISK_GB="${DISK_GB:-100}"
NAME="${NAME:-pg-yota-poc}"
DATA_REPO="${DATA_REPO:-Suuu-sh/ReysonGTO-data}"
BRANCH="${BRANCH:-main}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
export AWS_REGION="$REGION" AWS_PAGER=""

# Checked before buying anything so a missing token never leaves a paid instance behind.
: "${DATA_TOKEN:?set DATA_TOKEN (GitHub token with Contents read/write on $DATA_REPO)}"
command -v aws >/dev/null || { echo "install the AWS CLI first (see docs/ec2-runbook.md)" >&2; exit 1; }
aws sts get-caller-identity --query Account --output text >/dev/null ||
  { echo "AWS CLI is not logged in: run aws configure" >&2; exit 1; }
LIVE=$(aws ec2 describe-instances \
  --filters "Name=tag:Name,Values=$NAME" "Name=instance-state-name,Values=pending,running,stopping,stopped" \
  --query 'Reservations[].Instances[].InstanceId' --output text)
[ -z "$LIVE" ] || { echo "instance(s) named $NAME already exist: $LIVE (tools/ec2/terminate.sh first)" >&2; exit 1; }

echo "== Ubuntu 24.04 arm64 image"
AMI=$(aws ssm get-parameter \
  --name /aws/service/canonical/ubuntu/server/24.04/stable/current/arm64/hvm/ebs-gp3/ami-id \
  --query Parameter.Value --output text)
echo "$AMI"

echo "== network (defaults)"
if [ -n "${SUBNET_ID:-}" ]; then
  VPC=$(aws ec2 describe-subnets --subnet-ids "$SUBNET_ID" --query 'Subnets[0].VpcId' --output text)
elif [ -n "${VPC_ID:-}" ]; then
  VPC="$VPC_ID"
else
  VPC=$(aws ec2 describe-vpcs --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
  [ "$VPC" != "None" ] || { echo "no default VPC in $REGION; set VPC_ID / SUBNET_ID" >&2; exit 1; }
fi
if [ -z "${SUBNET_ID:-}" ]; then
  SUBNET_ID=$(aws ec2 describe-subnets --filters "Name=vpc-id,Values=$VPC" "Name=default-for-az,Values=true" \
    --query 'Subnets[0].SubnetId' --output text)
  [ "$SUBNET_ID" != "None" ] ||
    SUBNET_ID=$(aws ec2 describe-subnets --filters "Name=vpc-id,Values=$VPC" --query 'Subnets[0].SubnetId' --output text)
fi
[ "$SUBNET_ID" != "None" ] || { echo "no subnet in $VPC" >&2; exit 1; }
# The VPC's default security group: no inbound needed; its default outbound rule reaches GitHub.
SG=$(aws ec2 describe-security-groups --filters "Name=vpc-id,Values=$VPC" "Name=group-name,Values=default" \
  --query 'SecurityGroups[0].GroupId' --output text)
echo "vpc $VPC, subnet $SUBNET_ID, security group $SG (default)"

echo "== user data"
RUN_ENV="TRAINING=${TRAINING:-} VALIDATION=${VALIDATION:-} THREADS=${THREADS:-} SHARDS=${SHARDS:-} MEM_PER_SHARD_GB=${MEM_PER_SHARD_GB:-}"
mkdir -p "$ROOT/.local"
USER_DATA="$(mktemp "$ROOT/.local/user-data.XXXXXX")"
trap 'rm -f "$USER_DATA"' EXIT
chmod 600 "$USER_DATA"
# Values go through the environment, never into code or command-line arguments.
UD_BRANCH="$BRANCH" UD_DATA_REPO="$DATA_REPO" UD_RUN_ENV="$RUN_ENV" UD_RESULT_NAME="m4b" \
UD_CREDENTIAL_LINE="https://x-access-token:${DATA_TOKEN}@github.com" \
python3 - "$HERE/user-data.sh" "$USER_DATA" <<'PY'
import os, sys
text = open(sys.argv[1]).read()
for key in ("BRANCH", "DATA_REPO", "RUN_ENV", "RESULT_NAME", "CREDENTIAL_LINE"):
    text = text.replace(f"@{key}@", os.environ[f"UD_{key}"])
open(sys.argv[2], "w").write(text)
PY

echo "== $MARKET $TYPE named $NAME"
MARKET_OPTS=()
if [ "$MARKET" = "spot" ]; then
  # Persistent + stop: an interrupted instance keeps its disk and comes back when capacity returns;
  # an @reboot cron entry (bootstrap.sh) resumes the shards and the result sync.
  MARKET_OPTS=(--instance-market-options
    'MarketType=spot,SpotOptions={SpotInstanceType=persistent,InstanceInterruptionBehavior=stop}')
fi
ID=$(aws ec2 run-instances --image-id "$AMI" --instance-type "$TYPE" \
  --network-interfaces "DeviceIndex=0,SubnetId=$SUBNET_ID,Groups=$SG,AssociatePublicIpAddress=true,DeleteOnTermination=true" \
  --block-device-mappings "DeviceName=/dev/sda1,Ebs={VolumeSize=$DISK_GB,VolumeType=gp3,DeleteOnTermination=true}" \
  --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=$NAME}]" \
  --user-data "file://$USER_DATA" \
  "${MARKET_OPTS[@]}" --query 'Instances[0].InstanceId' --output text)
printf 'REGION=%s\nINSTANCE_ID=%s\nNAME=%s\n' "$REGION" "$ID" "$NAME" > "$STATE"
echo "instance $ID (recorded in $STATE)"
aws ec2 wait instance-running --instance-ids "$ID"
echo "running. The instance now installs, builds (about 10 minutes) and starts the comparison by itself."
echo "  progress:   tools/ec2/progress.sh --watch   (status.txt appears about 15-20 minutes after launch)"
echo "  setup log:  tools/ec2/console.sh"
echo "  finish:     tools/ec2/terminate.sh"
echo "This PC can be closed now."
