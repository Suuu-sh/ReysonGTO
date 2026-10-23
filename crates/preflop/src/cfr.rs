//! HU alternating DCFR with dense physical-combo regret/reach vectors.
//! Class equity is an abstraction; illegal private-card pairs are excluded exactly.
pub use crate::realization::RealizationConfig;
use crate::{
    equity::{EquityMetadata, EquityTable, HandClasses, ThreeWayEquityTable, CLASS_COUNT},
    tree::{Action, Node, PreflopTree, TerminalKind},
};
use cards::Combo;
use rayon::prelude::*;
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
    pub opening_by_position: std::collections::BTreeMap<String, OpeningFrequency>,
    pub btn_open: Option<f64>,
    pub bb_three_bet_given_btn_open: Option<f64>,
    pub bb_call_given_btn_open: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct OpeningFrequency {
    pub raise: f64,
    pub complete: f64,
}
#[derive(Debug, Serialize)]
pub struct SolveReport {
    pub iterations: u64,
    pub training_seconds: f64,
    pub report_seconds: f64,
    pub profile_ev_bb: Vec<f64>,
    pub best_response_ev_bb: Vec<f64>,
    pub best_response_gain_bb: Vec<f64>,
    /// Sum of unilateral improvement in this general-sum abstract game.
    pub nash_conv_bb: f64,
    /// Merely half of NashConv, not zero-sum exploitability.
    pub half_gap_bb: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_ordered_deals: Option<usize>,
    pub equity_metadata: EquityMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_way_metadata: Option<EquityMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_way_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximation: Option<String>,
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
    equity: EquityTable,
    three_way: Option<ThreeWayEquityTable>,
    ordered_three: Option<std::sync::Arc<Vec<f64>>>,
    /// Seats evaluated at once in a multiway report. Each owns caches of roughly
    /// 165 MB with the production 3-way table, so this bounds report memory
    /// independently of the Rayon pool size.
    report_parallelism: usize,
}
/// Default concurrent seat evaluations in a multiway report.
pub const DEFAULT_REPORT_PARALLELISM: usize = 2;
impl PreflopSolver {
    pub fn new(
        tree: PreflopTree,
        equity: EquityTable,
        realization: RealizationConfig,
    ) -> Result<Self, String> {
        Self::new_with_three_way(tree, equity, realization, None)
    }
    pub fn new_with_three_way(
        tree: PreflopTree,
        equity: EquityTable,
        realization: RealizationConfig,
        three_way: Option<ThreeWayEquityTable>,
    ) -> Result<Self, String> {
        if !(2..=6).contains(&tree.config.players) {
            return Err("CFR requires 2..6 players".into());
        }
        let players = tree.config.players;
        if players > 2 && three_way.is_none() {
            eprintln!(
                "warning: no three-way equity table; multiway product-HU equity is approximate"
            );
        }
        tree.config.validate().map_err(|e| e.to_string())?;
        equity.validate()?;
        if let Some(t) = &three_way {
            t.validate()?;
        }
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
                    if *actor >= players
                        || actions.is_empty()
                        || actions.len() != children.len()
                        || children.iter().any(|&id| id >= tree.nodes.len())
                    {
                        return Err("invalid HU decision node".into());
                    }
                    states.push(Some(NodeState {
                        regret: vec![
                            0.0;
                            actions.len()
                                * if players == 2 {
                                    COMBO_COUNT
                                } else {
                                    CLASS_COUNT
                                }
                        ],
                        average: vec![
                            0.0;
                            actions.len()
                                * if players == 2 {
                                    COMBO_COUNT
                                } else {
                                    CLASS_COUNT
                                }
                        ],
                    }));
                    kernels.push(None);
                }
                Node::Terminal(t) => {
                    if t.contributions.len() != players {
                        return Err("invalid HU terminal contributions".into());
                    }
                    if players > 2 {
                        states.push(None);
                        kernels.push(None);
                        continue;
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
            equity_metadata: equity.metadata.clone(),
            equity,
            ordered_three: three_way
                .as_ref()
                .filter(|_| {
                    std::env::var_os("PREFLOP_REFERENCE_TERMINALS").is_none()
                        && std::env::var_os("PREFLOP_REFERENCE_LOOKUP").is_none()
                })
                .map(|t| std::sync::Arc::new(t.ordered_hero_shares())),
            three_way,
            realization,
            report_parallelism: DEFAULT_REPORT_PARALLELISM,
        })
    }
    pub fn set_report_parallelism(&mut self, seats: usize) {
        self.report_parallelism = seats.max(1);
    }
    fn multi_walk(
        &mut self,
        id: usize,
        player: usize,
        strategy: &[Vec<f64>],
        reach: &[Vec<f64>],
        best: bool,
        update: bool,
    ) -> Vec<f64> {
        multi_traverse(
            &self.tree,
            &self.classes,
            &self.equity,
            self.three_way.as_ref(),
            &self.realization,
            strategy,
            &mut self.states,
            id,
            player,
            reach,
            best,
            update,
            true,
            &mut RangeCache::training(self.ordered_three.clone()),
        )
    }
    fn multi_evaluate(
        &self,
        id: usize,
        player: usize,
        strategy: &[Vec<f64>],
        reach: &[Vec<f64>],
        best: bool,
    ) -> Vec<f64> {
        multi_traverse(
            &self.tree,
            &self.classes,
            &self.equity,
            self.three_way.as_ref(),
            &self.realization,
            strategy,
            &mut [],
            id,
            player,
            reach,
            best,
            false,
            true,
            &mut RangeCache::with_dense(self.ordered_three.clone()),
        )
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
        for p in 0..self.tree.config.players {
            for (node, state) in self.tree.nodes.iter().zip(&mut self.states) {
                if let (Node::Decision { actor, .. }, Some(state)) = (node, state) {
                    if *actor == p {
                        discount_state(state, self.iterations + 1);
                    }
                }
            }
            let strategy = self.strategies(false);
            if self.tree.config.players > 2 {
                self.multi_walk(
                    self.tree.root,
                    p,
                    &strategy,
                    &vec![vec![1.0; COMBO_COUNT]; self.tree.config.players],
                    false,
                    true,
                );
                continue;
            }
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
        let seconds = start.elapsed().as_secs_f64();
        self.training_seconds += seconds;
        if std::env::var_os("PREFLOP_PROFILE_TERMINALS").is_some() {
            eprintln!(
                "training_step iteration={} seconds={seconds}",
                self.iterations
            );
        }
    }
    fn strategies(&self, average: bool) -> Vec<Vec<f64>> {
        self.states
            .iter()
            .map(|state| match state {
                None => Vec::new(),
                Some(s) => {
                    let source = if average { &s.average } else { &s.regret };
                    let stride = if self.tree.config.players == 2 {
                        COMBO_COUNT
                    } else {
                        CLASS_COUNT
                    };
                    let count = source.len() / stride;
                    let mut out = vec![0.0; source.len()];
                    for c in 0..stride {
                        let sum: f64 = (0..count).map(|a| source[a * stride + c].max(0.0)).sum();
                        for a in 0..count {
                            out[a * stride + c] = if sum > 0.0 {
                                source[a * stride + c].max(0.0) / sum
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
        self.report_impl(false)
    }
    pub fn report_summary(&self) -> SolveReport {
        self.report_impl(true)
    }
    fn report_impl(&self, summary_only: bool) -> SolveReport {
        let start = Instant::now();
        let strategy = self.strategies(true);
        let mut profile = vec![0.0; self.tree.config.players];
        let mut br = vec![0.0; self.tree.config.players];
        if self.tree.config.players > 2 {
            // Each counterfactual traversal is independent and owns bounded
            // caches. Indexed collection preserves seat order; HU and the
            // alternating training updates retain their original serial path.
            let seats: Vec<usize> = (0..self.tree.config.players).collect();
            let mut values = Vec::with_capacity(seats.len());
            for chunk in seats.chunks(self.report_parallelism) {
                values.extend(
                    chunk
                        .par_iter()
                        .map(|&p| {
                            let reach = vec![vec![1.0; COMBO_COUNT]; self.tree.config.players];
                            [false, true].map(|best| {
                                self.multi_evaluate(self.tree.root, p, &strategy, &reach, best)
                                    .iter()
                                    .sum::<f64>()
                                    / COMBO_COUNT as f64
                            })
                        })
                        .collect::<Vec<_>>(),
                );
            }
            for (p, value) in values.into_iter().enumerate() {
                profile[p] = value[0];
                br[p] = value[1];
            }
        } else {
            for p in 0..self.tree.config.players {
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
        }
        let gains: Vec<_> = br
            .iter()
            .zip(&profile)
            .map(|(b, p)| (b - p).max(0.0))
            .collect();
        let mut nodes = Vec::new();
        self.collect_nodes(
            self.tree.root,
            &strategy,
            vec![vec![1.0; COMBO_COUNT]; self.tree.config.players],
            Vec::new(),
            &mut nodes,
            summary_only,
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
        for n in &nodes {
            if n.history.iter().all(|a| a == "fold") && self.tree.config.positions[n.actor] != "BB"
            {
                let raise = n
                    .actions
                    .iter()
                    .zip(&n.conditional_action_frequencies)
                    .filter(|(a, _)| a.starts_with("raise_to_") || *a == "all_in")
                    .map(|(_, p)| p)
                    .sum();
                let complete = n
                    .actions
                    .iter()
                    .zip(&n.conditional_action_frequencies)
                    .filter(|(a, _)| *a == "call")
                    .map(|(_, p)| p)
                    .sum();
                summary.opening_by_position.insert(
                    self.tree.config.positions[n.actor].clone(),
                    OpeningFrequency { raise, complete },
                );
            }
        }
        SolveReport {
            iterations: self.iterations,
            training_seconds: self.training_seconds,
            report_seconds: start.elapsed().as_secs_f64(),
            profile_ev_bb: profile,
            best_response_ev_bb: br,
            best_response_gain_bb: gains.clone(),
            nash_conv_bb: gains.iter().sum(),
            half_gap_bb: gains.iter().sum::<f64>() / 2.0,
            legal_ordered_deals: (self.tree.config.players==2).then_some(LEGAL_ORDERED_DEALS),
            equity_metadata: self.equity_metadata.clone(),
            three_way_metadata: self.three_way.as_ref().map(|t| t.metadata.clone()),
            three_way_content_hash: self.three_way.as_ref().map(|t|t.content_hash.clone()),
            approximation: (self.tree.config.players > 2).then(|| "independent class chance for >=3 active; exact pair blockers for active2; independent folded reach; three-way table when supplied, raw product-HU otherwise (not constant-sum); multiway realization is unlearned".into()),
            realization: self.realization.clone(),
            summary,
            // Summaries keep only first-in and lone-opener response nodes (bounded).
            nodes,
        }
    }
    fn collect_nodes(
        &self,
        id: usize,
        strategy: &[Vec<f64>],
        reach: Vec<Vec<f64>>,
        history: Vec<String>,
        out: &mut Vec<NodeStrategy>,
        summary_only: bool,
    ) {
        if let Node::Decision {
            actor,
            actions,
            children,
        } = &self.tree.nodes[id]
        {
            let stride = if reach.len() == 2 {
                COMBO_COUNT
            } else {
                CLASS_COUNT
            };
            let mass = if reach.len() == 2 {
                compatible_mass(&reach[1 - actor])
            } else {
                let m: f64 = reach
                    .iter()
                    .enumerate()
                    .filter(|(p, _)| *p != *actor)
                    .map(|(_, r)| r.iter().sum::<f64>() / COMBO_COUNT as f64)
                    .product();
                vec![m * LEGAL_OPPONENT_COMBOS as f64; COMBO_COUNT]
            };
            let node_mass: f64 = (0..COMBO_COUNT).map(|c| reach[*actor][c] * mass[c]).sum();
            let conditional = (0..actions.len())
                .map(|a| {
                    if node_mass > 0.0 {
                        (0..COMBO_COUNT)
                            .map(|c| {
                                reach[*actor][c]
                                    * mass[c]
                                    * strategy[id][a * stride
                                        + if stride == COMBO_COUNT {
                                            c
                                        } else {
                                            self.classes.combo_classes[c]
                                        }]
                            })
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
                                .map(|c| {
                                    strategy[id][a * stride
                                        + if stride == COMBO_COUNT {
                                            c.id()
                                        } else {
                                            self.classes.combo_classes[c.id()]
                                        }]
                                })
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
                // Summaries keep every node reached by at most one non-fold action:
                // each first-in decision and every response to a lone opener
                // (e.g. BB facing a BTN open after SB folds), which M6 needs as ranges.
                let aggressive = history.iter().filter(|h| *h != "fold").count()
                    + usize::from(actions[a] != Action::Fold);
                if summary_only && aggressive > 1 {
                    continue;
                }
                let mut next = reach.clone();
                for c in 0..COMBO_COUNT {
                    next[*actor][c] *= strategy[id][a * stride
                        + if stride == COMBO_COUNT {
                            c
                        } else {
                            self.classes.combo_classes[c]
                        }];
                }
                let mut path = history.clone();
                path.push(action_label(&actions[a]));
                self.collect_nodes(child, strategy, next, path, out, summary_only);
            }
        }
    }
}
type EquityIntegral = (Vec<f64>, Vec<f64>);
type SharedIntegral = std::sync::Arc<EquityIntegral>;
#[derive(Default)]
struct RangeCache {
    pairs: std::collections::HashMap<Vec<u64>, EquityIntegral>,
    triples: std::collections::HashMap<Vec<u64>, EquityIntegral>,
    contractions: std::collections::HashMap<Vec<u64>, SharedIntegral>,
    contraction_fifo: std::collections::VecDeque<Vec<u64>>,
    contraction_hits: u64,
    contraction_seconds: f64,
    reference: bool,
    reference_lookup: bool,
    reference_integral: bool,
    dense: Option<std::sync::Arc<Vec<f64>>>,
    profile: bool,
    pair_calls: u64,
    pair_hits: u64,
    pair_seconds: f64,
    triple_calls: u64,
    triple_hits: u64,
    triple_seconds: f64,
}
impl RangeCache {
    fn new() -> Self {
        Self {
            pairs: Default::default(),
            triples: Default::default(),
            contractions: Default::default(),
            contraction_fifo: Default::default(),
            contraction_hits: 0,
            contraction_seconds: 0.0,
            reference: std::env::var_os("PREFLOP_REFERENCE_TERMINALS").is_some(),
            reference_lookup: std::env::var_os("PREFLOP_REFERENCE_LOOKUP").is_some(),
            reference_integral: std::env::var_os("PREFLOP_REFERENCE_INTEGRAL").is_some(),
            dense: None,
            profile: std::env::var_os("PREFLOP_PROFILE_TERMINALS").is_some(),
            pair_calls: 0,
            pair_hits: 0,
            pair_seconds: 0.0,
            triple_calls: 0,
            triple_hits: 0,
            triple_seconds: 0.0,
        }
    }
    fn with_dense(dense: Option<std::sync::Arc<Vec<f64>>>) -> Self {
        let mut cache = Self::new();
        cache.dense = dense;
        cache
    }
    fn training(dense: Option<std::sync::Arc<Vec<f64>>>) -> Self {
        // Sparse learned reaches make original-order integration cheaper than
        // building a full contraction. Keep training bit-exact; contraction
        // is reserved for repeated EV/BR reports unless the oracle is selected.
        let mut cache = Self::with_dense(dense);
        cache.reference_integral = true;
        cache
    }
    #[cfg(test)]
    fn reference_mode(mut self, reference: bool) -> Self {
        self.reference = reference;
        self
    }
    fn len(&self) -> usize {
        self.pairs.len()
    }
}
impl Drop for RangeCache {
    fn drop(&mut self) {
        if self.profile {
            eprintln!(
                "terminal_contraction_profile hits={} seconds={} entries={}",
                self.contraction_hits,
                self.contraction_seconds,
                self.contractions.len()
            );
            eprintln!(
                "terminal_pair_profile calls={} hits={} seconds={}",
                self.pair_calls, self.pair_hits, self.pair_seconds
            );
            eprintln!("terminal_profile triple_calls={} triple_hits={} triple_integral_seconds={} triple_entries={} pair_entries={} reference={}", self.triple_calls, self.triple_hits, self.triple_seconds, self.triples.len(), self.pairs.len(), self.reference);
        }
    }
}
// Contract one opponent into a 169x169 hero/other-opponent matrix. FIFO
// keeps at most 128 matrices (~58.5 MB). Values use the same independent-class
// game, but reassociate floating sums; reference modes retain original order.
fn contracted_opponent(
    dense: &[f64],
    b: &[f64],
    total_b: f64,
    cache: &mut RangeCache,
) -> std::sync::Arc<(Vec<f64>, Vec<f64>)> {
    let key: Vec<_> = b.iter().map(|v| v.to_bits()).collect();
    if let Some(value) = cache.contractions.get(&key) {
        cache.contraction_hits += u64::from(cache.profile);
        return value.clone();
    }
    let start = cache.profile.then(Instant::now);
    let mut share = vec![0.0; CLASS_COUNT * CLASS_COUNT];
    let mut legal = vec![0.0; CLASS_COUNT * CLASS_COUNT];
    if total_b > 0.0 {
        for hx in 0..CLASS_COUNT * CLASS_COUNT {
            for y in 0..CLASS_COUNT {
                if b[y] == 0.0 {
                    continue;
                }
                let e = dense[hx * CLASS_COUNT + y];
                if !e.is_nan() {
                    let w = b[y] / total_b;
                    share[hx] += e * w;
                    legal[hx] += w;
                }
            }
        }
    }
    if let Some(start) = start {
        cache.contraction_seconds += start.elapsed().as_secs_f64();
    }
    let value = std::sync::Arc::new((share, legal));
    if cache.contractions.len() == 128 {
        let oldest = cache.contraction_fifo.pop_front().unwrap();
        cache.contractions.remove(&oldest);
    }
    cache.contraction_fifo.push_back(key.clone());
    cache.contractions.insert(key, value.clone());
    value
}
// Exact raw weights in seat order form the key, without quantization or
// normalization. A traversal owns its cache, so table identity is implicit.
fn triple_expectation(
    three: &ThreeWayEquityTable,
    a: &[f64],
    b: &[f64],
    total_a: f64,
    total_b: f64,
    cache: &mut RangeCache,
) -> (Vec<f64>, Vec<f64>) {
    cache.triple_calls += u64::from(cache.profile);
    let key: Vec<u64> = a.iter().chain(b).map(|x| x.to_bits()).collect();
    if !cache.reference {
        if let Some(result) = cache.triples.get(&key) {
            cache.triple_hits += u64::from(cache.profile);
            return result.clone();
        }
    }
    let start = cache.profile.then(Instant::now);
    let mut shares = vec![0.0; CLASS_COUNT];
    let mut legal = vec![0.0; CLASS_COUNT];
    if !cache.reference && !cache.reference_lookup && cache.dense.is_none() {
        cache.dense = Some(std::sync::Arc::new(three.ordered_hero_shares()));
    }
    let dense = cache.dense.clone();
    if let Some(dense) = dense.as_ref().filter(|_| !cache.reference_integral) {
        let contracted = contracted_opponent(dense, b, total_b, cache);
        if total_a > 0.0 {
            for h in 0..CLASS_COUNT {
                for (x, &weight) in a.iter().enumerate() {
                    let w = weight / total_a;
                    shares[h] += contracted.0[h * CLASS_COUNT + x] * w;
                    legal[h] += contracted.1[h * CLASS_COUNT + x] * w;
                }
            }
        }
    } else if total_a > 0.0 && total_b > 0.0 {
        for h in 0..CLASS_COUNT {
            for x in 0..CLASS_COUNT {
                if a[x] == 0.0 {
                    continue;
                }
                for y in 0..CLASS_COUNT {
                    if b[y] == 0.0 {
                        continue;
                    }
                    let e = if let Some(dense) = &dense {
                        dense[(h * CLASS_COUNT + x) * CLASS_COUNT + y]
                    } else {
                        three.get(h, x, y).map_or(f64::NAN, |e| e[0])
                    };
                    if !e.is_nan() {
                        legal[h] += a[x] / total_a * b[y] / total_b;
                        shares[h] += e * a[x] / total_a * b[y] / total_b;
                    }
                }
            }
        }
    }
    if let Some(start) = start {
        cache.triple_seconds += start.elapsed().as_secs_f64();
    }
    let result = (shares, legal);
    // Separate bounded budget: ~89 MB including exact 338-u64 keys and values.
    if !cache.reference && cache.triples.len() < 16384 {
        cache.triples.insert(key, result.clone());
    }
    result
}
fn range_expectation(
    classes: &HandClasses,
    equity: &EquityTable,
    reach: &[f64],
    blockers: bool,
    cache: &mut RangeCache,
) -> (Vec<f64>, Vec<f64>) {
    cache.pair_calls += u64::from(cache.profile);
    let mut key: Vec<_> = classes
        .combos
        .iter()
        .map(|c| reach[c[0].id()].to_bits())
        .collect();
    key.push(blockers as u64);
    if let Some(value) = cache.pairs.get(&key) {
        cache.pair_hits += u64::from(cache.profile);
        return value.clone();
    }
    let start = cache.profile.then(Instant::now);
    static COUNTS: std::sync::OnceLock<Vec<f64>> = std::sync::OnceLock::new();
    let counts = COUNTS.get_or_init(|| {
        (0..CLASS_COUNT)
            .flat_map(|h| {
                let hero = classes.combos[h][0];
                (0..CLASS_COUNT).map(move |o| {
                    classes.combos[o]
                        .iter()
                        .filter(|c| c.mask() & hero.mask() == 0)
                        .count() as f64
                })
            })
            .collect()
    });
    let mut expected = vec![0.0; CLASS_COUNT];
    let mut mass = vec![0.0; CLASS_COUNT];
    for h in 0..CLASS_COUNT {
        for o in 0..CLASS_COUNT {
            let weight = reach[classes.combos[o][0].id()]
                * if blockers {
                    counts[h * CLASS_COUNT + o] / LEGAL_OPPONENT_COMBOS as f64
                } else {
                    classes.combos[o].len() as f64 / COMBO_COUNT as f64
                };
            expected[h] += equity.get(h, o) * weight;
            mass[h] += weight;
        }
    }
    if let Some(start) = start {
        cache.pair_seconds += start.elapsed().as_secs_f64();
    }
    let result = (expected, mass);
    // Bounded cache: at most about17MB, never scales with the public tree.
    if cache.len() < 4096 {
        cache.pairs.insert(key, result.clone());
    }
    result
}
// Multiway chance is independent class chance, not exact joint card deals.
// Active two-seat payoffs retain the exact HU blocker kernel; folded reach
// factors independently. Supported reaches start uniform and remain class-
// invariant: regrets/strategies are losslessly stored once per suit class.
#[allow(clippy::too_many_arguments)]
fn multi_traverse(
    tree: &PreflopTree,
    classes: &HandClasses,
    equity: &EquityTable,
    three: Option<&ThreeWayEquityTable>,
    realization: &RealizationConfig,
    strategy: &[Vec<f64>],
    states: &mut [Option<NodeState>],
    id: usize,
    player: usize,
    reach: &[Vec<f64>],
    best: bool,
    update: bool,
    prune_folds: bool,
    cache: &mut RangeCache,
) -> Vec<f64> {
    let Node::Decision {
        actor,
        actions,
        children,
    } = &tree.nodes[id]
    else {
        return multi_terminal_cached(
            tree,
            classes,
            equity,
            three,
            realization,
            id,
            player,
            reach,
            cache,
        );
    };
    let maximize = best && *actor == player;
    let mut values = vec![if maximize { f64::NEG_INFINITY } else { 0.0 }; COMBO_COUNT];
    let mut action_values = Vec::new();
    for (a, &child) in children.iter().enumerate() {
        let mut next = reach.to_vec();
        if update || *actor != player {
            for (c, r) in next[*actor].iter_mut().enumerate() {
                *r *= strategy[id][a * CLASS_COUNT + classes.combo_classes[c]];
            }
        }
        let v = if prune_folds && *actor == player && actions[a] == Action::Fold {
            // Once hero folds its contribution never changes; integrating the
            // remaining independent chance/strategies has total probability1.
            let mut terminal = child;
            while let Node::Decision { children, .. } = &tree.nodes[terminal] {
                terminal = children[0];
            }
            let Node::Terminal(t) = &tree.nodes[terminal] else {
                unreachable!()
            };
            let mass: f64 = reach
                .iter()
                .enumerate()
                .filter(|(p, _)| *p != player)
                .map(|(_, r)| r.iter().sum::<f64>() / COMBO_COUNT as f64)
                .product();
            vec![-t.contributions[player] * mass; COMBO_COUNT]
        } else {
            multi_traverse(
                tree,
                classes,
                equity,
                three,
                realization,
                strategy,
                states,
                child,
                player,
                &next,
                best,
                update,
                prune_folds,
                cache,
            )
        };
        for c in 0..COMBO_COUNT {
            if maximize {
                values[c] = values[c].max(v[c]);
            } else {
                values[c] += v[c]
                    * if *actor == player {
                        strategy[id][a * CLASS_COUNT + classes.combo_classes[c]]
                    } else {
                        1.0
                    };
            }
        }
        if update && *actor == player {
            action_values.push(v);
        }
    }
    if update && *actor == player {
        let state = states[id].as_mut().unwrap();
        for (a, v) in action_values.iter().enumerate() {
            for h in 0..CLASS_COUNT {
                let c = classes.combos[h][0].id();
                state.regret[a * CLASS_COUNT + h] += v[c] - values[c];
                state.average[a * CLASS_COUNT + h] +=
                    reach[player][c] * strategy[id][a * CLASS_COUNT + h];
            }
        }
    }
    values
}
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn multi_terminal(
    tree: &PreflopTree,
    classes: &HandClasses,
    equity: &EquityTable,
    three: Option<&ThreeWayEquityTable>,
    realization: &RealizationConfig,
    id: usize,
    player: usize,
    reach: &[Vec<f64>],
) -> Vec<f64> {
    multi_terminal_cached(
        tree,
        classes,
        equity,
        three,
        realization,
        id,
        player,
        reach,
        &mut RangeCache::new(),
    )
}
#[allow(clippy::too_many_arguments)]
fn multi_terminal_cached(
    tree: &PreflopTree,
    classes: &HandClasses,
    equity: &EquityTable,
    three: Option<&ThreeWayEquityTable>,
    realization: &RealizationConfig,
    id: usize,
    player: usize,
    reach: &[Vec<f64>],
    cache: &mut RangeCache,
) -> Vec<f64> {
    let Node::Terminal(t) = &tree.nodes[id] else {
        unreachable!()
    };
    let active: Vec<_> = t
        .active
        .iter()
        .enumerate()
        .filter_map(|(p, &a)| a.then_some(p))
        .collect();
    let independent_mass: f64 = reach
        .iter()
        .enumerate()
        .filter(|(p, _)| *p != player)
        .map(|(_, r)| r.iter().sum::<f64>() / COMBO_COUNT as f64)
        .product();
    let mass = vec![independent_mass; COMBO_COUNT];
    let pot = t.pot_bb();
    if t.kind == TerminalKind::Fold || !t.active[player] {
        let payoff = if t.winner == Some(player) {
            pot - t.contributions[player]
        } else {
            -t.contributions[player]
        };
        return mass.iter().map(|m| m * payoff).collect();
    }
    if active.len() == 2 {
        debug_assert!(reach.iter().all(|r| classes
            .combos
            .iter()
            .all(|cs| cs.iter().all(|c| r[c.id()] == r[cs[0].id()]))));
        let opponent = active.iter().copied().find(|&p| p != player).unwrap();
        let available = pot - tree.config.rake.amount_bb(pot, true);
        let (expected, legal) = range_expectation(classes, equity, &reach[opponent], true, cache);
        let class_values: Vec<_> = (0..CLASS_COUNT)
            .map(|h| {
                let factor = if t.kind == TerminalKind::Flop {
                    realization.get(
                        t.stage,
                        tree.config.is_ip(player, opponent),
                        &classes.names[h],
                    )
                } else {
                    1.0
                };
                expected[h] * available * factor - t.contributions[player] * legal[h]
            })
            .collect();
        let mut value: Vec<_> = classes
            .combo_classes
            .iter()
            .map(|&h| class_values[h])
            .collect();
        for (p, r) in reach.iter().enumerate() {
            if p != player && p != opponent {
                let m = r.iter().sum::<f64>() / COMBO_COUNT as f64;
                for v in &mut value {
                    *v *= m;
                }
            }
        }
        return value;
    }
    // >=3 active seats use independent class chance. Legal triple class sets
    // come from the table; suit-specific joint blockers are not modeled.
    let weights: Vec<Vec<f64>> = reach
        .iter()
        .map(|r| {
            classes
                .combos
                .iter()
                .map(|cs| cs.iter().map(|c| r[c.id()]).sum::<f64>())
                .collect()
        })
        .collect();
    let totals: Vec<f64> = weights.iter().map(|w| w.iter().sum()).collect();
    let opponents: Vec<_> = active.iter().copied().filter(|&p| p != player).collect();
    let mut shares = vec![0.0; CLASS_COUNT];
    let mut legal_mass = vec![1.0; CLASS_COUNT];
    if let Some(three) = three.filter(|_| active.len() == 3) {
        (shares, legal_mass) = triple_expectation(
            three,
            &weights[opponents[0]],
            &weights[opponents[1]],
            totals[opponents[0]],
            totals[opponents[1]],
            cache,
        );
    } else {
        let pair_expected: Vec<_> = opponents
            .iter()
            .map(|&p| range_expectation(classes, equity, &reach[p], false, cache).0)
            .collect();
        for h in 0..CLASS_COUNT {
            shares[h] = 1.0;
            for (index, &p) in opponents.iter().enumerate() {
                let e = if totals[p] > 0.0 {
                    pair_expected[index][h] * COMBO_COUNT as f64 / totals[p]
                } else {
                    0.0
                };
                shares[h] *= e;
            }
        }
    }
    let available = pot - tree.config.rake.amount_bb(pot, true);
    let factor = if t.kind == TerminalKind::Flop {
        if active.len() > 2 {
            realization.multiway
        } else {
            realization.get(t.stage, tree.config.is_ip(player, opponents[0]), "AA")
        }
    } else {
        1.0
    };
    (0..COMBO_COUNT)
        .map(|c| {
            let h = classes.combo_classes[c];
            let realization_factor = if t.kind == TerminalKind::Flop && active.len() == 2 {
                realization.get(
                    t.stage,
                    tree.config.is_ip(player, opponents[0]),
                    &classes.names[h],
                )
            } else {
                factor
            };
            mass[c]
                * (shares[h] * available * realization_factor
                    - t.contributions[player] * legal_mass[h])
        })
        .collect()
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
    fn config_three() -> PreflopConfig {
        let mut c = config();
        c.players = 3;
        c.positions = vec!["BTN".into(), "SB".into(), "BB".into()];
        c
    }
    fn golden_hash(mut report: serde_json::Value) -> u64 {
        for key in ["training_seconds", "report_seconds"] {
            report.as_object_mut().unwrap().remove(key);
        }
        report["realization"]
            .as_object_mut()
            .unwrap()
            .remove("multiway");
        report["summary"]
            .as_object_mut()
            .unwrap()
            .remove("opening_by_position");
        serde_json::to_vec(&report)
            .unwrap()
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ *b as u64).wrapping_mul(0x100000001b3)
            })
    }
    #[test]
    fn three_way_reference_and_cached_training_within_tolerance() {
        let table = ThreeWayEquityTable::generate(1, 71).unwrap();
        let make = || {
            PreflopSolver::new_with_three_way(
                PreflopTree::push_fold(config_three()).unwrap(),
                synthetic_equity(),
                RealizationConfig::default(),
                Some(table.clone()),
            )
            .unwrap()
        };
        let mut fast = make();
        let mut reference = make();
        let mut max_strategy_error = 0.0_f64;
        let mut max_value_error = 0.0_f64;
        for _ in 0..3 {
            fast.step();
            for p in 0..reference.tree.config.players {
                for (node, state) in reference.tree.nodes.iter().zip(&mut reference.states) {
                    if let (Node::Decision { actor, .. }, Some(state)) = (node, state) {
                        if *actor == p {
                            discount_state(state, reference.iterations + 1);
                        }
                    }
                }
                let strategy = reference.strategies(false);
                let mut cache = RangeCache::default().reference_mode(true);
                multi_traverse(
                    &reference.tree,
                    &reference.classes,
                    &reference.equity,
                    reference.three_way.as_ref(),
                    &reference.realization,
                    &strategy,
                    &mut reference.states,
                    reference.tree.root,
                    p,
                    &vec![vec![1.0; COMBO_COUNT]; 3],
                    false,
                    true,
                    true,
                    &mut cache,
                );
            }
            reference.iterations += 1;
            for (actual, expected) in fast.states.iter().zip(&reference.states) {
                match (actual, expected) {
                    (Some(a), Some(b)) => {
                        for (x, y) in a
                            .regret
                            .iter()
                            .chain(&a.average)
                            .zip(b.regret.iter().chain(&b.average))
                        {
                            assert_eq!(
                                x.to_bits(),
                                y.to_bits(),
                                "training state must remain bit-exact"
                            );
                        }
                    }
                    (None, None) => {}
                    _ => panic!("decision state shape changed"),
                }
            }
            for average in [false, true] {
                for (a, b) in fast
                    .strategies(average)
                    .iter()
                    .flatten()
                    .zip(reference.strategies(average).iter().flatten())
                {
                    max_strategy_error = max_strategy_error.max((a - b).abs());
                    assert!((a - b).abs() < 1e-10, "strategy {a} != {b}");
                }
            }
        }
        // Compare all player EV and BR with uncached/original get, not merely
        // a second optimized report. Reassociated sums use 1e-10 tolerance.
        let strategy = reference.strategies(true);
        for p in 0..3 {
            for best in [false, true] {
                let reach = vec![vec![1.0; COMBO_COUNT]; 3];
                let mut cache = RangeCache::default().reference_mode(true);
                let expected = multi_traverse(
                    &reference.tree,
                    &reference.classes,
                    &reference.equity,
                    reference.three_way.as_ref(),
                    &reference.realization,
                    &strategy,
                    &mut [],
                    reference.tree.root,
                    p,
                    &reach,
                    best,
                    false,
                    true,
                    &mut cache,
                );
                for (a, b) in fast
                    .multi_evaluate(fast.tree.root, p, &strategy, &reach, best)
                    .iter()
                    .zip(expected)
                {
                    max_value_error = max_value_error.max((a - b).abs());
                    assert!((a - b).abs() < 1e-10, "value {a} != {b}");
                }
            }
        }
        let reports: Vec<_> = [1, 2]
            .into_iter()
            .map(|threads| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap()
                    .install(|| fast.report_summary())
            })
            .collect();
        assert_eq!(reports[0].profile_ev_bb, reports[1].profile_ev_bb);
        assert_eq!(
            reports[0].best_response_ev_bb,
            reports[1].best_response_ev_bb
        );
        assert_eq!(reports[0].nash_conv_bb, reports[1].nash_conv_bb);
        eprintln!("synthetic_reference iterations=3 max_strategy_error={max_strategy_error} max_ev_br_error={max_value_error} tolerance=1e-10");
    }
    #[test]
    fn three_way_cached_integral_matches_reference() {
        let three = ThreeWayEquityTable::generate(1, 71).unwrap();
        let classes = HandClasses::new();
        for sparse in [false, true] {
            let a: Vec<_> = (0..CLASS_COUNT)
                .map(|h| {
                    if sparse && h % 19 != 0 {
                        0.0
                    } else {
                        classes.combos[h].len() as f64 * (h + 1) as f64 / 169.0
                    }
                })
                .collect();
            let b: Vec<_> = a.iter().rev().copied().collect();
            let (ta, tb) = (a.iter().sum(), b.iter().sum());
            let mut reference = RangeCache::default().reference_mode(true);
            let expected = triple_expectation(&three, &a, &b, ta, tb, &mut reference);
            let mut dense_reference = RangeCache::default().reference_mode(false);
            dense_reference.reference_integral = true;
            assert_eq!(
                triple_expectation(&three, &a, &b, ta, tb, &mut dense_reference),
                expected
            );
            let mut cached = RangeCache::default();
            let actual = triple_expectation(&three, &a, &b, ta, tb, &mut cached);
            for (a, b) in actual
                .0
                .iter()
                .chain(&actual.1)
                .zip(expected.0.iter().chain(&expected.1))
            {
                assert!((a - b).abs() < 1e-12, "integral {a} != {b}");
            }
            assert_eq!(
                triple_expectation(&three, &a, &b, ta, tb, &mut cached),
                actual
            );
            assert_eq!(cached.triples.len(), 1);
            // A different first opponent must miss the pair cache but reuse
            // the second-opponent contraction, with an independent oracle.
            let mut changed = a.clone();
            changed[0] += 0.25;
            let changed_total = changed.iter().sum();
            let expected =
                triple_expectation(&three, &changed, &b, changed_total, tb, &mut reference);
            let actual = triple_expectation(&three, &changed, &b, changed_total, tb, &mut cached);
            for (a, b) in actual
                .0
                .iter()
                .chain(&actual.1)
                .zip(expected.0.iter().chain(&expected.1))
            {
                assert!((a - b).abs() < 1e-12);
            }
            assert_eq!(cached.triples.len(), 2);
            assert_eq!(cached.contractions.len(), 1);
        }
        let aa = classes.names.iter().position(|name| name == "AA").unwrap();
        let mut concentrated = vec![0.0; CLASS_COUNT];
        concentrated[aa] = 1.0;
        let zero = vec![0.0; CLASS_COUNT];
        for (a, b) in [
            (&zero, &concentrated),
            (&concentrated, &zero),
            (&concentrated, &concentrated),
        ] {
            let mut oracle = RangeCache::default().reference_mode(true);
            let mut fast = RangeCache::default();
            let expected =
                triple_expectation(&three, a, b, a.iter().sum(), b.iter().sum(), &mut oracle);
            let actual =
                triple_expectation(&three, a, b, a.iter().sum(), b.iter().sum(), &mut fast);
            assert_eq!(actual, expected);
            assert_eq!(actual.0[aa], 0.0);
            assert_eq!(actual.1[aa], 0.0); // impossible AA/AA/AA excludes investment too
        }
    }
    #[test]
    #[ignore = "bounded production-table profiling; external readonly inputs required"]
    fn production_terminal_probe() {
        let three =
            ThreeWayEquityTable::load(std::env::var("PREFLOP_THREE_WAY_TABLE").unwrap()).unwrap();
        let classes = HandClasses::new();
        let a: Vec<_> = classes.combos.iter().map(|cs| cs.len() as f64).collect();
        for reference in [true, false] {
            let mut cache = RangeCache::default().reference_mode(reference);
            cache.profile = true;
            let start = Instant::now();
            let mut last = None;
            for _ in 0..20 {
                last = Some(triple_expectation(
                    &three,
                    &a,
                    &a,
                    COMBO_COUNT as f64,
                    COMBO_COUNT as f64,
                    &mut cache,
                ));
            }
            eprintln!(
                "production_probe reference={reference} seconds={} checksum={}",
                start.elapsed().as_secs_f64(),
                last.unwrap().0.iter().sum::<f64>()
            );
        }
    }
    #[test]
    fn full_legacy_hu_40bb_bit_exact_golden() {
        // Original HEAD 14d9246 isolated build, 10 iterations, MC seed20261015/sample1.
        // Hash covers all EV/BR values and every node/class/action frequency.
        let c: PreflopConfig =
            serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap();
        let mut solver = PreflopSolver::new(
            PreflopTree::build_legacy(c).unwrap(),
            EquityTable::generate(1, 20261015).unwrap(),
            RealizationConfig::default(),
        )
        .unwrap();
        solver.run(10);
        assert_eq!(
            golden_hash(serde_json::to_value(solver.report()).unwrap()),
            0x6f9de4c0ca91a00d
        );
    }
    #[test]
    #[ignore = "external read-only 100BB config required"]
    fn full_legacy_hu_100bb_bit_exact_golden() {
        let path = std::env::var("PREFLOP_100BB_CONFIG").expect("set PREFLOP_100BB_CONFIG");
        let c = PreflopConfig::from_path(path)
            .unwrap()
            .heads_up_btn_bb()
            .unwrap();
        let mut solver = PreflopSolver::new(
            PreflopTree::build_legacy(c).unwrap(),
            EquityTable::generate(1, 20261015).unwrap(),
            RealizationConfig::default(),
        )
        .unwrap();
        solver.run(10);
        assert_eq!(
            golden_hash(serde_json::to_value(solver.report()).unwrap()),
            0xdca0c834b8a1a935
        );
    }
    #[test]
    #[ignore = "full6 100BB benchmark: roughly 1.4GB and substantial CPU; run alone"]
    fn six_seat_100bb_three_iterations_smoke() {
        let mut c: PreflopConfig = if let Ok(path) = std::env::var("PREFLOP_100BB_CONFIG") {
            PreflopConfig::from_path(path).unwrap()
        } else {
            serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap()
        };
        c.players = 6;
        c.positions = ["UTG", "HJ", "CO", "BTN", "SB", "BB"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        c.stack_bb = 100.0;
        let mut solver = PreflopSolver::new(
            PreflopTree::build(c).unwrap(),
            synthetic_equity(),
            RealizationConfig::default(),
        )
        .unwrap();
        solver.run(3);
        let report = solver.report_summary();
        assert_eq!(report.iterations, 3);
        assert_eq!(report.profile_ev_bb.len(), 6);
        for seat in ["UTG", "HJ", "CO", "BTN", "SB"] {
            let s = &report.summary.opening_by_position[seat];
            assert!((0.0..=1.0).contains(&s.raise));
            assert!((0.0..=1.0).contains(&s.complete));
            assert!(s.raise + s.complete <= 1.0 + 1e-12);
        }
        assert!(report.profile_ev_bb.iter().all(|v| v.is_finite()));
        eprintln!("{}", serde_json::to_string(&report).unwrap());
    }
    #[test]
    fn folded_hero_subtree_pruning_matches_exhaustive_three_seat_values() {
        let mut solver = PreflopSolver::new(
            PreflopTree::push_fold(config_three()).unwrap(),
            synthetic_equity(),
            RealizationConfig::default(),
        )
        .unwrap();
        solver.run(3);
        let strategy = solver.strategies(true);
        let reach = vec![vec![1.0; COMBO_COUNT]; 3];
        for p in 0..3 {
            for best in [false, true] {
                let pruned = solver.multi_evaluate(solver.tree.root, p, &strategy, &reach, best);
                let full = multi_traverse(
                    &solver.tree,
                    &solver.classes,
                    &solver.equity,
                    None,
                    &solver.realization,
                    &strategy,
                    &mut [],
                    solver.tree.root,
                    p,
                    &reach,
                    best,
                    false,
                    false,
                    &mut RangeCache::new(),
                );
                assert!(pruned.iter().zip(full).all(|(a, b)| (a - b).abs() < 1e-12));
            }
        }
    }
    #[test]
    fn three_seat_push_fold_converges_and_preserves_class_reaches() {
        let mut solver = PreflopSolver::new(
            PreflopTree::push_fold(config_three()).unwrap(),
            synthetic_equity(),
            RealizationConfig::default(),
        )
        .unwrap();
        assert!(solver
            .states
            .iter()
            .flatten()
            .all(|s| s.regret.len() % CLASS_COUNT == 0 && s.regret.len() < COMBO_COUNT));
        solver.run(5);
        let early = solver.report();
        // Bounding concurrent seat evaluations must not change any report value.
        for seats in [1, 3] {
            solver.set_report_parallelism(seats);
            let bounded = solver.report();
            assert_eq!(bounded.profile_ev_bb, early.profile_ev_bb);
            assert_eq!(bounded.best_response_ev_bb, early.best_response_ev_bb);
        }
        solver.set_report_parallelism(DEFAULT_REPORT_PARALLELISM);
        solver.run(95);
        let late = solver.report();
        eprintln!(
            "three-seat toy NashConv iteration5={} iteration100={}",
            early.nash_conv_bb, late.nash_conv_bb
        );
        assert!(
            late.nash_conv_bb < early.nash_conv_bb,
            "{} -> {}",
            early.nash_conv_bb,
            late.nash_conv_bb
        );
        assert!(late.nash_conv_bb < 1e-3, "{}", late.nash_conv_bb);
        assert_eq!(late.profile_ev_bb.len(), 3);
        assert!((late.best_response_gain_bb.iter().sum::<f64>() - late.nash_conv_bb).abs() < 1e-12);
        for n in &late.nodes {
            for h in &n.classes {
                assert!((h.frequencies.iter().sum::<f64>() - 1.0).abs() < 1e-12);
                assert!(h.frequencies.iter().all(|v| (0.0..=1.0).contains(v)));
            }
        }
        let strategy = solver.strategies(true);
        let reach = vec![vec![1.0; COMBO_COUNT]; 3];
        let v = solver.multi_evaluate(solver.tree.root, 0, &strategy, &reach, false);
        for combos in &solver.classes.combos {
            let first = v[combos[0].id()];
            assert!(combos.iter().all(|c| (v[c.id()] - first).abs() < 1e-12));
        }
    }
    #[test]
    fn cached_range_expectation_matches_physical_combo_brute_force() {
        let classes = HandClasses::new();
        let equity = synthetic_equity();
        let reach: Vec<_> = classes
            .combo_classes
            .iter()
            .map(|h| (*h % 7) as f64 / 7.0)
            .collect();
        let mut cache = RangeCache::new();
        for blocked in [false, true] {
            let (expected, mass) =
                range_expectation(&classes, &equity, &reach, blocked, &mut cache);
            for h in [0, 42, 168] {
                let hero = classes.combos[h][0];
                let mut e = 0.0;
                let mut m = 0.0;
                for (c, &r) in reach.iter().enumerate() {
                    if !blocked || hero.mask() & Combo::from_id(c).unwrap().mask() == 0 {
                        e += r * equity.get(h, classes.combo_classes[c]);
                        m += r;
                    }
                }
                let denominator = if blocked {
                    LEGAL_OPPONENT_COMBOS
                } else {
                    COMBO_COUNT
                } as f64;
                assert!((expected[h] - e / denominator).abs() < 1e-12);
                assert!((mass[h] - m / denominator).abs() < 1e-12);
            }
            assert_eq!(
                (expected, mass),
                range_expectation(&classes, &equity, &reach, blocked, &mut cache)
            );
        }
        assert_eq!(cache.len(), 2);
    }
    #[test]
    fn folded_seat_reach_scales_active_hu_but_not_own_counterfactual_value() {
        let classes = HandClasses::new();
        let c = config_three();
        let tree = PreflopTree {
            config: c,
            root: 0,
            nodes: vec![Node::Terminal(Terminal {
                kind: TerminalKind::AllIn,
                contributions: vec![2.0, 0.5, 2.0],
                active: vec![true, false, true],
                winner: None,
                stage: 1,
                dead_blind_bb: 0.0,
            })],
        };
        let equity = synthetic_equity();
        let realization = RealizationConfig::default();
        let mut reach = vec![vec![1.0; COMBO_COUNT]; 3];
        let original = multi_terminal(&tree, &classes, &equity, None, &realization, 0, 0, &reach);
        reach[1].fill(0.25);
        let scaled = multi_terminal(&tree, &classes, &equity, None, &realization, 0, 0, &reach);
        assert!(original
            .iter()
            .zip(scaled)
            .all(|(a, b)| (a * 0.25 - b).abs() < 1e-12));
        let folded = multi_terminal(&tree, &classes, &equity, None, &realization, 0, 1, &reach);
        assert!(folded.iter().all(|v| (*v + 0.5).abs() < 1e-12));
    }
    #[test]
    fn three_way_all_in_payout_and_net_accounting_and_zero_reaches() {
        let classes = HandClasses::new();
        let table = ThreeWayEquityTable::generate(1, 99).unwrap();
        let mut c = config_three();
        c.rake.rate = 0.05;
        c.rake.cap_bb = 3.0;
        c.stack_bb = 100.0;
        let tree = PreflopTree {
            config: c,
            root: 0,
            nodes: vec![Node::Terminal(Terminal {
                kind: TerminalKind::AllIn,
                contributions: vec![100.0; 3],
                active: vec![true; 3],
                winner: None,
                stage: 3,
                dead_blind_bb: 0.0,
            })],
        };
        let ids: Vec<_> = ["AA", "KK", "QQ"]
            .iter()
            .map(|name| classes.names.iter().position(|n| n == name).unwrap())
            .collect();
        let mut reaches = vec![vec![0.0; COMBO_COUNT]; 3];
        for p in 0..3 {
            for combo in &classes.combos[ids[p]] {
                reaches[p][combo.id()] = COMBO_COUNT as f64 / classes.combos[ids[p]].len() as f64;
            }
        }
        let equity = synthetic_equity();
        let realization = RealizationConfig::default();
        let mut net = 0.0;
        for p in 0..3 {
            let value = multi_terminal(
                &tree,
                &classes,
                &equity,
                Some(&table),
                &realization,
                0,
                p,
                &reaches,
            );
            net += value[classes.combos[ids[p]][0].id()];
        }
        assert!((net + 3.0).abs() < 1e-10, "net={net}");
        assert!((net + 300.0 - 297.0).abs() < 1e-10);
        let baseline = multi_terminal(
            &tree,
            &classes,
            &equity,
            Some(&table),
            &realization,
            0,
            0,
            &reaches,
        );
        reaches[0].fill(0.0);
        assert_eq!(
            baseline,
            multi_terminal(
                &tree,
                &classes,
                &equity,
                Some(&table),
                &realization,
                0,
                0,
                &reaches
            )
        );
        reaches[1].fill(0.0);
        assert!(multi_terminal(
            &tree,
            &classes,
            &equity,
            Some(&table),
            &realization,
            0,
            0,
            &reaches
        )
        .iter()
        .all(|&v| v == 0.0));
    }
    #[test]
    fn hu_golden_legacy() {
        let mut solver = PreflopSolver::new(
            PreflopTree::build_legacy(config()).unwrap(),
            synthetic_equity(),
            RealizationConfig::default(),
        )
        .unwrap();
        solver.run(10);
        let mut value = serde_json::to_value(solver.report()).unwrap();
        value.as_object_mut().unwrap().remove("training_seconds");
        value.as_object_mut().unwrap().remove("report_seconds");
        value["realization"]
            .as_object_mut()
            .unwrap()
            .remove("multiway");
        value["summary"]
            .as_object_mut()
            .unwrap()
            .remove("opening_by_position");
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            include_str!("../../../fixtures/preflop-hu-golden.json")
        );
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
