# M2c: カーネル計測と最適化

実行日: 2026-10-15。ブランチ `feat/m2c_kernels`、基点 `bfb6f554acb59afefbe7caf4e98a7a8b47c0e3b4`。
M2 / M2b の記録を先に読み、既存結果は変更していません。
commit / push / reset / checkout / rebase / stash は行っていません。
OSS ポーカーソルバー・ポーカー crate・禁止ディレクトリは参照せず、新規依存は追加しません。

## 固定条件と計測の境界

- Qs Jh 2h、BB 1176 / BTN 690、非対称頻度をそのまま使用。
- 5.5 BB / 97.5 BB、50% pot bet、street-total 2.5x raise、OOP lead、レーキなし、merge なし、65536 units/BB。
- i16、alternating DCFR α=1.5 / β=0 / γ=4。木・頻度・収束パラメータは速度目的で変更しません。
- exploitability は初期ポットに対する %、戦略差は pp。worker 秒と wall 秒を区別します。
- 計算は逐次実行。ホスト状態・熱・他プロセスは制御できず、単回測定を普遍的な高速化とは扱いません。
- 外部参照はコピー済み `data/benchmark` の出力だけ。参照の solve 755.815893 秒 / 160 反復 / 0.2726748586% / RSS 8,966,111,232 bytes。

## 先行診断

### 変更前の保全と30反復

`cargo build --release` 後、M2b と同じ SHA256
`a1f6140078d03a507ff4c885cdec4b609de0f38f6f2f05f87b9fb0f4bf062d8a`
のバイナリを `data/m2c/baseline-cli` に保全しました。
ソースは `baseline-source.json`、実行は `baseline30/`。
10 threads、30 反復、exact metrics every 30（初期・終了も実行）。

| 指標 | 変更前 |
| --- | ---: |
| training / 平均1反復 秒 | 248.754590 / 8.291820 |
| 反復 p50 / min / max 秒 | 8.698354 / 4.832927 / 12.868060 |
| solve / process wall 秒 | 271.062766 / 272.796117 |
| metrics 秒 | 22.305647 |
| exploitability % | 7.004061638620 |
| 最大 RSS bytes | 9,704,751,104 |

### sample の制約

フルフロップ30反復の途中に macOS `sample` を試行しました。
名前指定は process list の sysctl が拒否され、telemetry から得た正確な PID 84031 を指定した
`/usr/bin/sample 84031 60 10 -mayDie -fullPaths -file ...` も
`do not have appropriate privileges to examine it` で失敗しました。
sudo / 権限変更 / 保護の迂回は行っていません。
**sample のスタック・行別ホットスポットは未取得**であり、取得できたと報告しません。
エラー原文: `baseline-sample-command.log` と `baseline-sample-pid-command.log`。
代替として以下の自前区間計時・正確な仕事量・タスク終了時刻・アセンブリを使用します。

### アセンブリ（変更前）

`xcrun llvm-objdump --disassemble --demangle --line-numbers --source data/m2c/baseline-cli`
を `baseline-asm.txt` に保存。release debug=1 は関数シンボルのみで、機械命令とソース行の
厳密な対応はありません。以下の対応は保全した HEAD のソース構造と命令からの判定です。
詳細: `data/m2c/baseline-asm-analysis.md`。

| ループ | 判定 / 命令の証拠 |
| --- | --- |
| i16 DCFR update | 既にSIMD。0x1000950f0–5278: scvtf.4s / fmul.4s / fadd.2d / fmaxnm.4s、8コンボ/batch |
| 通常のi16 encode | 既にSIMD。0x1000955d4–5674: fmul.4s / frinta.4s / fcvtzs.4s / sqxtn、scalar tailも残る |
| 2/3-action normalize_fixed | scalar f64 divide/multiply。0x100090854 の fdiv d2,d0,d2、comboごとの分岐・bounds check |
| terminal totals | scalar reduction/scatter。0x100093088–30cc: fadd と2カードのindexed load/store |
| 判断child reach | scalar multiply/copy。0x1000945b8–4704、冗長bounds check |
| chance reach | scalar mask/multiply。0x10008fddc–fe78 |

