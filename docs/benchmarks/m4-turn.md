# M4 ターン比較: 合成レンジの動作・時間確認

2026-10-17。**製品レンジの計測でも、サイズの最適性の証明でもない。**
外部 OSS ソルバーのコード・製品レンジ・既存計測は参照/変更していない。
以下の合成計測時点では既定の flop rule と data/m4b/manifest.json はこの checkout になく、本番比較は未実施だった。

## 再現用の合成入力とコマンド

```sh
mkdir -p data/m4-turn-synthetic-input
printf '%s\n' '{"player":"synthetic-oop","combos":{"AcAd":1,"KcKd":1}}' > data/m4-turn-synthetic-input/oop.json
printf '%s\n' '{"player":"synthetic-ip","combos":{"QcQd":1,"JcJd":1}}' > data/m4-turn-synthetic-input/ip.json
printf '%s\n' '{"version":1,"candidates":[0.33,0.66,1.0,1.25],"default_fraction":0.33,"rules":[]}' > data/m4-turn-synthetic-input/rules.json
python3 tools/m4_turn.py --flops 7c6d5d --lines XX,BC \
  --flop-rule data/m4-turn-synthetic-input/rules.json \
  --oop-range data/m4-turn-synthetic-input/oop.json \
  --ip-range data/m4-turn-synthetic-input/ip.json \
  --pot 2 --stack 3 --threads 1 --max-iters 1000 --max-hours 0.1 \
  --out data/m4-turn-synthetic-final --report data/m4-turn-synthetic-final/report.md
```

空の rule table はこの合成テストだけの明示入力。本番の欠損規則の代替ではない。
フロップから1回解き、2ライン×5種類のカードへ10 snapshot を出力。
この flop では FlushDraw 種類は空（既にツートーン）であり、存在しない第6種類を補完しない。
OOP/IP を別にして20状況、粗い80 solve、細かい40 solve が完了した。
全候補が目標精度を達成。flop 0.756933%、subgame 最大0.440689%。

## 実測（逐次、1 thread）

|段階|solve数|平均 wall秒|平均 solver秒|最大 RSS bytes|
|---|---:|---:|---:|---:|
|coarse|80|0.106089|0.002309|4358144|
|fine|40|0.106128|0.002508|4325376|
|flop|1|0.314941|0.219771|45072384|

機械的な外挿: `4800 × (4 × coarse + 2 × fine) + 200 × flop` = 約 3118.7秒（52.0分）。
**これは本番所要時間ではない。** 2コンボ/側、2BB pot・3BB stack の極小木であり、
実レンジ・97.5BB stack の分岐/メモリ/収束/セットアップは反映していない。
wall は100ms周期の watchdog 待ちが支配的。実環境の見積もりには、供給された実規則と実レンジで代表状況の追加計測が必要。
計算上の4800状況は全6種類が存在するとした上限目安。実際の空 bucket は飛ばす。

## 種類別の観測勝者（細かく解いた2候補内だけ）

|種類 / bettor|サイズ%: 件数|
|---|---|
|Blank/p0|{"33": 2}|
|Blank/p1|{"33": 1, "66": 1}|
|BoardPair/p0|{"33": 2}|
|BoardPair/p1|{"33": 1, "66": 1}|
|FlushComplete/p0|{"33": 2}|
|FlushComplete/p1|{"125": 1, "33": 1}|
|Overcard/p0|{"33": 2}|
|Overcard/p1|{"33": 2}|
|StraightComplete/p0|{"33": 2}|
|StraightComplete/p1|{"125": 1, "33": 1}|

OOP=0、IP=1 は分離。EV差0〜0.066112BB、20状況中10状況は合算 proxy 以下で「区別できない」。
これは raked general-sum の診断であって、均衡EVの誤差保証ではない。
粗い段階で落ちた2サイズは細かく解いていない。小スタックでは候補が同一all-inへmergeされ得る。
詳しい全EV/誤差/時間/メモリは local report と summary.json、入力/ソース/binary hash は manifest.json。

## 動作確認

- `python3 -m unittest discover -s tools -p test_m4_turn.py -v`: 6 tests pass。
- 実 CLI integration: 20/20状況、失敗0。上記のsource状態に対応するmanifest identity:
  `cca32da04d52f4bfbf2d8db3baf61f7e2451cb4ff5a5ef4d6ae53ab655880d2c`。

