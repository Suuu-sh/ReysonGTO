# EC2 でサイズの比較（M4：代表 120・確認用 40）を回す手順（どの PC からでも、すべてコマンド）

- 操作する PC には、ReysonGTO（公開）の clone と、AWS CLI・GitHub のトークンがあればよい。
- **AWS で作るのはインスタンス 1 台だけ（名前 `pg-yota-poc`）**。VPC・サブネット・セキュリティグループはデフォルトのものを使い、キーペアは作らない。**SSH もしない**。
- インスタンスは起動時のスクリプト（ユーザーデータ）で、準備・ビルド・計算の開始までを自分で行う。
- **レンジ（製品データ）は非公開の `Suuu-sh/ReysonGTO-data` から取り、結果と状態もそこへ 15 分ごとに push される**。

既定：東京（ap-northeast-1）、Graviton の c8g.8xlarge（32 vCPU）のスポット。下の例は c8g.48xlarge（192 vCPU、約 8 時間）。

## 0. 準備（その PC で最初の 1 回だけ）

### 0-1. 必要なもの

- macOS：`brew install awscli gh git`
- Windows：**WSL（Ubuntu）** を入れて、その中で以下をすべて実行する（PowerShell で `wsl --install`、再起動後に Ubuntu を開く）。
- Linux / WSL：

```bash
sudo apt-get update && sudo apt-get install -y git curl unzip gh
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

**別の AWS アカウントを使う場合**は、プロファイルを分けて設定し、以降のコマンドの前に `AWS_PROFILE=<名前>` を付ける
（`launch.sh` は購入の前にアカウント ID を表示して確認を求める。起動した PC の `terminate.sh` は、起動時のプロファイルを自動で使う）：

```bash
aws configure --profile other                  # 別のアカウントのアクセスキー
AWS_PROFILE=other aws sts get-caller-identity  # アカウント ID を確認
export AWS_PROFILE=other                       # このターミナルでは以降ずっと other を使う
```

### 0-3. GitHub CLI にログインする（進み具合を見るため）

```bash
gh auth login
```

### 0-4. GitHub のトークンを作る（インスタンスが ReysonGTO-data を読み書きするため）

GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens** → Generate new token。

| 項目 | 設定 |
|---|---|
| Repository access | Only select repositories → **Suuu-sh/ReysonGTO-data** だけ |
| Permissions | Repository permissions → **Contents: Read and write** |
| Expiration | 7 日など短く |

トークンは起動のときに環境変数 `DATA_TOKEN` で渡す。インスタンスのユーザーデータに入る（EC2 の権限がある人には見える）ので、
**このリポジトリだけ・短い期限**にし、終わったら削除する。

### 0-5. ReysonGTO を取ってくる

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
TYPE=c8g.8xlarge THREADS=10 tools/ec2/launch.sh   # 32 vCPU / 64GB で 3 つ同時に（約 2〜2.2 日）
```

**組織の SCP などで購入が禁止されていないかを、買わずに確かめる**（おすすめ。止められたら理由が表示される）：

```bash
DRY_RUN=1 TYPE=c8g.8xlarge THREADS=10 tools/ec2/launch.sh
```

`DRY RUN OK` なら、`DRY_RUN=1` を外して実行する。`DRY RUN DENIED` なら、表示された理由（どの操作・条件が拒否されたか）を管理者か Claude に伝える。

`launch.sh` がすること：

1. Ubuntu 24.04（arm64）のイメージと、デフォルトの VPC・サブネット・セキュリティグループを探す
2. スポットインスタンスを 1 台買う（名前 `pg-yota-poc`。中断されたら停止し、空きが戻ったら同じディスクで再開する設定）
3. 起動を待って終わる（**このあと PC は閉じてよい**）

インスタンスが自分ですること（ユーザーデータ）：

1. ツールの導入、ReysonGTO の取得とビルド、ReysonGTO-data からレンジの取得（約 10 分）
2. 代表 120・確認用 40 を選び、CPU とメモリに合わせた数（c8g.48xlarge なら 16）の分担で計算を開始
3. 15 分ごとに結果と状態を ReysonGTO-data の `results/m4b/` へ push（再起動しても自動で再開。完了を push したら止まる）
4. 準備に失敗したら、そのログを `results/m4b/setup-failed.log` として push する