「全ループが未ベクトル化」が原因ではありません。ベクトルレジスタによるゼロ初期化や
board-mask OR を、終端の浮動小数点演算のSIMD化と混同しません。



### 正確な仕事量と ns/演算

`--kernel-diagnostics` は opt-in。起動時に物理木から仕事量を数え、訓練2走査と
平均EV/BR各2走査のタイマーを分離します。既存profileのように metrics 分を訓練の分母に混ぜません。
タスクローカルの計時をjoinで集め、ホットループの共有atomic/lockは追加しません。
計時を無効にした通常solveと診断solveの速度は直接比較しません。

1反復 = OOP更新 + IP更新、以下は両走査の合計です。

| 仕事 | ノード訪問 | denseコンボ | board-activeコンボ |
| --- | ---: | ---: | ---: |
| 判断 | 1,871,040 | 1,745,680,320 | 1,604,968,992 |
| 判断action-combo | — | 4,233,088,176 | 3,891,923,664 |
| i16更新action-combo | — | 2,116,544,088 | 1,945,961,832 |
| fold出力 | 1,333,016 | 1,243,703,928 | 1,143,477,336 |
| showdown出力 | 1,688,736 | 1,575,590,688 | 1,448,310,828 |
| 終端opponent totals | — | 2,819,294,616 | 2,591,788,164 |
| showdown prefix | — | 1,575,590,688 | — |
| chance reach | 7,568 | 677,891,676 | 623,210,412 |
| chance加算 | — | 338,945,838 | 311,605,206 |

物理判断935,520、fold666,508、showdown844,368、chance3,784。
chance子は1反復363,286（物理1走査181,643）、同型削減なし。
board-activeは盤面非衝突であり、到達確率>0とは異なります。判断のblocked比率は約8.06%。
したがって「blockedだけを除けば2.5倍速」という仕事量上の根拠はありません。

10 threads / 5反復の訓練のみ:

| 区間 | worker elapsed秒合計 | ns/コンボ作業 | 分母（各反復×5） |
| --- | ---: | ---: | --- |
| 判断・正規化・更新等 | 145.851933 | 6.891 | action-combo |
| 判断reach/copy transport | 95.544601 | 4.514 | target value-action-combo |
| showdown | 134.879584 | 17.121 | dense出力combo |
| fold | 63.809424 | 10.261 | dense出力combo |
| chance reach/加算 | 9.777440 | 1.923 | reach+加算combo |

worker elapsedはdescheduleや待ちも含み得る区間経過時間の和で、CPU cyclesやwall割合ではありません。
1コンボは1命令ではありません。算術のみの**下限仕事量proxy**を
fold = `3*fold_dense + 4*fold_active`、showdown =
`3*showdown_dense + 3*prefix + 9*showdown_dense + 4*showdown_active`
と置くと、10 threadsは **fold 1.537 / showdown 0.917 ns/算術op**。
max、比較、変換、load/store、分岐、計時を分母に含めないため、純粋なFLOP latencyではありません。

理想的な2-lane f64を毎cycle1命令処理すると仮定した算術下限は
`1/(2*f_GHz) ns/op`（仮に4 GHzなら0.125 ns/op）。上記値はその仮定の約12.3倍/7.3倍ですが、
**4 GHzも毎cycle1命令も実機測定値ではなく、M5の達成可能rooflineではありません**。
特にtotalsは同じsumへの依存とカード別scatter、fold/showdownはgatherとbounds checkを持ち、
純算術の下限から遠くなる構造がアセンブリで確認できます。
PMU/cyclesと実効帯域を取得できていないため、計算律速・メモリレイテンシ・OS圧縮の寄与を秒単位で断定しません。

### 4 / 6 / 8 / 10 threads と最後の仕事待ち

