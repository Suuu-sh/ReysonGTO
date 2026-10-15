# EC2 でサイズの比較・本番計算を回す手順

状態：下書き（M4 の 21 ボード目以降を EC2 で回すため）。インスタンスの作成と AWS の設定はユーザーが行う。

## 1. インスタンス

| 項目 | おすすめ | 理由 |
|---|---|---|
| 種類 | **Graviton（ARM）のコンピュート向け**：c8g.48xlarge（192 vCPU / 384GB）または c7g.16xlarge（64 vCPU / 128GB） | ソルバーは aarch64 の NEON 向けに最適化済み。価格あたりの性能が良い |
| 買い方 | スポット（止められても、1 つの解ごとに保存して再開できる） | オンデマンドの 3〜4 割の料金 |
| OS | Ubuntu 24.04 LTS（arm64） | Rust の導入が簡単 |
| ディスク | gp3 100GB | 解のファイル（1 つ数百 MB〜数 GB）を一時的に置く |
| ネットワーク | SSH（22 番）を自分の IP だけに開く | — |

メモリ：1 つの解（本番の設定、f32）で最大 約 9〜16GB。192 vCPU / 384GB なら同時に 12〜16 個、64 vCPU / 128GB なら 4〜6 個。

## 2. 準備（インスタンスの上で 1 回だけ）

```sh
sudo apt-get update && sudo apt-get install -y build-essential git python3
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
git clone https://github.com/Suuu-sh/ReysonGTO.git && cd ReysonGTO
git checkout feat/m4_refine   # またはその時点の最新のブランチ
cargo build --release
```

レンジ（製品データ、Git に入っていない）は Mac から送る：

```sh
# Mac 側で
scp -r ~/Projects/Products/ReysonGTO/data/ranges <host>:ReysonGTO/data/
```

## 3. 実行（Mac での計算が終わった後に、同時実行の機能を足してから）

- 予定：代表 200 ボードのうち 21〜200 番目と、確認用 80 ボードを、**ボードの分担（シャード）**で同時に回す。
  例：16 個同時なら `--shard 0/16` 〜 `--shard 15/16` を、それぞれ `--threads 12` で起動する。
- 1 つの解ごとに保存されるので、止まっても同じコマンドで再開できる。
- `tmux` か `nohup` で起動し、SSH が切れても止まらないようにする。

## 4. 結果の回収

```sh
# Mac 側で（解の本体は大きいので、結果の JSON だけを回収する）
rsync -av --include='*/' --include='result.json' --include='*.json' --exclude='solution.json' \
  <host>:ReysonGTO/data/m4b/ ~/Projects/Products/ReysonGTO/data/m4b/
```

## 5. 止めるとき

- 回収が終わったら、インスタンスを**停止ではなく終了（terminate）**する（ディスクの料金も止まる）。
- AWS の予算アラートを設定しておく（ユーザーの作業）。
