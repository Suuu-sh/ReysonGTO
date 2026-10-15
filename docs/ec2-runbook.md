# EC2 でサイズの比較（M4：代表 200・確認用 80）を回す手順

インスタンスの作成と AWS の設定はユーザーが行う。立てたあとは、**Mac でコマンドを 1 つ実行するだけ**で、
準備・ビルド・レンジの送付・計算の開始まで進む。

## 1. インスタンスを立てる（AWS コンソール）

EC2 → インスタンスを起動、で次のように設定する。

| 項目 | 設定 |
|---|---|
| 名前 | reysongto-m4 |
| OS（AMI） | **Ubuntu Server 24.04 LTS、アーキテクチャは 64 ビット（Arm）** |
| インスタンスタイプ | **c8g.48xlarge**（192 vCPU / 384GB。速い）または **c7g.16xlarge**（64 vCPU / 128GB。台数を抑えたい場合） |
| キーペア | 新しく作る（例：reysongto）。ダウンロードした .pem を Mac の `~/.ssh/reysongto.pem` に置き、`chmod 400 ~/.ssh/reysongto.pem` |
| ネットワーク | セキュリティグループで **SSH（22 番）を「マイ IP」だけ**に許可。パブリック IP の自動割り当てを有効 |
| ストレージ | gp3 **100GB** |
| 購入オプション（高度な詳細） | **スポットインスタンス**（安い。止められても再開できる）。急ぐ・確実に回したい場合はオンデマンド |

起動したら、インスタンスの「パブリック IPv4 アドレス」を控える。

### 時間と費用の目安（概算）

| タイプ | 同時に解く数 | 280 ボードの時間 | 費用（スポット） | 費用（オンデマンド） |
|---|---|---|---|---|
| c8g.48xlarge | 16（12 スレッドずつ） | 約 15〜20 時間 | 約 40〜60 ドル | 約 120〜150 ドル |
| c7g.16xlarge | 5 | 約 2〜3 日 | 約 40〜60 ドル | 約 120〜160 ドル |

Mac（M5）では 1 ボード約 35〜45 分だった。EC2 の 1 ボードの時間は CPU の違いで前後する。予算アラートの設定を推奨。

## 2. Mac から 1 コマンドで開始

```sh
cd ~/Projects/Products/ReysonGTO-m4     # feat/m4_refine の作業フォルダ（data/ranges があること）
tools/ec2/deploy.sh ubuntu@<パブリックIP> ~/.ssh/reysongto.pem
```

これで次が自動で行われる。

1. インスタンスに必要なもの（コンパイラ、git、Python、Rust）を入れ、ReysonGTO を取ってきて release ビルドする（5〜10 分）
2. レンジ（製品データ、Git に入っていない）を送る
3. 代表 200・確認用 80 を選んで記録し、CPU とメモリに合わせた数の分担（シャード）で計算を始める。SSH を切っても止まらない

調整したい場合は環境変数で：`THREADS=12 SHARDS=16 MEM_PER_SHARD_GB=18 tools/ec2/deploy.sh ...`

## 3. 進み具合を見る

```sh
ssh -i ~/.ssh/reysongto.pem ubuntu@<パブリックIP> 'bash ReysonGTO/tools/ec2/status.sh'
```

完了したボードの数、動いている分担の数、各分担のログの最後の行、メモリの使用量が出る。

## 4. 止まった・再開したいとき

- 同じインスタンスで再開する場合：`ssh ... 'bash ReysonGTO/tools/ec2/run.sh'`（保存済みの解は飛ばす）。
- スポットが回収されてインスタンスがなくなった場合：新しく立てて、もう一度 2. を実行する（前のインスタンスの途中結果は失われる）。
- 計算を途中で止めたい場合：`ssh ... 'pkill -f tools/m4_trial.py; pkill -f "target/release/cli solve"'`

## 5. 結果を回収する

```sh
cd ~/Projects/Products/ReysonGTO-m4
tools/ec2/collect.sh ubuntu@<パブリックIP> ~/.ssh/reysongto.pem
```

インスタンス上で集計（docs/benchmarks/m4-full.md）を作り、結果の JSON とログ（大きい解の本体は除く）を Mac の `data/m4b/` にコピーする。

## 6. 終わったら

- 回収が終わったら、インスタンスを**停止ではなく終了（terminate）**する（ディスクの料金も止まる）。
- 使わなくなったセキュリティグループとキーペアは削除してよい。

## 補足

- スクリプト：`tools/ec2/deploy.sh`（Mac）、`bootstrap.sh`・`run.sh`・`status.sh`（インスタンス）、`collect.sh`（Mac）。
- 計算の中身は `tools/m4_trial.py`（全候補を約 5%、上位 2 つを約 0.5% で解く。1 つの解ごとに保存し、再開できる）。
- x86 のインスタンス（c7a など）でも動く（NEON の代わりにスカラーの実装になるので、やや遅い）。
