# EC2 でサイズの比較（M4：代表 120・確認用 40）を回す手順（どの PC からでも、すべてコマンド）

操作する PC には、ReysonGTO（公開）の clone と、AWS CLI・GitHub のトークンがあればよい。
**レンジ（製品データ）は非公開の `Suuu-sh/ReysonGTO-data` から取り、結果もそこへ 15 分ごとに push される**。
PC とインスタンスの間でファイルを直接やりとりしない。

既定：東京（ap-northeast-1）、Graviton の c8g.8xlarge（32 vCPU）のスポット。下の例は c8g.48xlarge（192 vCPU、約 8 時間）。

## 0. 準備（その PC で最初の 1 回だけ）

### 0-1. 必要なもの

- macOS：`brew install awscli git`
- Windows：**WSL（Ubuntu）** を入れて、その中で以下をすべて実行する（PowerShell で `wsl --install`、再起動後に Ubuntu を開く）。
- Linux / WSL：

```bash
sudo apt-get update && sudo apt-get install -y git curl unzip openssh-client
curl -fsSL "https://awscli.amazonaws.com/awscli-exe-linux-$(uname -m).zip" -o awscliv2.zip
unzip -q awscliv2.zip && sudo ./aws/install
```

### 0-2. AWS CLI にログインする

AWS コンソールの IAM で、作業用のユーザーにアクセスキーを作る。
必要な権限：AmazonEC2FullAccess、AmazonSSMReadOnlyAccess（イメージの取得）、ServiceQuotasFullAccess（上限の確認・申請）。

```bash
aws configure    # Access key / Secret key / region: ap-northeast-1 / output: json
aws sts get-caller-identity
```

### 0-3. GitHub のトークンを作る（インスタンスが ReysonGTO-data を読み書きするため）

GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens** → Generate new token。

| 項目 | 設定 |
|---|---|
| Repository access | Only select repositories → **Suuu-sh/ReysonGTO-data** だけ |
| Permissions | Repository permissions → **Contents: Read and write** |
| Expiration | 7 日など短く |

作ったトークンは、起動のときに環境変数 `DATA_TOKEN` で渡す（ファイルやリポジトリには保存しない）。

### 0-4. ReysonGTO を取ってくる

```bash
git clone https://github.com/Suuu-sh/ReysonGTO.git ~/ReysonGTO
cd ~/ReysonGTO
```

## 1. vCPU の上限を確認する（足りなければ申請）

使うタイプの vCPU 以上が必要（c8g.8xlarge は 32、c8g.48xlarge は 192）。

```bash
aws service-quotas get-service-quota --region ap-northeast-1 --service-code ec2 \
  --quota-code L-34B43A08 --query 'Quota.Value'
```

足りなければ申請（承認まで数分〜数日）：

```bash
aws service-quotas request-service-quota-increase --region ap-northeast-1 --service-code ec2 \
  --quota-code L-34B43A08 --desired-value 192
```

## 2. 予算のアラート（おすすめ）

```bash
ACCOUNT=$(aws sts get-caller-identity --query Account --output text)
aws budgets create-budget --account-id "$ACCOUNT" \
  --budget '{"BudgetName":"pg-yota-poc","BudgetLimit":{"Amount":"100","Unit":"USD"},"TimeUnit":"MONTHLY","BudgetType":"COST"}' \
  --notifications-with-subscribers '[{"Notification":{"NotificationType":"FORECASTED","ComparisonOperator":"GREATER_THAN","Threshold":80,"ThresholdType":"PERCENTAGE"},"Subscribers":[{"SubscriptionType":"EMAIL","Address":"<メール>"}]}]'
```

## 3. スポットインスタンスを買って、計算を始める（1 コマンド）

```bash
cd ~/ReysonGTO && git pull --ff-only
read -rs DATA_TOKEN && export DATA_TOKEN      # トークンを貼り付けて Enter（画面には出ない）
TYPE=c8g.48xlarge tools/ec2/launch.sh
```

これで次がすべて自動で行われる。

1. Ubuntu 24.04（arm64）のイメージを探し、キーペア（`~/.ssh/pg-yota-poc.pem`）と、この PC の IP からの SSH だけを許すセキュリティグループを作る
2. スポットインスタンスを買う（中断されたら停止し、空きが戻ったら同じディスクで再開する設定）
3. インスタンスに GitHub のトークンを渡し（権限 600 のファイル）、ツールの導入とビルド、ReysonGTO-data からレンジを取得
4. 代表 120・確認用 40 を選び、CPU とメモリに合わせた数（c8g.48xlarge なら 16）の分担で計算を開始
5. **15 分ごとに結果を ReysonGTO-data の `results/m4b/` へ push**（インスタンスが再起動しても自動で再開）

インスタンスの情報はこの PC の `~/ReysonGTO/.local/ec2.env` に記録される（Git には入らない）。

### ネットワークと名前

