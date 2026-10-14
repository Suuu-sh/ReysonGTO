# ReysonGTO

ReysonAI のための、クリーンルーム実装の Rust ポストフロップ CFR ソルバー。
HU のフロップ／ターン／リバー開始を扱います。プリフロップ・サイズ探索・配信 API は未実装です。

- [設計書](docs/design.md)
- [M1 の測定・検証・実装対応表](docs/benchmarks/m1.md)
- [M2 の固定条件比較・測定](docs/benchmarks/m2.md)
- [依存 crate とライセンス監査](docs/dependencies.md)

既存の OSS ポーカーソルバー・役判定のソースは参照・利用していません。
製品レンジは含めません。レンジは外部 JSON を実行時に読み込みます。

## ビルドとテスト

Rust stable と Cargo が必要です。外部依存は `Cargo.lock` に固定しています。

```sh
cargo build --release
cargo test
cargo clippy -- -D warnings
# 重い全探索: 全 133,784,560 通りの 7 枚を独立した素朴な役判定と比較
cargo test --release -- --ignored
```

通常の `cargo test` にランダム 100 万件の役判定比較を含みます。
全探索では、独立した 5 枚判定の全結果を一度キャッシュし、7 枚の 21 部分集合の最大値と比較します。
CFR のテストは終端の二重ループ比較、トイゲーム、BR、同型、決定性、レーキを扱います。

Cargo の既定キャッシュが読み取り専用の場合は、書き込み可能な作業用ディレクトリを
`CARGO_HOME` に指定してから同じコマンドを実行してください。

## CLI

既定のチップ精度は **1 BB = 1,000 単位**（`--units-per-bb` で変更可）、CLI の `--pot`, `--stack`, `--rake-cap` は BB です。
`--bet 50` はポットの 50%、`--target 0.3` は **この木の開始ポット**の 0.3% を意味します。
`--rake 0.05` は 5% です。小数のチップ額は最も近い整数に丸めます。

```sh
# 意味のない合成レンジ。製品レンジではありません。
printf '%s\n' '{"AhAd":1,"KhKd":1}' > /tmp/reyson-oop.json
printf '%s\n' '{"QcQd":1,"JhJd":1}' > /tmp/reyson-ip.json

cargo run --release -p cli -- solve \
  --board 2c3d7h8s9c --pot 2 --stack 1 \
  --oop-range /tmp/reyson-oop.json --ip-range /tmp/reyson-ip.json \
  --bet 50 --rake 0.05 --rake-cap 3 \
  --target 0.3 --max-iters 1000 --max-seconds 30 \
  --threads 10 --out /tmp/reyson-solution.json

cargo run --release -p cli -- bench
# bench は内部で単一スレッドのプロファイルも取り、終端の計算時間の割合を返します。
# 主計測自体も単一スレッドにする場合
cargo run --release -p cli -- bench --threads 1
cargo run --release -p cli -- solve --help
```

ボードは `AhKd2c`、`"Ah Kd 2c"`、`"Ah,Kd,2c"` 形式で 3〜5 枚指定できます。
`--oop-leads` はフロップの OOP の先行ベットを許可します（未指定では初手チェック固定）。
ターン・リバー開始は通常通り OOP からベット可能です。
`--algorithm cfr-plus` で CFR+、既定は DCFR（α=1.5, β=0, γ=2）です。
`--no-isomorphism` で同型削減を無効化します。
`--memory-mb` はメモリ見積もりの上限（既定 8192 MiB）です。
`--storage f32|i16` は後悔値・平均戦略の保存方式（既定 f32）。i16 はノード別倍率を持つ量子化です。
`--raise-multiplier` は既定 2.5。`--no-allin-merge` は 67% まとめを無効にします
（未指定では `--allin-merge-ratio 0.67`）。追加の強制 all-in／枝の頻度まとめ閾値はありません。
`--dcfr-alpha`, `--dcfr-beta`, `--dcfr-gamma` で DCFR の係数を指定できます。
`--metrics-every 10` が既定で、初期・10 反復ごと・終了時に EV / BR を再計算します。
セットアップと精度チェックの進捗は stderr に JSONL で出し、
`--progress-jsonl /path/to/new-file.jsonl` でも逐次 flush して保存できます（既存ファイルは上書きしません）。

### コピー済みの比較出力との照合

```sh
cargo run --release -p cli -- compare \
  --solution /path/to/reyson-solution.json \
  --reference-dir data/benchmark --out /tmp/reyson-compare.json
```

ルート OOP BB と OOP チェック後の IP BTN を、コンボ・クラスの両方で比較します。
比較用 CSV の `reach_weight` を共通の重みにして、アクション別の平均／最大絶対差、
全体頻度差を出します。クラス戦略も同じコンボ重みで再集計するため、到達分布の違いと戦略差を混同しません。
自分の到達重みによる全体頻度は別項目です。差は確率単位（0.01 = 1 percentage point）です。
設定・チップ精度・アクション・コンボ集合が一致しなければエラーになります。
ルートの正規化済み互換コンボ到達重みも照合します（float32 出力誤差のため L1 許容値 1e-4）。
固定比較の解法には `--units-per-bb 65536 --pot 5.5 --stack 97.5 --bet 50 --oop-leads`
と `--rake 0 --rake-cap 0 --raise-multiplier 2.5 --no-allin-merge` を使用します。
精度が違う途中経過の比較は収束性能の同等性を意味しません。コピー済みの出力だけを読み、OSS ソースは参照しません。

