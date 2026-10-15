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
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 2 | 33 | 0.229445079 / 4.171728710 | 1.855407926 / True | [0.09722160975284566, 0.10687326205506764] | 229.647 / 9288204288 |
| 3d3c2c {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"} | 2 | 66 | 0.206789089 / 3.759801619 | 1.895649312 / True | [0.10763953143074013, 0.1008818928603028] | 145.947 / 7458160640 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.434227314 / 7.895042069 | 4.089912319 / True | [0.2768324618281717, 0.17305789321862886] | 146.773 / 9189244928 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.461073799 / 8.383159990 | 4.140485523 / True | [0.2578808375048834, 0.19757257004202847] | 114.572 / 7379255296 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.432363144 / 7.861148065 | 3.963839085 / True | [0.2454281697902843, 0.1905941295497135] | 88.275 / 5686001664 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.408563332 / 7.428424221 | 4.051578163 / True | [0.234490274897905, 0.21118332306192258] | 85.251 / 5268652032 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.415656863 / 7.557397511 | 1.880965478 / True | [0.11988938844223798, 0.08701681417494637] | 162.368 / 7378665472 |
| 7c4d3h {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.375891386 / 6.834388830 | 1.989330639 / True | [0.13669141379500388, 0.08213495645049244] | 269.028 / 9166798848 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.478331894 / 8.696943531 | 4.408576632 / True | [0.28515700933291716, 0.19978642013676393] | 164.551 / 9159852032 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.510839150 / 9.287984551 | 4.084343211 / True | [0.2620475286602721, 0.18723022460101124] | 118.799 / 7357333504 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.475453436 / 8.644607933 | 4.748615882 / True | [0.27654863805222246, 0.24579910900206858] | 79.477 / 5669175296 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.425554492 / 7.737354404 | 4.076910419 / True | [0.2251484994152977, 0.2233116466310879] | 73.396 / 5253021696 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.456370318 / 8.297642152 | 1.952187849 / True | [0.11878416737456246, 0.09595649597630683] | 165.134 / 7357382656 |
| 7c6d5d {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.422835282 / 7.687914219 | 1.880366301 / True | [0.12559555681194412, 0.0812447362859569] | 230.759 / 9172647936 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.535239655 / 9.731630088 | 3.958998511 / True | [0.26733885125117035, 0.1681509849093885] | 149.624 / 9193816064 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.565518761 / 10.282159283 | 2.986060481 / True | [0.189750565496219, 0.1387160874337236] | 130.236 / 7353942016 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.566744796 / 10.304450839 | 4.192647480 / True | [0.2619982417360982, 0.19919298101314734] | 89.763 / 5666635776 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.528173092 / 9.603147130 | 2.926549265 / True | [0.16962087249164984, 0.15229954667779055] | 93.461 / 5250727936 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 100 | 0.539344918 / 9.806271239 | 1.931502190 / True | [0.11692605627782358, 0.09553918464984645] | 125.160 / 5666586624 |
| 8c5d4h {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.543985019 / 9.890636716 | 1.777947361 / True | [0.11620217004191136, 0.0793720396876854] | 178.254 / 7353958400 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.459258278 / 8.350150517 | 4.776654687 / True | [0.29233091200874717, 0.23310110352553953] | 139.892 / 9136242688 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.480434072 / 8.735164938 | 4.828341390 / True | [0.30287547777895113, 0.2282420751330447] | 98.607 / 7350075392 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.440678460 / 8.012335633 | 4.433078068 / True | [0.27818396556468394, 0.20945462196178688] | 77.600 / 5663670272 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.408708568 / 7.431064866 | 4.045236118 / True | [0.25447134166346563, 0.19050463136994553] | 71.484 / 5247877120 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.410133303 / 7.456969141 | 1.772957901 / True | [0.10382954837045986, 0.09119582073561679] | 160.563 / 7350173696 |
| 8c6c5d {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.408147589 / 7.420865258 | 1.849292226 / True | [0.11284003851127886, 0.09058210637146345] | 222.618 / 9130786816 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.454747806 / 8.268141919 | 4.238653644 / True | [0.2533382158918371, 0.21291368494502752] | 129.642 / 9104965632 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.431812050 / 7.851128176 | 4.192592905 / True | [0.2681021819967767, 0.1930830374994914] | 98.446 / 7314309120 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.383830318 / 6.978733059 | 4.182360733 / True | [0.23622202434575945, 0.22383765630952968] | 76.406 / 5636145152 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.379663563 / 6.902973872 | 4.160651812 / True | [0.2544526907431739, 0.2032190086291238] | 70.222 / 5222580224 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.417717220 / 7.594858554 | 1.938037616 / True | [0.1139978602408005, 0.09918627753595996] | 193.441 / 9084616704 |
| 9c8d4d {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.394119807 / 7.165814669 | 1.711044576 / True | [0.10283453889323482, 0.08538036449872573] | 134.985 / 7313768448 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 33 | 0.387386416 / 7.043389381 | 4.316784295 / True | [0.2577071448285278, 0.2171391276149904] | 100.655 / 9167093760 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 66 | 0.345326977 / 6.278672311 | 3.947732807 / True | [0.24525379634489833, 0.18899681243097108] | 80.048 / 7378485248 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 100 | 0.291355905 / 5.297380093 | 4.636464766 / True | [0.25472547279705837, 0.2552856514443235] | 60.545 / 5686083584 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 1 | 125 | 0.299254295 / 5.440987189 | 4.400547051 / True | [0.27378349529971385, 0.21027668028080332] | 56.025 / 5268570112 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 33 | 0.351626318 / 6.393205775 | 1.982065378 / True | [0.11683671686387641, 0.10119047476702189] | 149.110 / 9206956032 |
| Tc6d2c {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"} | 2 | 66 | 0.313125632 / 5.693193308 | 1.899418312 / True | [0.10924287449901976, 0.09969313980832345] | 102.098 / 7379091456 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 33 | 0.390190170 / 7.094366733 | 4.166509281 / True | [0.23920260691411646, 0.21911341394878575] | 102.205 / 9071345664 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 66 | 0.367977732 / 6.690504222 | 3.839564724 / True | [0.24623215728715694, 0.17611996238051392] | 78.726 / 7303249920 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 100 | 0.329243033 / 5.986236956 | 4.125901609 / True | [0.24062138170923844, 0.21322779528220115] | 59.273 / 5627609088 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 1 | 125 | 0.337028495 / 6.127790819 | 4.378463153 / True | [0.28007792971773404, 0.20155301710794868] | 55.947 / 5214552064 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 33 | 0.348815953 / 6.342108243 | 1.805007981 / True | [0.1020904186180294, 0.0964604593223694] | 157.750 / 9140600832 |
| Tc8d5h {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"} | 2 | 66 | 0.335505360 / 6.100097448 | 1.948132496 / True | [0.11402409950054324, 0.10027047505156537] | 97.756 / 7303102464 |

## Board summary (refined survivors only)
| Flop | Observed winner / runner-up | Gap BB / combined proxy BB | Diagnostic | Total wall seconds |
|---|---|---|---|---|
| 3d3c2c | 33 / 66 | 0.022655990 / 0.412616296 | 区別できない | 715.035 |
| 7c4d3h | 66 / 33 | 0.039765477 / 0.425732573 | 区別できない | 866.266 |
Mixed-precision apparent highest for 7c4d3h: 100%; not a refined winner.
| 7c6d5d | 66 / 33 | 0.033535036 / 0.421580956 | 区別できない | 832.117 |
Mixed-precision apparent highest for 7c6d5d: 100%; not a refined winner.
| 8c5d4h | 66 / 100 | 0.004640101 / 0.408039451 | 区別できない | 766.498 |
| 8c6c5d | 66 / 33 | 0.001985714 / 0.398447514 | 区別できない | 770.764 |
Mixed-precision apparent highest for 8c6c5d: 100%; not a refined winner.
| 9c8d4d | 33 / 66 | 0.023597414 / 0.401399041 | 区別できない | 703.142 |
| Tc6d2c | 33 / 66 | 0.038500686 / 0.426963206 | 区別できない | 548.481 |
| Tc8d5h | 33 / 66 | 0.013310594 / 0.412845452 | 区別できない | 551.656 |
| Jc5d4d | incomplete | — | 区別できない | 0.000 |
| Jc7d6c | incomplete | — | 区別できない | 0.000 |
| Jc8d4h | incomplete | — | 区別できない | 0.000 |
| Jc9d2h | incomplete | — | 区別できない | 0.000 |
| QcTd8d | incomplete | — | 区別できない | 0.000 |
| Kc3d2h | incomplete | — | 区別できない | 0.000 |
| KcTd8d | incomplete | — | 区別できない | 0.000 |
| KcJc8d | incomplete | — | 区別できない | 0.000 |
| Ac9d4c | incomplete | — | 区別できない | 0.000 |
| AcTd9h | incomplete | — | 区別できない | 0.000 |
| AcJd2d | incomplete | — | 区別できない | 0.000 |
| AcKdJc | incomplete | — | 区別できない | 0.000 |

## Gaps and cautious grouped trends
- 3d3c2c vs 66%: gap 0.022655990 BB (0.411927091% pot), combined proxy 0.412616296 BB; indistinguishable=True; eliminated/unrefined=False.
- 3d3c2c vs 100%: gap 0.039303296 BB (0.714605377% pot), combined proxy 0.682970737 BB; indistinguishable=True; eliminated/unrefined=True.
- 3d3c2c vs 125%: gap 0.051027506 BB (0.927772838% pot), combined proxy 0.650546942 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c4d3h vs 100%: gap 0.016706280 BB (0.303750554% pot), combined proxy 0.642928502 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c4d3h vs 125%: gap 0.007093531 BB (0.128973289% pot), combined proxy 0.652579801 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c4d3h vs 33%: gap 0.039765477 BB (0.723008681% pot), combined proxy 0.425732573 BB; indistinguishable=True; eliminated/unrefined=False.
- 7c6d5d vs 100%: gap 0.019083118 BB (0.346965782% pot), combined proxy 0.737088410 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c6d5d vs 125%: gap 0.030815826 BB (0.560287747% pot), combined proxy 0.663200809 BB; indistinguishable=True; eliminated/unrefined=True.
- 7c6d5d vs 33%: gap 0.033535036 BB (0.609727932% pot), combined proxy 0.421580956 BB; indistinguishable=True; eliminated/unrefined=False.
- 8c5d4h vs 100%: gap 0.004640101 BB (0.084365478% pot), combined proxy 0.408039451 BB; indistinguishable=True; eliminated/unrefined=False.
- 8c5d4h vs 33%: gap 0.008745365 BB (0.159006628% pot), combined proxy 0.631064046 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c5d4h vs 125%: gap 0.015811927 BB (0.287489587% pot), combined proxy 0.517494629 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c6c5d vs 100%: gap 0.030545157 BB (0.555366492% pot), combined proxy 0.682663957 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c6c5d vs 125%: gap 0.001424735 BB (0.025904275% pot), combined proxy 0.640001342 BB; indistinguishable=True; eliminated/unrefined=True.
- 8c6c5d vs 33%: gap 0.001985714 BB (0.036103883% pot), combined proxy 0.398447514 BB; indistinguishable=True; eliminated/unrefined=False.
- 9c8d4d vs 66%: gap 0.023597414 BB (0.429043885% pot), combined proxy 0.401399041 BB; indistinguishable=True; eliminated/unrefined=False.
- 9c8d4d vs 100%: gap 0.033886902 BB (0.616125495% pot), combined proxy 0.673243818 BB; indistinguishable=True; eliminated/unrefined=True.
- 9c8d4d vs 125%: gap 0.038053658 BB (0.691884682% pot), combined proxy 0.670855837 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc6d2c vs 66%: gap 0.038500686 BB (0.700012467% pot), combined proxy 0.426963206 BB; indistinguishable=True; eliminated/unrefined=False.
- Tc6d2c vs 125%: gap 0.052372022 BB (0.952218587% pot), combined proxy 0.702087367 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc6d2c vs 100%: gap 0.060270413 BB (1.095825682% pot), combined proxy 0.728038316 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc8d5h vs 125%: gap 0.011787458 BB (0.214317424% pot), combined proxy 0.680181825 BB; indistinguishable=True; eliminated/unrefined=True.
- Tc8d5h vs 66%: gap 0.013310594 BB (0.242010795% pot), combined proxy 0.412845452 BB; indistinguishable=True; eliminated/unrefined=False.
- Tc8d5h vs 100%: gap 0.019572921 BB (0.355871287% pot), combined proxy 0.652400055 BB; indistinguishable=True; eliminated/unrefined=True.
- Features {"connectivity": "Gapped", "height": "Low", "pairing": "Paired", "suits": "TwoTone"}: 3d3c2c: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "Rainbow"}: 7c4d3h: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "Low", "pairing": "Unpaired", "suits": "TwoTone"}: 7c6d5d: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"}: 8c5d4h: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Connected", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"}: 8c6c5d: observed 66%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "TwoTone"}: 9c8d4d: observed 33%; Tc6d2c: observed 33%. Descriptive only; no learned rule or optimality inference.
- Features {"connectivity": "Gapped", "height": "TenEight", "pairing": "Unpaired", "suits": "Rainbow"}: Tc8d5h: observed 33%. Descriptive only; no learned rule or optimality inference.

## Incomplete boards
Jc5d4d, Jc7d6c, Jc8d4h, Jc9d2h, QcTd8d, Kc3d2h, KcTd8d, KcJc8d, Ac9d4c, AcTd9h, AcJd2d, AcKdJc

## Reproducibility
Manifest includes binary, source, configuration and original input-file hashes (no range contents). Serial child watchdog kills and reaps; checkpoint and failed attempts retained.
```json
{
  "identity": "0812bd6cd301281b0089f4fd0bb5b531ecb85608a7280da47af14ebe25a07154",
  "provenance": {
    "hashes": {
      "/Users/yota/Projects/Products/ReysonGTO/target/release/cli": "e9057e689f942866efc69cfcd6102e081e46f42ad29a0c626aa0a4618bc083a0",
      "/Users/yota/Projects/Products/ReysonGTO/target/release/sizing": "9b5409184b32b84afe4ff7bf9a5cad4a14c7e5ffb319e4e7b7c86db705eb7a33",
      "/Users/yota/Projects/Products/ReysonGTO/data/ranges/bb_call.json": "86cd2ea0f2e74e1fce53737363771fa8ccfc480c5e14fd6f9de7a1d34fc2b804",
      "/Users/yota/Projects/Products/ReysonGTO/data/ranges/btn_open.json": "f7feca455cdbb27459f2e0ada93d0bf4f470d0fd335cbabc52c15be6635015ac",
      "/Users/yota/Projects/Products/ReysonGTO/Cargo.lock": "f3bac6b35268d1c4cfa09881b29c01cc952ee5b188e064f6e8ae59e7824db987",
      "/Users/yota/Projects/Products/ReysonGTO/Cargo.toml": "2be11c8c693cfdc5d24cd647a0dedb4fafdebf5050440755ec64421bdf99704d",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cards/Cargo.toml": "fbf282f031c283bd16853ec421001e9b922f89527285fd5f4fe0e050ab1aa7ee",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cards/src/lib.rs": "51b133746393f701e07d130c6ae3a4dc5371beb4a34c7e063b662a389f00f02f",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cfr/Cargo.toml": "3ea7aa00cf94a9586a614131f5e06d6e9af4f0e49a91074e187f27e2ecdc0938",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cfr/src/diagnostics.rs": "7e6db6158667825ee98628c64dd7ecd3dfd03a23269b3a3fd9b3bea7160cbba3",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cfr/src/lib.rs": "6856671c21de60e8654fbdcfc84b6fe9068c408b0066fc272329eb3cd8602517",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cfr/tests/no_alloc.rs": "479389a23f9976c797f6cd99f74122f720e6222ceaaac8e9adfa5640f41e29e3",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cfr/tests/public_properties.rs": "ceadd7cd68a74aaeb052d0cc2a48694f851479de95528b5cfcc08ff5980c339d",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cfr/tests/sizing_invariance.rs": "820e1ac96cdc96c8a4a9049ff8faef5f287bfb3ba2728b48ba23918d9d10fb35",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cli/Cargo.toml": "2dd21aa35d2ff17af90372ca701d29b5bf4be3d0dd0a3cdb8c3f2019c8a2da90",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cli/src/compare.rs": "08a7e2fef4003130b8ccd81303dbfec4b93d74addd6f9d23606689d1e7aa0112",
      "/Users/yota/Projects/Products/ReysonGTO/crates/cli/src/main.rs": "201e159cd279bb2a5a3839f81b2aa208a7893361955090f6f6d85c83cf822d39",
      "/Users/yota/Projects/Products/ReysonGTO/crates/ranges/Cargo.toml": "a6a65c84000f0be6649e2635f781ba59970f4236a0e25893649252251a787bee",
      "/Users/yota/Projects/Products/ReysonGTO/crates/ranges/src/lib.rs": "d324941c0a2176e7ed9d3df3f2adf37ffb695f5d60c16d371aeed239e74f9dc4",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/Cargo.toml": "39b61794ccef5e707abb074dc5497f2861985914e63af0331c4fe14b74d9cf9c",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/src/bin/sizing.rs": "6c9af3c7f9928c3ea5e4adb4d530655d8fd8b337f832442a91172e99c7b98095",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/src/compare.rs": "1f3c92e8c863bb863ac8e1b04a9573e6ca4ba042bdb35db50eb03df7a23e68b7",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/src/features.rs": "e5dec49716d692b239d8c7eb0ec854066fb45640fd9aa67b03ebb999f1dc7362",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/src/flops.rs": "8b6985b9a80628f9007214d43ed0b55b12ef47ccac2e6b5fd0226ecc44470088",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/src/lib.rs": "a89d96c1d902e5d2fa32969e31551cad407b30342770d9521f6fe4ff3b948d57",
      "/Users/yota/Projects/Products/ReysonGTO/crates/sizing/src/rules.rs": "29d3d90974644eb5b7a63b6f8241762b36c1390d8788e74eaf0fb3ee44008a3d",
      "/Users/yota/Projects/Products/ReysonGTO/crates/solver/Cargo.toml": "98975d5bcf234f448cbec39d426ac85973e9c09fdeca70259822d6ceee8fa7af",
      "/Users/yota/Projects/Products/ReysonGTO/crates/solver/src/lib.rs": "5ca64bf4865f30a810fdedb6a60be9aad3b53ad55806bea67723d368674fea32",
      "/Users/yota/Projects/Products/ReysonGTO/crates/tree/Cargo.toml": "046515452c1bb777f610bbba4dbb9f829151387ae81900aff6b58b5ad747ecbf",
      "/Users/yota/Projects/Products/ReysonGTO/crates/tree/examples/m2_tree_audit.rs": "55311ae25a8d9a1482f6c617b8e4134e7aebe4d89d42a31d1dd518e379158384",
      "/Users/yota/Projects/Products/ReysonGTO/crates/tree/src/lib.rs": "f35acb7c7994733cbf9bd3639fde223821e3fef5ff3b52df9ad5ec7d72326388",
      "/Users/yota/Projects/Products/ReysonGTO/tools/m3_bench.py": "0915e09092794d99602de510e1f2144afa2453c60ad68131ae3f62fb801982f3",
      "/Users/yota/Projects/Products/ReysonGTO/tools/m4_trial.py": "b037a0fd646f22436738b90fbbe73d619ac04d95340b1c119c9a1c7baedc7e7f"
    },
    "config": {
      "threads": 10,
      "memory_mb": 16384,
      "max_iters": 10000,
      "sizes": [
        33,
        66,
        100,
        125
      ],
      "targets": [
        5,
        2
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
    "/Users/yota/Projects/Products/ReysonGTO/target/release/sizing",
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
      "/Users/yota/Projects/Products/ReysonGTO/target/release/sizing",
      "select",
      "--training",
      "20",
      "--validation",
      "0",
      "--seed",
      "20261015"
    ],
    "solver_pid": 580,
    "exit_code": 0,
    "child_exit_code": 0,
    "timed_out": false,
    "wall_seconds": 0.11206666700309142,
    "user_cpu_seconds": 0.002828,
    "system_cpu_seconds": 0.0008719999999999999,
    "maximum_resident_set_bytes": 2277376,
    "major_page_faults": 0,
    "swaps": 0,
    "source": "os.wait4(exact solver PID); excludes prior solves and manifest commands",
    "platform": "Darwin"
  },
  "created_utc": "2026-10-14T22:44:09.978542+00:00",
  "budget_deadline_unix": 1792046649.8626912
}
```
