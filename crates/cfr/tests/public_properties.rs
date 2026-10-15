//! Independent public-API checks: explicit private deals and physical runouts.
use cards::{evaluate7_naive, Card};
use cfr::{Engine, EngineConfig};
use ranges::{DenseRange, Range};
use tree::{GameTree, TreeConfig};

fn board(text: &str) -> Vec<Card> {
    (0..text.len() / 2)
        .map(|i| text[2 * i..2 * i + 2].parse().unwrap())
        .collect()
}
fn range(entries: &[(&str, f64)], board: &[Card]) -> DenseRange {
    Range::from_combos(
        entries
            .iter()
            .map(|(s, w)| (s.parse().unwrap(), *w))
            .collect(),
    )
    .unwrap()
    .blocked(board)
    .unwrap()
}

// Deliberately independent of engine reach-vector normalization and chance weights.
fn exact_showdown(board: &[Card], ranges: &[DenseRange; 2], config: &TreeConfig) -> [f64; 2] {
    let mut ev = [0.0; 2];
    let mut joint_mass = 0.0;
    let public_mask = board.iter().fold(0, |m, c| m | c.mask());
    for oop in &ranges[0].combos {
        for ip in &ranges[1].combos {
            if oop.combo.mask() & ip.combo.mask() != 0 {
                continue;
            }
            let weight = oop.weight * ip.weight;
            joint_mass += weight;
            let used = public_mask | oop.combo.mask() | ip.combo.mask();
            let remaining: Vec<_> = (0..52).map(Card).filter(|c| used & c.mask() == 0).collect();
            let mut per_deal = [0.0; 2];
            let mut runout_count = 0usize;
            let mut observe = |runout: &[Card]| {
                let mut completed = board.to_vec();
                completed.extend_from_slice(runout);
                let strengths = [oop, ip].map(|entry| {
                    let hand = [
                        completed[0],
                        completed[1],
                        completed[2],
                        completed[3],
                        completed[4],
                        entry.combo.cards[0],
                        entry.combo.cards[1],
                    ];
                    evaluate7_naive(&hand)
                });
                let distributable = config.starting_pot as f64 - config.rake(config.starting_pot);
                let receipt = match strengths[0].cmp(&strengths[1]) {
                    std::cmp::Ordering::Greater => [distributable, 0.0],
                    std::cmp::Ordering::Less => [0.0, distributable],
                    std::cmp::Ordering::Equal => [distributable / 2.0; 2],
                };
                for p in 0..2 {
                    per_deal[p] += receipt[p] - config.starting_pot as f64 / 2.0;
                }
                runout_count += 1;
            };
            if board.len() == 4 {
                for &river in &remaining {
                    observe(&[river]);
                }
            } else {
                for a in 0..remaining.len() {
                    for b in a + 1..remaining.len() {
                        observe(&[remaining[a], remaining[b]]);
                    }
                }
            }
            for p in 0..2 {
                ev[p] += weight * per_deal[p] / runout_count as f64;
            }
        }
    }
    ev.map(|v| v / joint_mass)
}

#[test]
fn chance_blockers_rake_and_safe_symmetry_match_brute_deals() {
    for board_text in ["2h3h4h", "2h3h4h8h"] {
        let board = board(board_text);
        for asymmetric in [false, true] {
            let ranges = if asymmetric {
                [
                    range(&[("AcKd", 0.17), ("AsQs", 0.83)], &board),
                    range(&[("AcQd", 0.71), ("KsJd", 0.29)], &board),
                ]
            } else {
                [
                    range(&[("AhKh", 0.17), ("AhQh", 0.83)], &board),
                    range(&[("JhTh", 0.71), ("Kh9h", 0.29)], &board),
                ]
            };
            for rake_cap in [3_000, 100] {
                let config = TreeConfig {
                    starting_pot: 5_000,
                    effective_stack: 0,
                    rake_rate: 0.05,
                    rake_cap,
                    ..TreeConfig::default()
                };
                let expected = exact_showdown(&board, &ranges, &config);
                for suit_isomorphism in [false, true] {
                    let tree = GameTree::build(board.iter().map(|c| c.0).collect(), config.clone())
                        .unwrap();
                    let mut engine = Engine::new(
                        tree,
                        ranges.clone(),
                        EngineConfig {
                            suit_isomorphism,
                            ..EngineConfig::default()
                        },
                    )
                    .unwrap();
                    let metrics = engine.metrics();
                    for (p, expected_ev) in expected.iter().enumerate() {
                        assert!((metrics.average_ev[p]-expected_ev).abs()<1e-8,"board={board_text} asym={asymmetric} iso={suit_isomorphism} p={p}: got {:?}, expected {expected:?}",metrics.average_ev);
                        assert!((metrics.best_response_ev[p] - expected_ev).abs() < 1e-8);
                    }
                    assert!(
                        (metrics.average_ev.iter().sum::<f64>() + config.rake(config.starting_pot))
                            .abs()
                            < 1e-8
                    );
                    assert!(metrics.exploitability_chips.abs() < 1e-8);
                    if asymmetric || !suit_isomorphism {
                        assert!(!engine.isomorphism_used());
                    } else {
                        assert!(engine.isomorphism_used());
                    }
                }
            }
        }
    }
}

#[test]
fn public_engine_rejects_memory_limit_before_allocating_state() {
    let board = board("2h3h4h8h9c");
    let ranges = [
        range(&[("AhKh", 1.0)], &board),
        range(&[("JhTh", 1.0)], &board),
    ];
    let tree = GameTree::build(
        board.iter().map(|c| c.0).collect(),
        TreeConfig {
            effective_stack: 0,
            ..TreeConfig::default()
        },
    )
    .unwrap();
    let result = Engine::new(
        tree,
        ranges,
        EngineConfig {
            memory_limit_bytes: 1,
            ..EngineConfig::default()
        },
    );
    match result {
        Err(message) => assert!(message.contains("memory") && message.contains("limit")),
        Ok(_) => panic!("tiny memory limit unexpectedly accepted"),
    }
}