各5反復を順番に実行。全てexploitability **180.4006168448066%**、仕事量一致。

| threads | training秒/反復 | p50 / min / max秒 | RSS GB | 最後のtask待ち合計 / 最大秒 | finish-idle proxy % |
| ---: | ---: | --- | ---: | --- | ---: |
| 4 | 10.838 | 10.729 / 9.809 / 12.282 | 9.681 | 2.707 / 0.073 | 4.329 |
| 6 | 11.811 | 11.752 / 11.394 / 12.198 | 9.688 | 5.027 / 0.133 | 7.881 |
| 8 | 10.205 | 10.275 / 9.575 / 10.979 | 9.687 | 4.985 / 0.125 | 9.972 |
| 10 | 9.599 | 9.954 / 8.613 / 10.259 | 9.684 | 0.395 / 0.024 | 4.361 |

最後のtask待ち = 最終task終了−最後から2番目の終了、110 chance batchesの集計。
finish-idle proxy = 各参加workerの最終終了から最終task終了までの差の総和 /
`(threads * batch wall総和)`。開始・途中・不参加workerのidleは含めず、完全なCPU使用率ではありません。
10 threadsはtask1個の最大0.319秒、最終反復の各worker累積作業時間9.37–9.94秒。
Rayon worker IDはP/EコアIDではなく、OSのコア割当は未観測です。
**遅いEコアに最後の仕事が残ったと断定できず、10 threadsの最終tailだけでは2.5倍差を説明しません**。
今回の観測最速10 / grain1を維持します。単回かつ短い測定で6→8などの順序にもホスト状態が混入します。

### 同期とホスト状態

コード上、regret/average slabは`split_at_mut`で別領域を渡し、各chance childは独立scratchを所有。
集約はjoin後に親が行い、solverのホットな算術ループにMutexや共有atomicの加算はありません。
Rayon schedulerの同期は存在しますが、外部Rayonソースは読まず、プロファイル未取得なので
lock contentionの実測比率は不明です。隣接metadataや分割境界がcache lineを共有する可能性はあり、
**false sharingがゼロと証明したわけではありません**。worker-localな診断設計と小さな最終tailから、
根拠なく共有lock/paddingを変更するより計測で確認したscalar/コピーを優先しました。

4/6/8/10のCompressions増分は2,177,888 / 1,265,959 / 1,909,027 / 2,568,454 pages、
Decompressionsは1,878,544 / 988,813 / 1,627,110 / 2,269,336 pages。
Swapouts増分は全て0ですが、Pageouts/Swapinsは増加しています。
これらはホスト全体の累積処理回数で、solver専用でも常駐容量でもありません。
帯域の平均値が小さいだけでメモリ要因を排除しません。
生データ・集計再現: `threads{4,6,8,10}/`、`diagnostic-analysis.{json,md}`、
`python3 tools/m2c_summarize.py --help`。

## 候補を分けた実装・計測

1. **portable loop rewrite:** actor reachをzip、nonactor reachを`copy_from_slice`、
   2/3-action正規化を互いに重ならないsliceで処理。新アセンブリでdecision reachは
   `fcvtl` + `fmul.2d`（stage1 0x1000977a8–7e4）になりました。
   ただしこの段階の正規化はscalar fdiv、chance reachもunrolled scalarのままです。
2. **NEON正規化:** aarch64限定で2-lane f64の加算・除算・乗算を使用し、元の丸め位置・順序を維持。
   i16は境界内の2要素を安全に読み、f32の2要素load/storeも偶数pair内に限定、奇数tailはscalar。
   x86等にはportable実装を残します。fast-math/FMA/近似逆数は導入しません。
   stage2の2/3-action正規化で`fadd.2d/fdiv.2d/fmul.2d`を実際に確認しました
   （0x100093298–32e0、0x1000934b8–34e8）。
3. **board packing:** showdownの順位配列をboard-activeだけに詰め、同順位・dense IDの順序を維持。
   cached blocked IDの出力だけを0に戻すため、以前のscratch値は残りません。
   fold、regret/average slab、public dense IDsは変更しません。エンジン全配列のpackingではありません。

