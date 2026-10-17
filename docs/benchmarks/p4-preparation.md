# P4 準備：multiway tree / equity / DCFR

2026-10-17。クリーンルームの自前実装、依存追加なし。本番エクイティ表の生成・本番 solve は未実施。
M5 の既存記録は変更しない。新しいアクションを追加した木は別ゲームなので、旧 HU のビット一致は
明示的な `PreflopTree::build_legacy` / CLI `--legacy-hu` の旧木で確認する。

## 木

- 4bet：固定額と jam、同額は重複排除。
- 5bet：直前4betの live 総額 × `sizing.five_bet_multiplier`（既定2.2）。
  最小レイズを満たすよう補正し、追加投入額が残りスタック ×
  `sizing.five_bet_merge_remaining_fraction`（既定0.67）以上なら jam に統合する。
- 6bet以降：jamのみ。openへのcold callとsqueezeを維持し、4bet以降の新規cold callは禁止。
- 外部100BB設定は読取専用。製品設定・レンジのコピーは保存しない。

| 外部6人100BB | 変更前 | 変更後 |
| --- | ---: | ---: |
| 全ノード | 66,748 | 311,039 |
| decision | 31,070 | 143,549 |
| fold終端 | 4,651 | 24,023 |
| flop終端 | 4,615 | 20,750 |
| all-in終端 | 26,412 | 122,717 |
| 最大深さ | 21 | 24 |

変更前はソース変更前の計数、変更後はrelease CLI `tree-stats`で確認。約4.66倍だが既定上限2,000,000以内。

## 3人エクイティ表

169クラスの重複あり unordered triple は818,805個。各クラスのコンボを独立に抽出し、
カード重複があれば配り方全体を棄却するので、合法なjoint dealを一様に選ぶ。
残りカードからboardを抽出しM1で評価、引分は等分する。存在しないjoint deal（AA/AA/AA等）は`None`。
同クラス席の推定値は交換対称性で平均化する。組ごとの独立seedによりrayonのthread数に依存しない。

metadata・クラス順・長さ・合法性・確率範囲・和・同クラスの対称性を検証し、FNV-1a-64 checksumで
偶発的な破損を検出する。これは暗号学的な真正性の保証ではない。

### 小規模の時間測定

4 threads、等間隔に選んだ100 triples、seed123。

| samples / triple | wall秒 |
| --- | ---: |
| 1 | 0.002682 |
| 1,000 | 0.066540 |

差分からsamplingの傾きを求め、全818,805 triples × samples1,000へ外挿すると **約523秒（8.7分）**。
IO・checksumを除いた概算で、棄却率の分布、CPU競合、短時間測定誤差、スケーリングに左右される。
全表のsamples1,000生成は実行していない。検証用の全表はsamples1のみ。

## CLI

```sh
# 小サンプル確認。生成スレッドを明示する。
RAYON_NUM_THREADS=4 target/release/preflop equity-build-3way \
  --samples 1 --seed 20261017 --threads 4 --out /tmp/three-way-smoke.json

# 指定した2〜6席をそのまま解く。省略時に強制HU化しない。
# --equity-3wayを省略すると警告の上、3人終端にもproduct近似を使用。
RAYON_NUM_THREADS=4 target/release/preflop solve \
  --config "$CFG" --equity "$HU_TABLE" --equity-3way "$THREE_TABLE" \
  --realization configs/preflop-realization.json \
  --max-iters 3 --metrics-every 3 --target-bb 0 --summary-only --out /tmp/p4-smoke.json

# M5の外部100BB→BTN/BB旧木を再現する場合は両方を明示する。
target/release/preflop solve --config "$CFG" --heads-up --legacy-hu \
  --equity "$HU_TABLE" --realization configs/preflop-realization.json --out /tmp/m5-regression.json
```

`--summary-only`は巨大な全ノード別class JSONを省略するだけで、EV/BR/NashConvの木の走査は省略しない。
上記コマンドは使用例であり、本番入力を用いた実行済みという意味ではない。

## CFR と近似の境界

