//! HU alternating DCFR with dense physical-combo regret/reach vectors.
//! Class equity is an abstraction; illegal private-card pairs are excluded exactly.
pub use crate::realization::RealizationConfig;
use crate::{
    equity::{EquityMetadata, EquityTable, HandClasses, CLASS_COUNT},
    tree::{Action, Node, PreflopTree, TerminalKind},
};
use cards::Combo;
use serde::Serialize;
use std::time::Instant;

pub const COMBO_COUNT: usize = 1326;
pub const LEGAL_OPPONENT_COMBOS: usize = 1225;
pub const LEGAL_ORDERED_DEALS: usize = COMBO_COUNT * LEGAL_OPPONENT_COMBOS;

struct NodeState {
    regret: Vec<f64>,
    average: Vec<f64>,
}
struct TerminalKernel {
    /// Net payoff before chance weighting; player, hero class, opponent class.
    payoff: [Vec<f64>; 2],
    /// Same payoffs times exact legal-combo counts / 1225.
    symmetric: [Vec<f64>; 2],
}

#[derive(Debug, Serialize)]
pub struct ClassStrategy {
    pub hand: String,
    pub combo_count: usize,
    pub own_reach_probability: f64,
    pub frequencies: Vec<f64>,
}
#[derive(Debug, Serialize)]
pub struct NodeStrategy {
    pub node_id: usize,
    pub actor: usize,
    pub history: Vec<String>,
    pub actions: Vec<String>,
    /// Joint probability, including exact private-card compatibility.
    pub reach_probability: f64,
    pub conditional_action_frequencies: Vec<f64>,
    pub classes: Vec<ClassStrategy>,
}
#[derive(Debug, Serialize, Default)]
pub struct RangeSummary {
    pub btn_open: Option<f64>,
    pub bb_three_bet_given_btn_open: Option<f64>,
    pub bb_call_given_btn_open: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct SolveReport {
    pub iterations: u64,
    pub training_seconds: f64,
    pub report_seconds: f64,
    pub profile_ev_bb: [f64; 2],
    pub best_response_ev_bb: [f64; 2],
    pub best_response_gain_bb: [f64; 2],
    /// Sum of unilateral improvement in this general-sum abstract game.
    pub nash_conv_bb: f64,
    /// Merely half of NashConv, not zero-sum exploitability.
    pub half_gap_bb: f64,
    pub legal_ordered_deals: usize,
    pub equity_metadata: EquityMetadata,
    pub realization: RealizationConfig,
    pub summary: RangeSummary,
    pub nodes: Vec<NodeStrategy>,
}

pub struct PreflopSolver {
    tree: PreflopTree,
    classes: HandClasses,
    states: Vec<Option<NodeState>>,
    kernels: Vec<Option<TerminalKernel>>,
    iterations: u64,
    training_seconds: f64,
    equity_metadata: EquityMetadata,
    realization: RealizationConfig,
}
impl PreflopSolver {
    pub fn new(
        tree: PreflopTree,
        equity: EquityTable,
        realization: RealizationConfig,
    ) -> Result<Self, String> {
        if tree.config.players != 2 {
            return Err("P3 CFR supports exactly two players; P4 is not implemented".into());
        }
        tree.config.validate().map_err(|e| e.to_string())?;
        equity.validate()?;
        realization.validate()?;
        let classes = HandClasses::new();
        let mut legal_counts = vec![0.0; CLASS_COUNT * CLASS_COUNT];
        for a in 0..CLASS_COUNT {
            let hero = classes.combos[a][0];
            for b in 0..CLASS_COUNT {
                legal_counts[a * CLASS_COUNT + b] = classes.combos[b]
                    .iter()
                    .filter(|opponent| hero.mask() & opponent.mask() == 0)
                    .count() as f64;
            }
        }
        let mut states = Vec::with_capacity(tree.nodes.len());
        let mut kernels = Vec::with_capacity(tree.nodes.len());
        for node in &tree.nodes {
            match node {
                Node::Decision {
                    actor,
                    actions,
                    children,
                } => {
                    if *actor > 1
                        || actions.is_empty()
                        || actions.len() != children.len()
                        || children.iter().any(|&id| id >= tree.nodes.len())
                    {
                        return Err("invalid HU decision node".into());
                    }
                    states.push(Some(NodeState {
                        regret: vec![0.0; actions.len() * COMBO_COUNT],
                        average: vec![0.0; actions.len() * COMBO_COUNT],
                    }));
                    kernels.push(None);
                }
                Node::Terminal(t) => {
                    if t.contributions.len() != 2 {
                        return Err("invalid HU terminal contributions".into());
                    }
                    let pot = t.pot_bb();
                    // Fold terminals never pay rake, regardless of external flags.
                    let available = pot
                        - if t.kind == TerminalKind::Fold {
                            0.0
                        } else {
                            tree.config.rake.amount_bb(pot, true)
                        };
                    let mut payoff = [
                        vec![0.0; CLASS_COUNT * CLASS_COUNT],
                        vec![0.0; CLASS_COUNT * CLASS_COUNT],
                    ];
                    let mut symmetric = payoff.clone();
                    for p in 0..2 {
                        for a in 0..CLASS_COUNT {
                            let realization = if t.kind == TerminalKind::Flop {
                                realization.get(
                                    t.stage,
                                    tree.config.is_ip(p, 1 - p),
                                    &classes.names[a],
                                )
                            } else {
                                1.0
                            };
                            for b in 0..CLASS_COUNT {
                                let i = a * CLASS_COUNT + b;
                                payoff[p][i] = if t.kind == TerminalKind::Fold {
                                    if t.winner == Some(p) {
                                        pot - t.contributions[p]
                                    } else {
                                        -t.contributions[p]
                                    }
                                } else {
                                    equity.get(a, b) * available * realization - t.contributions[p]
                                };
                                symmetric[p][i] =
                                    payoff[p][i] * legal_counts[i] / LEGAL_OPPONENT_COMBOS as f64;
                            }
                        }
                    }
                    states.push(None);
                    kernels.push(Some(TerminalKernel { payoff, symmetric }));
                }
            }
        }
        if tree.root >= tree.nodes.len() {
            return Err("invalid root".into());
        }
        Ok(Self {
            tree,
            classes,
            states,
            kernels,
            iterations: 0,
            training_seconds: 0.0,
            equity_metadata: equity.metadata,
            realization,
        })
    }
    pub fn iterations(&self) -> u64 {
        self.iterations
    }
    pub fn tree(&self) -> &PreflopTree {
        &self.tree
    }
    pub fn run(&mut self, iterations: u64) {
        for _ in 0..iterations {
            self.step();
        }
    }
    pub fn step(&mut self) {
        let start = Instant::now();
        for p in 0..2 {
            for (node, state) in self.tree.nodes.iter().zip(&mut self.states) {
                if let (Node::Decision { actor, .. }, Some(state)) = (node, state) {
                    if *actor == p {
                        discount_state(state, self.iterations + 1);
                    }
                }
            }
            let strategy = self.strategies(false);
            traverse(
                &self.tree,
                &self.classes,
                &self.kernels,
                &strategy,
                &mut self.states,
                self.tree.root,
                p,
                &vec![1.0; COMBO_COUNT],
                &vec![1.0; COMBO_COUNT],
            );
        }
        self.iterations += 1;
        self.training_seconds += start.elapsed().as_secs_f64();
    }
    fn strategies(&self, average: bool) -> Vec<Vec<f64>> {
        self.states
            .iter()
            .map(|state| match state {
                None => Vec::new(),
                Some(s) => {
                    let source = if average { &s.average } else { &s.regret };
                    let count = source.len() / COMBO_COUNT;
                    let mut out = vec![0.0; source.len()];
                    for c in 0..COMBO_COUNT {
                        let sum: f64 = (0..count)
                            .map(|a| source[a * COMBO_COUNT + c].max(0.0))
                            .sum();
                        for a in 0..count {
                            out[a * COMBO_COUNT + c] = if sum > 0.0 {
                                source[a * COMBO_COUNT + c].max(0.0) / sum
                            } else {
                                1.0 / count as f64
                            };
                        }
                    }
                    out
                }
            })
            .collect()
    }
    pub fn report(&self) -> SolveReport {
        let start = Instant::now();
        let strategy = self.strategies(true);
        let mut profile = [0.0; 2];
        let mut br = [0.0; 2];
        for p in 0..2 {
            for (best_response, target) in [(false, &mut profile[p]), (true, &mut br[p])] {
                *target = evaluate(
                    &self.tree,
                    &self.classes,
                    &self.kernels,
                    &strategy,
                    self.tree.root,
                    p,
                    &vec![1.0; COMBO_COUNT],
                    best_response,
                )
                .iter()
                .sum::<f64>()
                    / COMBO_COUNT as f64;
            }
        }
        let gains = [(br[0] - profile[0]).max(0.0), (br[1] - profile[1]).max(0.0)];
        let mut nodes = Vec::new();
        self.collect_nodes(
            self.tree.root,
            &strategy,
            [vec![1.0; COMBO_COUNT], vec![1.0; COMBO_COUNT]],
            Vec::new(),
            &mut nodes,
        );
        let mut summary = RangeSummary::default();
        if self.tree.config.positions == ["BTN", "BB"] {
            if let Some(root) = nodes.iter().find(|n| n.node_id == self.tree.root) {
                summary.btn_open = Some(
                    root.actions
                        .iter()
                        .zip(&root.conditional_action_frequencies)
                        .filter(|(a, _)| a.starts_with("raise_to_") || *a == "all_in")
                        .map(|(_, p)| p)
                        .sum(),
                );
            }
            // The configured fixed-size open has one BB response node.
            if let Some(bb) = nodes.iter().find(|n| {
                n.actor == 1 && n.history.len() == 1 && n.history[0].starts_with("raise_to_")
            }) {
                summary.bb_three_bet_given_btn_open = Some(
                    bb.actions
                        .iter()
                        .zip(&bb.conditional_action_frequencies)
                        .filter(|(a, _)| a.starts_with("raise_to_") || *a == "all_in")
                        .map(|(_, p)| p)
                        .sum(),
                );
                summary.bb_call_given_btn_open = Some(
                    bb.actions
                        .iter()
                        .zip(&bb.conditional_action_frequencies)
                        .filter(|(a, _)| *a == "call")
                        .map(|(_, p)| p)
                        .sum(),
                );
            }
        }
        SolveReport {
            iterations: self.iterations,
            training_seconds: self.training_seconds,
            report_seconds: start.elapsed().as_secs_f64(),
            profile_ev_bb: profile,
            best_response_ev_bb: br,
            best_response_gain_bb: gains,
            nash_conv_bb: gains.iter().sum(),
            half_gap_bb: gains.iter().sum::<f64>() / 2.0,
            legal_ordered_deals: LEGAL_ORDERED_DEALS,
            equity_metadata: self.equity_metadata.clone(),
            realization: self.realization.clone(),
            summary,
            nodes,
        }
    }
    fn collect_nodes(
        &self,
        id: usize,
        strategy: &[Vec<f64>],
        reach: [Vec<f64>; 2],
        history: Vec<String>,
        out: &mut Vec<NodeStrategy>,
    ) {
        if let Node::Decision {
            actor,
            actions,
            children,
        } = &self.tree.nodes[id]
        {
            let mass = compatible_mass(&reach[1 - actor]);
            let node_mass: f64 = (0..COMBO_COUNT).map(|c| reach[*actor][c] * mass[c]).sum();
            let conditional = (0..actions.len())
                .map(|a| {
                    if node_mass > 0.0 {
                        (0..COMBO_COUNT)
                            .map(|c| reach[*actor][c] * mass[c] * strategy[id][a * COMBO_COUNT + c])
                            .sum::<f64>()
                            / node_mass
                    } else {
                        0.0
                    }
                })
                .collect();
            let classes = self
                .classes
                .combos
                .iter()
                .enumerate()
                .map(|(h, combos)| ClassStrategy {
                    hand: self.classes.names[h].clone(),
                    combo_count: combos.len(),
                    own_reach_probability: combos
                        .iter()
                        .map(|c| reach[*actor][c.id()])
                        .sum::<f64>()
                        / combos.len() as f64,
                    frequencies: (0..actions.len())
                        .map(|a| {
                            combos
                                .iter()
                                .map(|c| strategy[id][a * COMBO_COUNT + c.id()])
                                .sum::<f64>()
                                / combos.len() as f64
                        })
                        .collect(),
                })
                .collect();
            out.push(NodeStrategy {
                node_id: id,
                actor: *actor,
                history: history.clone(),
                actions: actions.iter().map(action_label).collect(),
                reach_probability: node_mass / LEGAL_ORDERED_DEALS as f64,
                conditional_action_frequencies: conditional,
                classes,
            });
            for (a, &child) in children.iter().enumerate() {
                let mut next = reach.clone();
                for c in 0..COMBO_COUNT {
                    next[*actor][c] *= strategy[id][a * COMBO_COUNT + c];
                }
                let mut path = history.clone();
                path.push(action_label(&actions[a]));
                self.collect_nodes(child, strategy, next, path, out);
            }
        }
    }
}
// Before update t, apply the discount belonging to completed update t-1.
// Thus normalized average contributions have weights 1^2, 2^2, ..., t^2.
fn discount_state(state: &mut NodeState, iteration: u64) {
    let previous = (iteration - 1) as f64;
    let positive = previous.powf(1.5) / (previous.powf(1.5) + 1.0);
    let average = (previous / iteration as f64).powi(2);
    for regret in &mut state.regret {
        *regret *= if *regret > 0.0 { positive } else { 0.5 };
    }
    for sum in &mut state.average {
        *sum *= average;
    }
}

fn action_label(action: &Action) -> String {
    match action {
        Action::Fold => "fold".into(),
        Action::Check => "check".into(),
        Action::Call => "call".into(),
        Action::RaiseTo(to) => format!("raise_to_{to}"),
        Action::AllIn => "all_in".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn traverse(
    tree: &PreflopTree,
    classes: &HandClasses,
    kernels: &[Option<TerminalKernel>],
    strategy: &[Vec<f64>],
    states: &mut [Option<NodeState>],
    id: usize,
    player: usize,
    own: &[f64],
    opponent: &[f64],
) -> Vec<f64> {
    let Node::Decision {
        actor, children, ..
    } = &tree.nodes[id]
    else {
        return terminal_values(kernels[id].as_ref().unwrap(), classes, player, opponent);
    };
    let mut value = vec![0.0; COMBO_COUNT];
    let mut action_values = Vec::with_capacity(children.len());
    for (a, &child) in children.iter().enumerate() {
        let mut next = if *actor == player {
            own.to_vec()
        } else {
            opponent.to_vec()
        };
        for c in 0..COMBO_COUNT {
            next[c] *= strategy[id][a * COMBO_COUNT + c];
        }
        let child_value = if *actor == player {
            traverse(
                tree, classes, kernels, strategy, states, child, player, &next, opponent,
            )
        } else {
            traverse(
                tree, classes, kernels, strategy, states, child, player, own, &next,
            )
        };
        for c in 0..COMBO_COUNT {
            value[c] += child_value[c]
                * if *actor == player {
                    strategy[id][a * COMBO_COUNT + c]
                } else {
                    1.0
                };
        }
        action_values.push(child_value);
    }
    if *actor == player {
        let state = states[id].as_mut().unwrap();
        for (a, values) in action_values.iter().enumerate() {
            for c in 0..COMBO_COUNT {
                state.regret[a * COMBO_COUNT + c] += values[c] - value[c];
                // Own realization reach only. Opponent reach would bias averaging.
                state.average[a * COMBO_COUNT + c] += own[c] * strategy[id][a * COMBO_COUNT + c];
            }
        }
    }
    value
}
#[allow(clippy::too_many_arguments)]
fn evaluate(
    tree: &PreflopTree,
    classes: &HandClasses,
    kernels: &[Option<TerminalKernel>],
    strategy: &[Vec<f64>],
    id: usize,
    player: usize,
    opponent: &[f64],
    best_response: bool,
) -> Vec<f64> {
    let Node::Decision {
        actor, children, ..
    } = &tree.nodes[id]
    else {
        return terminal_values(kernels[id].as_ref().unwrap(), classes, player, opponent);
    };
    let maximize = *actor == player && best_response;
    let mut value = vec![if maximize { f64::NEG_INFINITY } else { 0.0 }; COMBO_COUNT];
    for (a, &child) in children.iter().enumerate() {
        let mut next = opponent.to_vec();
        if *actor != player {
            for c in 0..COMBO_COUNT {
                next[c] *= strategy[id][a * COMBO_COUNT + c];
            }
        }
        let child_value = evaluate(
            tree,
            classes,
            kernels,
            strategy,
            child,
            player,
            &next,
            best_response,
        );
        for c in 0..COMBO_COUNT {
            if maximize {
                value[c] = value[c].max(child_value[c]);
            } else {
                value[c] += child_value[c]
                    * if *actor == player {
                        strategy[id][a * COMBO_COUNT + c]
                    } else {
                        1.0
                    };
            }
        }
    }
    value
}

fn compatible_mass(reach: &[f64]) -> Vec<f64> {
    let total: f64 = reach.iter().sum();
    let mut card = [0.0; 52];
    for (id, &weight) in reach.iter().enumerate() {
        let c = Combo::from_id(id).unwrap();
        card[c.cards[0].0 as usize] += weight;
        card[c.cards[1].0 as usize] += weight;
    }
    (0..COMBO_COUNT)
        .map(|id| {
            let c = Combo::from_id(id).unwrap();
            total - card[c.cards[0].0 as usize] - card[c.cards[1].0 as usize] + reach[id]
        })
        .collect()
}
fn terminal_values(
    kernel: &TerminalKernel,
    classes: &HandClasses,
    player: usize,
    reach: &[f64],
) -> Vec<f64> {
    // Strategies start uniform and all game inputs are suit-invariant. Detect
    // this rather than silently assuming it, so arbitrary asymmetric ranges
    // still use exact per-card blocker subtraction below.
    let invariant = classes.combos.iter().all(|combos| {
        let first = reach[combos[0].id()];
        combos.iter().all(|c| reach[c.id()] == first)
    });
    if invariant {
        let weights: Vec<_> = classes.combos.iter().map(|c| reach[c[0].id()]).collect();
        let values: Vec<f64> = kernel.symmetric[player]
            .as_chunks::<CLASS_COUNT>()
            .0
            .iter()
            .map(|row| row.iter().zip(&weights).map(|(x, w)| x * w).sum())
            .collect();
        return classes.combo_classes.iter().map(|&c| values[c]).collect();
    }
    let mut total = vec![0.0; CLASS_COUNT];
    let mut card = vec![0.0; 52 * CLASS_COUNT];
    for (id, &weight) in reach.iter().enumerate() {
        let class = classes.combo_classes[id];
        let combo = Combo::from_id(id).unwrap();
        total[class] += weight;
        for c in combo.cards {
            card[c.0 as usize * CLASS_COUNT + class] += weight;
        }
    }
    (0..COMBO_COUNT)
        .map(|id| {
            let combo = Combo::from_id(id).unwrap();
            let h = classes.combo_classes[id];
            let a = combo.cards[0].0 as usize * CLASS_COUNT;
            let b = combo.cards[1].0 as usize * CLASS_COUNT;
            (0..CLASS_COUNT)
                .map(|o| {
                    let compatible =
                        total[o] - card[a + o] - card[b + o] + if h == o { reach[id] } else { 0.0 };
                    compatible * kernel.payoff[player][h * CLASS_COUNT + o]
                })
                .sum::<f64>()
                / LEGAL_OPPONENT_COMBOS as f64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::PreflopConfig, tree::Terminal};
    use std::sync::OnceLock;
    fn config() -> PreflopConfig {
        let mut c: PreflopConfig =
            serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap();
        c.rake.rate = 0.0;
        c.dead_blind_bb = 0.0;
        c.stack_bb = 2.0;
        c
    }
    fn synthetic_equity() -> EquityTable {
        static TABLE: OnceLock<EquityTable> = OnceLock::new();
        TABLE
            .get_or_init(|| {
                let mut t = EquityTable::generate(1, 17).unwrap();
                t.values.fill(0.5);
                t
            })
            .clone()
    }
    #[test]
    fn dcfr_discount_uses_completed_iteration_and_square_average_weights() {
        let mut state = NodeState {
            regret: vec![2.0, -2.0],
            average: vec![0.0; 2],
        };
        discount_state(&mut state, 2);
        assert_eq!(state.regret, [1.0, -1.0]);
        for (i, a) in [1.0, 0.0, 0.25].into_iter().enumerate() {
            discount_state(&mut state, i as u64 + 1);
            state.average[0] += a;
            state.average[1] += 1.0 - a;
        }
        let actual = state.average[0] / state.average.iter().sum::<f64>();
        let expected = (1.0 + 9.0 * 0.25) / (1.0 + 4.0 + 9.0);
        assert!((actual - expected).abs() < 1e-14);
    }

    #[test]
    fn exact_blocker_mass_and_asymmetric_kernel_match_brute_force() {
        let classes = HandClasses::new();
        assert_eq!(LEGAL_ORDERED_DEALS, 1_624_350);
        assert!(compatible_mass(&vec![1.0; COMBO_COUNT])
            .iter()
            .all(|&x| x == 1225.0));
        let payoff: Vec<_> = (0..CLASS_COUNT * CLASS_COUNT)
            .map(|i| i as f64 / 1000.0 - 9.0)
            .collect();
        let kernel = TerminalKernel {
            payoff: [payoff.clone(), payoff],
            symmetric: [Vec::new(), Vec::new()],
        };
        let reach: Vec<_> = (0..COMBO_COUNT).map(|i| (i % 7) as f64 / 7.0).collect();
        let actual = terminal_values(&kernel, &classes, 0, &reach);
        for hero_id in [0, 51, 700, 1325] {
            let hero = Combo::from_id(hero_id).unwrap();
            let h = classes.combo_classes[hero_id];
            let expected = (0..COMBO_COUNT)
                .filter(|&i| hero.mask() & Combo::from_id(i).unwrap().mask() == 0)
                .map(|i| reach[i] * kernel.payoff[0][h * CLASS_COUNT + classes.combo_classes[i]])
                .sum::<f64>()
                / 1225.0;
            assert!((actual[hero_id] - expected).abs() < 1e-12);
        }
    }
    #[test]
    fn push_fold_equal_equity_matches_analytic_zero_ev_and_call() {
        let tree = PreflopTree::push_fold(config()).unwrap();
        let mut solver =
            PreflopSolver::new(tree, synthetic_equity(), RealizationConfig::default()).unwrap();
        solver.run(10);
        let early = solver.report();
        solver.run(90);
        let late = solver.report();
        assert!(late.nash_conv_bb < early.nash_conv_bb);
        assert!(late.nash_conv_bb < 0.0001, "{}", late.nash_conv_bb);
        assert!(late.profile_ev_bb.iter().all(|v| v.abs() < 0.0001));
        let bb = late.nodes.iter().find(|n| n.actor == 1).unwrap();
        assert!(bb.conditional_action_frequencies[1] > 0.9999);
        // BTN is indifferent when called at 50%; do not invent a unique jam range.
        assert!(late.best_response_gain_bb.iter().all(|v| *v >= 0.0));
    }
    #[test]
    fn stronger_pair_classes_have_higher_shove_and_call_frequency() {
        let mut equity = synthetic_equity();
        let strength: Vec<f64> = equity
            .classes
            .iter()
            .map(|c| {
                if c.len() == 2 {
                    let rank = b"23456789TJQKA"
                        .iter()
                        .position(|&r| r == c.as_bytes()[0])
                        .unwrap();
                    rank as f64 / 12.0
                } else {
                    0.15
                }
            })
            .collect();
        for a in 0..CLASS_COUNT {
            for b in 0..CLASS_COUNT {
                equity.values[a * CLASS_COUNT + b] = 0.5 + 0.45 * (strength[a] - strength[b]);
            }
        }
        let mut solver = PreflopSolver::new(
            PreflopTree::push_fold(config()).unwrap(),
            equity,
            RealizationConfig::default(),
        )
        .unwrap();
        solver.run(100);
        let report = solver.report();
        for actor in 0..2 {
            let n = report.nodes.iter().find(|n| n.actor == actor).unwrap();
            let frequency = |name: &str| {
                n.classes
                    .iter()
                    .find(|c| c.hand == name)
                    .unwrap()
                    .frequencies[1]
            };
            assert!(
                frequency("AA") > frequency("22") + 0.1,
                "actor {actor}: AA={} 22={}",
                frequency("AA"),
                frequency("22")
            );
        }
    }
    #[test]
    fn average_uses_own_reach_not_opponent_reach() {
        let mut solver = PreflopSolver::new(
            PreflopTree::push_fold(config()).unwrap(),
            synthetic_equity(),
            RealizationConfig::default(),
        )
        .unwrap();
        let mut strategy = solver.strategies(false);
        let root = solver.tree.root;
        for c in 0..COMBO_COUNT {
            strategy[root][c] = 0.9;
            strategy[root][COMBO_COUNT + c] = 0.1;
        }
        let response = match &solver.tree.nodes[root] {
            Node::Decision { children, .. } => children[1],
            _ => unreachable!(),
        };
        traverse(
            &solver.tree,
            &solver.classes,
            &solver.kernels,
            &strategy,
            &mut solver.states,
            root,
            1,
            &vec![0.7; COMBO_COUNT],
            &vec![1.0; COMBO_COUNT],
        );
        let state = solver.states[response].as_ref().unwrap();
        for c in 0..COMBO_COUNT {
            assert!((state.average[c] + state.average[COMBO_COUNT + c] - 0.7).abs() < 1e-12);
        }
    }
    #[test]
    fn fold_no_rake_all_in_no_realization_and_flop_uses_config() {
        let mut config = config();
        config.rake.rate = 0.05;
        config.rake.cap_bb = 3.0;
        config.stack_bb = 100.0;
        for (kind, expected) in [
            (TerminalKind::Fold, [100.0, -100.0]),
            (TerminalKind::AllIn, [-1.5, -1.5]),
            (TerminalKind::Flop, [-50.75, -50.75]),
        ] {
            let tree = PreflopTree {
                config: config.clone(),
                root: 0,
                nodes: vec![Node::Terminal(Terminal {
                    kind,
                    contributions: vec![100.0, 100.0],
                    active: vec![true, true],
                    winner: Some(0),
                    stage: 1,
                    dead_blind_bb: 0.0,
                })],
            };
            let mut realization = RealizationConfig::default();
            for row in [
                &mut realization.limp,
                &mut realization.srp,
                &mut realization.three_bet,
                &mut realization.four_bet,
            ] {
                for h in [&mut row.ip, &mut row.oop] {
                    h.pair = 0.5;
                    h.suited_connector = 0.5;
                    h.suited = 0.5;
                    h.offsuit_broadway = 0.5;
                    h.offsuit = 0.5;
                }
            }
            let report = PreflopSolver::new(tree, synthetic_equity(), realization)
                .unwrap()
                .report();
            for (actual, expected) in report.profile_ev_bb.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-10,
                    "{kind:?}: {actual} vs {expected}"
                );
            }
            assert!(report.nash_conv_bb < 1e-12);
        }
    }
    #[test]
    fn supplied_realization_file_matches_default() {
        let from_file = RealizationConfig::from_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/preflop-realization.json"
        ))
        .unwrap();
        let default = RealizationConfig::default();
        for stage in 0..4 {
            for ip in [false, true] {
                for hand in ["AA", "AKs", "98s", "AKo", "72o"] {
                    assert!(
                        (from_file.get(stage, ip, hand) - default.get(stage, ip, hand)).abs()
                            < 1e-12
                    );
                }
            }
        }
    }
}