### 正規化単体の小さい実測

`cargo test --release -p cfr normalization_microbench -- --ignored --nocapture`、
各10,000回、hot-cache合成入力。ns/actor-combo（action-comboではありません）:

| i16 combos / actions | 元scalar oracle | portable zip | NEON |
| --- | ---: | ---: | ---: |
| 1176 / 2 | 1.749 | 1.130 | 0.932 |
| 1176 / 3 | 1.295 | 0.955 | 1.017 |
| 690 / 2 | 0.783 | 0.642 | 0.663 |
| 690 / 3 | 1.043 | 0.818 | 0.945 |

checksum一致、別途bitwise parity testも実施。NEONは旧scalarより短い一方、zipより速いのは
この測定では1176/2だけでした。小さい逐次単回測定で、周波数・warmup・入力分布の影響を含みます。
NEONが全条件で優れるとはしません。全木の測定を併用して採否を決めます。
生ログ: `normalization-microbench.log`、`stage{1,2}-asm.txt`。

### 大きなホスト変動を対照実行で確認

stage2（同一保存バイナリ、同一10反復）をpacking測定の前後で再実行したところ、
trainingは **48.483 → 100.620秒** と約2.08倍に変化しました。
packingの100.758秒だけをstage2初回と比べて「packingで2倍遅くなった」とは解釈しません。
一方、packingがフル木で速くなった証拠もこの対照からは得ていません。

stage2初回 / packing / stage2再実行のsystem CPUは30.720 / 124.588 / 117.161秒、
RSSは9.683 / 8.825 / 9.584 GB。初回からpackingへのホストCompressions増分は
3.684M → 14.063M pages、Decompressionsは3.311M → 13.498M pagesへ増えました。
OS圧縮の負荷は無視できませんが、他プロセスも含むため単独原因とは断定しません。
レンジや精度を変えたり、都合のよい測定だけを採用したりしていません。

### 到達確率0の計算を安全な範囲で省略

stage3bはopponent totals / showdown prefixで `w == 0.0` の加算・カードscatterを省略します。
blockedだけでなく戦略により0になったcomboも対象です。0以外の加算順序は変更しません。
自分のreachが0でもcounterfactual valueは必要なので、target-zeroを理由に判断木を切りません。
Trainの後悔値discount・平均戦略更新も省略せず、平均EV・最終BRは正確な全評価を維持します。
一般の部分木pruningは今回追加しません（安全なTrain更新を別走査する必要があるため）。

同一ターン100反復のopt-in診断では、totalsの1,238,694,800論理combo訪問中
1,054,580,712（約85.14%）、packed prefixの638,268,400訪問中539,460,835（約84.52%）が0でした。
これはターンの測定であり、フロップや全ての収束段階の割合ではありません。
通常計測のtrainingはpacking単体1.970457秒 → zero skip込み1.721504秒（約12.6%減）。
診断を有効にした別実行は1.926809秒で、通常実行との速度比較には使いません。
全て100反復のexploitability **0.31142989066672644%** は一致。
生データ: `turn-stage3b{,-diagnostic}/`。

### 同じ入力・反復数での候補比較

各solveは保存バイナリを使用し、診断off、exact metricsは初期と最終。

