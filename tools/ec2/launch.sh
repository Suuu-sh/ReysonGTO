#!/usr/bin/env bash
# Runs ON THE MAC: buys a spot (or on-demand) instance with the AWS CLI, waits for SSH,
# then runs deploy.sh to install, build and start the size comparison.
# Usage: tools/ec2/launch.sh
# Env (all optional):
#   REGION (ap-northeast-1)  TYPE (c8g.8xlarge)  MARKET (spot | ondemand)  DISK_GB (100)
#   NAME (reysongto-m4)  KEY_NAME (reysongto)  KEY_PATH (~/.ssh/reysongto.pem)  SG_NAME (reysongto-ssh)
#   DEPLOY (1; set 0 to only create the instance)
set -euo pipefail
REGION="${REGION:-ap-northeast-1}"
TYPE="${TYPE:-c8g.8xlarge}"
MARKET="${MARKET:-spot}"
DISK_GB="${DISK_GB:-100}"
NAME="${NAME:-reysongto-m4}"
KEY_NAME="${KEY_NAME:-reysongto}"
KEY_PATH="${KEY_PATH:-$HOME/.ssh/reysongto.pem}"
SG_NAME="${SG_NAME:-reysongto-ssh}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
STATE="$ROOT/.local/ec2.env"
export AWS_REGION="$REGION" AWS_PAGER=""

command -v aws >/dev/null || { echo "install the AWS CLI first: brew install awscli" >&2; exit 1; }
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
    --query KeyMaterial --output text > "$KEY_PATH"
  chmod 400 "$KEY_PATH"
fi

echo "== security group $SG_NAME (SSH from this Mac's IP only)"
VPC=$(aws ec2 describe-vpcs --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
[ "$VPC" != "None" ] || { echo "no default VPC in $REGION" >&2; exit 1; }
SG=$(aws ec2 describe-security-groups --filters "Name=group-name,Values=$SG_NAME" "Name=vpc-id,Values=$VPC" \
  --query 'SecurityGroups[0].GroupId' --output text)
if [ "$SG" = "None" ]; then
  SG=$(aws ec2 create-security-group --group-name "$SG_NAME" --description "ReysonGTO SSH" \
    --vpc-id "$VPC" --query GroupId --output text)
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
  --security-group-ids "$SG" \
  --block-device-mappings "DeviceName=/dev/sda1,Ebs={VolumeSize=$DISK_GB,VolumeType=gp3,DeleteOnTermination=true}" \
  --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=$NAME}]" \
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