- 2〜6席の交互更新DCFR、割引はM5と同じ。各席のreachは1,326コンボ。
- 2人ゲームの数値カーネルは維持。3席以上の初期reachと対応する終端はクラス内でsuit不変なので、
  regret/strategyの重複値は169クラスで保持する。reachそのものは1,326のままで、
  任意の非対称なコンボ入力に適用できる一般的な圧縮とは主張しない。
- 2人残存の終端ではHUクラス表と合法相手コンボ数によるブロッカー補正を使用。
  ただし元の卓のフォールド済み席とのカード重複は独立reachとして近似する。
- 3人表がある場合は相手2席のクラス到達重みで積分し、ゼロ重みを飛ばす。
  クラス内のjoint suit blockerは近似。存在しないtripleは勝率だけでなくその組の投資項も除外する。
- 4人以上／3人表なしではHU勝率の積をそのまま使用し、全席の勝率和への再正規化はしない。
  HUの引分半分を含む値の積であり、multiwayの引分・順位相関は正確には扱わない。
  このfallbackの受取額合計はポットと一致する保証がない。
- multiway flop実現率は未学習の0.8（`multiway`設定で変更可能、既存JSONでは省略時0.8）。
  all-inには実現率を掛けない。foldはレーキなし。
- NashConvは各席のBR改善幅の合計、BB。一般和・近似ゲームであり、6人DCFRの収束保証や
  実戦のGTO精度を意味しない。3反復の頻度は本番レンジや外部目安との精度比較に使わない。
- 3人のall-in会計テストでは、受取額の和は`pot-rake`、投資差引後のnet価値の和は`-rake`。
  flopの実現率やfallback積を含むゲーム全体がゼロ和であるという意味ではない。

## HU golden の出所

変更前HEAD `14d92462308bfa3b094af93f90d5d0018ecabe43` のソースを別の一時ディレクトリでビルドし、
合成40BBと外部100BB→HUの旧木を測定。現在の`--legacy-hu`と、時間を除いたEV・BR・NashConv・
summaryおよび全ノード全クラスの頻度を比較する。100k本番HU表はこの作業ディレクトリにないため、
この回帰には自前のsamples1/seed20261015表を使う。M5掲載の本番100k表による数値そのものを
今回再測定したという主張ではない。

| 識別対象 | hash |
| --- | --- |
| 旧 `cfr.rs` SHA-256 | `9709a092c821069f341ad6b1dd54132d10188d076b3796dd2f5ee19a0a9e41e4` |
| 隔離した旧binary SHA-256 | `bd4c9d5f13cbc5edb394dd0d560f8c90ceba51d460457381d4a1d8af67a28d47` |
| samples1 HU表 SHA-256 | `f1c5dd1fd2975eb9650aefee638935e7efb07e6ef45f69a71f005e26b8404108` |
| 合成40BB canonical report FNV64 | `6f9de4c0ca91a00d` |
| 外部100BB canonical report FNV64 | `dca0c834b8a1a935` |

外部設定自体はfixtureに保存しない。旧HU goldenは新木の解との一致を要求するものではない。

## 対象テスト名

木・設定：
- `four_bet_fixed_and_jam_five_bet_multiplier_then_six_bet_jam`
- `five_bet_merge_uses_additional_chips_and_remaining_stack`
- `four_bet_stack_cap_deduplicates_jam_and_node_limit_applies`
- `calls_before_four_bet_allowed_but_new_cold_calls_after_forbidden`
- `five_bet_defaults_and_parameter_validation`
- `three_player_push_fold_has_joint_allin_and_limit`
- `shallow_multiway_push_fold_never_reacts_with_allin_seat`

エクイティ：
- `three_way_known_matchup_and_indexing`
- `three_way_roundtrip_order_corruption_and_threads`
- `three_way_generation_estimate`（明示実行する小規模計測）

CFR：
- `hu_golden_legacy`
- `full_legacy_hu_40bb_bit_exact_golden`
- `full_legacy_hu_100bb_bit_exact_golden`（外部設定が必要なので通常はignore）
- `three_seat_push_fold_converges_and_preserves_class_reaches`
- `three_way_all_in_payout_and_net_accounting_and_zero_reaches`
- `folded_seat_reach_scales_active_hu_but_not_own_counterfactual_value`
- `folded_hero_subtree_pruning_matches_exhaustive_three_seat_values`
- `six_seat_100bb_three_iterations_smoke`（重い計測なので通常はignore）

