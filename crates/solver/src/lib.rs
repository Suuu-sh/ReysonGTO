//! Public solving API. Input ranges remain external; chip amounts are integers.
use cards::{Card, Combo};
use cfr::{Engine, EngineConfig, Metrics};
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

#[derive(Clone, Debug, Serialize)]
pub struct StopConditions {
    pub target_exploitability_pct: f64,
    pub max_iterations: usize,
    pub max_seconds: f64,
    /// Evaluate average EV and best responses at this interval and at final stop.
    pub metrics_every: usize,
}
impl Default for StopConditions {
    fn default() -> Self {
        Self {
            target_exploitability_pct: 0.3,
            max_iterations: 1_000,
            max_seconds: 60.0,
            metrics_every: 10,
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
            let fresh = engine.iterations() % stop_conditions.metrics_every == 0;
            if fresh {
                let metric_start = Instant::now();
                metrics = engine.metrics();
                metrics_seconds += metric_start.elapsed().as_secs_f64();
                measured_iteration = engine.iterations();
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
    /// Reach includes both players' average-profile action probabilities and
    /// exact private-card compatibility. Public runout likelihood is a common
    /// node factor and is omitted (it cancels in normalized frequencies).
    fn reaches(&self, target: usize) -> Result<[Vec<f64>; 2], String> {
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
