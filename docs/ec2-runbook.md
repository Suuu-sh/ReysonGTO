# EC2 でサイズの比較（M4：代表 120・確認用 40）を回す手順（すべてコマンド）

Mac のターミナルで、上から順に実行する。AWS の認証情報の入力（0-2）と、上限の引き上げの申請（1）はユーザーが行う。
既定のリージョンは東京（ap-northeast-1）、インスタンスは c8g.8xlarge（Graviton、32 vCPU / 64GB、同時に 2 ボード）のスポット。

## 0. 準備（最初の 1 回だけ）

### 0-1. AWS CLI を入れる

```bash
brew install awscli
```

### 0-2. AWS CLI にログインする

AWS コンソールの IAM で、作業用のユーザー（またはロール）にアクセスキーを作り、次で設定する。
必要な権限：EC2 の操作（AmazonEC2FullAccess）、AMI の取得（AmazonSSMReadOnlyAccess）、上限の確認と申請（ServiceQuotasFullAccess）、予算（AWSBudgetsActionsWithAWSResourceControlAccess など、2. を使う場合）。

```bash
aws configure    # Access key / Secret key / region: ap-northeast-1 / output: json
```

確認：

```bash
aws sts get-caller-identity
```

### 0-3. ReysonGTO の作業フォルダ

```bash
cd ~/Projects/Products/ReysonGTO && git checkout main && git pull --ff-only
ls data/ranges   # btn_open.json と bb_call.json があること（製品のレンジ。Git には入っていない）
```

## 1. vCPU の上限を確認する（足りなければ申請）

スポットの vCPU の上限（全標準インスタンス）が、使うタイプの vCPU 以上必要（c8g.8xlarge なら 32、c8g.16xlarge なら 64、c8g.48xlarge なら 192）。

```bash
aws service-quotas get-service-quota --region ap-northeast-1 --service-code ec2 \
  --quota-code L-34B43A08 --query 'Quota.Value'
```

足りなければ申請する（承認まで数分〜数日）：

```bash
aws service-quotas request-service-quota-increase --region ap-northeast-1 --service-code ec2 \
  --quota-code L-34B43A08 --desired-value 64
```

オンデマンドで回す場合の上限は `L-1216C47A`（同じコマンドで確認・申請）。

## 2. 予算のアラート（おすすめ）

月 100 ドルを超えそうになったらメールで知らせる。`<メール>` を自分のアドレスに置き換える。

```bash
ACCOUNT=$(aws sts get-caller-identity --query Account --output text)
aws budgets create-budget --account-id "$ACCOUNT" \
  --budget '{"BudgetName":"reysongto","BudgetLimit":{"Amount":"100","Unit":"USD"},"TimeUnit":"MONTHLY","BudgetType":"COST"}' \
  --notifications-with-subscribers '[{"Notification":{"NotificationType":"FORECASTED","ComparisonOperator":"GREATER_THAN","Threshold":80,"ThresholdType":"PERCENTAGE"},"Subscribers":[{"SubscriptionType":"EMAIL","Address":"<メール>"}]}]'
```

## 3. スポットインスタンスを買って、計算を始める（1 コマンド）

```bash
tools/ec2/launch.sh
```

これで次がすべて自動で行われる。

1. Ubuntu 24.04（arm64）の最新イメージを探す
2. キーペア（`~/.ssh/reysongto.pem`）とセキュリティグループ（この Mac の IP からの SSH だけ許可）を作る
3. **c8g.8xlarge のスポットインスタンス**を買う（中断されたら停止し、空きが戻ったら同じディスクで再開する設定）
4. 起動と SSH を待つ
5. ツールの導入とビルド（5〜10 分）、レンジの送付、代表 120・確認用 40 の選択、CPU とメモリに合わせた数の分担で計算を開始
   （インスタンスが再起動したときも自動で再開する）

インスタンスの情報は `.local/ec2.env` に記録される（Git には入らない）。

変えたい場合は環境変数で指定する：

```bash
TYPE=c8g.16xlarge tools/ec2/launch.sh          # 64 vCPU / 128GB（同時に 5 ボード。早いが同じくらいの費用）
TYPE=c8g.48xlarge tools/ec2/launch.sh          # 192 vCPU / 384GB（同時に 16 ボード）
MARKET=ondemand tools/ec2/launch.sh            # 中断されない（料金は 3〜4 倍）
REGION=us-east-1 tools/ec2/launch.sh           # スポットの空きや価格で選ぶ
TRAINING=200 VALIDATION=80 tools/ec2/launch.sh # 比べるボードの数を変える（既定は代表 120・確認用 40）
```

### 時間と費用の目安（概算。1 ボード約 45 分、同時に解く 1 つあたり 1 時間に約 1.3 ボード）

| タイプ | 同時に解く数 | 1 時間あたり（スポット） | 160 ボード（既定） | 280 ボード | 総額 |
|---|---|---|---|---|---|
| c8g.8xlarge（既定） | 2 | 約 0.5 ドル | 約 2.5 日 | 約 4〜5 日 | 160 で約 30 ドル |
| c8g.16xlarge | 5 | 約 1 ドル | 約 1 日 | 約 2 日 | 160 で約 25〜30 ドル |
| c8g.48xlarge | 16 | 約 2.5〜3 ドル | 約 8 時間 | 約 13〜15 時間 | 160 で約 25〜30 ドル |

総額はボードの数でほぼ決まり、タイプを小さくすると時間が延びるだけ。

## 4. 進み具合を見る・操作する

```bash
tools/ec2/remote.sh status    # 完了したボードの数、動いている分担、ログの最後の行、メモリ
tools/ec2/remote.sh resume    # 止まっていたら続きから再開（保存済みは飛ばす）
tools/ec2/remote.sh stop      # 計算を止める（インスタンスは動いたまま）
tools/ec2/remote.sh ssh       # インスタンスに入る
```

## 5. 結果を回収する

```bash
tools/ec2/remote.sh collect
```

インスタンス上で集計（`docs/benchmarks/m4-full.md`）を作り、結果の JSON とログ（大きい解の本体は除く）を Mac の `data/m4b/` にコピーする。

## 6. 終わったら必ず終了する

```bash
tools/ec2/terminate.sh             # スポットのリクエストを取り消し、インスタンスを終了（確認あり）
tools/ec2/terminate.sh --cleanup   # さらにセキュリティグループとキーペアも消す
```

**回収してから終了すること**（ディスクごと消える）。スポットのリクエストを取り消さずにインスタンスだけ終了すると、
代わりのインスタンスが立ち上がるので、必ずこのスクリプトで終了する。

確認：

```bash
aws ec2 describe-instances --region ap-northeast-1 \
  --filters Name=tag:Name,Values=reysongto-m4 Name=instance-state-name,Values=pending,running,stopping,stopped \
  --query 'Reservations[].Instances[].InstanceId'      # [] なら残っていない
```

## 補足

- スクリプト：`launch.sh`・`deploy.sh`・`remote.sh`・`collect.sh`・`terminate.sh`（Mac）、`bootstrap.sh`・`run.sh`・`status.sh`（インスタンス）。
- 計算の中身は `tools/m4_trial.py`（全候補を約 5%、上位 2 つを約 0.5% で解く。1 つの解ごとに保存し、再開できる）。
- x86 のインスタンス（例：`TYPE=c7a.48xlarge`）でも動くが、`launch.sh` は arm64 のイメージを使うので、x86 の場合はイメージの指定を変える必要がある。