| 項目 | 既定 | 変える場合 |
|---|---|---|
| VPC | そのリージョンの**デフォルト VPC** | `VPC_ID=vpc-xxxx` |
| サブネット | その VPC のデフォルトのサブネット（なければ最初のサブネット）。**パブリック IP を付ける** | `SUBNET_ID=subnet-xxxx`（インターネットゲートウェイへの経路が必要） |
| セキュリティグループ | `pg-yota-poc`。**入る通信は SSH（22 番）を、起動した PC の IP（/32）からだけ**。出る通信は全部許可（GitHub とパッケージの取得のため） | `SG_NAME=...` |
| キーペア | `pg-yota-poc`（秘密鍵は `~/.ssh/pg-yota-poc.pem`、権限 400） | `KEY_NAME=...`、`KEY_PATH=...` |
| Name タグ | インスタンス・ディスク・セキュリティグループ・キーペアに `pg-yota-poc` | `NAME=...`（終了のときも同じ名前を指定） |

例：`VPC_ID=vpc-0abc SUBNET_ID=subnet-0def TYPE=c8g.48xlarge tools/ec2/launch.sh`

別の PC から起動し直すと、セキュリティグループに、その PC の IP からの SSH が追加で許可される（前の PC の IP は残る）。

### 時間と費用の目安（概算。1 ボード約 45 分、同時に解く 1 つあたり 1 時間に約 1.3 ボード）

| タイプ | 同時に解く数 | 1 時間あたり（スポット） | 160 ボード | 総額 |
|---|---|---|---|---|
| c8g.8xlarge（既定） | 2 | 約 0.5 ドル | 約 2.5 日 | 約 30 ドル |
| c8g.16xlarge | 5 | 約 1 ドル | 約 1 日 | 約 25〜30 ドル |
| c8g.48xlarge | 16 | 約 2.5〜3 ドル | 約 8 時間 | 約 25〜30 ドル |

## 4. 進み具合を見る（GitHub で。PC を閉じてもよい）

`launch.sh` が最後に `done. results appear in ...` と出したら、計算と結果の push はインスタンスの中で動いているので、
**起動した PC は閉じてよい**。進み具合はどの PC からでも見られる（GitHub CLI でログインしていれば）：

```bash
tools/ec2/progress.sh            # 今の status.txt
tools/ec2/progress.sh --watch    # 5 分ごとに更新し、完了したら知らせて止まる
```

ブラウザなら：

- `https://github.com/Suuu-sh/ReysonGTO-data/blob/main/results/m4b/status.txt`：1 行目が全体の状態
  - **「状態：完了」なら終わり**（全ボードが完了し、計算も止まっている）
  - 「状態：計算中」なら続いている（`boards complete: N / 160` が進み具合）
  - 「状態：停止（未完了あり…）」なら、途中で止まっている（`tools/ec2/remote.sh resume` で再開）
  - 15 分ごとに更新される（`updated:` の時刻を見る。1 時間以上古ければ、インスタンスが止まっている可能性がある）
- 完了したときのコミットは「results/m4b 完了 …」というメッセージになるので、ReysonGTO-data のコミット一覧でも分かる
- 自動のコミットは、この完了のコミットを push したところで止まる（終了までの間にコミットが増え続けることはない）
- `results/m4b/report.md`：その時点までの集計

インスタンスを直接操作したい場合（起動した PC から。`.local/ec2.env` を使う）：

```bash
tools/ec2/remote.sh status    # 今の状態
tools/ec2/remote.sh collect   # 今すぐ結果を push させる
tools/ec2/remote.sh resume    # 止まっていたら続きから再開
tools/ec2/remote.sh stop      # 計算を止める
tools/ec2/remote.sh ssh       # インスタンスに入る
```

## 5. 終わったら（「状態：完了」を確認してから。どの PC からでもよい）

完了の結果は自動で push 済みなので、そのまま終了してよい。AWS CLI でログインしている PC なら、起動した PC でなくてもよい
（起動した PC でなければ、インスタンスの名前 `pg-yota-poc` で探す）。

```bash
tools/ec2/terminate.sh             # スポットのリクエストを取り消し、インスタンスを終了（確認あり）
tools/ec2/terminate.sh --cleanup   # さらにセキュリティグループとキーペアも消す
```

- 完了の前に止める場合は、先に `tools/ec2/remote.sh collect`（起動した PC から）で最新の結果を push してから終了する（ディスクごと消える）。
- スポットのリクエストを取り消さずにインスタンスだけ終了すると、代わりのインスタンスが立ち上がるので、必ずこのスクリプトで終了する。
- 使い終わったトークンは GitHub で削除する。

残っていないかの確認：

```bash
aws ec2 describe-instances --region ap-northeast-1 \
  --filters Name=tag:Name,Values=pg-yota-poc Name=instance-state-name,Values=pending,running,stopping,stopped \
  --query 'Reservations[].Instances[].InstanceId'      # [] なら残っていない
```

## 補足

- スクリプト：`launch.sh`・`deploy.sh`・`remote.sh`・`collect.sh`（起動した PC）、`progress.sh`（GitHub CLI がある PC）、`terminate.sh`（AWS CLI がある PC）、
  `bootstrap.sh`・`run.sh`・`status.sh`・`sync.sh`（インスタンス）。
- 比べるボードの数：`TRAINING=200 VALIDATION=80 TYPE=... tools/ec2/launch.sh`
- 計算の中身は `tools/m4_trial.py`（全候補を約 5%、上位 2 つを約 0.5% で解く。1 つの解ごとに保存し、再開できる）。
- `remote.sh` と `collect.sh` は、起動した PC の `.local/ec2.env`（と SSH の鍵）を使う。`progress.sh` と `terminate.sh` は、どの PC からでも使える。
