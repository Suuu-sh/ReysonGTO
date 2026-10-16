//! Opt-in measurement only. Dense IDs, policies and traversal order are unchanged.
use super::*;

/// Exact visited workload for one alternating iteration. Active means board-unblocked,
/// not positive reach. Action-combo counts use actor hands; value counts use target hands.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct KernelWorkload {
    pub decision_visits: u64,
    pub decision_dense_combos: u64,
    pub decision_active_combos: u64,
    pub action_dense_combos: u64,
    pub action_active_combos: u64,
    pub normalization_dense_combos: u64,
    pub normalization_active_combos: u64,
    /// Action entries whose positive/sum normalization work executes; packed
    /// NEON and portable 2/3 loops include masked lanes (except blocked odd tail).
    pub normalization_evaluated_combos: u64,
    pub update_dense_combos: u64,
    pub update_active_combos: u64,
    pub value_action_dense_combos: u64,
    pub value_action_active_combos: u64,
    pub fold_visits: u64,
    pub fold_dense_combos: u64,
    pub fold_active_combos: u64,
    pub showdown_visits: u64,
    pub showdown_dense_combos: u64,
    pub showdown_active_combos: u64,
    /// Actually evaluated own sorted entries (dense before packing, active after).
    pub showdown_evaluated_combos: u64,
    /// Actual sorted opposing prefix entries visited (not all active opponent hands).
    pub showdown_prefix_combos: u64,
    /// Logical terminal demand, including reach-scoped Totals cache hits.
    pub terminal_totals_dense_combos: u64,
    pub terminal_totals_active_combos: u64,
    pub chance_visits: u64,
    pub chance_representative_children: u64,
    pub chance_physical_children: u64,
    pub chance_reach_dense_combos: u64,
    pub chance_reach_active_combos: u64,
    pub chance_add_dense_combos: u64,
    pub chance_add_active_combos: u64,
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct KernelTimes {
    pub traversals: u64,
    pub wall_ns: u128,
    pub showdown_ns: u128,
    pub fold_ns: u128,
    pub decision_ns: u128,
    pub chance_ns: u128,
    /// Reach copies/multiplications and action-value copies, outside decision_ns.
    pub decision_transport_ns: u128,
    /// Actual zero contributions skipped; static workload fields are logical
    /// dense/packed visits. These counters are collected only when opted in.
    pub terminal_totals_zero_combos: u64,
    /// Actual opposing dense entries scanned on cache misses.
    pub terminal_totals_scanned_combos: u64,
    /// Terminal calls reusing exactly unchanged opposing reach totals.
    pub terminal_totals_cache_hits: u64,
    pub showdown_prefix_zero_combos: u64,
}
impl KernelTimes {
    pub(super) fn add(&mut self, rhs: Self) {
        self.traversals += rhs.traversals;
        self.wall_ns += rhs.wall_ns;
        self.showdown_ns += rhs.showdown_ns;
        self.fold_ns += rhs.fold_ns;
        self.decision_ns += rhs.decision_ns;
        self.chance_ns += rhs.chance_ns;
        self.decision_transport_ns += rhs.decision_transport_ns;
        self.terminal_totals_zero_combos += rhs.terminal_totals_zero_combos;
        self.terminal_totals_scanned_combos += rhs.terminal_totals_scanned_combos;
        self.terminal_totals_cache_hits += rhs.terminal_totals_cache_hits;
        self.showdown_prefix_zero_combos += rhs.showdown_prefix_zero_combos;
    }
    pub(super) fn section(&mut self, section: usize, ns: u128) {
        match section {
            0 => self.showdown_ns += ns,
            1 => self.fold_ns += ns,
            2 => self.decision_ns += ns,
            3 => self.chance_ns += ns,
            _ => unreachable!(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ChanceTask {
    pub boundary_node: usize,
    pub target: usize,
    pub phase: &'static str,
    pub first_child_node: usize,
    pub child_count: usize,
    /// Rayon worker identity only; it does not identify a hardware core/type.
    pub worker: Option<usize>,
    /// Relative to the enclosing traversal's start, not process start.
    pub start_ns: u128,
    pub end_ns: u128,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ChanceBatch {
    pub boundary_node: usize,
    pub target: usize,
    pub tasks: usize,
    pub workers_used: usize,
    pub wall_ns: u128,
    pub task_min_ns: u128,
    pub task_max_ns: u128,
    pub task_sum_ns: u128,
    /// Last task finish minus penultimate task finish (zero for <2 tasks).
    pub final_task_tail_ns: u128,
    /// Sum (batch end - each participating worker's last finish).
    /// Excludes workers which executed no task and is not whole-pool idle time.
    pub participating_worker_finish_idle_ns: u128,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct KernelIteration {
    pub iteration: usize,
    pub times: KernelTimes,
    pub chance_batches: Vec<ChanceBatch>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct KernelDiagnostics {
    pub per_iteration_workload: KernelWorkload,
    /// Two target traversals per metrics call, separately for each metric kind.
    pub per_average_evaluation_workload: KernelWorkload,
    pub per_best_response_evaluation_workload: KernelWorkload,
    pub training: KernelTimes,
    pub average_evaluation: KernelTimes,
    pub best_response_evaluation: KernelTimes,
    pub iterations: Vec<KernelIteration>,
    /// Raw tasks only for the latest alternating training iteration.
    pub latest_training_tasks: Vec<ChanceTask>,
    /// Raw tasks only for the latest four-traversal exact metrics call.
    pub latest_metrics_tasks: Vec<ChanceTask>,
}
#[derive(Default)]
pub(super) struct RunDiagnostics {
    pub times: KernelTimes,
    pub tasks: Vec<ChanceTask>,
    pub batches: Vec<ChanceBatch>,
}
impl RunDiagnostics {
    pub fn merge(&mut self, mut other: Self) {
        self.times.add(other.times);
        self.tasks.append(&mut other.tasks);
        self.batches.append(&mut other.batches);
    }
    pub fn batch(&mut self, node: usize, target: usize, wall_ns: u128) {
        let tasks: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.boundary_node == node && t.target == target)
            .collect();
        if tasks.is_empty() {
            return;
        }
        let mut finishes: Vec<_> = tasks.iter().map(|t| t.end_ns).collect();
        finishes.sort_unstable();
        let end = *finishes.last().unwrap();
        let mut workers = BTreeMap::new();
        for t in &tasks {
            workers
                .entry(t.worker)
                .and_modify(|v: &mut u128| *v = (*v).max(t.end_ns))
                .or_insert(t.end_ns);
        }
        self.batches.push(ChanceBatch {
            boundary_node: node,
            target,
            tasks: tasks.len(),
            workers_used: workers.len(),
            wall_ns,
            task_min_ns: tasks.iter().map(|t| t.end_ns - t.start_ns).min().unwrap(),
            task_max_ns: tasks.iter().map(|t| t.end_ns - t.start_ns).max().unwrap(),
            task_sum_ns: tasks.iter().map(|t| t.end_ns - t.start_ns).sum(),
            final_task_tail_ns: if finishes.len() > 1 {
                end - finishes[finishes.len() - 2]
            } else {
                0
            },
            participating_worker_finish_idle_ns: workers.values().map(|&t| end - t).sum(),
        });
    }
}

fn normalization_evaluated(masks: &[u64], board: u64, actions: u64, active: u64) -> u64 {
    let lanes = if actions == 2 || actions == 3 {
        if cfg!(target_arch = "aarch64") {
            let paired = masks.len() & !1;
            paired as u64 + u64::from(masks.len() != paired && masks[paired] & board == 0)
        } else {
            masks.len() as u64
        }
    } else {
        active
    };
    actions * lanes
}
pub(super) fn workload(engine: &Engine) -> [KernelWorkload; 3] {
    // A board cache exists only during opt-in initialization, no per-node growth.
    let mut boards = BTreeMap::<u64, [u64; 2]>::new();
    fn visit(
        e: &Engine,
        state: &State,
        target: usize,
        mode: Mode,
        boards: &mut BTreeMap<u64, [u64; 2]>,
        w: &mut KernelWorkload,
    ) {
        let node = &e.tree.nodes[state.id];
        let board = board_mask(&node.board);
        let active = *boards.entry(board).or_insert_with(|| {
            std::array::from_fn(|p| e.masks[p].iter().filter(|&&m| m & board == 0).count() as u64)
        });
        let n = e.masks.each_ref().map(|m| m.len() as u64);
        match &node.kind {
            NodeKind::Decision {
                player, actions, ..
            } => {
                let p = *player;
                let a = actions.len() as u64;
                w.decision_visits += 1;
                w.decision_dense_combos += n[p];
                w.decision_active_combos += active[p];
                w.action_dense_combos += a * n[p];
                w.action_active_combos += a * active[p];
                w.value_action_dense_combos += a * n[target];
                w.value_action_active_combos += a * active[target];
                if !(p == target && matches!(mode, Mode::BestResponse)) {
                    w.normalization_dense_combos += a * n[p];
                    w.normalization_active_combos += a * active[p];
                    w.normalization_evaluated_combos +=
                        normalization_evaluated(&e.masks[p], board, a, active[p]);
                }
                if p == target && matches!(mode, Mode::Train) {
                    w.update_dense_combos += a * n[p];
                    w.update_active_combos += a * active[p];
                }
            }
            NodeKind::Terminal { winner } => {
                w.terminal_totals_dense_combos += n[1 - target];
                w.terminal_totals_active_combos += active[1 - target];
                if winner.is_some() {
                    w.fold_visits += 1;
                    w.fold_dense_combos += n[target];
                    w.fold_active_combos += active[target];
                } else {
                    w.showdown_visits += 1;
                    w.showdown_dense_combos += n[target];
                    w.showdown_active_combos += active[target];
                    let table = &e.tables[e.terminal_table[state.id].unwrap().get() as usize - 1];
                    w.showdown_evaluated_combos += table.sorted[target].len() as u64;
                    w.showdown_prefix_combos += table.groups[target]
                        .last()
                        .map_or(0, |g| g.opp_after as u64);
                }
            }
            NodeKind::Chance { .. } => {
                w.chance_visits += 1;
                w.chance_representative_children += state.children.len() as u64;
                for (c, orbit) in state.children.iter().zip(&state.orbits) {
                    let cb = board_mask(&e.tree.nodes[c.id].board);
                    let ca = *boards.entry(cb).or_insert_with(|| {
                        std::array::from_fn(|p| {
                            e.masks[p].iter().filter(|&&m| m & cb == 0).count() as u64
                        })
                    });
                    w.chance_physical_children += orbit.len() as u64;
                    w.chance_reach_dense_combos += n[0] + n[1];
                    w.chance_reach_active_combos += ca[0] + ca[1];
                    w.chance_add_dense_combos += orbit.len() as u64 * n[target];
                    w.chance_add_active_combos += orbit.len() as u64 * ca[target];
                }
            }
        }
        for c in &state.children {
            visit(e, c, target, mode, boards, w);
        }
    }
    std::array::from_fn(|i| {
        let mut w = KernelWorkload::default();
        let mode = [Mode::Train, Mode::Average, Mode::BestResponse][i];
        for target in 0..2 {
            visit(engine, &engine.state, target, mode, &mut boards, &mut w);
        }
        w
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_work_counts_blocked_odd_tail_and_portable_masked_lanes() {
        let board = 1;
        for actions in [2, 3] {
            assert_eq!(
                normalization_evaluated(&[2, 4, 1], board, actions, 2),
                actions * if cfg!(target_arch = "aarch64") { 2 } else { 3 }
            );
            assert_eq!(
                normalization_evaluated(&[2, 1, 4], board, actions, 2),
                actions * 3
            );
            assert_eq!(
                normalization_evaluated(&[1, 1], board, actions, 0),
                actions * 2
            );
            assert_eq!(
                normalization_evaluated(&[1], board, actions, 0),
                if cfg!(target_arch = "aarch64") {
                    0
                } else {
                    actions
                }
            );
        }
        assert_eq!(normalization_evaluated(&[2, 4, 1], board, 4, 2), 8);
    }
}