CLI：`three_way_build_and_multiway_solve_flags_parse`。既存のHU・CLI・workspaceテストも維持する。

小さい3席・2BB push/fold（合成HU equity=0.5、3人表なし）のNashConvは、
5反復で0.1176475864 BB、100反復で0.00002064221 BB。
テストは低下に加えて絶対値0.001BB未満を要求する。これは真の6人ゲームの収束実証ではない。

追加のカーネル検証：`cached_range_expectation_matches_physical_combo_brute_force`。
multiwayの範囲期待値cacheは各traversal最大4,096 entriesで、到達ベクトルの正確なbit列をkeyにする。
既存HUの数値経路は変更せず、multiway内の同じ相手reachに対する行列計算を再利用する。

会計検証の範囲：3人全員がactiveで合法なクラス組に集中したall-in終端を検証する。
既にfoldした席の投資は独立reachの固定損失として積分するため、fold済み席と不可能なactive tripleを
同時に含む履歴全体について、共通の厳密なjoint card分布や定和性を証明するものではない。

## 6人100BB・3反復の実測

外部設定の全311,039ノード、HU表は上記samples1/seed20261015、3人表なし（警告あり）。
`--max-iters 3 --max-seconds 480 --metrics-every 3 --target-bb 0 --summary-only`。
CFR計算は逐次、`RAYON_NUM_THREADS=4`。同居ジョブがある環境での単発測定である。

| 指標 | 実測 |
| --- | ---: |
| 反復数 / 終了 | 3 / max_iters、exit0、timeoutなし |
| 学習合計 | 123.422966秒 |
| 1反復平均（学習のみ） | **41.140989秒** |
| 初期レポート | 47.688秒 |
| 最終EV/BRレポート | 193.119675秒 |
| プロセスwall | 366.801675秒 |
| peak RSS | **1,587,167,232 bytes（1.59 GB / 1.48 GiB）** |
| CPU user / system | 273.002164 / 11.881221秒 |
| NashConv 初期 → 3反復 | 74.8518382564 → 4.95654786189 BB |

| 頻度（動作確認用、精度比較不可） | % |
| --- | ---: |
| UTG open | 17.0740 |
| HJ open | 17.9641 |
| CO open | 18.9119 |
| BTN open | 21.5033 |
| SB raise | 18.6172 |
| SB complete | 27.4679 |

全5席のraise/completeが[0,1]、それぞれの和が1以下であること、6席EVが有限、3反復完了をassertした。
**未収束**であり、これらの頻度を本番レンジや7.1節の参考値との精度比較に使わない。
フル3人表付きの6人solve速度・メモリは未測定。表積分のコストはこのfallback測定とは異なる。

測定はPython `resource.getrusage(RUSAGE_CHILDREN)`で単独子プロセスを集計。
この環境では`/usr/bin/time -l`のRSS取得に制約があるため使用していない。
旧測定試行2回は初期レポートが遅く、学習開始前に停止。fold後の固定損失の積分と
範囲期待値cacheを追加した後の上記成功値だけを採用する。停止試行を完走や速度比較に含めない。

- 実測binary SHA-256：`3cff92b42cda98e54bd5941795ed0de8c12f887b7afc21dd78eb82b6fe4d2af4`
- ローカル証跡：`/tmp/p4-six-cached.json`、`/tmp/p4-six-cached.metrics.json`、`.stdout`、`.stderr`
- 重いignored testをさらに反復実行せず、同じ3反復のCLI実行と上記assertで確認した。
  外部100BB goldenも既述の旧／新CLI全値比較を使い、外部設定依存testの重複実行を避けた。

## 最終検証

- `RUST_TEST_THREADS=4 RAYON_NUM_THREADS=4 cargo test --release -p preflop -j 4`：lib42＋CLI5成功、3 ignored。
- `RUST_TEST_THREADS=4 RAYON_NUM_THREADS=4 cargo test --release --workspace -j 4`：**147成功、5 ignored**。
- ignoredのうち既存2件はcards全列挙・既存microbenchmark。追加3件は3way時間計測、外部100BB golden、6人100BB smoke。
  3way時間計測は明示実行済み。後二者の対象は外部CLI比較／計測で検証済み（同じ重い計算は再実行しない）。