### レンジ形式

```json
{"AhKd":0.85,"QsQh":0.2}
```

またはメタデータ付き `{"player":"OOP","combos":{"AhKd":0.85}}`。
クラス指定 `{"AKs":0.5,"QQ":0.6}` も利用できます。
**クラスの重みは総量**で、AKs の各コンボは 0.5 / 4、QQ の各コンボは 0.6 / 6 です。
ボード重複除去後にプレイヤー別の密な番号を振り、互いに重ならない私有カードの
組について入力重みの積で条件付き正規化します。クラスと個別コンボの二重指定はエラーです。
JSON の同一キー重複は避けてください（serde_json は最後の値を採用します）。

### 出力と停止条件

JSON は同じディレクトリの一時ファイルに書き切ってから rename で置換します。
設定一式、ボード除去後の入力レンジ、ルートと OOP チェック後の IP のコンボ別／クラス別平均戦略、
両者 EV、BR、反復履歴／時間、メモリ見積もり、停止理由、訓練・metrics・終端等のプロファイルを含みます。
戦略配列の順は同じオブジェクトの `actions` と対応します。
クラス戦略は、両者の平均戦略による到達と私有カードの互換性を反映した `reach_weight` による平均です。

EV の単位はチップです。`受取額 − 開始後の自分の投資 − 開始ポット / 2` で表示するため、
レーキなしでは両者の EV の和は 0、レーキありでは期待レーキ分だけ負になります。
引き分けはレーキ後のポットを折半します。フォールド終端もそのノードのポット全額にレーキを計算します。

停止条件は目標精度・反復数・解法ループの経過時間のいずれかです。
時間は反復境界で確認し、木構築・初期化は `setup_seconds` として別記します。
1 反復や初期 BR の途中を中断しないため、短い時間上限を超える場合があります。
精度計算は訓練とは別の 4 回の木走査を含み、その時間も `solve_seconds` に含まれます。
チェックをしない反復の履歴 `metrics` は null で、最終 metrics は必ず最新です。
`training_seconds` と `metrics_seconds` は壁時計、プロファイルの終端・後悔値・chance はワーカー時間の合計です。
タイミングを除く数値結果は同一入力・同一スレッド数で決定的です。

## ライブラリ API

`solver::solve(GameSpec, [Range; 2], StopConditions) -> Result<Solution, String>`。
`GameSpec::units_per_bb` は入力チップ精度、`StopConditions::metrics_every` は精度チェック間隔です。
`solve_with_progress` はセットアップと最新精度チェックを callback に送り、callback のエラーは伝播します。
`Solution::strategy(node_id)` は任意の判断ノードのコンボ／クラス別戦略、
`Solution::root_ev()` はルートの両プレイヤー EV、`Solution::tree()` は公開アリーナを返します。

`tree::SizeRule::Custom(Arc<dyn Fn(&SizeContext) -> f64 + Send + Sync>)` により、
ストリート・実ボード・プレイヤー・最初のベット・チェック後かを用いてサイズを変えられます。
クロージャは決定的な純粋関数としてください。クロージャ自体は JSON にシリアライズできません。
CLI は再現可能な `SizeRule::Fixed` を使用します。

`cfr::estimate_memory_bytes_with_config(&tree, &dense_ranges, &config)` で保存方式込みの見積もりを得られます。
`Engine::new` はワーカー分を加算して上限をチェックします。
`solve` は木構築にもメモリから求めた保守的なノード数制限を掛け、履歴用メモリも予算に含めます。
後悔値・平均戦略は連続した f32 または i16 配列で、現在戦略は深さ／並列サブツリー別 scratch に生成します。
反復用 scratch と履歴は事前確保します。`persistent_storage_bytes` は後悔値と平均戦略の実保存量です。
低水準の `Engine` で並列反復を直接呼ぶ場合は `with_pool` 内でループしてください
（外から毎回 Rayon のプールへ投入すると、Rayon 内部のキューが断続的に確保されます）。
見積もりは RSS 実測ではなく、アロケータや OS の実メモリ使用量を厳密に保証するものではありません。

### 同型削減の範囲

ボードを保存し、**コンボ置換後も両レンジの重みが bit 単位で完全一致する**スート置換だけを用います。
代表カードを一度計算し、同値なカード数で重み付けします。
同値ノードの戦略は、代表ノードからコンボ番号を正しく写し戻します。
非対称レンジを近似的に対称化せず、範囲外は正確な非削減計算にフォールバックします。
カスタムサイズ規則はスート同変性を保証できないため削減しません。
公開アリーナ自体は全カードを保持し、同値枝の永続戦略領域と走査を共有します。
なお、異なるランク 3 枚のレインボーフロップではボード保存の非自明なスート置換がなく、
設計書の例と異なり、この同型定義だけでターン 49 枚を大幅には減らせません。

フルレンジ・深いスタックのフロップ木は大きく、M1 の小規模ベンチ結果を本番性能と解釈しないでください。
固定した外部出力との M2 比較の条件・精度・時間・メモリの制限は、測定レポートを参照してください。