| 条件 | 反復 | training秒 | solve秒 | 最大RSS GB | exploitability % |
| --- | ---: | ---: | ---: | ---: | ---: |
| turn-before | 100 | 2.339314 | 2.425229 | 0.375 | 0.311429891 |
| turn-stage1 | 100 | 2.025272 | 2.099557 | 0.375 | 0.311429891 |
| turn-stage2 | 100 | 2.021899 | 2.094563 | 0.375 | 0.311429891 |
| turn-stage3a | 100 | 1.970457 | 2.040511 | 0.375 | 0.311429891 |
| turn-stage3b | 100 | 1.721504 | 1.906315 | 0.375 | 0.311429891 |
| flop-before | 10 | 64.193195 | 87.508011 | 9.692 | 68.266512420 |
| flop-stage1 | 10 | 84.372024 | 109.880489 | 9.691 | 68.266512420 |
| flop-stage2 | 10 | 48.483263 | 63.752039 | 9.683 | 68.266512420 |
| flop-stage2-repeat | 10 | 100.619889 | 128.743032 | 9.584 | 68.266512420 |
| flop-stage3a | 10 | 100.758198 | 126.315142 | 8.825 | 68.266512420 |
| flop-stage3b | 10 | 38.637665 | 53.592579 | 9.683 | 68.266512420 |

全候補で、それぞれ変更前と最終metricsが保存値で一致（exploitability差0 percentage points）。
エクスポートしたroot / after-checkの全combo/action確率も最大絶対差0。
これだけで内部の全ノード戦略まで比較したとは主張せず、内部の正しさは別途回帰テストで確認します。
数値照合は `data/m2c/candidate-comparison.json`。

採否: portable単体はフル木で改善を示せず、単独の速度改善としては不採用。
最終はportable + NEON + packed showdown + exact zero skipの**組合せ**を採用し、
全体での効果を以下の30反復と本計測で評価しました。各段階の短縮率は加算・乗算しません。
packing単体のフル木速度向上は未証明で、確認できた効果はループ仕事量削減とターン約2.5%短縮です。
「NEONが常にzipより速い」「メモリ帯域が原因でない」「Eコアが遅延原因」はいずれも結論にしません。

## 検証と識別

- `cargo test`: **56 passed / 0 failed / 2 ignored**。ignoredは既存のcards全組合せ検査と
  新設のopt-in microbenchで、後者は別コマンドで実行済みです。
- 通常の32反復+metricsでallocation 0の厳格テストを維持・成功。
  opt-in診断はレポート用Vecを確保するため、このallocation-free保証の対象外です。
- `cargo clippy --all-targets -- -D warnings`: 成功。
  初回指摘のNEON出力index loopをenumerateへ、借用分離のためのtraverse引数数には理由付き局所属性、
  診断test moduleを末尾へ移動。全体の警告抑制やテスト許容誤差緩和はしません。
- `cargo build --release`、`cargo fmt --all -- --check`、Rust変更の`git diff --check`: 成功。
- NEON / portable / 元scalarのbitwise比較：690/1176/1326、短い配列・奇数tail、blocked、
  i16極値・0・f32微小値/最大値を含む。enumerate変更後のrelease比較も成功。
- packing：1128対690のdense入力、将来カードblocker、全blocked hero、poisoned scratch、
  両target、fold/showdown/rakeをlegacy oracleとbit比較。zero skip：signed zero / all zero /
  sparse reachの全出力とskip数を比較。
- read-only reviewでsolver/unsafeのblocking指摘なし。診断のodd blocked tail / 非aarch64の
  evaluated-combo数を修正し、focused回帰テスト成功。この修正前の固定aarch64偶数長計測には影響なし。
- インストール済みtargetはaarch64-apple-darwinのみ。
  **x86のクロスビルド・実行、sanitizerは未実施**。portable fallbackの算術はARM上で直接検査しています。
- Python runnerはfake childで成功・非0終了・hard timeout/回収・既存出力拒否を検査。
  exact PIDの`wait4`でsample/telemetryのCPU/RSSを除外し、solver最大RSSを取得します。
  OS telemetryは別スレッド、solveは一度に1プロセスです。

rustc 1.98.0 / cargo 1.98.0、release debug=1、外部依存追加なし、Cargo.lock変更なし。
最終ソース・バイナリ対応を `data/m2c/final-source.json` で保存し、全ソースハッシュを現在ファイルと照合済み。
最終バイナリは `data/m2c/final-cli`、SHA256:
`15410fe2df4ff9113e3e8b8021b2e4bb8db6a0bfad8b3c2ff3934ac05339b491`。
実行時manifestは起動時working treeの記録であって、保全バイナリとのbuild対応保証ではないため、
このbuild時記録と区別します。
ログ: `final-{test,clippy,clippy-fixed,neon-release-test,fmt,build}.log`、
各stageのfocused test、`review.md`、`installed-targets.log`、`probe-tests/summary.json`。

