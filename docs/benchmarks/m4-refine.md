# M4：フロップのサイズ比較（20 ボード、上位 2 つを精度 0.5%）

BTN オープン → BB コールの SRP、本番の設定（レーキ 5%・上限 3BB、ドンクなし、2.5 倍レイズ無制限、67% でオールインにまとめる）。
BTN のフロップ最初のベット 33 / 66 / 100 / 125% を比較（ターン・リバーは 66% 固定）。全候補を約 5% で解き、BTN の EV の上位 2 つを約 0.5% で解き直して比べた。
この Mac（M5、24GB）で約 12 時間（試しの 8 ボードの粗い段階は再利用）。

| ボード | 高さ | ペア | スート | つながり | 選ばれたサイズ | 2 位 | 差（ポット比） |
|---|---|---|---|---|---|---|---|
| 3d3c2c | 7以下 | ペア | ツートーン | ドローあり | **33%** | 66% | 0.18% |
| 7c4d3h | 7以下 | なし | レインボー | つながり | **66%** | 33% | 0.86% |
| 7c6d5d | 7以下 | なし | ツートーン | つながり | **66%** | 33% | 0.73% |
| 8c5d4h | T〜8 | なし | レインボー | つながり | **100%** | 66% | 0.12% |
| 8c6c5d | T〜8 | なし | ツートーン | つながり | **66%** | 33% | 0.12% |
| 9c8d4d | T〜8 | なし | ツートーン | ドローあり | **33%** | 66% | 0.25% |
| Tc6d2c | T〜8 | なし | ツートーン | ドローあり | **33%** | 66% | 0.42% |
| Tc8d5h | T〜8 | なし | レインボー | ドローあり | **33%** | 66% | 0.15% |
| Jc5d4d | Q・J | なし | ツートーン | ドローあり | **33%** | 66% | 0.16% |
| Jc7d6c | Q・J | なし | ツートーン | ドローあり | **33%** | 66% | 0.04% |
| Jc8d4h | Q・J | なし | レインボー | ドローあり | **125%** | 33% | 0.07% |
| Jc9d2h | Q・J | なし | レインボー | ドローあり | **33%** | 66% | 0.01% |
| QcTd8d | Q・J | なし | ツートーン | つながり | **66%** | 33% | 0.12% |
| Kc3d2h | K | なし | レインボー | ドローあり | **33%** | 66% | 0.05% |
| KcTd8d | K | なし | ツートーン | ドローあり | **66%** | 33% | 0.09% |
| KcJc8d | K | なし | ツートーン | ドローあり | **125%** | 66% | 0.19% |
| Ac9d4c | A | なし | ツートーン | ドローあり | **33%** | 66% | 0.35% |
| AcTd9h | A | なし | レインボー | ドローあり | **66%** | 33% | 0.15% |
| AcJd2d | A | なし | ツートーン | ドローあり | **33%** | 100% | 0.09% |
| AcKdJc | A | なし | ツートーン | つながり | **66%** | 33% | 0.14% |

## 傾向（20 ボードから読み取れる範囲。断定はしない）
- 乾いたボード・高いカードのボード・ペアのボードは **33%** が多い。
- 低くつながったボード（743、765、865）や、高くつながったボード（QT8、KT8、AT9、AKJ）は **66% 以上**。KJ8・J84 では 125%。
- 差が大きいのは低くつながったボード（743 で 0.86%、765 で 0.73%）。半分以上のボードで差はポットの 0.2% 未満で、どちらを選んでも失う EV は小さい。
- 精度 0.5% の EV の誤差はポットの 0.5% 程度までありうるので、差が 0.2% 未満のボードの順位は入れ替わる可能性がある。

---

# M4 sizing trial (pilot)

BTN/IP player 1 root game EV, including check; pot 5.5 BB, stack 97.5 BB. Production rake 5% cap 3 BB; flop-only sizing varies, turn/river 66%.
All four sizes at 5%, then observed top two at 2% (fresh solves). Coarse eliminated candidates remain unrefined.
**No confident optimal-size claim.** Raked general-sum exploitability is not a rigorous equilibrium-EV bound. Per-candidate diagnostic proxy = 2 × exploitability% × 5.5 / 100 BB; gap comparison sums both proxies. This is a conservative heuristic diagnostic only.
20 training flops, validation 0, seed 20261015: limited feature coverage, not the full 61 strata or held-out validation.