### ネットワークと名前

| 項目 | 使うもの |
|---|---|
| VPC・サブネット | デフォルト VPC とそのデフォルトのサブネット（変える場合 `VPC_ID=` / `SUBNET_ID=`。インターネットへの経路が必要） |
| セキュリティグループ | その VPC の **default**（入る通信は不要。出る通信で GitHub とパッケージを取得） |
| パブリック IP | 付ける（GitHub に出るため） |
| キーペア・SSH | 使わない |
| ディスク・メタデータ | ディスクは暗号化（アカウントの既定の KMS キー）、インスタンスのメタデータは IMDSv2 のみ（組織の SCP でよく求められる設定） |
| 名前 | インスタンスの Name タグ `pg-yota-poc` だけ |

### 時間と費用の目安（概算。1 ボード約 45 分、同時に解く 1 つあたり 1 時間に約 1.3 ボード）

| タイプ | 同時に解く数 | 1 時間あたり（スポット） | 160 ボード | 総額 |
|---|---|---|---|---|
| c8g.8xlarge（既定） | 2 | 約 0.5 ドル | 約 2.5 日 | 約 30 ドル |
| c8g.16xlarge | 5 | 約 1 ドル | 約 1 日 | 約 25〜30 ドル |
| c8g.48xlarge | 16 | 約 2.5〜3 ドル | 約 8 時間 | 約 25〜30 ドル |

## 4. 進み具合を見る（どの PC からでも）

```bash
tools/ec2/progress.sh            # ReysonGTO-data の status.txt（GitHub CLI）
tools/ec2/progress.sh --watch    # 5 分ごとに更新し、完了したら知らせて止まる
tools/ec2/console.sh             # status.txt が出る前の準備の様子（AWS のコンソール出力）
```

ブラウザなら `https://github.com/Suuu-sh/ReysonGTO-data/blob/main/results/m4b/status.txt`。1 行目が全体の状態：

- **「状態：完了」なら終わり**（全ボードが完了し、計算も止まっている）
- 「状態：計算中」なら続いている（`boards complete: N / 160` が進み具合）
- 「状態：停止（未完了あり…）」なら途中で止まっている → `tools/ec2/resume.sh`（インスタンスを再起動して続きから）
- `updated:` の時刻が 1 時間以上古ければ、インスタンスが止まっている可能性がある（スポットの中断など）
- `results/m4b/setup-failed.log` があれば、準備に失敗している（中身を見て、`terminate.sh` で終了してから直して起動し直す）

完了したときのコミットは「results/m4b 完了 …」というメッセージになるので、ReysonGTO-data のコミット一覧でも分かる。

## 5. 終わったら（「状態：完了」を確認してから。どの PC からでもよい）

```bash
tools/ec2/terminate.sh     # 名前 pg-yota-poc のインスタンスのスポットのリクエストを取り消し、終了（確認あり）
```

- 完了の結果は自動で push 済みなので、そのまま終了してよい。完了の前に止めると、最後の push より新しい結果は失われる。
- スポットのリクエストを取り消さずにインスタンスだけ終了すると、代わりのインスタンスが立ち上がるので、必ずこのスクリプトで終了する。
- 使い終わったトークンは GitHub で削除する。

残っていないかの確認：

```bash
aws ec2 describe-instances --region ap-northeast-1 \
  --filters Name=tag:Name,Values=pg-yota-poc Name=instance-state-name,Values=pending,running,stopping,stopped \
  --query 'Reservations[].Instances[].InstanceId'      # [] なら残っていない
```

## 補足

- スクリプト：`launch.sh`（購入と起動）、`progress.sh`（進み具合、GitHub CLI）、`console.sh`（準備の様子）、`resume.sh`（再開）、
  `terminate.sh`（終了）は操作する PC で。`user-data.sh`・`bootstrap.sh`・`run.sh`・`status.sh`・`sync.sh` はインスタンスの中で動く。
- 比べるボードの数：`TRAINING=200 VALIDATION=80 TYPE=... tools/ec2/launch.sh`
- 計算の中身は `tools/m4_trial.py`（全候補を約 5%、上位 2 つを約 0.5% で解く。1 つの解ごとに保存し、再開できる）。
