//! Public solving API. Input ranges remain external; chip amounts are integers.
mod hand_ev;
use cards::{Card, Combo};
use cfr::{Engine, EngineConfig, Metrics};
pub use hand_ev::{ClassValue, ComboValue, HandEvReport, PlayerHandValues};
use ranges::Range;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;
use tree::{Action, GameTree, NodeKind, TreeConfig};

#[derive(Clone, Debug, Serialize)]
pub struct GameSpec {
    pub board: Vec<Card>,
    /// Chip precision used by the caller (the engine operates on integer chips).
    pub units_per_bb: i64,
    pub tree: TreeConfig,
    pub engine: EngineConfig,
}

/// Adaptive cadence for exact average-EV and best-response evaluations.
#[derive(Clone, Debug, Serialize)]
pub struct AdaptiveMetrics {
    pub early_every: usize,
    pub near_every: usize,
    /// Enter the near phase permanently after an exact metric reaches target * multiplier.
    pub near_target_multiplier: f64,
}
impl Default for AdaptiveMetrics {
    fn default() -> Self {
        Self {
            early_every: 25,
            near_every: 10,
            near_target_multiplier: 2.0,
        }
    }
}
impl AdaptiveMetrics {
    fn interval(
        &self,
        near: bool,
        previous: Option<(usize, f64)>,
        current: (usize, f64),
        target: f64,
    ) -> usize {
        if near {
            return self.near_every;
        }
        let Some((iteration, exploitability)) = previous else {
            return self.early_every;
        };
        let improvement = exploitability - current.1;
        if current.0 <= iteration || !improvement.is_finite() || improvement <= 0.0 {
            return self.early_every;
        }
        // A local slope is only a scheduling hint, never a substitute for exact
        // EV/BR or a stopping decision. Check halfway to the predicted near
        // boundary; cap both ends because convergence can be noisy/nonlinear.
        let remaining = (current.1 - target * self.near_target_multiplier)
            * (current.0 - iteration) as f64
            / improvement;
        ((remaining * 0.5) as usize)
            .max(self.near_every)
            .min(self.early_every)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StopConditions {
    pub target_exploitability_pct: f64,
    pub max_iterations: usize,
    pub max_seconds: f64,
    /// Evaluate average EV and best responses at this interval and at final stop.
    pub metrics_every: usize,
    /// None preserves the fixed metrics_every cadence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_metrics: Option<AdaptiveMetrics>,
}
impl Default for StopConditions {
    fn default() -> Self {
        Self {
            target_exploitability_pct: 0.3,
            max_iterations: 1_000,
            max_seconds: 60.0,
            metrics_every: 10,
            adaptive_metrics: None,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct IterationRecord {
    pub iteration: usize,
    pub elapsed_seconds: f64,
    pub iteration_seconds: f64,
    /// None on iterations without a fresh metric evaluation.
    pub metrics: Option<Metrics>,
    pub training_seconds: f64,
    pub metrics_seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComboStrategy {
    pub dense_id: usize,
    pub combo_id: usize,
    pub combo: String,
    pub weight: f64,
    /// Compatible-joint average-profile reach at this node (unnormalized).
    pub reach_weight: f64,
    pub probabilities: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeStrategy {
    pub node: usize,
    pub player: usize,
    pub actions: Vec<Action>,
    pub combos: Vec<ComboStrategy>,
    /// Compatible-joint-reach-weighted average within each hand class.
    pub classes: BTreeMap<String, Vec<f64>>,
}

/// Portable continuation input. Each weight is INITIAL own combo weight times
/// only that player's average action probabilities. It is not a joint marginal.
/// Chance likelihood is omitted: legal private deals all have the same runout
/// factor; private-card blockers are applied explicitly instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReachSnapshot {
    pub source_node: usize,
    pub board: Vec<Card>,
    pub pot: i64,
    pub stacks: [i64; 2],
    /// Chips sunk since the source solution root. EV is centered on half the
    /// starting pot; use source_frame_ev rather than subtracting these alone.
    pub invested: [i64; 2],
    pub source_starting_pot: i64,
    pub units_per_bb: i64,
    pub ranges: [Range; 2],
    pub tree_config: TreeConfig,
    pub is_street_start: bool,
    pub can_deal_next: bool,
}
impl ReachSnapshot {
    /// Condition a pre-turn / pre-river chance snapshot on one public card.
    /// Zero entries are retained in the snapshot for auditing dense combo IDs.
    pub fn with_card(mut self, card: Card) -> Result<Self, String> {
        if !self.can_deal_next || !(3..=4).contains(&self.board.len()) {
            return Err("snapshot is not a pre-turn or pre-river chance state".into());
        }
        self.board.push(card);
        let mask = cards::validate_board(&self.board)?;
        for range in &mut self.ranges {
            // Derived serde accepts raw Card IDs; validate before any bit shift.
            // This also rejects duplicate combos and invalid weights while the
            // snapshot itself retains its zero entries and dense ordering.
            Range::from_combos(range.combos.iter().map(|e| (e.combo, e.weight)).collect())?;
            for entry in &mut range.combos {
                entry.combo = Combo::new(entry.combo.cards[0], entry.combo.cards[1])?;
                if entry.combo.mask() & mask != 0 {
                    entry.weight = 0.0;
                }
            }
        }
        self.is_street_start = true;
        self.can_deal_next = false;
        Ok(self)
    }
    pub fn source_frame_ev(&self, continuation_ev: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|p| {
            continuation_ev[p] + (self.pot as f64 - self.source_starting_pot as f64) / 2.0
                - self.invested[p] as f64
        })
    }
}

/// Start a fresh turn / river game at a street boundary. Pending wagers and
/// unequal stacks are deliberately rejected rather than silently reconstructed.
/// The inherited full pot is raked once at terminal; past rake is not charged.
/// Exploitability is reported against this snapshot's starting pot.
pub fn solve_subgame(
    snapshot: ReachSnapshot,
    engine: EngineConfig,
    stop: StopConditions,
) -> Result<Solution, String> {
    solve_subgame_with_progress(snapshot, engine, stop, |_| Ok(()))
}
pub fn solve_subgame_with_progress(
    snapshot: ReachSnapshot,
    engine: EngineConfig,
    stop: StopConditions,
    progress: impl FnMut(&ProgressEvent) -> Result<(), String> + Send,
) -> Result<Solution, String> {
    if !snapshot.is_street_start || !(4..=5).contains(&snapshot.board.len()) {
        return Err("subgame requires the beginning of a turn or river".into());
    }
    if snapshot.stacks[0] != snapshot.stacks[1] || snapshot.stacks[0] < 0 {
        return Err("subgame requires equal nonnegative remaining stacks".into());
    }
    let mut config = snapshot.tree_config;
    config.starting_pot = snapshot.pot;
    config.effective_stack = snapshot.stacks[0];
    solve_with_progress(
        GameSpec {
            board: snapshot.board,
            units_per_bb: snapshot.units_per_bb,
            tree: config,
            engine,
        },
        snapshot.ranges,
        stop,
        progress,
    )
}
pub fn solve_turn(
    snapshot: ReachSnapshot,
    engine: EngineConfig,
    stop: StopConditions,
) -> Result<Solution, String> {
    if snapshot.board.len() != 4 {
        return Err("turn solve requires four board cards".into());
    }
    solve_subgame(snapshot, engine, stop)
}
pub fn solve_river(
    snapshot: ReachSnapshot,
    engine: EngineConfig,
    stop: StopConditions,
) -> Result<Solution, String> {
    if snapshot.board.len() != 5 {
        return Err("river solve requires five board cards".into());
    }
    solve_subgame(snapshot, engine, stop)
}

pub struct Solution {
    engine: Engine,
    pub game_spec: GameSpec,
    pub stop_conditions: StopConditions,
    pub history: Vec<IterationRecord>,
    pub metrics: Metrics,
    pub setup_seconds: f64,
    pub solve_seconds: f64,
    pub training_seconds: f64,
    pub metrics_seconds: f64,
    pub stopped_by: String,
    pub estimated_memory_bytes: usize,
}
#[derive(Serialize)]
pub struct SolutionReport<'a> {
    pub solver_version: &'static str,
    pub game_spec: &'a GameSpec,
    pub stop_conditions: &'a StopConditions,
    pub ranges: &'a [ranges::DenseRange; 2],
    pub iterations: usize,
    pub history: &'a [IterationRecord],
    pub metrics: &'a Metrics,
    pub root_ev_chips: [f64; 2],
    pub root_strategy: Option<NodeStrategy>,
    pub after_check_strategy: Option<NodeStrategy>,
    pub memory_bytes: usize,
    pub persistent_storage_bytes: usize,
    pub arena_nodes: usize,
    pub isomorphism_used: bool,
    pub merged_chance_children: usize,
    pub setup_seconds: f64,
    pub solve_seconds: f64,
    /// Sum of timed terminal kernel work, not parallel wall-clock time.
    pub terminal_seconds: f64,
    pub training_seconds: f64,
    pub metrics_seconds: f64,
    pub profile: cfr::WorkProfile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_diagnostics: Option<&'a cfr::KernelDiagnostics>,
    pub stopped_by: &'a str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ProgressEvent {
    Setup {
        setup_seconds: f64,
        memory_bytes: usize,
        arena_nodes: usize,
    },
    Checkpoint {
        record: IterationRecord,
    },
}

pub fn solve(
    game_spec: GameSpec,
    input_ranges: [Range; 2],
    stop_conditions: StopConditions,
) -> Result<Solution, String> {
    solve_with_progress(game_spec, input_ranges, stop_conditions, |_| Ok(()))
}

/// Callback runs on the solver worker and must not re-enter the engine. Errors stop
/// the solve rather than silently losing progress evidence.
pub fn solve_with_progress(
    game_spec: GameSpec,
    input_ranges: [Range; 2],
    stop_conditions: StopConditions,
    mut progress: impl FnMut(&ProgressEvent) -> Result<(), String> + Send,
) -> Result<Solution, String> {
    if game_spec.units_per_bb <= 0 {
        return Err("units_per_bb must be positive".into());
    }
    if stop_conditions.metrics_every == 0 {
        return Err("metrics_every must be positive".into());
    }
    if !stop_conditions.target_exploitability_pct.is_finite()
        || stop_conditions.target_exploitability_pct < 0.0
        || !stop_conditions.max_seconds.is_finite()
        || stop_conditions.max_seconds < 0.0
    {
        return Err("stop conditions must be finite and nonnegative".into());
    }
    if let Some(schedule) = &stop_conditions.adaptive_metrics {
        if schedule.early_every == 0 || schedule.near_every == 0 {
            return Err("adaptive metrics intervals must be positive".into());
        }
        if !schedule.near_target_multiplier.is_finite() || schedule.near_target_multiplier < 1.0 {
            return Err("near_target_multiplier must be finite and at least 1".into());
        }
    }
    let setup_start = Instant::now();
    let dense = [
        input_ranges[0].blocked(&game_spec.board)?,
        input_ranges[1].blocked(&game_spec.board)?,
    ];
    // Tree-only construction cap. Compact engine scratch is depth/worker based,
    // not hand_count * arena_nodes. Each node has small boards/edges/actions;
    // 512 bytes accounts for their bounded allocations and Vec capacity growth.
    // Engine::new separately checks the complete mode-aware solver payload.
    let max_nodes = game_spec.engine.memory_limit_bytes / 512;
    let tree = GameTree::build_with_node_limit(
        game_spec.board.iter().map(|c| c.0).collect(),
        game_spec.tree.clone(),
        max_nodes,
    )?;
    let mut engine = Engine::new(tree, dense, game_spec.engine.clone())?;
    let history_bytes = stop_conditions
        .max_iterations
        .checked_mul(std::mem::size_of::<IterationRecord>())
        .ok_or("history memory estimate overflow")?;
    let estimated_memory_bytes = engine
        .memory_bytes()
        .checked_add(history_bytes)
        .ok_or("memory estimate overflow")?;
    if estimated_memory_bytes > game_spec.engine.memory_limit_bytes {
        return Err("engine plus iteration history exceeds memory limit".into());
    }
    let mut history: Vec<IterationRecord> = Vec::new();
    history
        .try_reserve_exact(stop_conditions.max_iterations)
        .map_err(|e| format!("cannot reserve iteration history: {e}"))?;
    let setup_seconds = setup_start.elapsed().as_secs_f64();
    progress(&ProgressEvent::Setup {
        setup_seconds,
        memory_bytes: estimated_memory_bytes,
        arena_nodes: engine.tree().nodes.len(),
    })?;
    let start = Instant::now();
    let mut training_seconds = 0.0;
    let mut metrics_seconds = 0.0;
    let (metrics, stopped_by) = engine.with_pool(|engine| -> Result<_, String> {
        let metric_start = Instant::now();
        let mut metrics = engine.metrics();
        metrics_seconds += metric_start.elapsed().as_secs_f64();
        progress(&ProgressEvent::Checkpoint {
            record: IterationRecord {
                iteration: 0,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                iteration_seconds: 0.0,
                metrics: Some(metrics.clone()),
                training_seconds,
                metrics_seconds,
            },
        })?;
        let mut measured_iteration = 0;
        let mut previous_measurement = None;
        let mut near_target = stop_conditions
            .adaptive_metrics
            .as_ref()
            .is_some_and(|schedule| {
                metrics.exploitability_pct
                    <= stop_conditions.target_exploitability_pct * schedule.near_target_multiplier
            });
        let reason = loop {
            if measured_iteration == engine.iterations()
                && metrics.exploitability_pct <= stop_conditions.target_exploitability_pct
            {
                break "target";
            }
            let limit = if engine.iterations() >= stop_conditions.max_iterations {
                Some("max_iterations")
            } else if start.elapsed().as_secs_f64() >= stop_conditions.max_seconds {
                Some("max_seconds")
            } else {
                None
            };
            if let Some(reason) = limit {
                if measured_iteration != engine.iterations() {
                    let metric_start = Instant::now();
                    metrics = engine.metrics();
                    metrics_seconds += metric_start.elapsed().as_secs_f64();
                    if let Some(record) = history.last_mut() {
                        record.metrics = Some(metrics.clone());
                        record.metrics_seconds = metrics_seconds;
                        record.elapsed_seconds = start.elapsed().as_secs_f64();
                        progress(&ProgressEvent::Checkpoint {
                            record: record.clone(),
                        })?;
                    }
                }
                break if metrics.exploitability_pct <= stop_conditions.target_exploitability_pct {
                    "target"
                } else {
                    reason
                };
            }
            let iter_start = Instant::now();
            engine.iterate();
            let iteration_seconds = iter_start.elapsed().as_secs_f64();
            training_seconds += iteration_seconds;
            let interval = stop_conditions.adaptive_metrics.as_ref().map_or(
                stop_conditions.metrics_every,
                |schedule| {
                    schedule.interval(
                        near_target,
                        previous_measurement,
                        (measured_iteration, metrics.exploitability_pct),
                        stop_conditions.target_exploitability_pct,
                    )
                },
            );
            let fresh = engine.iterations() - measured_iteration >= interval;
            if fresh {
                previous_measurement = Some((measured_iteration, metrics.exploitability_pct));
                let metric_start = Instant::now();
                metrics = engine.metrics();
                metrics_seconds += metric_start.elapsed().as_secs_f64();
                measured_iteration = engine.iterations();
                if let Some(schedule) = &stop_conditions.adaptive_metrics {
                    near_target |= metrics.exploitability_pct
                        <= stop_conditions.target_exploitability_pct
                            * schedule.near_target_multiplier;
                }
            }
            history.push(IterationRecord {
                iteration: engine.iterations(),
                elapsed_seconds: start.elapsed().as_secs_f64(),
                iteration_seconds,
                metrics: fresh.then(|| metrics.clone()),
                training_seconds,
                metrics_seconds,
            });
            if fresh {
                progress(&ProgressEvent::Checkpoint {
                    record: history.last().unwrap().clone(),
                })?;
            }
        };
        Ok((metrics, reason))
    })?;
    Ok(Solution {
        engine,
        game_spec,
        stop_conditions,
        history,
        metrics,
        setup_seconds,
        solve_seconds: start.elapsed().as_secs_f64(),
        training_seconds,
        metrics_seconds,
        stopped_by: stopped_by.to_owned(),
        estimated_memory_bytes,
    })
}
impl Solution {
    pub fn tree(&self) -> &GameTree {
        self.engine.tree()
    }
    pub fn root_ev(&self) -> [f64; 2] {
        self.metrics.average_ev
    }
    pub fn after_check_node(&self) -> Option<usize> {
        let NodeKind::Decision {
            actions, children, ..
        } = &self.tree().nodes[self.tree().root].kind
        else {
            return None;
        };
        actions
            .iter()
            .position(|a| *a == Action::Check)
            .map(|i| children[i])
            .filter(|&id| {
                matches!(
                    self.tree().nodes[id].kind,
                    NodeKind::Decision { player: 1, .. }
                )
            })
    }
    /// Own action reach for both players, without opponent compatibility. Public runout likelihood is a common
    /// node factor and is omitted (it cancels in normalized frequencies).
    pub fn reaches(&self, target: usize) -> Result<[Vec<f64>; 2], String> {
        fn path(tree: &GameTree, id: usize, target: usize, out: &mut Vec<(usize, usize)>) -> bool {
            if id == target {
                return true;
            }
            let children = match &tree.nodes[id].kind {
                NodeKind::Decision { children, .. } | NodeKind::Chance { children, .. } => children,
                _ => return false,
            };
            for (a, &child) in children.iter().enumerate() {
                out.push((id, a));
                if path(tree, child, target, out) {
                    return true;
                }
                out.pop();
            }
            false
        }
        let mut edges = Vec::new();
        if !path(self.tree(), self.tree().root, target, &mut edges) {
            return Err("unknown node".into());
        }
        let ranges = self.engine.ranges();
        let mut reaches = ranges
            .each_ref()
            .map(|r| r.combos.iter().map(|c| c.weight).collect::<Vec<_>>());
        for (id, action) in edges {
            if let NodeKind::Decision { player, .. } = self.tree().nodes[id].kind {
                let probabilities = self.engine.strategy(id)?;
                let len = ranges[player].len();
                for (hand, reach) in reaches[player].iter_mut().enumerate() {
                    *reach *= probabilities[action * len + hand] as f64;
                }
            }
        }
        let mask = self.tree().nodes[target]
            .board
            .iter()
            .fold(0, |m, &c| m | (1u64 << c));
        for p in 0..2 {
            for (i, entry) in ranges[p].combos.iter().enumerate() {
                if entry.combo.mask() & mask != 0 {
                    reaches[p][i] = 0.0;
                }
            }
        }
        Ok(reaches)
    }
    /// Both own-reach ranges and exact public state at any arena node. Suit
    /// aliases are expanded by Engine::strategy before multiplying own actions.
    pub fn reach_snapshot(&self, node: usize) -> Result<ReachSnapshot, String> {
        let state = self.tree().nodes.get(node).ok_or("unknown node")?;
        let reaches = self.reaches(node)?;
        let ranges = std::array::from_fn(|p| Range {
            player: self.engine.ranges()[p].player.clone(),
            combos: self.engine.ranges()[p]
                .combos
                .iter()
                .enumerate()
                .map(|(i, c)| ranges::WeightedCombo {
                    combo: c.combo,
                    weight: reaches[p][i],
                })
                .collect(),
        });
        let is_street_start = node == self.tree().root || self.tree().nodes.iter().any(|parent| {
            matches!(&parent.kind, NodeKind::Chance { children, .. } if children.contains(&node))
        });
        Ok(ReachSnapshot {
            source_node: node,
            board: state.board.iter().copied().map(Card).collect(),
            pot: state.pot,
            stacks: state.stacks,
            invested: state.invested,
            source_starting_pot: self.game_spec.tree.starting_pot,
            units_per_bb: self.game_spec.units_per_bb,
            ranges,
            tree_config: self.game_spec.tree.clone(),
            is_street_start,
            can_deal_next: matches!(state.kind, NodeKind::Chance { .. }) && state.board.len() < 5,
        })
    }
    /// Exact average-profile and best-response metrics conditioned on a node.
    /// EV is in the original solution's root frame (includes sunk investment).
    /// Exploitability's denominator is the selected node's pot, not flop pot.
    pub fn node_metrics(&mut self, node: usize) -> Result<Metrics, String> {
        let reaches = self.reaches(node)?;
        self.engine.metrics_at_node(node, reaches)
    }
    pub fn strategy(&self, node: usize) -> Result<NodeStrategy, String> {
        let n = self.tree().nodes.get(node).ok_or("unknown node")?;
        let NodeKind::Decision {
            player, actions, ..
        } = &n.kind
        else {
            return Err("node is not a decision".into());
        };
        let range = &self.engine.ranges()[*player];
        let probabilities = self.engine.strategy(node)?;
        let reaches = self.reaches(node)?;
        let other = &self.engine.ranges()[1 - *player];
        let board_mask = n.board.iter().fold(0u64, |m, &c| m | (1u64 << c));
        let mut combos = Vec::new();
        let mut classes: BTreeMap<String, (f64, Vec<f64>)> = BTreeMap::new();
        for (id, entry) in range.combos.iter().enumerate() {
            if entry.combo.mask() & board_mask != 0 {
                continue;
            }
            let probs: Vec<f32> = (0..actions.len())
                .map(|a| probabilities[a * range.len() + id])
                .collect();
            let reach_weight = reaches[*player][id]
                * other
                    .combos
                    .iter()
                    .enumerate()
                    .filter(|(_, opponent)| opponent.combo.mask() & entry.combo.mask() == 0)
                    .map(|(i, _)| reaches[1 - *player][i])
                    .sum::<f64>();
            let class = classes
                .entry(hand_class(entry.combo))
                .or_insert_with(|| (0.0, vec![0.0; actions.len()]));
            class.0 += reach_weight;
            for (sum, &prob) in class.1.iter_mut().zip(&probs) {
                *sum += reach_weight * prob as f64;
            }
            combos.push(ComboStrategy {
                dense_id: id,
                combo_id: entry.combo.id(),
                combo: entry.combo.to_string(),
                weight: entry.weight,
                reach_weight,
                probabilities: probs,
            });
        }
        Ok(NodeStrategy {
            node,
            player: *player,
            actions: actions.clone(),
            combos,
            classes: classes
                .into_iter()
                .map(|(name, (weight, mut values))| {
                    for value in &mut values {
                        if weight > 0.0 {
                            *value /= weight;
                        }
                    }
                    (name, values)
                })
                .collect(),
        })
    }
    pub fn report(&self) -> SolutionReport<'_> {
        SolutionReport {
            solver_version: env!("CARGO_PKG_VERSION"),
            game_spec: &self.game_spec,
            stop_conditions: &self.stop_conditions,
            ranges: self.engine.ranges(),
            iterations: self.engine.iterations(),
            history: &self.history,
            metrics: &self.metrics,
            root_ev_chips: self.root_ev(),
            root_strategy: self.strategy(self.tree().root).ok(),
            after_check_strategy: self
                .after_check_node()
                .and_then(|id| self.strategy(id).ok()),
            memory_bytes: self.estimated_memory_bytes,
            persistent_storage_bytes: self.engine.persistent_storage_bytes(),
            arena_nodes: self.tree().nodes.len(),
            isomorphism_used: self.engine.isomorphism_used(),
            merged_chance_children: self.engine.merged_chance_children(),
            setup_seconds: self.setup_seconds,
            solve_seconds: self.solve_seconds,
            terminal_seconds: self.engine.terminal_seconds(),
            training_seconds: self.training_seconds,
            metrics_seconds: self.metrics_seconds,
            profile: self.engine.profile(),
            kernel_diagnostics: self.engine.kernel_diagnostics(),
            stopped_by: &self.stopped_by,
        }
    }
}
pub fn hand_class(combo: Combo) -> String {
    let [a, b] = combo.cards;
    let high = a.rank().max(b.rank()) as usize;
    let low = a.rank().min(b.rank()) as usize;
    let ranks = b"23456789TJQKA";
    if high == low {
        format!("{}{}", ranks[high] as char, ranks[low] as char)
    } else {
        format!(
            "{}{}{}",
            ranks[high] as char,
            ranks[low] as char,
            if a.suit() == b.suit() { 's' } else { 'o' }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> (GameSpec, [Range; 2]) {
        let board = ["2c", "3d", "7h", "8s", "9c"]
            .iter()
            .map(|c| c.parse().unwrap())
            .collect();
        let ranges = [
            Range::from_json(r#"{"AhAd":1,"KhKd":1}"#).unwrap(),
            Range::from_json(r#"{"QcQd":1,"JhJd":1}"#).unwrap(),
        ];
        (
            GameSpec {
                board,
                units_per_bb: tree::BB,
                tree: TreeConfig {
                    starting_pot: 2000,
                    effective_stack: 1000,
                    bet_size: tree::SizeRule::Fixed(0.5),
                    rake_rate: 0.0,
                    ..TreeConfig::default()
                },
                engine: EngineConfig::default(),
            },
            ranges,
        )
    }
    #[test]
    fn solve_report_and_arbitrary_node_strategy() {
        let (game, ranges) = example();
        let result = solve(
            game,
            ranges,
            StopConditions {
                max_iterations: 3,
                target_exploitability_pct: 0.0,
                max_seconds: 30.0,
                metrics_every: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result.history.len() <= 3);
        assert!(result.root_ev().iter().all(|x| x.is_finite()));
        for (id, node) in result.tree().nodes.iter().enumerate() {
            if matches!(node.kind, NodeKind::Decision { .. }) {
                let strategy = result.strategy(id).unwrap();
                for combo in strategy.combos {
                    assert!((combo.probabilities.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                }
            }
        }
        assert!(result.strategy(usize::MAX).is_err());
        assert!(serde_json::to_value(result.report()).unwrap()["game_spec"].is_object());
    }
    #[test]
    fn metric_cadence_progress_final_and_reach_weights() {
        let (game, ranges) = example();
        let mut checkpoints = Vec::new();
        let result = solve_with_progress(
            game,
            ranges,
            StopConditions {
                max_iterations: 5,
                max_seconds: 30.0,
                target_exploitability_pct: 0.0,
                metrics_every: 2,
                ..Default::default()
            },
            |event| {
                if let ProgressEvent::Checkpoint { record } = event {
                    checkpoints.push(record.iteration);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(checkpoints.first(), Some(&0));
        assert_eq!(checkpoints.last(), Some(&result.engine.iterations()));
        for record in &result.history {
            assert_eq!(
                record.metrics.is_some(),
                record.iteration % 2 == 0 || record.iteration == result.engine.iterations()
            );
        }
        assert!(result.metrics_seconds >= 0.0 && result.training_seconds >= 0.0);
        let root = result.strategy(result.tree().root).unwrap();
        // Two compatible unit-weight opponent hands for every own hand.
        assert!(root.combos.iter().all(|c| c.reach_weight == 2.0));
        let after = result.strategy(result.after_check_node().unwrap()).unwrap();
        let check = root
            .actions
            .iter()
            .position(|a| *a == Action::Check)
            .unwrap();
        let expected: f64 = root
            .combos
            .iter()
            .map(|c| c.weight * c.probabilities[check] as f64)
            .sum();
        assert!(after
            .combos
            .iter()
            .all(|c| (c.reach_weight - expected).abs() < 1e-12));
        assert!(serde_json::to_value(result.report()).unwrap()["after_check_strategy"].is_object());
    }
    fn action_child(solution: &Solution, node: usize, action: Action) -> usize {
        let NodeKind::Decision {
            actions, children, ..
        } = &solution.tree().nodes[node].kind
        else {
            panic!("decision required");
        };
        children[actions.iter().position(|a| *a == action).unwrap()]
    }
    #[test]
    fn own_reach_card_blockers_snapshot_roundtrip() {
        let (mut game, _) = example();
        game.board.truncate(4);
        game.engine.threads = 1;
        game.tree.rake_rate = 0.05;
        game.tree.rake_cap = 3000;
        let ranges = [
            Range::from_json(r#"{"AhAd":2,"KhKd":3}"#).unwrap(),
            Range::from_json(r#"{"QcQd":5,"JhJd":7}"#).unwrap(),
        ];
        let mut solution = solve(
            game,
            ranges,
            StopConditions {
                max_iterations: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let root = solution.tree().root;
        let after = action_child(&solution, root, Action::Check);
        let chance = action_child(&solution, after, Action::Check);
        let snapshot = solution.reach_snapshot(chance).unwrap();
        let root_policy = solution.strategy(root).unwrap();
        for (entry, strategy) in snapshot.ranges[0]
            .combos
            .iter()
            .zip(root_policy.combos.iter())
        {
            assert!(
                (entry.weight - strategy.weight * strategy.probabilities[0] as f64).abs() < 1e-12
            );
        }
        // Only IP's own check is multiplied: OOP compatibility is NOT included.
        assert_eq!(
            snapshot.ranges[1]
                .combos
                .iter()
                .find(|e| e.combo == "QcQd".parse::<Combo>().unwrap())
                .unwrap()
                .weight,
            2.5
        );
        let conditioned = snapshot.clone().with_card("Ah".parse().unwrap()).unwrap();
        assert_eq!(
            conditioned.ranges[0]
                .combos
                .iter()
                .find(|e| e.combo == "AhAd".parse::<Combo>().unwrap())
                .unwrap()
                .weight,
            0.0
        );
        let card_node = match &solution.tree().nodes[chance].kind {
            NodeKind::Chance { cards, children } => {
                children[cards
                    .iter()
                    .position(|&c| c == "Ah".parse::<Card>().unwrap().0)
                    .unwrap()]
            }
            _ => unreachable!(),
        };
        let direct = solution.reach_snapshot(card_node).unwrap();
        for p in 0..2 {
            for (a, b) in conditioned.ranges[p]
                .combos
                .iter()
                .zip(&direct.ranges[p].combos)
            {
                assert_eq!(a.weight, b.weight);
            }
        }
        assert!(snapshot.clone().with_card("2c".parse().unwrap()).is_err());
        let restored: ReachSnapshot =
            serde_json::from_str(&serde_json::to_string(&conditioned).unwrap()).unwrap();
        let result = solve_river(
            restored.clone(),
            EngineConfig {
                threads: 1,
                ..Default::default()
            },
            StopConditions {
                max_iterations: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.tree().nodes[result.tree().root].pot, restored.pot);
        assert_eq!(
            result.tree().nodes[result.tree().root].stacks,
            restored.stacks
        );
        // Raked all-in continuation: centered-pot offset, not invested-only.
        let bet = action_child(&solution, root, Action::Bet(1000));
        let allin = action_child(&solution, bet, Action::Call);
        let terminal = match &solution.tree().nodes[allin].kind {
            NodeKind::Chance { cards, children } => {
                children[cards
                    .iter()
                    .position(|&c| c == "Ts".parse::<Card>().unwrap().0)
                    .unwrap()]
            }
            _ => unreachable!(),
        };
        let allin_snapshot = solution
            .reach_snapshot(allin)
            .unwrap()
            .with_card("Ts".parse().unwrap())
            .unwrap();
        assert_eq!(allin_snapshot.invested, [1000; 2]);
        let source_ev = solution.node_metrics(terminal).unwrap().average_ev;
        let continuation = solve_river(
            allin_snapshot.clone(),
            EngineConfig {
                threads: 1,
                ..Default::default()
            },
            StopConditions {
                max_iterations: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let translated = allin_snapshot.source_frame_ev(continuation.root_ev());
        for p in 0..2 {
            assert!((source_ev[p] - translated[p]).abs() < 1e-10);
        }
        let mut unequal = allin_snapshot;
        unequal.invested = [200, 300];
        unequal.pot = 2500;
        unequal.source_starting_pot = 2000;
        assert_eq!(unequal.source_frame_ev([1000.0, 1000.0]), [1050.0, 950.0]);
        assert!(solve_turn(restored, EngineConfig::default(), StopConditions::default()).is_err());
        assert!(solve_subgame(
            solution.reach_snapshot(after).unwrap(),
            EngineConfig::default(),
            StopConditions::default()
        )
        .is_err());
    }
    #[test]
    fn snapshot_conditioning_rejects_malformed_serialized_ranges() {
        let (mut game, ranges) = example();
        game.board.truncate(4);
        let snapshot = ReachSnapshot {
            source_node: 0,
            board: game.board,
            pot: 2000,
            stacks: [1000; 2],
            invested: [0; 2],
            source_starting_pot: 2000,
            units_per_bb: 1000,
            ranges,
            tree_config: game.tree,
            is_street_start: false,
            can_deal_next: true,
        };
        let mut serialized = serde_json::to_value(&snapshot).unwrap();
        serialized["ranges"][0]["combos"][0]["combo"]["cards"] = serde_json::json!([200, 201]);
        let malformed: ReachSnapshot = serde_json::from_value(serialized).unwrap();
        assert!(malformed.with_card("Ts".parse().unwrap()).is_err());
        let mut duplicate = snapshot.clone();
        duplicate.ranges[0]
            .combos
            .push(duplicate.ranges[0].combos[0].clone());
        assert!(duplicate.with_card("Ts".parse().unwrap()).is_err());
        for weight in [-1.0, f64::NAN, f64::INFINITY] {
            let mut invalid = snapshot.clone();
            invalid.ranges[0].combos[0].weight = weight;
            assert!(invalid.with_card("Ts".parse().unwrap()).is_err());
        }
    }
    #[test]
    fn suit_alias_reach_and_conditioned_metrics_match_full_tree() {
        let (mut game, _) = example();
        game.board = ["2c", "3c", "7c"]
            .iter()
            .map(|c| c.parse().unwrap())
            .collect();
        game.tree.oop_can_lead_flop = true;
        game.tree.allow_raises = false;
        game.engine.threads = 1;
        game.engine.suit_isomorphism = true;
        let ranges = [
            Range::from_json(r#"{"AhAd":1,"AhAs":1,"AdAs":1}"#).unwrap(),
            Range::from_json(r#"{"KhKd":1,"KhKs":1,"KdKs":1}"#).unwrap(),
        ];
        let stop = StopConditions {
            max_iterations: 1,
            target_exploitability_pct: 0.0,
            ..Default::default()
        };
        let mut iso = solve(game.clone(), ranges.clone(), stop.clone()).unwrap();
        assert!(iso.report().merged_chance_children > 0);
        let mut asymmetric = iso.reaches(iso.tree().root).unwrap();
        asymmetric[0][0] *= 2.0;
        assert!(iso
            .engine
            .metrics_at_node(iso.tree().root, asymmetric)
            .is_err());
        game.engine.suit_isomorphism = false;
        let mut full = solve(game, ranges, stop).unwrap();
        let checked = action_child(&iso, iso.tree().root, Action::Check);
        let chance = action_child(&iso, checked, Action::Check);
        let node = match &iso.tree().nodes[chance].kind {
            NodeKind::Chance { cards, children } => {
                children[cards
                    .iter()
                    .position(|&c| c == "Kh".parse::<Card>().unwrap().0)
                    .unwrap()]
            }
            _ => unreachable!(),
        };
        let after = action_child(&iso, node, Action::Check);
        let a = iso.reach_snapshot(after).unwrap();
        let b = full.reach_snapshot(after).unwrap();
        for p in 0..2 {
            for (a, b) in a.ranges[p].combos.iter().zip(&b.ranges[p].combos) {
                assert!((a.weight - b.weight).abs() < 1e-6);
            }
        }
        let a = iso.node_metrics(after).unwrap();
        let b = full.node_metrics(after).unwrap();
        for p in 0..2 {
            assert!((a.average_ev[p] - b.average_ev[p]).abs() < 1e-6);
            assert!((a.best_response_ev[p] - b.best_response_ev[p]).abs() < 1e-6);
        }
    }
    #[test]
    fn flop_subtree_matches_turn_resolve() {
        let (mut game, ranges) = example();
        game.board.truncate(3);
        game.tree.effective_stack = 2000;
        game.tree.oop_can_lead_flop = false;
        game.tree.allin_merge_ratio = 0.67;
        game.tree.allow_raises = false;
        game.tree.rake_rate = 0.05;
        game.tree.rake_cap = 3000;
        game.engine.threads = 1;
        game.engine.suit_isomorphism = false;
        let stop = StopConditions {
            target_exploitability_pct: 0.001,
            max_iterations: 300,
            max_seconds: 60.0,
            metrics_every: 25,
            ..Default::default()
        };
        let mut flop = solve(game.clone(), ranges, stop.clone()).unwrap();
        let root = flop.tree().root;
        let checked = action_child(&flop, root, Action::Check);
        let chance = action_child(&flop, checked, Action::Check);
        let node = match &flop.tree().nodes[chance].kind {
            NodeKind::Chance { cards, children } => {
                children[cards
                    .iter()
                    .position(|&c| c == "8s".parse::<Card>().unwrap().0)
                    .unwrap()]
            }
            _ => unreachable!(),
        };
        let snapshot = flop.reach_snapshot(node).unwrap();
        assert_eq!(snapshot.invested, [0; 2]);
        let original = flop.node_metrics(node).unwrap();
        let fresh = solve_turn(snapshot.clone(), game.engine, stop).unwrap();
        let adjusted_ev = snapshot.source_frame_ev(fresh.root_ev());
        let ev_diff = (0..2)
            .map(|p| (original.average_ev[p] - adjusted_ev[p]).abs())
            .fold(0.0, f64::max);
        let original_strategy = flop.strategy(node).unwrap();
        let fresh_strategy = fresh.strategy(fresh.tree().root).unwrap();
        let mut max_diff = 0.0_f64;
        let mut weighted = 0.0_f64;
        let mut mass = 0.0;
        for a in &original_strategy.combos {
            if let Some(b) = fresh_strategy
                .combos
                .iter()
                .find(|b| b.combo_id == a.combo_id)
            {
                for (&x, &y) in a.probabilities.iter().zip(&b.probabilities) {
                    let diff = (x as f64 - y as f64).abs();
                    max_diff = max_diff.max(diff);
                    weighted += a.reach_weight * diff;
                    mass += a.reach_weight;
                }
            }
        }
        eprintln!("subgame comparison: flop iterations={} flop exploitability_pct={} conditioned exploitability_pct={} turn iterations={} turn exploitability_pct={} ev_max_diff_chips={} root_strategy_weighted_abs_diff={} root_strategy_max_abs_diff={}", flop.history.len(), flop.metrics.exploitability_pct, original.exploitability_pct, fresh.history.len(), fresh.metrics.exploitability_pct, ev_diff, weighted/mass, max_diff);
        assert!(
            original.exploitability_pct < 0.3,
            "conditioned profile must independently meet accuracy"
        );
        assert!(fresh.metrics.exploitability_pct < 0.001);
        // Distinct equilibria need not yield identical policies. This fixture
        // compares measured policy AND payoff errors, not only BR gap.
        assert!(ev_diff < snapshot.pot as f64 * 0.001);
        assert!(weighted / mass < 0.005);
        assert!(max_diff < 0.01);
    }
    #[test]
    fn adaptive_exact_checkpoints_and_no_duplicate_final() {
        // Cover final stops both on and off cadence, and both scheduling phases.
        for (near, max_iterations) in [(false, 6), (false, 7), (true, 6), (true, 7)] {
            let (game, ranges) = example();
            let mut checkpoints = Vec::new();
            let result = solve_with_progress(
                game,
                ranges,
                StopConditions {
                    target_exploitability_pct: 1e-12,
                    max_iterations,
                    max_seconds: 30.0,
                    adaptive_metrics: Some(AdaptiveMetrics {
                        early_every: 3,
                        near_every: 2,
                        near_target_multiplier: if near { 1e18 } else { 1.0 },
                    }),
                    ..Default::default()
                },
                |event| {
                    if let ProgressEvent::Checkpoint { record } = event {
                        checkpoints.push(record.iteration);
                    }
                    Ok(())
                },
            )
            .unwrap();
            let final_iteration = result.engine.iterations();
            assert_eq!(checkpoints.first(), Some(&0));
            assert_eq!(checkpoints.last(), Some(&final_iteration));
            assert!(checkpoints.windows(2).all(|pair| pair[0] < pair[1]));
            let interval = if near { 2 } else { 3 };
            assert!(checkpoints
                .windows(2)
                .all(|pair| pair[1] - pair[0] <= interval));
            if let Some(record) = result.history.last() {
                assert_eq!(
                    record.metrics.as_ref().unwrap().exploitability_pct,
                    result.metrics.exploitability_pct
                );
            }
        }
    }
    #[test]
    fn adaptive_prediction_is_bounded_and_near_cadence_is_stable() {
        let schedule = AdaptiveMetrics {
            early_every: 100,
            near_every: 10,
            near_target_multiplier: 2.0,
        };
        assert_eq!(schedule.interval(false, None, (0, 10.0), 1.0), 100);
        // Slope 0.1 per iteration: near boundary in 30 iterations; check halfway.
        assert_eq!(schedule.interval(false, Some((0, 6.0)), (10, 5.0), 1.0), 15);
        assert_eq!(schedule.interval(false, Some((0, 6.0)), (10, 2.1), 1.0), 10);
        assert_eq!(
            schedule.interval(false, Some((0, 6.0)), (10, 5.99), 1.0),
            100
        );
        assert_eq!(
            schedule.interval(false, Some((0, 6.0)), (10, 6.0), 1.0),
            100
        );
        assert_eq!(
            schedule.interval(false, Some((0, 6.0)), (10, 7.0), 1.0),
            100
        );
        assert_eq!(schedule.interval(true, Some((0, 6.0)), (10, 7.0), 1.0), 10);
    }
    #[test]
    fn metric_cadences_leave_training_strategy_ev_and_br_identical() {
        let (game, ranges) = example();
        let stop = StopConditions {
            target_exploitability_pct: 0.0,
            max_iterations: 7,
            max_seconds: 30.0,
            metrics_every: 1,
            ..Default::default()
        };
        let mut initial_exploitability = 0.0;
        let dense = solve_with_progress(game.clone(), ranges.clone(), stop.clone(), |event| {
            if let ProgressEvent::Checkpoint { record } = event {
                if record.iteration == 0 {
                    initial_exploitability = record.metrics.as_ref().unwrap().exploitability_pct;
                }
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(dense.engine.iterations(), 7);
        for adaptive_metrics in [
            None,
            Some(AdaptiveMetrics {
                early_every: 4,
                near_every: 2,
                near_target_multiplier: 2.0,
            }),
        ] {
            let sparse = solve(
                game.clone(),
                ranges.clone(),
                StopConditions {
                    metrics_every: 4,
                    adaptive_metrics,
                    ..stop.clone()
                },
            )
            .unwrap();
            assert_eq!(sparse.engine.iterations(), dense.engine.iterations());
            assert_eq!(sparse.stopped_by, "max_iterations");
            assert_eq!(
                serde_json::to_value(&sparse.metrics).unwrap(),
                serde_json::to_value(&dense.metrics).unwrap()
            );
            for (id, node) in dense.tree().nodes.iter().enumerate() {
                if matches!(node.kind, NodeKind::Decision { .. }) {
                    assert_eq!(
                        serde_json::to_value(sparse.strategy(id).unwrap()).unwrap(),
                        serde_json::to_value(dense.strategy(id).unwrap()).unwrap()
                    );
                }
            }
            assert_eq!(
                serde_json::to_value(sparse.history.last().unwrap().metrics.as_ref().unwrap())
                    .unwrap(),
                serde_json::to_value(&sparse.metrics).unwrap()
            );
        }
        assert!(dense.metrics.exploitability_pct < initial_exploitability);
        let target = (initial_exploitability + dense.metrics.exploitability_pct) / 2.0;
        let mut checkpoints = Vec::new();
        let targeted = solve_with_progress(
            game,
            ranges,
            StopConditions {
                target_exploitability_pct: target,
                adaptive_metrics: Some(AdaptiveMetrics {
                    early_every: 2,
                    near_every: 1,
                    ..Default::default()
                }),
                ..stop
            },
            |event| {
                if let ProgressEvent::Checkpoint { record } = event {
                    checkpoints.push(record.iteration);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(targeted.stopped_by, "target");
        assert!(targeted.engine.iterations() > 0);
        assert!(targeted.metrics.exploitability_pct <= target);
        assert_eq!(checkpoints.last(), Some(&targeted.engine.iterations()));
        assert!(checkpoints.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            serde_json::to_value(targeted.history.last().unwrap().metrics.as_ref().unwrap())
                .unwrap(),
            serde_json::to_value(&targeted.metrics).unwrap()
        );
    }
    #[test]
    fn initial_target_time_and_iteration_limits_keep_fresh_metrics() {
        for (target, max_seconds, max_iterations, reason) in [
            (1e9, 30.0, 10, "target"),
            (0.0, 0.0, 10, "max_seconds"),
            (0.0, 30.0, 0, "max_iterations"),
        ] {
            let (game, ranges) = example();
            let mut checkpoints = Vec::new();
            let result = solve_with_progress(
                game,
                ranges,
                StopConditions {
                    target_exploitability_pct: target,
                    max_iterations,
                    max_seconds,
                    adaptive_metrics: Some(AdaptiveMetrics::default()),
                    ..Default::default()
                },
                |event| {
                    if let ProgressEvent::Checkpoint { record } = event {
                        checkpoints.push(record.clone());
                    }
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(result.stopped_by, reason);
            assert!(result.history.is_empty());
            assert_eq!(checkpoints.len(), 1);
            assert_eq!(checkpoints[0].iteration, 0);
            assert_eq!(
                serde_json::to_value(checkpoints[0].metrics.as_ref().unwrap()).unwrap(),
                serde_json::to_value(&result.metrics).unwrap()
            );
        }
    }
    #[test]
    fn invalid_adaptive_schedule() {
        let (game, ranges) = example();
        for schedule in [
            AdaptiveMetrics {
                early_every: 0,
                ..Default::default()
            },
            AdaptiveMetrics {
                near_every: 0,
                ..Default::default()
            },
            AdaptiveMetrics {
                near_target_multiplier: f64::NAN,
                ..Default::default()
            },
            AdaptiveMetrics {
                near_target_multiplier: 0.5,
                ..Default::default()
            },
        ] {
            assert!(solve(
                game.clone(),
                ranges.clone(),
                StopConditions {
                    adaptive_metrics: Some(schedule),
                    ..Default::default()
                }
            )
            .is_err());
        }
    }
    #[test]
    fn invalid_cadence_and_progress_errors() {
        let (game, ranges) = example();
        assert!(solve(
            game.clone(),
            ranges.clone(),
            StopConditions {
                metrics_every: 0,
                ..Default::default()
            }
        )
        .is_err());
        let error = solve_with_progress(game, ranges, StopConditions::default(), |_| {
            Err("progress unavailable".into())
        })
        .err()
        .unwrap();
        assert_eq!(error, "progress unavailable");
    }
    #[test]
    fn zero_time_and_memory_limit() {
        let (game, ranges) = example();
        let result = solve(
            game.clone(),
            ranges.clone(),
            StopConditions {
                max_seconds: 0.0,
                target_exploitability_pct: 0.0,
                ..StopConditions::default()
            },
        )
        .unwrap();
        assert_eq!(result.history.len(), 0);
        let mut too_small = game;
        too_small.engine.memory_limit_bytes = 1;
        assert!(solve(too_small, ranges, StopConditions::default()).is_err());
    }
}