最終CLIで`equity-build-3way`と明示的に命名する修正を行った。
workspaceテスト後のClippy指摘は`is_multiple_of`、`while let`、Option分岐の等価な書き換えのみ。
これらの後は影響するpreflopテストとworkspace Clippyを再確認し、無関係なworkspaceテストを重複実行しない。
測定binaryのhashは上記実測時のものとして固定し、このlint／CLI名修正後のbinaryと混同しない。

最終lint修正後：preflop 47件成功、workspace
`cargo clippy --release --workspace -j 4 -- -D warnings`成功（警告0）、
`cargo fmt --all -- --check`・`git diff --check`成功。
CLI `equity-build-3way --samples 1 --seed 7 --threads 4 --out /tmp/p4-cli-three-way.json`も成功し、
stderrの818,805件の進捗と保存を確認した（CLI報告0.602554秒、process実時間1.02秒、保存込み）。
このsample1単発値は固定費が支配的なので、samples1,000の推定には前掲の差分プローブを使う。

最終binary SHA-256：`13797735475c01641bba0f707bbf9017c7add0ee7a5f9a221309995838be7457`。
検証ログは`/tmp/reyson-final-{preflop,workspace,clippy,build,cli}.log`。
commit/push/reset/checkout・追跡ファイル削除は実行していない。

## 追記：本番3人表を読取専用で用いた終端最適化（2026-10-17）

前節までの「表付き未測定」は当時の記録。今回は既存の本番HU表・samples1,000の3人表を
**読取専用**で使用し、設定 `configs/preflop/cash-6max-100bb.json` の全311,039ノードを測定した。
表の再生成・書換え、本番solveの200反復は行っていない。別の10-threadジョブが同居する環境で、
全採用測定を `RAYON_NUM_THREADS=2` に制限した。初期の重複実行を含むfallback測定は除外した。

### 実装と数値境界

- 同じtraversal内の相手range期待値を再利用。HU pair cacheは最大4,096件（約17 MB）、
  3人の相手2席の順序付き生weightの正確なbit列をkeyとするcacheは最大16,384件（約89 MB）。
  量子化・丸め・range正規化によるkeyの同一視はしない。自分のreachをkeyに含めない。
- 3人表のhero順序付きdense view（約39 MB）をsolver内で共有し、毎回のsort/index変換を除去。
  元表の`None`は内部NaN sentinelとし、equityと投資項のlegal massの両方から除外する。
  合法tripleへの再正規化はしない。
- 学習は元のx/y加算・乗除順序を保つdense積分＋exact cache。学習を並列化していない。
- EV/BRレポートだけ、一方の相手rangeを先に積分した169×169行列を再利用する。
  各traversal最大128行列、FIFO（約58.5 MB）。加算の再結合によりbit一致ではなく許容差で比較する。
  各席のEV/BRを独立した最大6タスクとしてRayonで処理し、結果の席順序を維持する。
- cacheの寿命はtraversal内。fold済み席の独立massは外側で適用し、2人HUの数値経路は変更しない。
  近似ゲーム・レーキ・実現率・アクション木は変えていない。

### 実測（単発、秒）

3人表なしは1反復。時間field以外の全report値は再帰比較でf64 bit一致。

| 3人表なし | HEAD隔離build | 最適化binary |
| --- | ---: | ---: |
| 初期レポート | 13.906786 | 9.653209 |
| 学習1反復 | 8.643755 | 8.788842 |
| 最終EV/BRレポート | 13.886379 | 9.796804 |
| process wall | 36.858568 | 28.407022 |
| peak RSS bytes | 1,619,329,024 | 1,622,032,384 |

変更前はHEAD `2892539f955dc85c89543e9b277d5b4c556e8b7e` を`git archive`で
`/tmp/p4-head-baseline.Gzwgc9`に展開し、独立targetにrelease `-j 2`で新規buildしたもの。
source対応未確認だった既存artifactの旧測定は採用除外した。

