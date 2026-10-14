//! Public solving API. Input ranges remain external; chip amounts are integers.
use cards::{Card, Combo};
use cfr::{Engine, EngineConfig, Metrics};
use ranges::Range;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Instant;
use tree::{Action, GameTree, NodeKind, TreeConfig};

#[derive(Clone, Debug, Serialize)]
pub struct GameSpec {
    pub board: Vec<Card>,
    pub tree: TreeConfig,
    pub engine: EngineConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct StopConditions {
    pub target_exploitability_pct: f64,
    pub max_iterations: usize,
    pub max_seconds: f64,
}
impl Default for StopConditions {
    fn default() -> Self {
        Self {
            target_exploitability_pct: 0.3,
            max_iterations: 1_000,
            max_seconds: 60.0,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct IterationRecord {
    pub iteration: usize,
    pub elapsed_seconds: f64,
    pub iteration_seconds: f64,
    pub metrics: Metrics,
}
#[derive(Clone, Debug, Serialize)]
pub struct ComboStrategy {
    pub dense_id: usize,
    pub combo_id: usize,
    pub combo: String,
    pub weight: f64,
    pub probabilities: Vec<f32>,
}
#[derive(Clone, Debug, Serialize)]
pub struct NodeStrategy {
    pub node: usize,
    pub player: usize,
    pub actions: Vec<Action>,
    pub combos: Vec<ComboStrategy>,
    /// Input-range-weighted average of the board-valid combos in each class.
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
    pub memory_bytes: usize,
    pub arena_nodes: usize,
    pub isomorphism_used: bool,
    pub merged_chance_children: usize,
    pub setup_seconds: f64,
    pub solve_seconds: f64,
    /// Sum of timed terminal kernel work, not parallel wall-clock time.
    pub terminal_seconds: f64,
    pub stopped_by: &'a str,
}

pub fn solve(
    game_spec: GameSpec,
    input_ranges: [Range; 2],
    stop_conditions: StopConditions,
) -> Result<Solution, String> {
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
    // Conservative arena allowance includes Vec capacity growth, board/actions/
    // chance child heaps (at most 49 branches), plus a reach-buffer allowance.
    // Engine::new performs the complete solver-payload check after construction.
    let hand_count = dense[0].len() + dense[1].len();
    let per_node_allowance = 2048usize
        .checked_add(
            hand_count
                .checked_mul(8)
                .ok_or("memory estimate overflow")?,
        )
        .ok_or("memory estimate overflow")?;
    let max_nodes = game_spec.engine.memory_limit_bytes / per_node_allowance;
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
    let mut history = Vec::new();
    history
        .try_reserve_exact(stop_conditions.max_iterations)
        .map_err(|e| format!("cannot reserve iteration history: {e}"))?;
    let setup_seconds = setup_start.elapsed().as_secs_f64();
    let start = Instant::now();
    // Keep the full loop on one initialized Rayon worker. Repeated external
    // ThreadPool::install calls may allocate Rayon injector queue blocks.
    let (metrics, stopped_by) = engine.with_pool(|engine| {
        let mut metrics = engine.metrics();
        let reason = loop {
            if metrics.exploitability_pct <= stop_conditions.target_exploitability_pct {
                break "target";
            }
            if history.len() >= stop_conditions.max_iterations {
                break "max_iterations";
            }
            if start.elapsed().as_secs_f64() >= stop_conditions.max_seconds {
                break "max_seconds";
            }
            let iter_start = Instant::now();
            engine.iterate();
            let iteration_seconds = iter_start.elapsed().as_secs_f64();
            metrics = engine.metrics();
            history.push(IterationRecord {
                iteration: history.len() + 1,
                elapsed_seconds: start.elapsed().as_secs_f64(),
                iteration_seconds,
                metrics: metrics.clone(),
            });
        };
        (metrics, reason)
    });
    Ok(Solution {
        engine,
        game_spec,
        stop_conditions,
        history,
        metrics,
        setup_seconds,
        solve_seconds: start.elapsed().as_secs_f64(),
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
            let class = classes
                .entry(hand_class(entry.combo))
                .or_insert_with(|| (0.0, vec![0.0; actions.len()]));
            class.0 += entry.weight;
            for (sum, &prob) in class.1.iter_mut().zip(&probs) {
                *sum += entry.weight * prob as f64;
            }
            combos.push(ComboStrategy {
                dense_id: id,
                combo_id: entry.combo.id(),
                combo: entry.combo.to_string(),
                weight: entry.weight,
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
                        *value /= weight;
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
            iterations: self.history.len(),
            history: &self.history,
            metrics: &self.metrics,
            root_ev_chips: self.root_ev(),
            root_strategy: self.strategy(self.tree().root).ok(),
            memory_bytes: self.estimated_memory_bytes,
            arena_nodes: self.tree().nodes.len(),
            isomorphism_used: self.engine.isomorphism_used(),
            merged_chance_children: self.engine.merged_chance_children(),
            setup_seconds: self.setup_seconds,
            solve_seconds: self.solve_seconds,
            terminal_seconds: self.engine.terminal_seconds(),
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
