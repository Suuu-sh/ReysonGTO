#!/usr/bin/env bash
# Runs ON YOUR PC (macOS, Linux or Windows WSL): buys a spot (or on-demand) instance with the AWS CLI,
# waits for SSH, then runs deploy.sh to install, build and start the size comparison.
# Usage: DATA_TOKEN=<github token> tools/ec2/launch.sh
# Env: DATA_TOKEN (required unless DEPLOY=0; see deploy.sh). Optional:
#   REGION (ap-northeast-1)  TYPE (c8g.8xlarge)  MARKET (spot | ondemand)  DISK_GB (100)
#   NAME (pg-yota-poc)  KEY_NAME (pg-yota-poc)  KEY_PATH (~/.ssh/pg-yota-poc.pem)  SG_NAME (pg-yota-poc)
#   VPC_ID (default VPC)  SUBNET_ID (a subnet of VPC_ID; default: the first default/any subnet of the VPC)
#   DEPLOY (1; set 0 to only create the instance)
# Network: the instance gets a public IP; its security group allows SSH (22) only from this PC's IP
# and all outbound traffic (GitHub, packages).
set -euo pipefail
REGION="${REGION:-ap-northeast-1}"
TYPE="${TYPE:-c8g.8xlarge}"
MARKET="${MARKET:-spot}"
DISK_GB="${DISK_GB:-100}"
NAME="${NAME:-pg-yota-poc}"
KEY_NAME="${KEY_NAME:-pg-yota-poc}"
KEY_PATH="${KEY_PATH:-$HOME/.ssh/pg-yota-poc.pem}"
SG_NAME="${SG_NAME:-pg-yota-poc}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
export AWS_REGION="$REGION" AWS_PAGER=""

if [ "${DEPLOY:-1}" = "1" ] && [ -z "${DATA_TOKEN:-}" ]; then
  # Checked before buying anything so a missing token never leaves a paid instance behind.
  echo "set DATA_TOKEN (GitHub token with read/write on Suuu-sh/ReysonGTO-data) or DEPLOY=0" >&2
  exit 1
fi
command -v aws >/dev/null || { echo "install the AWS CLI first (see docs/ec2-runbook.md)" >&2; exit 1; }
aws sts get-caller-identity --query Account --output text >/dev/null ||
  { echo "AWS CLI is not logged in: run aws configure" >&2; exit 1; }
if [ -f "$STATE" ]; then
  echo "an instance is already recorded in $STATE; terminate it first (tools/ec2/terminate.sh)" >&2
  exit 1
fi

echo "== Ubuntu 24.04 arm64 image"
AMI=$(aws ssm get-parameter \
  --name /aws/service/canonical/ubuntu/server/24.04/stable/current/arm64/hvm/ebs-gp3/ami-id \
  --query Parameter.Value --output text)
echo "$AMI"

echo "== key pair $KEY_NAME"
if [ ! -f "$KEY_PATH" ]; then
  if aws ec2 describe-key-pairs --key-names "$KEY_NAME" >/dev/null 2>&1; then
    echo "key pair $KEY_NAME exists in AWS but $KEY_PATH is missing; set KEY_NAME to a new name" >&2
    exit 1
  fi
  mkdir -p "$(dirname "$KEY_PATH")"
  aws ec2 create-key-pair --key-name "$KEY_NAME" --key-type ed25519 \
    --tag-specifications "ResourceType=key-pair,Tags=[{Key=Name,Value=$NAME}]" \
    --query KeyMaterial --output text > "$KEY_PATH"
  chmod 400 "$KEY_PATH"
fi

echo "== network"
if [ -n "${SUBNET_ID:-}" ]; then
  VPC=$(aws ec2 describe-subnets --subnet-ids "$SUBNET_ID" --query 'Subnets[0].VpcId' --output text)
elif [ -n "${VPC_ID:-}" ]; then
  VPC="$VPC_ID"