## 最終バイナリの30反復 A/B

同じ固定フロップ・30反復・10 threads・metrics every30、診断off。
最終バイナリは本計測と同一です。元の保全バイナリと最終版で
**metrics全項目とroot/after-checkのエクスポート確率が保存値で一致**。
exploitability差0 percentage points、出力確率の最大絶対差0です。

| 指標 | M2b保全baseline | M2c最終 |
| --- | ---: | ---: |
| 反復 | 30 | 30 |
| exploitability % | 7.004061638620 | 7.004061638620 |
| training秒 | 248.754590 | 209.045398 |
| training秒/反復 | 8.291820 | 6.968180 |
| p50 / min / max秒 | 8.698354 / 4.832927 / 12.868060 | 6.949769 / 3.245063 / 13.407923 |
| solve秒 | 271.062766 | 228.544773 |
| metrics秒 | 22.305647 | 19.498897 |
| process wall秒 | 272.796117 | 230.342981 |
| 最大RSS bytes | 9,704,751,104 | 9,693,167,616 |
| user / system CPU秒 | 2012.581 / 139.865 | 1249.179 / 230.648 |

訓練時間は約15.96%、solveは約15.68%短縮。
**この30反復の平均では6秒以下を未達**。短いstage3bの3.864秒/反復だけを最終速度として採用しません。
最終版でもsystem CPUと反復のばらつきが大きく、ホスト変動込みの観測比較です。
0.3%への収束時間ではなく、同じ途中精度でのthroughput比較に限ります。
生データ: `baseline30/00-QsJh2h/`、`final30/`、`final30/comparison.json`。

## 本計測の再現

新しい出力ディレクトリを使用します。診断off / profiling on / grain1。
0.3%で早期停止、soft2250秒と独立watchdog2390秒を併用し、初期・停止時の厳密metricsを省略しません。

```sh
export CARGO_HOME=/tmp/reyson-m1-audit-cargo
cargo build --release
python3 tools/m2c_probe.py --binary target/release/cli \
  --max-iters 1000 --max-seconds 2250 --timeout-seconds 2390 \
  --adaptive-metrics --out data/m2c/reproduce
```

runnerの固定条件は本稿冒頭の通り、adaptive early25 / near5 / near multiplier2、目標0.3%。
実際の本計測は保存済み `data/m2c/final-cli` を使用し、`main/command.json` とmanifestに全引数・ハッシュを保存。
生データ・レンジ・保存バイナリはGit管理外の `data/m2c/`。ツールと本報告は追跡対象です。
`sample`を再試行できる環境ではrunnerに `--sample-seconds 60 --sample-delay 25` を付けられますが、
今回の実測は権限拒否済みのためこのオプションを付けていません。

## 最終フルフロップ結果

**200反復で0.2970240654131527%、target停止。0.3%以下に到達しました。**
process wall **1684.129311秒（28分4.1秒）**、exit0 / `timed_out=false`。
40分枠は達成、**15分以内と平均6秒/反復は未達**です。短時間候補の3.864秒を本計測結果の代用にしません。

| 指標 | コピー済み参照 | M2b本計測 | M2c本計測 |
| --- | ---: | ---: | ---: |
| 反復 | 160 | 191 | 200 |
| exploitability % | 0.272674859 | 0.313625554 | 0.297024065 |
| solve秒 | 755.815893 | 2276.081613 | 1682.590541 |
| process wall秒 | 762.170000 | 2277.476243 | 1684.129311 |
| 最大RSS bytes | 8,966,111,232 | 9,685,188,608 | 9,682,468,864 |
| 0.3%以下 | 達成 | 未達 | 達成 |