**変更前の表付き完走時間は未取得。** 30.013058秒で外部timeoutし、初期レポート未完了。
ユーザー報告の「約37分経過しても未出力」とは別の、上限付き再現試行である。変更前の初期・1反復・最終レポートを
全部実測する要件は満たしていない。変更後ソースに保持した元の積分ループoracleの20回probe（積分区間合計1.717268083秒）、
最終profileの初期triple呼出
327,606回を掛けた **約28,128秒（7.81時間）** は旧serialカーネルだけの外挿であり、実測wallや
完走時間ではない。HEAD隔離buildを直接測った推定とも主張せず、反復時間にも転用しない。

次のreferenceは**変更前binaryではない**。同じ最適化binaryのdense lookup・exact cache・
2-threadレポートを使い、`PREFLOP_REFERENCE_INTEGRAL=1`で元の積分順序だけを残した数値比較基準。

| 本番3人表付き・3反復 | 元順序reference | 最終mixed（学習元順序／report contraction） |
| --- | ---: | ---: |
| 初期レポート | 11.486999 | 8.582422 |
| 学習3反復合計 | 41.944737 | 40.243835 |
| 最終EV/BRレポート | 419.165950 | 124.639982 |
| process wall | 472.949756 | 174.127635 |
| peak RSS bytes | 1,838,120,960 | 1,953,677,312 |

最終mixedの学習各反復は13.982763／14.601369／11.659703秒。
初期NashConv69.7547094675 → 3反復4.31197917385 BB。未収束で、頻度の精度向上を示すものではない。
最終reportのprofile集計ではtriple呼出327,606、exact cache hit221,272、contraction hit80,470。
profile秒は`Instant.elapsed`による各traversalのwall区間の合計であり、CPU時間ではない。
並列workerの区間は重なるので、全体wallとして足さない。

| 区間 | wall秒 | triple積分区間合計秒 | HU pair積分区間合計秒 |
| --- | ---: | ---: | ---: |
| 初期report | 8.582 | 1.264 | 0.0145 |
| 学習1反復目 | 13.983 | 5.825 | 0.3673 |
| 最終report | 124.640 | 180.643 | 13.799 |

contraction時間はtriple積分の内数（初期1.220秒、最終175.863秒）。別項として加算しない。
最終reportではcontraction構築が主要な積分コスト。学習ではexact cache後の元順序積分と
木の走査・reach/strategy処理が残り、レポートだけの高速化を学習に換算しない。

### 数値比較・再現

本番表付き6人3反復で、時間を除いた全値の絶対許容差を`1e-10`とした。
初期を含む最大差`9.7486463346e-12`、3反復時点最大差`1.0658141036e-13`、
EV最大差`7.4315553711e-14`、BR最大差`5.6510351953e-14`、summary頻度差0。
これは元順序referenceとの比較であり、Monte Carlo表の真値に対する誤差ではない。

```sh
# 実行済み最終測定。外部入力はread-only。外部wrapper上限は1,200秒。
RAYON_NUM_THREADS=2 RUST_TEST_THREADS=2 PREFLOP_PROFILE_TERMINALS=1 \
  target/release/preflop solve \
  --config configs/preflop/cash-6max-100bb.json \
  --equity /Users/yota/Projects/Products/ReysonGTO/data/equity/hu169.json \
  --equity-3way /Users/yota/Projects/Products/ReysonGTO/data/equity/three_way_169_s1000.json \
  --realization configs/preflop-realization.json \
  --max-iters 3 --max-seconds 1100 --metrics-every 3 --target-bb 0 \
  --summary-only --out /tmp/p4-final-mixed3.json
```

同一コマンドに`PREFLOP_REFERENCE_INTEGRAL=1`を追加して元順序referenceを測定。
`PREFLOP_REFERENCE_TERMINALS=1`は3人cache/denseを使わない旧積分oracle、
`PREFLOP_REFERENCE_LOOKUP=1`はdense lookupを無効化する比較用opt-in。
`PREFLOP_PROFILE_TERMINALS=1`は内訳出力だけで、通常実行には不要。
3人表なし比較は`--equity-3way`を省略、`--max-iters 1 --metrics-every 1`で実行した。