| Flop / features | Stage | Size % | BTN EV BB / pot % | Exploit % / target met | BR improvements BB (OOP, IP) | Wall sec / RSS bytes |
|---|---:|---:|---:|---|---|---|
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 1 | 33 | 0.256605384 / 4.665552441 | 4.434549421 / True | [0.2132724747458675, 0.27452796156740006] | 92.457 / 9332588544 |
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 1 | 66 | 0.233976783 / 4.254123330 | 4.360516234 / True | [0.26650014824537493, 0.21315663746095007] | 101.356 / 7458095104 |
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 1 | 100 | 0.190141783 / 3.457123332 | 4.353416955 / True | [0.24530616137454525, 0.23356970368375596] | 75.194 / 5746769920 |
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 1 | 125 | 0.178417573 / 3.243955872 | 4.058655180 / True | [0.22331900046078546, 0.2231330693748292] | 70.434 / 5324767232 |
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 2 | 33 | 0.200076346 / 3.637751737 | 0.479988350 / True | [0.0266980044578479, 0.02610071404870368] | 357.761 / 9314844672 |
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 2 | 66 | 0.190008078 / 3.454692321 | 0.466888252 / True | [0.025570915560260914, 0.025786792151077605] | 221.522 / 7458242560 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.434227314 / 7.895042069 | 4.089912319 / True | [0.2768324618281717, 0.17305789321862886] | 146.773 / 9189244928 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.461073799 / 8.383159990 | 4.140485523 / True | [0.2578808375048834, 0.19757257004202847] | 114.572 / 7379255296 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.432363144 / 7.861148065 | 3.963839085 / True | [0.2454281697902843, 0.1905941295497135] | 88.275 / 5686001664 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.408563332 / 7.428424221 | 4.051578163 / True | [0.234490274897905, 0.21118332306192258] | 85.251 / 5268652032 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.373605759 / 6.792831990 | 0.482613041 / True | [0.031192798818203527, 0.021894635655691504] | 249.014 / 7379238912 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.326329488 / 5.933263413 | 0.472913093 / True | [0.031646626203792494, 0.020373814040756996] | 356.245 / 9236742144 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.478331894 / 8.696943531 | 4.408576632 / True | [0.28515700933291716, 0.19978642013676393] | 164.551 / 9159852032 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.510839150 / 9.287984551 | 4.084343211 / True | [0.2620475286602721, 0.18723022460101124] | 118.799 / 7357333504 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.475453436 / 8.644607933 | 4.748615882 / True | [0.27654863805222246, 0.24579910900206858] | 79.477 / 5669175296 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.425554492 / 7.737354404 | 4.076910419 / True | [0.2251484994152977, 0.2233116466310879] | 73.396 / 5253021696 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.416263359 / 7.568424702 | 0.495632265 / True | [0.029113605115117708, 0.025405944081622465] | 256.251 / 7357382656 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.375944449 / 6.835353618 | 0.494905747 / True | [0.03297345443104359, 0.021466177755371234] | 355.098 / 9208856576 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.535239655 / 9.731630088 | 3.958998511 / True | [0.26733885125117035, 0.1681509849093885] | 149.624 / 9193816064 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.565518761 / 10.282159283 | 2.986060481 / True | [0.189750565496219, 0.1387160874337236] | 130.236 / 7353942016 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.566744796 / 10.304450839 | 4.192647480 / True | [0.2619982417360982, 0.19919298101314734] | 89.763 / 5666635776 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.528173092 / 9.603147130 | 2.926549265 / True | [0.16962087249164984, 0.15229954667779055] | 93.461 / 5250727936 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 100 | 0.513378349 / 9.334151805 | 0.477305665 / True | [0.030279705105433322, 0.022223918079805345] | 207.001 / 5666684928 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.506643811 / 9.211705652 | 0.499534336 / True | [0.03346031454709032, 0.021488462452243937] | 279.739 / 7354056704 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.459258278 / 8.350150517 | 4.776654687 / True | [0.29233091200874717, 0.23310110352553953] | 139.892 / 9136242688 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.480434072 / 8.735164938 | 4.828341390 / True | [0.30287547777895113, 0.2282420751330447] | 98.607 / 7350075392 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.440678460 / 8.012335633 | 4.433078068 / True | [0.27818396556468394, 0.20945462196178688] | 77.600 / 5663670272 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.408708568 / 7.431064866 | 4.045236118 / True | [0.25447134166346563, 0.19050463136994553] | 71.484 / 5247877120 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.377239054 / 6.858891886 | 0.481747240 / True | [0.027194914005069903, 0.025797282348285877] | 244.455 / 7350255616 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.370767879 / 6.741234169 | 0.493422746 / True | [0.02988040444935905, 0.02439609758598948] | 326.044 / 9177792512 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.454747806 / 8.268141919 | 4.238653644 / True | [0.2533382158918371, 0.21291368494502752] | 129.642 / 9104965632 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.431812050 / 7.851128176 | 4.192592905 / True | [0.2681021819967767, 0.1930830374994914] | 98.446 / 7314309120 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.383830318 / 6.978733059 | 4.182360733 / True | [0.23622202434575945, 0.22383765630952968] | 76.406 / 5636145152 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.379663563 / 6.902973872 | 4.160651812 / True | [0.2544526907431739, 0.2032190086291238] | 70.222 / 5222580224 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.380434316 / 6.916987560 | 0.484229284 / True | [0.02825567991454295, 0.025009541327689136] | 306.767 / 9220554752 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.366579717 / 6.665085760 | 0.462667590 / True | [0.026184292009314276, 0.024709142883824656] | 243.120 / 7314309120 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.387386416 / 7.043389381 | 4.316784295 / True | [0.2577071448285278, 0.2171391276149904] | 100.655 / 9167093760 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.345326977 / 6.278672311 | 3.947732807 / True | [0.24525379634489833, 0.18899681243097108] | 80.048 / 7378485248 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.291355905 / 5.297380093 | 4.636464766 / True | [0.25472547279705837, 0.2552856514443235] | 60.545 / 5686083584 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.299254295 / 5.440987189 | 4.400547051 / True | [0.27378349529971385, 0.21027668028080332] | 56.025 / 5268570112 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.316839653 / 5.760720961 | 0.473056123 / True | [0.027152952887051243, 0.02488322067613535] | 312.162 / 9215115264 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.293612903 / 5.338416425 | 0.491304762 / True | [0.02664703795310311, 0.027396485838508455] | 209.560 / 7379140608 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.390190170 / 7.094366733 | 4.166509281 / True | [0.23920260691411646, 0.21911341394878575] | 102.205 / 9071345664 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.367977732 / 6.690504222 | 3.839564724 / True | [0.24623215728715694, 0.17611996238051392] | 78.726 / 7303249920 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.329243033 / 5.986236956 | 4.125901609 / True | [0.24062138170923844, 0.21322779528220115] | 59.273 / 5627609088 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.337028495 / 6.127790819 | 4.378463153 / True | [0.28007792971773404, 0.20155301710794868] | 55.947 / 5214552064 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.318421306 / 5.789478289 | 0.468845674 / True | [0.02598485415410505, 0.025588170019963285] | 293.017 / 9119301632 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.309953518 / 5.635518503 | 0.484028510 / True | [0.025796069912121085, 0.027447066137760545] | 218.791 / 7303217152 |
| Jc5d4d {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.229536923 / 4.173398606 | 4.275678929 / True | [0.2684733505415835, 0.20185133167784858] | 102.129 / 9179021312 |
| Jc5d4d {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.193396504 / 3.516300067 | 4.196751332 / True | [0.24815559017501038, 0.2134870563808118] | 77.473 / 7350501376 |
| Jc5d4d {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.160524504 / 2.918627354 | 4.452506529 / True | [0.255979155928449, 0.2337965622553499] | 60.004 / 5663981568 |
| Jc5d4d {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.154486799 / 2.808850889 | 4.357542199 / True | [0.25556886048257615, 0.2237607814027342] | 55.778 / 5248253952 |
| Jc5d4d {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.157246847 / 2.859033582 | 0.464235635 / True | [0.027879842121403386, 0.023186077705936812] | 331.448 / 9200140288 |
| Jc5d4d {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.148391156 / 2.698021022 | 0.473795151 / True | [0.02550665063992176, 0.026610815958062035] | 234.498 / 7350468608 |
| Jc7d6c {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.178229068 / 3.240528505 | 3.932063404 / True | [0.23937845148193912, 0.1931485229554978] | 98.668 / 9145155584 |
| Jc7d6c {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.156553248 / 2.846422683 | 4.082829913 / True | [0.23830105036117344, 0.21081024003092091] | 77.658 / 7307001856 |
| Jc7d6c {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.127052425 / 2.310044099 | 4.134696215 / True | [0.23613591597200775, 0.2186806677234543] | 60.283 / 5630558208 |
| Jc7d6c {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.127770557 / 2.323101040 | 4.020426768 / True | [0.2432310144476272, 0.19901592999025586] | 55.606 / 5217271808 |
| Jc7d6c {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.112124085 / 2.038619736 | 0.463178761 / True | [0.027694394477489708, 0.023255269234425457] | 315.215 / 9074655232 |
| Jc7d6c {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.110159459 / 2.002899253 | 0.476792974 / True | [0.02388868694778705, 0.02855854015264313] | 235.690 / 7307083776 |
| Jc8d4h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.196387983 / 3.570690602 | 4.149251276 / True | [0.2512217850524438, 0.20519585528023326] | 99.870 / 9091350528 |
| Jc8d4h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.171208708 / 3.112885602 | 3.762730729 / True | [0.21893179206504737, 0.19496858817719112] | 90.076 / 7303004160 |
| Jc8d4h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.162088317 / 2.947060316 | 4.586746512 / True | [0.2673235141143397, 0.2372186022450794] | 60.891 / 5627756544 |
| Jc8d4h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.175086395 / 3.183389002 | 4.206305972 / True | [0.2833435388089268, 0.17935011810272974] | 56.194 / 5214617600 |
| Jc8d4h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.131529627 / 2.391447765 | 0.492723140 / True | [0.02672243338435817, 0.027477112068863333] | 282.774 / 9119449088 |
| Jc8d4h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 125 | 0.135240072 / 2.458910398 | 0.481432452 / True | [0.026124047292775232, 0.026833522396566622] | 168.694 / 5214781440 |
| Jc9d2h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.182125526 / 3.311373202 | 4.307321487 / True | [0.2641100028706816, 0.20969536072551426] | 103.741 / 9088090112 |
| Jc9d2h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.151175859 / 2.748651974 | 3.713221762 / True | [0.2265651297496729, 0.18188926402622715] | 77.530 / 7317553152 |
| Jc9d2h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.138713979 / 2.522072343 | 4.529244524 / True | [0.2558635214804109, 0.24235337617155567] | 60.429 / 5638651904 |
| Jc9d2h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.147931304 / 2.689660072 | 4.144559945 / True | [0.26788191682187845, 0.18801967711985765] | 56.296 / 5224857600 |
| Jc9d2h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.114816035 / 2.087564277 | 0.472489014 / True | [0.02630622404433234, 0.025667567494552544] | 319.202 / 9088925696 |
| Jc9d2h {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.114100206 / 2.074549192 | 0.473290812 / True | [0.025316577152369176, 0.02674541218230375] | 212.256 / 7317602304 |
| QcTd8d {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.041457115 / 0.753765731 | 4.195340632 / True | [0.2707486328327893, 0.1907388367319843] | 98.846 / 9059811328 |
| QcTd8d {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.029170327 / 0.530369575 | 3.973546115 / True | [0.24190617376750742, 0.1951838989197493] | 77.187 / 7256391680 |
| QcTd8d {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.019470401 / 0.354007297 | 3.973914588 / True | [0.2317073625950002, 0.20542324205087023] | 59.839 / 5591580672 |
| QcTd8d {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.026806079 / 0.487383247 | 4.961935446 / True | [0.3114759170620514, 0.23433698197375655] | 46.649 / 5181292544 |
| QcTd8d {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.000097780 / 0.001777824 | 0.490181864 / True | [0.028831192196470398, 0.02508881285605626] | 298.778 / 9064677376 |
| QcTd8d {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.006809816 / 0.123814830 | 0.460402986 / True | [0.025001682197728768, 0.025642646217897042] | 256.465 / 7257030656 |
| Kc3d2h {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.081039868 / 1.473452143 | 4.761789354 / True | [0.33986794173477003, 0.18392888724630738] | 204.299 / 9165111296 |
| Kc3d2h {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.052069599 / 0.946719976 | 4.134731860 / True | [0.281817218324432, 0.17300328627410766] | 161.111 / 7358251008 |
| Kc3d2h {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.027027524 / 0.491409529 | 3.812824554 / True | [0.2452296505283461, 0.17418105040606918] | 144.412 / 5669126144 |
| Kc3d2h {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.021792087 / 0.396219758 | 3.771326323 / True | [0.23672129945409978, 0.17812459605013958] | 130.389 / 5253939200 |
| Kc3d2h {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.009734539 / 0.176991620 | 0.476989359 / True | [0.02750956063545857, 0.024959268889950148] | 918.912 / 9187344384 |
| Kc3d2h {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.006969891 / 0.126725287 | 0.467626751 / True | [0.026256434386493197, 0.025182508180187965] | 283.706 / 7357661184 |
| KcTd8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.085091174 / 1.547112261 | 4.046460237 / True | [0.2475587274830724, 0.19755189861618577] | 74.970 / 9119858688 |
| KcTd8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.075227523 / 1.367773151 | 4.147991588 / True | [0.2473046518805296, 0.208974422824394] | 62.271 / 7234830336 |
| KcTd8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.070360878 / 1.279288685 | 3.766560197 / True | [0.23132122739905878, 0.18300039427850817] | 49.723 / 5575016448 |
| KcTd8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.064866267 / 1.179386677 | 3.715467703 / True | [0.22705709849205982, 0.18164434882520653] | 46.246 / 5165924352 |
| KcTd8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.038993646 / 0.708975384 | 0.499126477 / True | [0.02801202475142429, 0.026891887744798525] | 272.068 / 9032564736 |
| KcTd8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.043977461 / 0.799590191 | 0.466451554 / True | [0.02329152061406137, 0.028018150303227764] | 236.726 / 7235010560 |
| KcJc8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | -0.033992626 / -0.618047745 | 4.224438651 / True | [0.2540319572725588, 0.21065629437217523] | 99.181 / 9024667648 |
| KcJc8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | -0.019986591 / -0.363392573 | 4.465807468 / True | [0.27180017345008883, 0.21943864807359575] | 75.773 / 7224016896 |
| KcJc8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | -0.027370170 / -0.497639455 | 4.065035807 / True | [0.24317876446376868, 0.20397517425199221] | 58.662 / 5566676992 |
| KcJc8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | -0.026044684 / -0.473539715 | 4.020161491 / True | [0.24867399719741365, 0.193543766845849] | 54.848 / 5158387712 |
| KcJc8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | -0.050506343 / -0.918297154 | 0.491698992 / True | [0.02473367587947348, 0.02935321329315523] | 262.260 / 7223410688 |
| KcJc8d {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 125 | -0.040244734 / -0.731722445 | 0.463667021 / True | [0.025137933427973054, 0.025865438894041425] | 171.634 / 5158371328 |
| Ac9d4c {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.422793843 / 7.687160790 | 4.501092255 / True | [0.2803037140929265, 0.21481643399176176] | 98.480 / 9055715328 |
| Ac9d4c {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.413161745 / 7.512031720 | 4.611371275 / True | [0.31209075599762887, 0.1951600842584328] | 77.320 / 7253114880 |
| Ac9d4c {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.380490099 / 6.918001808 | 4.618434039 / True | [0.28494219482215827, 0.22308554950543003] | 59.588 / 5589106688 |
| Ac9d4c {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.381368476 / 6.933972287 | 4.993678998 / True | [0.31394940894263484, 0.23535528079709098] | 56.002 / 5179097088 |
| Ac9d4c {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.380113355 / 6.911151912 | 0.494300932 / True | [0.022209970339561097, 0.032163132169836274] | 340.203 / 9055272960 |
| Ac9d4c {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.361013846 / 6.563888103 | 0.487282662 / True | [0.024827311131952797, 0.028773781726355876] | 256.472 / 7253049344 |
| AcTd9h {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.373667588 / 6.793956137 | 3.929814054 / True | [0.22081809691152807, 0.21146144904881334] | 120.187 / 9011986432 |
| AcTd9h {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.366090086 / 6.656183385 | 4.277708709 / True | [0.24547359815710512, 0.2250743598091295] | 95.159 / 7202095104 |
| AcTd9h {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.353445338 / 6.426278877 | 4.090530348 / True | [0.23035481813658043, 0.21960352019650856] | 73.171 / 5549817856 |
| AcTd9h {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.354986220 / 6.454294900 | 4.230189801 / True | [0.26113862025455775, 0.20418225788324662] | 67.644 / 5142528000 |
| AcTd9h {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.337205869 / 6.131015803 | 0.471318032 / True | [0.026172108287046966, 0.0256728752216045] | 361.350 / 8994947072 |
| AcTd9h {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.345707030 / 6.285582363 | 0.485832629 / True | [0.02323940795816793, 0.030202181221247598] | 285.820 / 7202193408 |
| AcJd2d {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.256725592 / 4.667738029 | 3.813463545 / True | [0.2395765830935692, 0.17990440686366832] | 121.312 / 9069297664 |
| AcJd2d {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.237046938 / 4.309944322 | 3.980603208 / True | [0.24492727232261166, 0.1929390805073514] | 92.349 / 7263830016 |
| AcJd2d {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.238388970 / 4.334344909 | 4.121161562 / True | [0.2622143394244922, 0.19111343238226738] | 70.928 / 5597396992 |
| AcJd2d {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.231620141 / 4.211275283 | 4.072856890 / True | [0.25869910759993753, 0.1893151502751259] | 64.821 / 5186584576 |
| AcJd2d {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.222226145 / 4.040475357 | 0.480529415 / True | [0.026725522462554066, 0.026132713185124827] | 349.012 / 9069346816 |
| AcJd2d {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 100 | 0.217458041 / 3.953782571 | 0.493083705 / True | [0.02395371897946757, 0.03028548859389039] | 185.999 / 5597347840 |
| AcKdJc {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.090327058 / 1.642310139 | 4.234991530 / True | [0.28159961029555247, 0.18424945795666017] | 116.780 / 8892710912 |
| AcKdJc {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.070451980 / 1.280945086 | 4.887484551 / True | [0.3028523378503819, 0.23477096275929954] | 78.868 / 7162773504 |
| AcKdJc {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.033963987 / 0.617527037 | 2.516398222 / True | [0.14234810018956376, 0.1344557042127136] | 80.246 / 5519491072 |
| AcKdJc {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.034106114 / 0.620111157 | 4.594197322 / True | [0.3065223232465952, 0.19883938215361024] | 56.888 / 5114675200 |
| AcKdJc {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.038149545 / 0.693628093 | 0.483842129 / True | [0.02770757962702453, 0.0255150545773544] | 330.145 / 8892104704 |
| AcKdJc {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.045768306 / 0.832151016 | 0.469301461 / True | [0.023921532739956206, 0.02770162792209692] | 239.217 / 7162740736 |

## Board summary (refined survivors only)
| Flop | Observed winner / runner-up | Gap BB / combined proxy BB | Diagnostic | Total wall seconds |
|---|---|---|---|---|
| 3d3c2c | 33 / 66 | 0.010068268 / 0.104156426 | 区別できない | 918.725 |
| 7c4d3h | 66 / 33 | 0.047276272 / 0.105107875 | 区別できない | 1040.129 |
Mixed-precision apparent highest for 7c4d3h: 100%; not a refined winner.
| 7c6d5d | 66 / 33 | 0.040318910 / 0.108959181 | 区別できない | 1047.572 |
Mixed-precision apparent highest for 7c6d5d: 100%; not a refined winner.
| 8c5d4h | 100 / 66 | 0.006734538 / 0.107452400 | 区別できない | 949.824 |
Mixed-precision apparent highest for 8c5d4h: 33%; not a refined winner.
| 8c6c5d | 66 / 33 | 0.006471174 / 0.107268698 | 区別できない | 958.082 |
Mixed-precision apparent highest for 8c6c5d: 100%; not a refined winner.
| 9c8d4d | 33 / 66 | 0.013854599 / 0.104158656 | 区別できない | 924.602 |
Mixed-precision apparent highest for 9c8d4d: 100%; not a refined winner.
| Tc6d2c | 33 / 66 | 0.023226749 / 0.106079697 | 区別できない | 818.994 |
| Tc8d5h | 33 / 66 | 0.008467788 / 0.104816160 | 区別できない | 807.959 |
Mixed-precision apparent highest for Tc8d5h: 125%; not a refined winner.
| Jc5d4d | 33 / 66 | 0.008855691 / 0.103183386 | 区別できない | 861.331 |
Mixed-precision apparent highest for Jc5d4d: 100%; not a refined winner.
| Jc7d6c | 33 / 66 | 0.001964627 / 0.103396891 | 区別できない | 843.120 |
Mixed-precision apparent highest for Jc7d6c: 125%; not a refined winner.
| Jc8d4h | 125 / 33 | 0.003710445 / 0.107157115 | 区別できない | 758.498 |
Mixed-precision apparent highest for Jc8d4h: 66%; not a refined winner.
| Jc9d2h | 33 / 66 | 0.000715830 / 0.104035781 | 区別できない | 829.453 |
Mixed-precision apparent highest for Jc9d2h: 125%; not a refined winner.
| QcTd8d | 66 / 33 | 0.006712035 / 0.104564333 | 区別できない | 837.764 |
Mixed-precision apparent highest for QcTd8d: 125%; not a refined winner.
| Kc3d2h | 33 / 66 | 0.002764648 / 0.103907772 | 区別できない | 1842.828 |
Mixed-precision apparent highest for Kc3d2h: 100%; not a refined winner.
| KcTd8d | 66 / 33 | 0.004983814 / 0.106213583 | 区別できない | 742.004 |
Mixed-precision apparent highest for KcTd8d: 100%; not a refined winner.
| KcJc8d | 125 / 66 | 0.010261609 / 0.105090261 | 区別できない | 722.359 |
Mixed-precision apparent highest for KcJc8d: 100%; not a refined winner.
| Ac9d4c | 33 / 66 | 0.019099509 / 0.107974195 | 区別できない | 888.065 |
Mixed-precision apparent highest for Ac9d4c: 125%; not a refined winner.
| AcTd9h | 66 / 33 | 0.008501161 / 0.105286573 | 区別できない | 1003.331 |
Mixed-precision apparent highest for AcTd9h: 125%; not a refined winner.
| AcJd2d | 33 / 100 | 0.004768103 / 0.107097443 | 区別できない | 884.422 |
Mixed-precision apparent highest for AcJd2d: 66%; not a refined winner.
| AcKdJc | 66 / 33 | 0.007618761 / 0.104845795 | 区別できない | 902.143 |

## Gaps and cautious grouped trends
- 3d3c2c vs 100%: gap 0.009934562 BB (0.180628405% pot), combined proxy 0.531674584 BB; indistinguishable=True; eliminated/unrefined=True.
- 3d3c2c vs 66%: gap 0.010068268 BB (0.183059416% pot), combined proxy 0.104156426 BB; indistinguishable=True; eliminated/unrefined=False.
- 3d3c2c vs 125%: gap 0.021658773 BB (0.393795865% pot), combined proxy 0.499250788 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c4d3h vs 100%: gap 0.058757384 BB (1.068316075% pot), combined proxy 0.489109734 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c4d3h vs 125%: gap 0.034957573 BB (0.635592232% pot), combined proxy 0.498761032 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c4d3h vs 33%: gap 0.047276272 BB (0.859568577% pot), combined proxy 0.105107875 BB; indistinguishable=True; eliminated/unrefined=False.
- 7c6d5d vs 100%: gap 0.059190078 BB (1.076183231% pot), combined proxy 0.576867296 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c6d5d vs 125%: gap 0.009291134 BB (0.168929702% pot), combined proxy 0.502979695 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c6d5d vs 33%: gap 0.040318910 BB (0.733071084% pot), combined proxy 0.108959181 BB; indistinguishable=True; eliminated/unrefined=False.
- 8c5d4h vs 33%: gap 0.021861306 BB (0.397478283% pot), combined proxy 0.487993459 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c5d4h vs 125%: gap 0.014794743 BB (0.268995324% pot), combined proxy 0.374424042 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c5d4h vs 66%: gap 0.006734538 BB (0.122446153% pot), combined proxy 0.107452400 BB; indistinguishable=True; eliminated/unrefined=False.
- 8c6c5d vs 100%: gap 0.063439406 BB (1.153443747% pot), combined proxy 0.540630784 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c6c5d vs 125%: gap 0.031469514 BB (0.572172980% pot), combined proxy 0.497968169 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c6c5d vs 33%: gap 0.006471174 BB (0.117657717% pot), combined proxy 0.107268698 BB; indistinguishable=True; eliminated/unrefined=False.
- 9c8d4d vs 100%: gap 0.003396002 BB (0.061745499% pot), combined proxy 0.513324902 BB; indistinguishable=True; eliminated/unrefined=True.
- 9c8d4d vs 125%: gap 0.000770753 BB (0.014013689% pot), combined proxy 0.510936921 BB; indistinguishable=True; eliminated/unrefined=True.
- 9c8d4d vs 66%: gap 0.013854599 BB (0.251901801% pot), combined proxy 0.104158656 BB; indistinguishable=True; eliminated/unrefined=False.
- Tc6d2c vs 125%: gap 0.017585357 BB (0.319733772% pot), combined proxy 0.536096349 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc6d2c vs 66%: gap 0.023226749 BB (0.422304536% pot), combined proxy 0.106079697 BB; indistinguishable=True; eliminated/unrefined=False.
- Tc6d2c vs 100%: gap 0.025483748 BB (0.463340868% pot), combined proxy 0.562047298 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc8d5h vs 125%: gap 0.018607189 BB (0.338312530% pot), combined proxy 0.533203971 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc8d5h vs 100%: gap 0.010821727 BB (0.196758668% pot), combined proxy 0.505422201 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc8d5h vs 66%: gap 0.008467788 BB (0.153959786% pot), combined proxy 0.104816160 BB; indistinguishable=True; eliminated/unrefined=False.
- Jc5d4d vs 100%: gap 0.003277657 BB (0.059593771% pot), combined proxy 0.540841638 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc5d4d vs 125%: gap 0.002760048 BB (0.050182694% pot), combined proxy 0.530395562 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc5d4d vs 66%: gap 0.008855691 BB (0.161012561% pot), combined proxy 0.103183386 BB; indistinguishable=True; eliminated/unrefined=False.
- Jc7d6c vs 125%: gap 0.015646472 BB (0.284481304% pot), combined proxy 0.493196608 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc7d6c vs 100%: gap 0.014928340 BB (0.271424363% pot), combined proxy 0.505766247 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc7d6c vs 66%: gap 0.001964627 BB (0.035720483% pot), combined proxy 0.103396891 BB; indistinguishable=True; eliminated/unrefined=False.
- Jc8d4h vs 66%: gap 0.035968636 BB (0.653975204% pot), combined proxy 0.466857950 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc8d4h vs 100%: gap 0.026848245 BB (0.488149918% pot), combined proxy 0.557499686 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc8d4h vs 33%: gap 0.003710445 BB (0.067462633% pot), combined proxy 0.107157115 BB; indistinguishable=True; eliminated/unrefined=False.
- Jc9d2h vs 125%: gap 0.033115269 BB (0.602095795% pot), combined proxy 0.507875385 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc9d2h vs 100%: gap 0.023897944 BB (0.434508066% pot), combined proxy 0.550190689 BB; indistinguishable=True; eliminated/unrefined=True.
- Jc9d2h vs 66%: gap 0.000715830 BB (0.013015084% pot), combined proxy 0.104035781 BB; indistinguishable=True; eliminated/unrefined=False.
- QcTd8d vs 125%: gap 0.019996263 BB (0.363568417% pot), combined proxy 0.596457227 BB; indistinguishable=True; eliminated/unrefined=True.
- QcTd8d vs 100%: gap 0.012660586 BB (0.230192467% pot), combined proxy 0.487774933 BB; indistinguishable=True; eliminated/unrefined=True.
- QcTd8d vs 33%: gap 0.006712035 BB (0.122037006% pot), combined proxy 0.104564333 BB; indistinguishable=True; eliminated/unrefined=False.
- Kc3d2h vs 100%: gap 0.017292985 BB (0.314417908% pot), combined proxy 0.471879530 BB; indistinguishable=True; eliminated/unrefined=True.
- Kc3d2h vs 125%: gap 0.012057548 BB (0.219228138% pot), combined proxy 0.467314725 BB; indistinguishable=True; eliminated/unrefined=True.
- Kc3d2h vs 66%: gap 0.002764648 BB (0.050266333% pot), combined proxy 0.103907772 BB; indistinguishable=True; eliminated/unrefined=False.
- KcTd8d vs 100%: gap 0.026383417 BB (0.479698493% pot), combined proxy 0.465631293 BB; indistinguishable=True; eliminated/unrefined=True.
- KcTd8d vs 125%: gap 0.020888807 BB (0.379796485% pot), combined proxy 0.460011118 BB; indistinguishable=True; eliminated/unrefined=True.
- KcTd8d vs 33%: gap 0.004983814 BB (0.090614807% pot), combined proxy 0.106213583 BB; indistinguishable=True; eliminated/unrefined=False.
- KcJc8d vs 100%: gap 0.012874564 BB (0.234082990% pot), combined proxy 0.498157311 BB; indistinguishable=True; eliminated/unrefined=True.
- KcJc8d vs 33%: gap 0.006252109 BB (0.113674700% pot), combined proxy 0.515691624 BB; indistinguishable=True; eliminated/unrefined=True.
- KcJc8d vs 66%: gap 0.010261609 BB (0.186574709% pot), combined proxy 0.105090261 BB; indistinguishable=True; eliminated/unrefined=False.
- Ac9d4c vs 125%: gap 0.001255121 BB (0.022820375% pot), combined proxy 0.603677792 BB; indistinguishable=True; eliminated/unrefined=True.
- Ac9d4c vs 100%: gap 0.000376744 BB (0.006849896% pot), combined proxy 0.562400847 BB; indistinguishable=True; eliminated/unrefined=True.
- Ac9d4c vs 66%: gap 0.019099509 BB (0.347263809% pot), combined proxy 0.107974195 BB; indistinguishable=True; eliminated/unrefined=False.
- AcTd9h vs 125%: gap 0.009279190 BB (0.168712537% pot), combined proxy 0.518762467 BB; indistinguishable=True; eliminated/unrefined=True.
- AcTd9h vs 100%: gap 0.007738308 BB (0.140696514% pot), combined proxy 0.503399928 BB; indistinguishable=True; eliminated/unrefined=True.
- AcTd9h vs 33%: gap 0.008501161 BB (0.154566560% pot), combined proxy 0.105286573 BB; indistinguishable=True; eliminated/unrefined=False.
- AcJd2d vs 66%: gap 0.014820793 BB (0.269468965% pot), combined proxy 0.490724588 BB; indistinguishable=True; eliminated/unrefined=True.
- AcJd2d vs 125%: gap 0.009393996 BB (0.170799926% pot), combined proxy 0.500872494 BB; indistinguishable=True; eliminated/unrefined=True.
- AcJd2d vs 100%: gap 0.004768103 BB (0.086692786% pot), combined proxy 0.107097443 BB; indistinguishable=True; eliminated/unrefined=False.
- AcKdJc vs 33%: gap 0.007618761 BB (0.138522923% pot), combined proxy 0.104845795 BB; indistinguishable=True; eliminated/unrefined=False.
- AcKdJc vs 125%: gap 0.011662192 BB (0.212039859% pot), combined proxy 0.556984866 BB; indistinguishable=True; eliminated/unrefined=True.
- AcKdJc vs 100%: gap 0.011804319 BB (0.214623979% pot), combined proxy 0.328426965 BB; indistinguishable=True; eliminated/unrefined=True.
- Features {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"}: 3d3c2c: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"}: 7c4d3h: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"}: 7c6d5d: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"}: 8c5d4h: observed 100%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"}: 8c6c5d: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"}: 9c8d4d: observed 33%; Tc6d2c: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"}: Tc8d5h: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"}: Jc5d4d: observed 33%; Jc7d6c: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "QueenJack", "pairing": "Unpaired", "suits": "Rainbow"}: Jc8d4h: observed 125%; Jc9d2h: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "QueenJack", "pairing": "Unpaired", "suits": "TwoTone"}: QcTd8d: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "Rainbow"}: Kc3d2h: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "King", "pairing": "Unpaired", "suits": "TwoTone"}: KcTd8d: observed 66%; KcJc8d: observed 125%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"}: Ac9d4c: observed 33%; AcJd2d: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "Ace", "pairing": "Unpaired", "suits": "Rainbow"}: AcTd9h: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "Ace", "pairing": "Unpaired", "suits": "TwoTone"}: AcKdJc: observed 66%. Descriptive only; no learned rule or optimality inference.

## Incomplete boards
None.

## Reproducibility
Manifest includes binary, source, configuration and original input-file hashes (no range contents). Serial child watchdog kills and reaps; checkpoint and failed attempts retained.
```json
{
  "identity": "04a9ad2300b79ff199878625337f70874f12e9990e20efc00561b7df4edd296a",
  "provenance": {
    "hashes": {
      "/Users/yota/Projects/Products/ReysonGTO-m4/target/release/cli": "9445e0900085b4de22f8159f34366ded98debf1a6e5ecf9bec35fb7d9162ab34",
      "/Users/yota/Projects/Products/ReysonGTO-m4/target/release/sizing": "510e3b1e92be19c6529ce64ba63a20b5276ba186efa796ffaa2ddc1fa6951926",
      "/Users/yota/Projects/Products/ReysonGTO-m4/data/ranges/bb_call.json": "86cd2ea0f2e74e1fce53737363771fa8ccfc480c5e14fd6f9de7a1d34fc2b804",
      "/Users/yota/Projects/Products/ReysonGTO-m4/data/ranges/btn_open.json": "f7feca455cdbb27459f2e0ada93d0bf4f470d0fd335cbabc52c15be6635015ac",
      "/Users/yota/Projects/Products/ReysonGTO-m4/Cargo.lock": "f3bac6b35268d1c4cfa09881b29c01cc952ee5b188e064f6e8ae59e7824db987",
      "/Users/yota/Projects/Products/ReysonGTO-m4/Cargo.toml": "2be11c8c693cfdc5d24cd647a0dedb4fafdebf5050440755ec64421bdf99704d",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cards/Cargo.toml": "fbf282f031c283bd16853ec421001e9b922f89527285fd5f4fe0e050ab1aa7ee",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cards/src/lib.rs": "51b133746393f701e07d130c6ae3a4dc5371beb4a34c7e063b662a389f00f02f",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cfr/Cargo.toml": "3ea7aa00cf94a9586a614131f5e06d6e9af4f0e49a91074e187f27e2ecdc0938",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cfr/src/diagnostics.rs": "7e6db6158667825ee98628c64dd7ecd3dfd03a23269b3a3fd9b3bea7160cbba3",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cfr/src/lib.rs": "6856671c21de60e8654fbdcfc84b6fe9068c408b0066fc272329eb3cd8602517",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cfr/tests/no_alloc.rs": "479389a23f9976c797f6cd99f74122f720e6222ceaaac8e9adfa5640f41e29e3",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cfr/tests/public_properties.rs": "ceadd7cd68a74aaeb052d0cc2a48694f851479de95528b5cfcc08ff5980c339d",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cfr/tests/sizing_invariance.rs": "820e1ac96cdc96c8a4a9049ff8faef5f287bfb3ba2728b48ba23918d9d10fb35",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cli/Cargo.toml": "2dd21aa35d2ff17af90372ca701d29b5bf4be3d0dd0a3cdb8c3f2019c8a2da90",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cli/src/compare.rs": "08a7e2fef4003130b8ccd81303dbfec4b93d74addd6f9d23606689d1e7aa0112",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/cli/src/main.rs": "201e159cd279bb2a5a3839f81b2aa208a7893361955090f6f6d85c83cf822d39",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/ranges/Cargo.toml": "a6a65c84000f0be6649e2635f781ba59970f4236a0e25893649252251a787bee",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/ranges/src/lib.rs": "d324941c0a2176e7ed9d3df3f2adf37ffb695f5d60c16d371aeed239e74f9dc4",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/Cargo.toml": "39b61794ccef5e707abb074dc5497f2861985914e63af0331c4fe14b74d9cf9c",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/src/bin/sizing.rs": "6c9af3c7f9928c3ea5e4adb4d530655d8fd8b337f832442a91172e99c7b98095",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/src/compare.rs": "1f3c92e8c863bb863ac8e1b04a9573e6ca4ba042bdb35db50eb03df7a23e68b7",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/src/features.rs": "e5dec49716d692b239d8c7eb0ec854066fb45640fd9aa67b03ebb999f1dc7362",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/src/flops.rs": "8b6985b9a80628f9007214d43ed0b55b12ef47ccac2e6b5fd0226ecc44470088",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/src/lib.rs": "a89d96c1d902e5d2fa32969e31551cad407b30342770d9521f6fe4ff3b948d57",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/sizing/src/rules.rs": "29d3d90974644eb5b7a63b6f8241762b36c1390d8788e74eaf0fb3ee44008a3d",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/solver/Cargo.toml": "98975d5bcf234f448cbec39d426ac85973e9c09fdeca70259822d6ceee8fa7af",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/solver/src/lib.rs": "5ca64bf4865f30a810fdedb6a60be9aad3b53ad55806bea67723d368674fea32",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/tree/Cargo.toml": "046515452c1bb777f610bbba4dbb9f829151387ae81900aff6b58b5ad747ecbf",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/tree/examples/m2_tree_audit.rs": "55311ae25a8d9a1482f6c617b8e4134e7aebe4d89d42a31d1dd518e379158384",
      "/Users/yota/Projects/Products/ReysonGTO-m4/crates/tree/src/lib.rs": "f35acb7c7994733cbf9bd3639fde223821e3fef5ff3b52df9ad5ec7d72326388",
      "/Users/yota/Projects/Products/ReysonGTO-m4/tools/m3_bench.py": "0915e09092794d99602de510e1f2144afa2453c60ad68131ae3f62fb801982f3",
      "/Users/yota/Projects/Products/ReysonGTO-m4/tools/m4_trial.py": "3508f8bd3250da2922f05a8feefe5095a3376da506841627f97fba6e51dc7aa5"
    },
    "config": {
      "threads": 10,
      "memory_mb": 16384,
      "max_iters": 10000,
      "targets": [
        5.0,
        0.5
      ],
      "sizes": [
        33,
        66,
        100,
        125
      ],
      "pot": 5.5,
      "stack": 97.5,
      "player": 1,
      "turn": 66,
      "river": 66,
      "preset": "production"
    }
  },
  "selection": {
    "seed": 20261015,
    "training": [
      {
        "features": {
          "connectivity": "Gapped",
          "height": "Low",
          "pairing": "Paired",
          "suits": "TwoTone"
        },
        "flop": "3d3c2c",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Connected",
          "height": "Low",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "7c4d3h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Connected",
          "height": "Low",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "7c6d5d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Connected",
          "height": "TenEight",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "8c5d4h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Connected",
          "height": "TenEight",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "8c6c5d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "TenEight",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "9c8d4d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "TenEight",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "Tc6d2c",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "TenEight",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "Tc8d5h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "QueenJack",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "Jc5d4d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "QueenJack",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "Jc7d6c",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "QueenJack",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "Jc8d4h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "QueenJack",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "Jc9d2h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Connected",
          "height": "QueenJack",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "QcTd8d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "King",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "Kc3d2h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "King",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "KcTd8d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "King",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "KcJc8d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "Ace",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "Ac9d4c",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "Ace",
          "pairing": "Unpaired",
          "suits": "Rainbow"
        },
        "flop": "AcTd9h",
        "weight": 24
      },
      {
        "features": {
          "connectivity": "Gapped",
          "height": "Ace",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "AcJd2d",
        "weight": 12
      },
      {
        "features": {
          "connectivity": "Connected",
          "height": "Ace",
          "pairing": "Unpaired",
          "suits": "TwoTone"
        },
        "flop": "AcKdJc",
        "weight": 12
      }
    ],
    "validation": []
  },
  "selection_command": [
    "/Users/yota/Projects/Products/ReysonGTO-m4/target/release/sizing",
    "select",
    "--training",
    "20",
    "--validation",
    "0",
    "--seed",
    "20261015"
  ],
  "selection_resource": {
    "command": [
      "/Users/yota/Projects/Products/ReysonGTO-m4/target/release/sizing",
      "select",
      "--training",
      "20",
      "--validation",
      "0",
      "--seed",
      "20261015"
    ],
    "solver_pid": 14614,
    "exit_code": 0,
    "child_exit_code": 0,
    "timed_out": false,
    "wall_seconds": 0.10677825001766905,
    "user_cpu_seconds": 0.002881,
    "system_cpu_seconds": 0.001063,
    "maximum_resident_set_bytes": 2244608,
    "major_page_faults": 0,
    "swaps": 0,
    "source": "os.wait4(exact solver PID); excludes prior solves and manifest commands",
    "platform": "Darwin"
  },
  "created_utc": "2026-10-15T00:48:34.919606+00:00",
  "budget_deadline_unix": 1792082914.80384
}
```