M2cも参照の精度0.272675%には達しておらず、solveは参照の約2.23倍。
**参照より速い／同等精度で優れるとは結論しません**。
旧M2bは上限内に0.3%未到達なので、M2bの「0.3%までの時間短縮率」は計算できません。

setup 0.455054秒、training 1425.681315秒、metrics 256.878747秒
（うちBR wall 123.759140秒）。訓練平均 **7.128407秒/反復**、
p50 6.477638 / min 5.352880 / max 14.249494秒。
user / system CPU 9989.924086 / 1436.200793秒。
保守的memory estimate 12,894,854,263 bytes、persistent regret+average 8,466,176,352 bytes。
RSS・estimate・保存配列容量は別指標です。ノード2,450,180、同型削減なし、非対称頻度不変。
getrusage major faults / swapsは0 / 0ですが、下記ホスト圧縮・swapの不存在を意味しません。

### 本計測の厳密チェック点

| 反復 | solve経過秒 | exploitability % |
| ---: | ---: | ---: |
| 25 | 203.616 | 11.287620574 |
| 50 | 369.448 | 2.539755677 |
| 75 | 574.558 | 1.270646623 |
| 100 | 739.839 | 0.780335340 |
| 125 | 890.729 | 0.546687985 |
| 130 | 940.134 | 0.517573754 |
| 135 | 984.628 | 0.491973505 |
| 140 | 1030.370 | 0.467375923 |
| 145 | 1079.114 | 0.445638396 |
| 150 | 1126.918 | 0.425803379 |
| 155 | 1173.424 | 0.407257489 |
| 160 | 1220.486 | 0.390563315 |
| 165 | 1282.882 | 0.374793903 |
| 170 | 1334.501 | 0.360438640 |
| 175 | 1376.132 | 0.348783229 |
| 180 | 1429.087 | 0.336830372 |
| 185 | 1477.179 | 0.326138344 |
| 190 | 1536.234 | 0.315560256 |
| 195 | 1601.916 | 0.306773409 |
| 200 | 1682.590 | 0.297024065 |

M2bとの共通チェック点では全てexploitability差0。観測した共通チェック点の精度経路は一致しています。
同じadaptive25/5の代表点（異なる日時・ホスト状態の観測比較）:

| 反復 | M2b経過秒 | M2c経過秒 | 観測短縮率 |
| ---: | ---: | ---: | ---: |
| 50 | 547.506 | 369.448 | 32.5% |
| 100 | 1046.223 | 739.839 | 29.3% |
| 150 | 1650.679 | 1126.918 | 31.7% |
| 190 | 2244.381 | 1536.234 | 31.6% |

### 参照との戦略差

比較CLIは木・アクション・コンボ集合・root reach整合性を検証して成功。
参照CSVのcompatible-joint reachを両戦略の共通重みに使い、クラスも同じ重みから再集計します。
差はpp、頻度は%。Bet(180224)=2.75 BB。

| ノード | 単位 | action | 加重平均絶対差 pp | 最大差 pp | 参照頻度 % | 最終共通重み頻度 % | 全体頻度差 pp |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| root_oop_bb | combo | Check | 1.287356 | 29.812008 | 90.433163 | 90.756034 | 0.322871 |
| root_oop_bb | combo | Bet(180224) | 1.287356 | 29.812009 | 9.566837 | 9.243966 | 0.322871 |
| root_oop_bb | class | Check | 0.900914 | 22.497831 | 90.433163 | 90.756034 | 0.322871 |
| root_oop_bb | class | Bet(180224) | 0.900914 | 22.497834 | 9.566837 | 9.243966 | 0.322871 |
| after_check_ip_btn | combo | Check | 2.020948 | 18.750644 | 55.948800 | 56.868165 | 0.919365 |
| after_check_ip_btn | combo | Bet(180224) | 2.020948 | 18.750639 | 44.051200 | 43.131835 | 0.919365 |
| after_check_ip_btn | class | Check | 1.391969 | 9.418832 | 55.948800 | 56.868165 | 0.919365 |
| after_check_ip_btn | class | Bet(180224) | 1.391969 | 9.418833 | 44.051200 | 43.131835 | 0.919365 |

