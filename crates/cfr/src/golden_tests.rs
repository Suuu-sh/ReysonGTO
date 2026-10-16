use super::*;
use cards::Combo;
use ranges::WeightedCombo;
use tree::{Action, SizeRule, TreeConfig};

// Golden bits from origin/main e4f1b31d5405a8012e80b5710bf27eb90935f3f1, instrumented source copy only.
// Debug capture: nice -n 19 cargo test -p cfr --lib golden_capture::capture_origin_main_golden -j4 -- --nocapture.
struct Golden {
    algorithm: Algorithm,
    storage_mode: StorageMode,
    slabs: [u64; 2],
    metrics: [u64; 6],
    strategies: [&'static [u32]; 3],
}

// Both worker counts independently produced these same constants on origin/main.
const GOLDENS: [Golden; 4] = [
    Golden {
        algorithm: Algorithm::Dcfr,
        storage_mode: StorageMode::F32,
        slabs: [12724461744450669533, 9092332278451644623],
        metrics: [
            4640908404498985534,
            13867010077209131509,
            4643747571283838657,
            13863706197935703991,
            4636806642995382135,
            4621874598901568198,
        ],
        strategies: [
            &[
                1055203291, 1056964608, 1049403902, 1050412418, 1056907796, 1057530204, 1051597586,
                1057845266, 1056964608, 1060744961, 1060240703, 1056993014, 1055833416, 1059648119,
            ],
            &[
                1051410161, 1051372203, 999163263, 998245243, 1055946004, 1057586034, 996503313,
                1053006661, 1051372203, 1044671089, 1053400225, 1050222325, 1049310653, 1043443106,
                1049699787, 1051372203, 1062062425, 1058681256, 1047320557, 1044620991, 1062383400,
            ],
            &[
                1065302171, 1051372203, 999766532, 1050675158, 0, 1009342500, 0, 986146025,
                1051372203, 999766532, 1044540569, 0, 1009342500, 0, 986146025, 1051372203,
                1065198360, 1056883165, 0, 1065006399, 0,
            ],
        ],
    },
    Golden {
        algorithm: Algorithm::Dcfr,
        storage_mode: StorageMode::I16,
        slabs: [584945466633960078, 8552123151378687421],
        metrics: [
            4640948998616719642,
            13867016533913641774,
            4643751443732417995,
            13863820625292433552,
            4636672279827097672,
            4621767108366940627,
        ],
        strategies: [
            &[
                1055779114, 1056964608, 1049822447, 1050899173, 1056741963, 1057506256, 1051712124,
                1057557355, 1056964608, 1060535688, 1059997325, 1057075931, 1055881312, 1059590850,
            ],
            &[
                1050565515, 1051372203, 998776926, 997516567, 1055304877, 1056778794, 996385096,
                1054348230, 1051372203, 1044534094, 1055299152, 1050727830, 1051193472, 1043328449,
                1049202863, 1051372203, 1062099692, 1057734643, 1047591803, 1043712683, 1062412527,
            ],
            &[
                1065304206, 1051372203, 999269371, 1051286983, 0, 1009364008, 0, 985625065,
                1051372203, 999269371, 1045252431, 0, 1009364008, 0, 985625065, 1051372203,
                1065206128, 1055915409, 0, 1065005727, 0,
            ],
        ],
    },
    Golden {
        algorithm: Algorithm::CfrPlus,
        storage_mode: StorageMode::F32,
        slabs: [10750073626145924242, 13892922374106141478],
        metrics: [
            4641229933380484922,
            13867264937688377714,
            4644506565370480445,
            13862689553098262866,
            4639118542137355369,
            4624052858971742376,
        ],
        strategies: [
            &[
                1057904881, 1056964608, 1057630482, 1056655006, 1058037909, 1053694281, 1055460482,
                1055084062, 1056964608, 1055632859, 1057119409, 1054818007, 1058599771, 1057716671,
            ],
            &[
                1049285365, 1051372203, 1010086947, 1031835413, 1055227667, 1056891300, 1012285855,
                1045270308, 1051372203, 1042932544, 1052328346, 1049686921, 1050798818, 1045988582,
                1057436349, 1051372203, 1062384735, 1058229584, 1049202021, 1044276979, 1061586368,
            ],
            &[
                1065137477, 1051372203, 1011260931, 1053639913, 0, 1032507156, 0, 1003663062,
                1051372203, 1011260931, 1048356652, 0, 1032507156, 0, 1003663062, 1051372203,
                1064946448, 1052010369, 0, 1063078971, 0,
            ],
        ],
    },
    Golden {
        algorithm: Algorithm::CfrPlus,
        storage_mode: StorageMode::I16,
        slabs: [15073784609761084079, 16646713667900692500],
        metrics: [
            4641230353440220447,
            13867265184389478073,
            4644508762445170087,
            13862690018664602122,
            4639120543100107980,
            4624056060512146554,
        ],
        strategies: [
            &[
                1057901366, 1056964608, 1057630759, 1056655318, 1058035976, 1053693355, 1055462212,
                1055091091, 1056964608, 1055632306, 1057119253, 1054821872, 1058600234, 1057715806,
            ],
            &[
                1049300881, 1051372203, 1010084614, 1031833542, 1055247395, 1056903012, 1012282492,
                1045302068, 1051372203, 1042932743, 1052321932, 1049709716, 1050812378, 1045986819,
                1057420651, 1051372203, 1062384722, 1058233025, 1049159497, 1044226435, 1061586861,
            ],
            &[
                1065136899, 1051372203, 1011288218, 1053638379, 0, 1032525566, 0, 1003700043,
                1051372203, 1011288218, 1048358981, 0, 1032525566, 0, 1003700043, 1051372203,
                1064945595, 1052010738, 0, 1063074369, 0,
            ],
        ],
    },
];

fn fixture() -> (GameTree, [DenseRange; 2], [usize; 3]) {
    let board: Vec<_> = "Qs Jh 2h 3c"
        .split_whitespace()
        .map(|c| c.parse::<Card>().unwrap().0)
        .collect();
    let tree = GameTree::build(
        board,
        TreeConfig {
            starting_pot: 1000,
            effective_stack: 2000,
            bet_size: SizeRule::Fixed(0.5),
            raise_multiplier: 2.0,
            allin_merge_enabled: false,
            allow_raises: true,
            rake_rate: 0.05,
            rake_cap: 150,
            ..TreeConfig::default()
        },
    )
    .unwrap();
    let range = |hands: &[(&str, f64)]| {
        let mut combos: Vec<_> = hands
            .iter()
            .map(|&(c, weight)| WeightedCombo {
                combo: c.parse::<Combo>().unwrap(),
                weight,
            })
            .collect();
        combos.sort_by_key(|c| c.combo.id());
        DenseRange {
            player: None,
            combos,
        }
    };
    let ranges = [
        range(&[
            ("AcAh", 1.0),
            ("AcKd", 0.3),
            ("AdKs", 0.8),
            ("KcKh", 0.5),
            ("TcTh", 0.0),
            ("9c9d", 0.65),
            ("QcJd", 0.1),
        ]),
        range(&[
            ("AcAh", 0.2),
            ("AcKd", 0.7),
            ("AdKd", 1.0),
            ("KcKh", 0.25),
            ("9c9d", 0.0),
            ("ThTs", 0.4),
        ]),
    ];
    let has_raise = |id: usize| matches!(&tree.nodes[id].kind, NodeKind::Decision { actions, .. } if actions.iter().any(|a| matches!(a, Action::Raise(_))));
    let facing_raise = (0..tree.nodes.len())
        .find(|&id| tree.nodes[id].board.len() == 4 && has_raise(id))
        .unwrap();
    let river_blocked = (0..tree.nodes.len())
        .find(|&id| {
            tree.nodes[id].board.last() == Some(&"Ac".parse::<Card>().unwrap().0) && has_raise(id)
        })
        .unwrap();
    let selected = [tree.root, facing_raise, river_blocked];
    assert_eq!(tree.nodes.len(), 3429);
    assert_eq!(selected, [0, 1011, 930]);
    assert!(tree
        .nodes
        .iter()
        .any(|n| matches!(n.kind, NodeKind::Chance { .. })));
    // Construct DenseRange directly: Range::from_combos would discard zero-weight slots.
    assert!(ranges
        .iter()
        .all(|r| r.combos.iter().any(|c| c.weight == 0.0)));
    (tree, ranges, selected)
}
fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash = (*hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
}
fn hash_scales(hash: &mut u64, state: &State<f64>, which: usize) {
    hash_bytes(hash, &state.scales[which].to_bits().to_le_bytes());
    for child in &state.children {
        hash_scales(hash, child, which);
    }
}
// FNV-1a over raw little-endian slab bytes; I16 also includes the matching
// scale's f32 bits for every state in deterministic depth-first child order.
fn slab_hash(slab: &Slab, state: &State<f64>, which: usize) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    match slab {
        Slab::F32(values) => {
            for value in values {
                hash_bytes(&mut hash, &value.to_bits().to_le_bytes());
            }
        }
        Slab::I16(values) => {
            for value in values {
                hash_bytes(&mut hash, &value.to_le_bytes());
            }
            hash_scales(&mut hash, state, which);
        }
    }
    hash
}
fn metric_bits(m: Metrics) -> [u64; 6] {
    [
        m.average_ev[0],
        m.average_ev[1],
        m.best_response_ev[0],
        m.best_response_ev[1],
        m.exploitability_chips,
        m.exploitability_pct,
    ]
    .map(f64::to_bits)
}
#[test]
fn default_f64_matches_origin_main_golden_bits() {
    let (tree, ranges, nodes) = fixture();
    for golden in &GOLDENS {
        for threads in [1, 4] {
            let config = EngineConfig {
                algorithm: golden.algorithm,
                storage_mode: golden.storage_mode,
                threads,
                ..EngineConfig::default()
            };
            // Deliberately omit scratch_precision so this protects the public default.
            assert_eq!(config.scratch_precision, ScratchPrecision::F64);
            let mut engine = Engine::new(tree.clone(), ranges.clone(), config).unwrap();
            for _ in 0..7 {
                engine.iterate();
            }
            let EngineInner::F64(core) = &engine.inner else {
                panic!("default scratch must construct the f64 core");
            };
            assert!(core.initial_reach.iter().all(|r| r.contains(&0.0)));
            assert_eq!(
                [
                    slab_hash(&core.regret, &core.state, 0),
                    slab_hash(&core.average, &core.state, 1)
                ],
                golden.slabs,
                "regret/average slabs: {:?} {:?} threads={threads}",
                golden.algorithm,
                golden.storage_mode,
            );
            assert_eq!(
                metric_bits(engine.metrics()),
                golden.metrics,
                "all metric bits: {:?} {:?} threads={threads}",
                golden.algorithm,
                golden.storage_mode,
            );
            for (&node, expected) in nodes.iter().zip(golden.strategies) {
                let actual: Vec<_> = engine
                    .strategy(node)
                    .unwrap()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect();
                assert_eq!(
                    actual, expected,
                    "strategy node={node}: {:?} {:?} threads={threads}",
                    golden.algorithm, golden.storage_mode,
                );
            }
        }
    }
}