else
  VPC=$(aws ec2 describe-vpcs --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
  [ "$VPC" != "None" ] || { echo "no default VPC in $REGION; set VPC_ID and SUBNET_ID" >&2; exit 1; }
fi
if [ -z "${SUBNET_ID:-}" ]; then
  SUBNET_ID=$(aws ec2 describe-subnets --filters "Name=vpc-id,Values=$VPC" "Name=default-for-az,Values=true" \
    --query 'Subnets[0].SubnetId' --output text)
  if [ "$SUBNET_ID" = "None" ]; then
    SUBNET_ID=$(aws ec2 describe-subnets --filters "Name=vpc-id,Values=$VPC" --query 'Subnets[0].SubnetId' --output text)
  fi
fi
[ "$SUBNET_ID" != "None" ] || { echo "no subnet in $VPC" >&2; exit 1; }
echo "vpc $VPC, subnet $SUBNET_ID (needs a route to an internet gateway)"

echo "== security group $SG_NAME (SSH from this PC's IP only)"
SG=$(aws ec2 describe-security-groups --filters "Name=group-name,Values=$SG_NAME" "Name=vpc-id,Values=$VPC" \
  --query 'SecurityGroups[0].GroupId' --output text)
if [ "$SG" = "None" ]; then
  SG=$(aws ec2 create-security-group --group-name "$SG_NAME" --description "$SG_NAME ssh" \
    --vpc-id "$VPC" --tag-specifications "ResourceType=security-group,Tags=[{Key=Name,Value=$NAME}]" \
    --query GroupId --output text)
fi
MYIP=$(curl -fsS https://checkip.amazonaws.com | tr -d '[:space:]')
aws ec2 authorize-security-group-ingress --group-id "$SG" --protocol tcp --port 22 \
  --cidr "$MYIP/32" >/dev/null 2>&1 || true   # already allowed

echo "== $MARKET $TYPE"
MARKET_OPTS=()
if [ "$MARKET" = "spot" ]; then
  # Persistent + stop: an interrupted instance keeps its disk and comes back when capacity returns;
  # an @reboot cron entry (bootstrap.sh) resumes the shards.
  MARKET_OPTS=(--instance-market-options
    'MarketType=spot,SpotOptions={SpotInstanceType=persistent,InstanceInterruptionBehavior=stop}')
fi
ID=$(aws ec2 run-instances --image-id "$AMI" --instance-type "$TYPE" --key-name "$KEY_NAME" \
  --network-interfaces "DeviceIndex=0,SubnetId=$SUBNET_ID,Groups=$SG,AssociatePublicIpAddress=true,DeleteOnTermination=true" \
  --block-device-mappings "DeviceName=/dev/sda1,Ebs={VolumeSize=$DISK_GB,VolumeType=gp3,DeleteOnTermination=true}" \
  --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=$NAME}]" "ResourceType=volume,Tags=[{Key=Name,Value=$NAME}]" \
  "${MARKET_OPTS[@]}" --query 'Instances[0].InstanceId' --output text)
mkdir -p "$ROOT/.local"
printf 'REGION=%s\nINSTANCE_ID=%s\nKEY_NAME=%s\nKEY_PATH=%s\nSG_ID=%s\nSG_NAME=%s\n' \
  "$REGION" "$ID" "$KEY_NAME" "$KEY_PATH" "$SG" "$SG_NAME" > "$STATE"
echo "instance $ID (recorded in $STATE)"

aws ec2 wait instance-running --instance-ids "$ID"
IP=$(aws ec2 describe-instances --instance-ids "$ID" \
  --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
echo "HOST=ubuntu@$IP" >> "$STATE"
echo "running at $IP; waiting for SSH"
for _ in $(seq 1 60); do
  ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=accept-new -i "$KEY_PATH" "ubuntu@$IP" true 2>/dev/null && break
  sleep 5
done

if [ "${DEPLOY:-1}" = "1" ]; then
  "$HERE/deploy.sh" "ubuntu@$IP" "$KEY_PATH"
fi
echo "progress: tools/ec2/remote.sh status   |   collect: tools/ec2/remote.sh collect   |   end: tools/ec2/terminate.sh"
