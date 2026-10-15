#!/bin/bash
# EC2 user data (runs once as root on first boot). launch.sh fills in the @...@ placeholders.
# No SSH is needed: the instance prepares itself, starts the shards and pushes results and
# status to the private data repository. A failed setup pushes its log there as setup-failed.log.
set -euo pipefail
LOG=/var/log/pg-yota-poc-setup.log
# Also copy to the serial console so `aws ec2 get-console-output` (tools/ec2/console.sh) shows progress.
exec > >(tee -a "$LOG" > /dev/console) 2>&1

BRANCH='@BRANCH@'
DATA_REPO='@DATA_REPO@'
RUN_ENV='@RUN_ENV@'
RESULT_NAME='@RESULT_NAME@'
U=ubuntu
H=/home/$U

echo "[setup] $(date -u +%FT%TZ) start"
apt-get update -y
DEBIAN_FRONTEND=noninteractive apt-get install -y git

# GitHub credentials for the data repository (token never echoed; file mode 600).
install -o "$U" -g "$U" -m 600 /dev/null "$H/.git-credentials"
cat > "$H/.git-credentials" <<'CRED'
@CREDENTIAL_LINE@
CRED
sudo -u "$U" -H git config --global credential.helper store
sudo -u "$U" -H git config --global user.name pg-yota-poc
sudo -u "$U" -H git config --global user.email pg-yota-poc@users.noreply.github.com

report_failure() {
  echo "[setup] FAILED at $(date -u +%FT%TZ)"
  sudo -u "$U" -H bash -c "
    [ -d $H/ReysonGTO-data/.git ] || git clone -q https://github.com/$DATA_REPO.git $H/ReysonGTO-data
    mkdir -p $H/ReysonGTO-data/results/$RESULT_NAME
    tail -n 200 $LOG | grep -v 'x-access-token' > $H/ReysonGTO-data/results/$RESULT_NAME/setup-failed.log
    cd $H/ReysonGTO-data && git add -A results/$RESULT_NAME &&
      git commit -q -m 'results/$RESULT_NAME setup failed' && git push -q origin HEAD:main" || true
}
trap report_failure ERR

sudo -u "$U" -H bash -c "
  set -euo pipefail
  [ -d $H/ReysonGTO/.git ] || git clone -q https://github.com/Suuu-sh/ReysonGTO.git $H/ReysonGTO
  cd $H/ReysonGTO
  git fetch -q origin && git checkout -q '$BRANCH' && git pull -q --ff-only origin '$BRANCH'
  bash tools/ec2/bootstrap.sh '$BRANCH' '$DATA_REPO'
  env $RUN_ENV bash tools/ec2/run.sh
"
echo "[setup] $(date -u +%FT%TZ) done: shards running, results go to $DATA_REPO every 15 minutes"