自身のreachで重み付けしたBet頻度はOOP9.243966%、IP43.215887%。
IPの共通参照重み43.131835%とは区別します。root combo最大差は約29.812ppあり、
全体頻度が近いことを全戦略一致とは扱いません。
M2bのroot combo最大差29.2566ppより悪化する一方、IP最大差19.7120→18.7506ppは改善。
全ての局所差が改善したとは主張しません。
生データ: `main-compare.json`、再現:

```sh
data/m2c/final-cli compare --solution data/m2c/main/solution.json \
  --reference-dir data/benchmark --out data/m2c/new-compare.json
```

### 最終profile・速度変動・OS

通常profileは訓練とmetricsの両方を含むworker elapsed総和。
BR wallは別区分で重複し、下表に加算しません。decision transportはこの通常profileでは別集計されません。

| 部分 | worker秒 | 4区分の合計比 |
| --- | ---: | ---: |
| 判断・正規化・更新 | 7890.397 | 55.87% |
| showdown | 3782.203 | 26.78% |
| fold | 2020.201 | 14.30% |
| chance | 430.163 | 3.05% |

| 訓練反復窓 | 平均秒 | p50秒 | 最大秒 |
| --- | ---: | ---: | ---: |
| 1–25 | 7.315 | 6.673 | 10.179 |
| 100–129 | 5.861 | 5.565 | 8.904 |
| 130–159 | 6.982 | 6.992 | 9.276 |
| 160–200 | 8.778 | 8.797 | 14.249 |

ホストvm_stat（16,384 bytes/page、累積counter）:

| 指標 | 開始 | 終了後 | 差分 |
| --- | ---: | ---: | ---: |
| Compressions | 707,260,913 | 810,033,519 | +102,772,606 |
| Decompressions | 687,665,336 | 789,662,716 | +101,997,380 |
| Pageouts | 615,580 | 621,267 | +5,687 |
| Swapins | 28,114,724 | 28,116,375 | +1,651 |
| Swapouts | 29,070,789 | 29,070,789 | +0 |
| Pages stored in compressor | 1,794,521 | 1,879,129 | +84,608 |
| Pages occupied by compressor | 375,438 | 416,368 | +40,930 |

ホスト全体の値であり、同じページの再圧縮を含みます。solver専用の容量・I/O量への換算はしません。
raw swap usage / ps / thermalの成功・失敗も `main/telemetry.jsonl` に保存。
熱警告がないことは温度やthrottling不存在の証明ではありません。

## 最終判定・未解決

- **完了:** 計測後のカーネル実装、同反復での数値一致、通常テスト・release build・clippy、
  固定Qs Jh 2hの0.3%到達、40分枠、最大RSSと参照戦略差の記録。
- **未達:** 本計測の平均6秒/反復、15分以内、参照と同精度以上での速度優位。
- **環境制約:** sampleは権限拒否でスタック／行別サンプル未取得。代替の区間計時・仕事量・asmは取得。
  P/Eコア割当、PMU、lock contention/false-sharing実測、x86実機は未検証。
- **実装範囲:** packed arrayはshowdown順位配列。全slab/foldのpackingや一般のzero-reach subtree pruningは未実装。
  安全性を優先し、厳密なBRとTrain discount/平均更新を省略して速度を稼いでいません。
- **今後の課題:** ホストメモリ状態を制御したA/B、scalar terminal gather、依然大きいdense slab更新。
  これらの改善率は未測定です。今回の数値から保証しません。

最終成果物: `main/{manifest,command,resource,solution}.json`、`main/progress.jsonl`、
`main/telemetry.jsonl`、`main-summary.json`、`main-compare.json`。
全ての長い計算は1つずつ実行。コミット・push・ブランチ操作は行っていません。
