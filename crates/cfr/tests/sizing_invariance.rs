//! Opt-in sizing callbacks must never allow unsafe suit aliasing.
use cards::Card;
use cfr::{Engine, EngineConfig};
use ranges::{DenseRange, Range};
use std::sync::Arc;
use tree::{GameTree, NodeKind, SizeRule, TreeConfig};

fn inputs() -> (Vec<u8>, [DenseRange; 2]) {
    let board: Vec<Card> = ["2c", "7c", "9c", "Jc"]
        .iter()
        .map(|text| text.parse().unwrap())
        .collect();
    let range = |hands: &[&str]| {
        Range::from_combos(
            hands
                .iter()
                .map(|hand| (hand.parse().unwrap(), 1.0))
                .collect(),
        )
        .unwrap()
        .blocked(&board)
        .unwrap()
    };
    (
        board.iter().map(|card| card.0).collect(),
        [range(&["AcAd", "3c4d"]), range(&["KcKd"])],
    )
}

fn engine(rule: SizeRule, use_iso: bool) -> Engine {
    let (board, ranges) = inputs();
    let tree = GameTree::build(
        board,
        TreeConfig {
            starting_pot: 1_000,
            effective_stack: 600,
            bet_size: rule,
            allow_raises: false,
            ..TreeConfig::default()
        },
    )
    .unwrap();
    Engine::new(
        tree,
        ranges,
        EngineConfig {
            suit_isomorphism: use_iso,
            ..EngineConfig::default()
        },
    )
    .unwrap()
}

fn assert_parity(a: &mut Engine, b: &mut Engine, tolerance: f64) {
    for _ in 0..64 {
        a.iterate();
        b.iterate();
    }
    let ma = a.metrics();
    let mb = b.metrics();
    for player in 0..2 {
        assert!((ma.average_ev[player] - mb.average_ev[player]).abs() <= tolerance);
        assert!((ma.best_response_ev[player] - mb.best_response_ev[player]).abs() <= tolerance);
    }
    assert!((ma.exploitability_chips - mb.exploitability_chips).abs() <= tolerance);
    for (id, node) in a.tree().nodes.iter().enumerate() {
        if matches!(node.kind, NodeKind::Decision { .. }) {
            for (x, y) in a.strategy(id).unwrap().iter().zip(b.strategy(id).unwrap()) {
                assert!(
                    (*x as f64 - y as f64).abs() <= tolerance,
                    "node {id}: {x} vs {y}"
                );
            }
        }
    }
}

#[test]
fn fixed_and_declared_invariant_callbacks_have_identical_numerics() {
    let mut fixed = engine(SizeRule::Fixed(0.25), true);
    let mut callback = engine(SizeRule::SuitInvariant(Arc::new(|_| 0.25)), true);
    assert!(fixed.isomorphism_used() && callback.isomorphism_used());
    assert_eq!(
        fixed.merged_chance_children(),
        callback.merged_chance_children()
    );
    assert_parity(&mut fixed, &mut callback, 0.0);
}

#[test]
fn invariant_board_dependent_callback_matches_unmerged_tree() {
    let rule = SizeRule::SuitInvariant(Arc::new(|c| {
        if c.board.iter().all(|card| card % 4 == c.board[0] % 4) {
            0.25
        } else {
            0.5
        }
    }));
    let mut reduced = engine(rule.clone(), true);
    let mut plain = engine(rule, false);
    assert!(reduced.isomorphism_used());
    assert_parity(&mut reduced, &mut plain, 0.002);
}

#[test]
fn invalid_declaration_disables_iso_but_preserves_rule_and_results() {
    let rule = SizeRule::SuitInvariant(Arc::new(|c| {
        if c.board.last().unwrap() % 4 == 2 {
            0.25
        } else {
            0.5
        }
    }));
    assert!(rule.check_suit_invariance(0x726579736f6e, 64).is_err());
    let mut checked = engine(rule.clone(), true);
    let mut plain = engine(rule, false);
    assert!(!checked.isomorphism_used());
    assert_eq!(checked.merged_chance_children(), 0);
    assert_parity(&mut checked, &mut plain, 0.0);
}

#[test]
fn structural_fallback_resets_partial_aliases_when_sampling_misses_bad_rule() {
    // The random boards deliberately miss this specific prefix. The first
    // affected river rank comes late, so aliases have already been written.
    let (prefix, _) = inputs();
    let rule = SizeRule::SuitInvariant(Arc::new(move |c| {
        if c.board.len() == 5 && c.board[..4] == prefix && c.board[4] == 50 {
            0.25
        } else {
            0.5
        }
    }));
    assert!(rule.check_suit_invariance(0x726579736f6e, 64).is_ok());
    let mut checked = engine(rule.clone(), true);
    let mut plain = engine(rule, false);
    assert!(!checked.isomorphism_used());
    assert_eq!(checked.merged_chance_children(), 0);
    assert_parity(&mut checked, &mut plain, 0.0);
}