### 200反復の予算（未実測）

2・3反復目平均13.130536秒を固定すると学習200反復は約44分。
仮に50反復ごとのreportを4回、毎回今回の最終reportと同じ約125秒と置けば、初期report込み約52分。
25反復ごと8回なら2 threadsで約17分を追加し、合計約1時間。後半のrange/cache分布やreport時間は未確認なので、運用予算は**1〜2時間程度の仮置き**。
10 threadsにしても逐次学習は10倍／5倍速にならない。reportは最大6タスクなので、2→6タスクの
理想スケーリングだけを置けばreport約6分、合計約50分という楽観値になるが、実測・保証ではない。
200反復の収束、10-threadでの速度・メモリ、同居ジョブの影響は未検証。

### 証跡

- HEAD隔離build binary SHA-256：`9817713a7847ec93f1396ab52b048054cd3e79eeed9b3bd4a3f3583815fae1ff`。
  buildログ`/tmp/p4-head-baseline-build.log`、source/lock hash`/tmp/p4-head-baseline-hashes.txt`。
- reference／表なし最適化測定binary SHA-256：`463aa3bc8934882fba71ddc7bed3b6044499715d3dc67d26aac30224852e184f`。
  最終mixedは学習積分の呼出選択を変更したもの。表なし経路は同じで、最終mixed binaryによる重複再測定はしていない。
- 最終測定binary SHA-256：`91e3ff81be097d5ab5a35aa47df3fa2e3028abca2e893cb46e9d7ee9722de643`。
- 使用3人表の検証済みcontent hash：`fnv1a64:a75fbc4810284f25`。
- wrapper：`/tmp/p4-single-run.py`。測定ごとの`.pid.json`に実行コマンドとPIDを記録。
- source baseline wrapper：`/tmp/p4-source-measure.py`。
  `/tmp/p4-fallback-before-source.*`、`/tmp/p4-fallback-after-final.*`、
  bit比較`/tmp/p4-source-baseline-comparison.json`。
- `/tmp/p4-table-before-source.*`（HEAD隔離buildの表付きtimeout、部分RSSを完走peakとみなさない）。
- `/tmp/p4-production-probe.log`（保持した元の積分ループoracleの20回probe。
  process内probe wallは1.718536958秒、積分区間合計は1.717268083秒）。
- `/tmp/p4-final-{reference3,mixed3}.{json,stderr,stdout,metrics.json,pid.json}`、
  `/tmp/p4-final-comparison.json`、`/tmp/p4-final-breakdown.json`。

最終検証：`RUST_TEST_THREADS=2 RAYON_NUM_THREADS=2 cargo test --release --workspace -j 2`
は**150成功、0失敗、6 ignored**（`/tmp/p4-final-workspace.log`）。
旧HU golden、3人cacheと旧積分の許容差比較、同じ相手range contractionの再利用、ゼロreach・
不可能tripleの検証を含む。重い6人ignoreは上記実測CLI比較で検証し、重複実行していない。
`cargo clippy --release --workspace -j 2 -- -D warnings`は警告0
（`/tmp/p4-final-clippy.log`）。lint後の同順序`enumerate`書換えに対するカーネルtestも1件成功
（`/tmp/p4-postlint-kernel-test.log`）。測定binaryのhashはlint前の測定対象として固定する。
変更した`cfr.rs`・`equity.rs`のtargeted rustfmt checkと`git diff --check`は成功。
既存の無関係なsizing書式は変更せず、今回の`cargo fmt --all -- --check`成功とは主張しない。

`three_way_reference_and_cached_training_within_tolerance`は合成3席3反復で、元の`get`積分oracleに
対するregret・average stateの全f64 bit一致、EV/BRの絶対差`1e-10`未満、Rayon 1対2 threadsの
レポート値一致も確認する。HUは`hu_golden_legacy`・`full_legacy_hu_40bb_bit_exact_golden`を維持。
本番6人のserial対Rayon 2の比較も、時間とcheckpointsを除く最終report全fieldでf64 bit一致
（`/tmp/p4-parallel-comparison.json`）。これはcontraction-all中間版同士の並列化比較であり、
最終mixed対旧積分の許容差比較とは区別する。
