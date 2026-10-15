//! Physical reference arena audit without loading any product range.
//! Amounts/settings are the copied data/benchmark/tree-preflight.json snapshot.
use tree::{GameTree, NodeKind, SizeRule, TreeConfig};
fn main() {
    let config = TreeConfig {
        starting_pot: 360_448,
        effective_stack: 6_389_760,
        oop_can_lead_flop: true,
        bet_size: SizeRule::Fixed(0.5),
        allin_merge_enabled: false,
        rake_rate: 0.0,
        rake_cap: 0,
        ..Default::default()
    };
    let t = GameTree::build_with_node_limit(vec![43, 38, 2], config, 3_000_000).unwrap();
    let (mut decisions, mut slots, mut chance, mut terminals) = (0, 0, 0, 0);
    for n in &t.nodes {
        match &n.kind {
            NodeKind::Decision { actions, .. } => {
                decisions += 1;
                slots += actions.len();
            }
            NodeKind::Chance { .. } => chance += 1,
            NodeKind::Terminal { .. } => terminals += 1,
        }
    }
    println!(
        "nodes={} decisions={} action_slots={} chance={} terminals={}",
        t.nodes.len(),
        decisions,
        slots,
        chance,
        terminals
    );
}
