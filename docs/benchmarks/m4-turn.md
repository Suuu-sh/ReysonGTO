# M4 ターン比較: 合成レンジの動作・時間確認

2026-10-17。**製品レンジの計測でも、サイズの最適性の証明でもない。**
外部 OSS ソルバーのコード・製品レンジ・既存計測は参照/変更していない。
既定の flop rule と data/m4b/manifest.json はこの checkout にはなく、本番比較は未実施。

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