- 最初の同一入力試行 `data/m4-turn-synthetic` を同じコマンドで再開し、122 attempt のまま増加なし（flop1 + card selection1 + subgame120）。
- 未達 candidate は metrics を残して target_not_met とし、再開時は新しい attempt で再試行する。
- 計測後、reportへの種類別集計のみ追加して保存済み結果から再生成した。source hash が変わった場合は既存outへの再開を拒否するため、再計測は新しいoutを使う。
- pytest はインストールされていないため、依存追加なしの unittest を使用。


## 2026-10-18: ターン規則の供給済み計測結果

ここからは上記の合成レンジ試験とは別の、ユーザー供給の計測結果。
本変更で solve を再実行した結果でも、実装の再現性・収束精度を新たに検証した結果でもない。
現行の規則設定は `configs/sizing/srp-btn-vs-bb.json` を参照。

対象は20フロップ・324状況、候補サイズは33 / 66 / 100 / 125% pot。
供給報告では全候補を精度0.1%で解いている。
以下の損失は **ターン開始時 pot に対する割合（%）** であり、上記の合成試験の
EV差（BB）や solver の収束精度とは異なる指標。

|選択方法|平均損失|最大損失|
|---|---:|---:|
|現行ターン規則|0.081%|1.034%|
|フロップ単位 leave-one-out CV|0.093%|1.034%|
|全状況66%固定|0.188%|—|
|bettor × 直前フローの4行規則|0.114%|1.336%|

- 平均損失 `< 0.1%` の目標は達成。一方、最大損失 `< 0.5%` の目標は未達。
- 現行規則で324状況中12状況が0.5%を超え、主にIPのベット状況に集中する（供給報告）。
- CV は同じ20フロップ内での評価であり、独立した holdout の結果ではない。

### 範囲と検証上の限界

- 直前ストリートの履歴として計測したのは `X,X` と `X,B,C`（OOPチェック→IPベット→OOPコール、レイズなし）のみ。先打ち→コールやレイズ→コールは計測対象外。
- 過去の計測は片側のサイズ候補を変え、相手側を66%固定した条件。両側を非66%にした組み合わせは未検証。
- 新しい `previous_street_line` の `CheckedThrough` / `IpBetCalled` は上記の計測済みフローに対応する。`LeadCalled` / `Raised` / 不明な履歴は `default_fraction`（0.66）へフォールバックする。
- 両側のサイズを扱う再計測は `--other-side-rules configs/sizing/srp-btn-vs-bb.json` を使う計画（オプションを追加済み）。この追記時点で再計測・solveはしておらず、既存の履歴値を新結果として扱わない。オプションなしの従来動作は維持する。
- フロップは20枚のみで、独立 holdout は未評価。
- 各ターン種類につき代表カード1枚のみ。全ターンカードを網羅していない。
- フロップのサイズは規則から選択し、リバーは66%固定。この条件外への一般化は未確認。
- 読み取り専用の供給データ `turn_losses_20flops.json` について、324行・20フロップ、
  各行の4候補、および66%固定の平均損失0.1875787696%（丸めて0.188%）だけを軽量集計で確認した。
  データには規則の選択結果やCVのfold結果が含まれないため、規則/CVの集計値、
  超過件数、全候補の収束精度は供給報告として記載し、本変更での独立検証済みとは扱わない。
- この追記では本番 solve・build は実施していない。設定の実装検証と上記の計測根拠は区別する。


## 2026-10-19: 追加10フロップの供給済み検証結果

上記の20フロップによる規則選択とは別の追加検証として、以下の10フロップ・172状況が供給された。

`5c4d2h, 7c6d6c, Qc6h6d, Jc9c2c, Tc9c7d, Kc5d3h, Ac4d2h, 9c8h8d, QcJc8d, Ac8d6c`

供給報告では33 / 66 / 100 / 125%の全候補を精度0.1%まで解いている。
損失は**ターン開始時 pot に対する割合（%）**で、収束精度とは異なる。

|選択方法|平均損失|最大損失|0.5%超過|
|---|---:|---:|---:|
|現行ターン規則|0.082%|0.775%|4 / 172（すべてIP）|
|全状況66%固定|0.167%|—|—|

平均 `< 0.1%` は達成したが、最大 `< 0.5%` は引き続き未達。
この追記では追加検証の原データを独立集計しておらず、数値・収束精度は供給報告として扱う。
追加10フロップの結果だけで独立holdoutの設計・汎化性能や最適性が証明されたとは扱わない。
供給規則と各ターン種類の代表カードによる条件に限られ、全カード網羅ではない。
この変更では本番 solve・build は実施していない。
