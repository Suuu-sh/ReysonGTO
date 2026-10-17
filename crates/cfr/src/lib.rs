//! Clean-room vector alternating CFR. Values are net future receipts minus future
//! investments and half the starting pot (so zero-rake values sum to zero).
//! Dense combo IDs stay stable across streets; newly blocked hands have zero strategy.
//! Suit reduction uses only board-history stabilizers preserving BOTH exact weighted
//! ranges. Dense combo permutations map representative values back to physical cards.
//! Persistent regret/average slabs are f32 or per-node scaled i16; traversal scratch
//! is depth-local (and independent only at the first parallel chance boundary).
mod diagnostics;
mod hand_ev;
pub use hand_ev::HandValue;
#[cfg(test)]
mod golden_tests;
use cards::{evaluate7, Card, SuitPermutation};
pub use diagnostics::{KernelDiagnostics, KernelWorkload};
use diagnostics::{KernelIteration, RunDiagnostics};
use ranges::DenseRange;
use serde::{Deserialize, Serialize};
#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;
use tree::{GameTree, NodeKind};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Algorithm {
    #[default]
    Dcfr,
    CfrPlus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineConfig {
    pub algorithm: Algorithm,
    pub threads: usize,
    /// Maximum first-chance children visited sequentially per Rayon task.
    #[serde(default = "default_parallel_grain")]
    pub parallel_grain: usize,
    pub memory_limit_bytes: usize,
    pub suit_isomorphism: bool,
    pub profiling: bool,
    /// Opt-in per-phase workload, worker timers and parallel task diagnostics.
    #[serde(default)]
    pub kernel_diagnostics: bool,
    #[serde(default)]
    pub storage_mode: StorageMode,
    #[serde(default)]
    pub scratch_precision: ScratchPrecision,
    #[serde(default = "default_alpha")]
    pub dcfr_alpha: f64,
    #[serde(default)]
    pub dcfr_beta: f64,
    #[serde(default = "default_gamma")]
    pub dcfr_gamma: f64,
}
fn default_parallel_grain() -> usize {
    1
}
fn default_alpha() -> f64 {
    1.5
}
fn default_gamma() -> f64 {
    2.0
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum StorageMode {
    #[default]
    F32,
    I16,
}
/// Traversal scratch arithmetic, independent of persistent policy storage.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScratchPrecision {
    #[default]
    F64,
    /// f32 reach/value arithmetic and blocker/prefix totals. Tiny positive
    /// normalized weights may underflow; training is intentionally approximate.
    /// Public metrics still evaluate the stored policy with f64 scratch.
    F32,
    /// Experimental f32 traversal with f64 blocker/prefix totals, chance sums
    /// and showdown payoff arithmetic. Public metrics always evaluate in f64.
    F32WideTotals,
}
trait Scratch:
    Copy
    + Default
    + Send
    + Sync
    + PartialEq
    + PartialOrd
    + std::ops::Add<Output = Self>
    + std::ops::Sub<Output = Self>
    + std::ops::Mul<Output = Self>
    + std::ops::Div<Output = Self>
    + std::ops::Neg<Output = Self>
    + std::ops::AddAssign
    + std::iter::Sum
{
    const ZERO: Self;
    const NEG_INFINITY: Self;
    fn from_f64(value: f64) -> Self;
    fn to_f64(self) -> f64;
    fn max(self, other: Self) -> Self;
}
macro_rules! scratch_scalar {
    ($t:ty) => {
        impl Scratch for $t {
            const ZERO: Self = 0.0;
            const NEG_INFINITY: Self = Self::NEG_INFINITY;
            #[inline]
            fn from_f64(value: f64) -> Self {
                value as Self
            }
            #[inline]
            fn to_f64(self) -> f64 {
                self as f64
            }
            #[inline]
            fn max(self, other: Self) -> Self {
                self.max(other)
            }
        }
    };
}
scratch_scalar!(f64);
scratch_scalar!(f32);
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct WorkProfile {
    pub showdown_seconds: f64,
    pub fold_seconds: f64,
    pub regret_seconds: f64,
    pub chance_seconds: f64,
    pub best_response_seconds: f64,
    pub training_seconds: f64,
}
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::Dcfr,
            threads: 1,
            parallel_grain: default_parallel_grain(),
            memory_limit_bytes: 2 * 1024 * 1024 * 1024,
            suit_isomorphism: false,
            profiling: false,
            kernel_diagnostics: false,
            storage_mode: StorageMode::F32,
            scratch_precision: ScratchPrecision::F64,
            dcfr_alpha: 1.5,
            dcfr_beta: 0.0,
            dcfr_gamma: 2.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metrics {
    pub average_ev: [f64; 2],
    pub best_response_ev: [f64; 2],
    pub exploitability_chips: f64,
    pub exploitability_pct: f64,
}

// Small recursive metadata; no combo-sized buffers are retained per tree node.
struct State<S: Scratch = f64> {
    id: usize,
    children: Vec<State<S>>,
    offset: usize,
    own_len: usize,
    span: usize,
    scales: [f32; 2],
    work: [u64; 4],          // showdown, fold, regret, chance (exclusive)
    orbits: Vec<Vec<usize>>, // indices of dense combo permutations, member -> representative
    parallel_scratch: Vec<Vec<Level<S>>>,
}
struct Level<S: Scratch = f64> {
    reach: [Vec<S>; 2],
    value: Vec<S>,
    action_values: Vec<S>,
    strategy: Vec<f32>,
    regret: Vec<f32>,
    average: Vec<f32>,
    average_path_reach: Vec<S>,
    // Only WideTotals allocates this: chance sums and the unrounded showdown
    // prefix share one depth-local buffer, never live at the same node kind.
    wide_value: Vec<f64>,
}
fn levels<S: Scratch>(depth: usize, actions: usize, n: [usize; 2]) -> Vec<Level<S>> {
    (0..depth)
        .map(|_| Level {
            reach: [vec![S::ZERO; n[0]], vec![S::ZERO; n[1]]],
            value: vec![S::ZERO; n[0].max(n[1])],
            action_values: vec![S::ZERO; actions * n[0].max(n[1])],
            strategy: vec![0.0; actions * n[0].max(n[1])],
            regret: vec![0.0; actions * n[0].max(n[1])],
            average: vec![0.0; actions * n[0].max(n[1])],
            average_path_reach: vec![S::ZERO; n[0].max(n[1])],
            wide_value: Vec::new(),
        })
        .collect()
}
fn tree_shape(tree: &GameTree, id: usize) -> (usize, usize) {
    let (children, own_actions) = match &tree.nodes[id].kind {
        NodeKind::Decision {
            children, actions, ..
        } => (children.as_slice(), actions.len()),
        NodeKind::Chance { children, .. } => (children.as_slice(), 0),
        _ => (&[][..], 0),
    };
    let mut shape = (1, own_actions);
    for &child in children {
        let s = tree_shape(tree, child);
        shape.0 = shape.0.max(1 + s.0);
        shape.1 = shape.1.max(s.1);
    }
    shape
}
impl<S: Scratch> State<S> {
    fn prepare_wide_values(&mut self, n: usize) {
        for scratch in &mut self.parallel_scratch {
            for level in scratch {
                level.wide_value = vec![0.0; n];
            }
        }
        for child in &mut self.children {
            child.prepare_wide_values(n);
        }
    }
    fn copy_scales<U: Scratch>(&mut self, source: &State<U>) {
        self.scales = source.scales;
        for (child, source) in self.children.iter_mut().zip(&source.children) {
            child.copy_scales(source);
        }
    }
    fn new(
        tree: &GameTree,
        id: usize,
        n: [usize; 2],
        offset: usize,
        aliases: &[u32],
        alias_perms: &[u32],
        parallel: bool,
    ) -> Self {
        let (ids, count, chance) = match &tree.nodes[id].kind {
            NodeKind::Decision {
                player,
                children,
                actions,
            } => (children.as_slice(), actions.len() * n[*player], false),
            NodeKind::Chance { children, .. } => (children.as_slice(), 0, true),
            _ => (&[][..], 0, false),
        };
        let child_count = if chance {
            ids.iter().filter(|&&c| aliases[c] as usize == c).count()
        } else {
            ids.len()
        };
        let mut end = offset + count;
        // A two-action node must not retain Vec's default four-State allocation.
        // Count representatives first so aliases also leave no unused capacity.
        let mut children = Vec::with_capacity(child_count);
        let mut orbits = Vec::with_capacity(if chance { child_count } else { 0 });
        let mut parallel_scratch =
            Vec::with_capacity(if chance && parallel { child_count } else { 0 });
        for &c in ids {
            if chance && aliases[c] as usize != c {
                continue;
            }
            let child = Self::new(tree, c, n, end, aliases, alias_perms, parallel && !chance);
            end += child.span;
            if chance {
                let members = ids.iter().filter(|&&x| x == c || aliases[x] as usize == c);
                let mut orbit = Vec::with_capacity(members.clone().count());
                orbit.extend(members.map(|&x| if x == c { 0 } else { alias_perms[x] as usize }));
                orbits.push(orbit);
                if parallel {
                    let shape = tree_shape(tree, c);
                    parallel_scratch.push(levels(shape.0, shape.1, n));
                }
            }
            children.push(child);
        }
        Self {
            id,
            children,
            offset,
            own_len: count,
            span: end - offset,
            scales: [1.0; 2],
            work: [0; 4],
            orbits,
            parallel_scratch,
        }
    }
    fn find(&self, id: usize) -> Option<&Self> {
        if self.id == id {
            Some(self)
        } else {
            self.children.iter().find_map(|s| s.find(id))
        }
    }
    fn find_mut(&mut self, id: usize) -> Option<&mut Self> {
        if self.id == id {
            Some(self)
        } else {
            self.children.iter_mut().find_map(|s| s.find_mut(id))
        }
    }
    fn work(&self) -> [u128; 4] {
        // Widen before summing worker/node durations; one node's nanosecond
        // counter covers over 584 years, while the aggregate stays u128.
        let mut w = self.work.map(u128::from);
        for c in &self.children {
            for (a, b) in w.iter_mut().zip(c.work()) {
                *a += b;
            }
        }
        w
    }
}
#[derive(Clone)]
struct ComboPermutation {
    suit: SuitPermutation,
    indices: [Vec<usize>; 2],
}
enum Slab {
    F32(Vec<f32>),
    I16(Vec<i16>),
}
impl Slab {
    #[cfg(test)]
    fn f32_mut(&mut self) -> &mut [f32] {
        match self {
            Self::F32(v) => v,
            Self::I16(_) => panic!("test expects f32"),
        }
    }
    #[cfg(test)]
    fn bits(&self) -> Vec<u32> {
        match self {
            Self::F32(v) => v.iter().map(|x| x.to_bits()).collect(),
            Self::I16(v) => v.iter().map(|&x| x as u32).collect(),
        }
    }
    fn new(mode: StorageMode, len: usize) -> Self {
        match mode {
            StorageMode::F32 => Self::F32(vec![0.0; len]),
            StorageMode::I16 => Self::I16(vec![0; len]),
        }
    }
    fn slice(&mut self) -> SlabSlice<'_> {
        match self {
            Self::F32(v) => SlabSlice::F32(v),
            Self::I16(v) => SlabSlice::I16(v),
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn normalize_policy(
        &self,
        offset: usize,
        len: usize,
        output: &mut [f32],
        n: usize,
        actions: usize,
        masks: &[u64],
        board: u64,
    ) {
        match self {
            Self::F32(v) => normalize(&v[offset..offset + len], output, n, actions, masks, board),
            Self::I16(v) => {
                normalize_i16(&v[offset..offset + len], output, n, actions, masks, board)
            }
        }
    }
    #[cfg(test)]
    fn read(&self, offset: usize, out: &mut [f32], scale: f32) {
        match self {
            Self::F32(v) => out.copy_from_slice(&v[offset..offset + out.len()]),
            Self::I16(v) => {
                for (x, &q) in out.iter_mut().zip(&v[offset..]) {
                    *x = q as f32 * scale;
                }
            }
        }
    }
}
enum SlabSlice<'a> {
    F32(&'a mut [f32]),
    I16(&'a mut [i16]),
}
impl<'a> SlabSlice<'a> {
    fn split(self, at: usize) -> (Self, Self) {
        match self {
            Self::F32(v) => {
                let (a, b) = v.split_at_mut(at);
                (Self::F32(a), Self::F32(b))
            }
            Self::I16(v) => {
                let (a, b) = v.split_at_mut(at);
                (Self::I16(a), Self::I16(b))
            }
        }
    }
    #[cfg(test)]
    fn read(&self, out: &mut [f32], scale: f32) {
        match self {
            Self::F32(v) => out.copy_from_slice(v),
            Self::I16(v) => {
                for (x, &q) in out.iter_mut().zip(v.iter()) {
                    *x = q as f32 * scale;
                }
            }
        }
    }
    fn normalize_policy(
        &self,
        output: &mut [f32],
        n: usize,
        actions: usize,
        masks: &[u64],
        board: u64,
    ) {
        match self {
            Self::F32(v) => normalize(v, output, n, actions, masks, board),
            // The node's strictly positive scale cancels in every policy ratio.
            // Normalizing exact stored integers also avoids f32 decode rounding.
            Self::I16(v) => normalize_i16(v, output, n, actions, masks, board),
        }
    }
    #[cfg(test)]
    fn write(&mut self, input: &[f32], scale: &mut f32) {
        match self {
            Self::F32(v) => v.copy_from_slice(input),
            Self::I16(v) => {
                let max = input.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
                encode_i16(input, v, scale, max);
            }
        }
    }
}
// `max` can be reduced while producing the updated scratch values. Keep this
// encoder shared with the reference path so subnormal handling stays identical.
fn encode_i16(input: &[f32], output: &mut [i16], scale: &mut f32, max: f32) {
    // A tiny nonzero node must not produce scale=0 or an infinite
    // f32 reciprocal (both would erase/sign-saturate its policy).
    *scale = if max > 0.0 {
        (max / 32767.0).max(f32::from_bits(1))
    } else {
        1.0
    };
    let inverse = 1.0 / *scale;
    if inverse.is_finite() {
        for (q, &x) in output.iter_mut().zip(input) {
            *q = (x * inverse).round() as i16;
        }
    } else {
        // Subnormal scale rounding can undershoot by a sizeable
        // fraction of one step. Round it upward before encoding so
        // the largest signed value cannot saturate by many steps.
        if max as f64 > 32767.0 * *scale as f64 {
            *scale = f32::from_bits(scale.to_bits() + 1);
        }
        let inverse = 1.0 / *scale as f64;
        for (q, &x) in output.iter_mut().zip(input) {
            *q = (x as f64 * inverse).round() as i16;
        }
    }
}

#[derive(Clone, Copy)]
struct SortedHand {
    dense: u16,
    same: u16, // u16::MAX means no identical opposing combo
    cards: [Card; 2],
}
#[derive(Clone, Copy)]
struct RankGroup {
    own_begin: u16,
    own_end: u16,
    opp_before: u16,
    opp_after: u16,
}
struct RankTable {
    sorted: [Vec<SortedHand>; 2],
    groups: [Vec<RankGroup>; 2],
    blocked: [Vec<u16>; 2],
}
impl RankTable {
    fn new(
        ranks: [Vec<u32>; 2],
        cards: &[Vec<[Card; 2]>; 2],
        same: &[Vec<Option<usize>>; 2],
    ) -> Self {
        let order: [Vec<usize>; 2] = std::array::from_fn(|p| {
            // Rank zero is the engine's board-blocked sentinel. Public dense
            // IDs remain stable while all showdown computation uses this pack.
            let mut order = Vec::with_capacity(ranks[p].len());
            order.extend((0..ranks[p].len()).filter(|&i| ranks[p][i] != 0));
            order.shrink_to_fit();
            order.sort_unstable_by_key(|&i| (ranks[p][i], i));
            order
        });
        let groups = std::array::from_fn(|p| {
            let mut groups = Vec::new();
            let mut own = 0;
            let mut opp = 0;
            while own < order[p].len() {
                let begin = own;
                let rank = ranks[p][order[p][own]];
                while own < order[p].len() && ranks[p][order[p][own]] == rank {
                    own += 1;
                }
                while opp < order[1 - p].len() && ranks[1 - p][order[1 - p][opp]] < rank {
                    opp += 1;
                }
                let before = opp;
                while opp < order[1 - p].len() && ranks[1 - p][order[1 - p][opp]] == rank {
                    opp += 1;
                }
                // Validation guarantees unique canonical combos (at most 1326),
                // so both dense IDs and exclusive group ends fit in u16.
                groups.push(RankGroup {
                    own_begin: begin as u16,
                    own_end: own as u16,
                    opp_before: before as u16,
                    opp_after: opp as u16,
                });
            }
            groups.shrink_to_fit();
            groups
        });
        let sorted = std::array::from_fn(|p| {
            order[p]
                .iter()
                .map(|&i| SortedHand {
                    dense: i as u16,
                    same: same[p][i].map_or(u16::MAX, |j| j as u16),
                    cards: cards[p][i],
                })
                .collect()
        });
        let blocked = std::array::from_fn(|p| {
            let mut blocked = Vec::with_capacity(ranks[p].len() - order[p].len());
            blocked.extend(
                (0..ranks[p].len())
                    .filter(|&i| ranks[p][i] == 0)
                    .map(|i| i as u16),
            );
            blocked
        });
        Self {
            sorted,
            groups,
            blocked,
        }
    }
}
struct Context<'a> {
    tree: &'a GameTree,
    ranges: &'a [DenseRange; 2],
    masks: &'a [Vec<u64>; 2],
    hand_cards: &'a [Vec<[Card; 2]>; 2],
    same: &'a [Vec<Option<usize>>; 2],
    tables: &'a [RankTable],
    terminal_table: &'a [Option<NonZeroU32>],
    config: &'a EngineConfig,
    permutations: &'a [ComboPermutation],
    average_reach_scale: &'a [Vec<f64>; 2],
    initial_reach: &'a [Vec<f64>; 2],
    discounts: [f64; 3],
    iteration: usize,
    run_start: Instant,
}
#[derive(Clone, Copy)]
enum Mode {
    Train,
    Average,
    BestResponse,
}

struct Evaluation {
    state: State<f64>,
    scratch: Vec<Level<f64>>,
}

struct EngineCore<S: Scratch = f64, T: Scratch = S> {
    totals_precision: std::marker::PhantomData<T>,
    tree: GameTree,
    ranges: [DenseRange; 2],
    config: EngineConfig,
    state: State<S>,
    masks: [Vec<u64>; 2],
    hand_cards: [Vec<[Card; 2]>; 2],
    same: [Vec<Option<usize>>; 2],
    tables: Vec<RankTable>,
    terminal_table: Vec<Option<NonZeroU32>>,
    initial_reach: [Vec<f64>; 2],
    average_reach_scale: [Vec<f64>; 2],
    joint_mass: f64,
    iterations: usize,
    memory_bytes: usize,
    pool: Arc<rayon::ThreadPool>,
    aliases: Vec<u32>,
    merged_children: usize,
    regret: Slab,
    average: Slab,
    scratch: Vec<Level<S>>,
    // Evaluation always uses f64 arithmetic on the current stored policy. This
    // is prepared once; the allocation-free training path never touches it.
    evaluation: Option<Evaluation>,
    permutations: Vec<ComboPermutation>,
    alias_perms: Vec<u32>,
    wall_profile: WorkProfile,
    kernel_diagnostics: Option<KernelDiagnostics>,
}

/// Conservative owned solver payload estimate, including per-node traversal
/// buffers, rank caches and tree/ranges. Thread stacks and allocator overhead are
/// covered by a 25% margin plus 1 MiB per configured worker in Engine::new.
pub fn estimate_memory_bytes(tree: &GameTree, ranges: &[DenseRange; 2]) -> Result<usize, String> {
    estimate_memory_bytes_with_config(tree, ranges, &EngineConfig::default())
}
/// Upper bound without allocating combo-sized state. Suit reduction may lower
/// actual persistent storage; estimates deliberately assume every physical branch.
pub fn estimate_memory_bytes_with_config(
    tree: &GameTree,
    ranges: &[DenseRange; 2],
    config: &EngineConfig,
) -> Result<usize, String> {
    validate(tree, ranges)?;
    let n = [ranges[0].combos.len(), ranges[1].combos.len()];
    let mut bytes = 0usize;
    let mut boards = BTreeSet::new();
    for node in &tree.nodes {
        let (actions, children) = match &node.kind {
            NodeKind::Decision {
                player,
                actions,
                children,
            } => (actions.len() * n[*player], children.len()),
            NodeKind::Chance { children, .. } => (0, children.len()),
            _ => (0, 0),
        };
        let scalar = match config.storage_mode {
            StorageMode::F32 => 8,
            StorageMode::I16 => 4,
        };
        bytes = bytes
            .checked_add(
                actions
                    .checked_mul(scalar)
                    .ok_or("memory estimate overflow")?
                    + std::mem::size_of::<State<f64>>()
                    + std::mem::size_of::<tree::Node>()
                    + node.board.len()
                    + children * 48
                    + 64,
            )
            .ok_or("memory estimate overflow")?;
        if matches!(node.kind, NodeKind::Terminal { winner: None }) {
            boards.insert(board_mask(&node.board));
        }
    }
    let shape = tree_shape(tree, tree.root);
    let scalar_bytes = match config.scratch_precision {
        ScratchPrecision::F64 => 8,
        ScratchPrecision::F32 | ScratchPrecision::F32WideTotals => 4,
    };
    let level_bytes = (n[0] + n[1] + 2 * n[0].max(n[1])) * scalar_bytes
        + shape.1 * n[0].max(n[1]) * (scalar_bytes + 12)
        + std::mem::size_of::<Level<f64>>();
    let wide_level_bytes = if config.scratch_precision == ScratchPrecision::F32WideTotals {
        n[0].max(n[1]) * 8
    } else {
        0
    };
    fn scratch_roots(tree: &GameTree, id: usize) -> usize {
        match &tree.nodes[id].kind {
            NodeKind::Chance { children, .. } => children.len(),
            NodeKind::Decision { children, .. } => {
                children.iter().map(|&c| scratch_roots(tree, c)).sum()
            }
            _ => 0,
        }
    }
    let scratch_count = if config.threads > 1 {
        1 + scratch_roots(tree, tree.root)
    } else {
        1
    };
    let evaluation_bytes = if config.scratch_precision == ScratchPrecision::F64 {
        0
    } else {
        // The f64 evaluator shares tree, ranks, permutations and policy slabs,
        // but owns mirrored metadata (including orbits) and depth-local scratch.
        let metadata: usize = tree
            .nodes
            .iter()
            .map(|node| {
                let children = match &node.kind {
                    NodeKind::Decision { children, .. } | NodeKind::Chance { children, .. } => {
                        children.len()
                    }
                    _ => 0,
                };
                std::mem::size_of::<State<f64>>() + children * 48 + 64
            })
            .sum();
        let f64_level_bytes = (n[0] + n[1] + 2 * n[0].max(n[1])) * 8
            + shape.1 * n[0].max(n[1]) * 20
            + std::mem::size_of::<Level<f64>>();
        metadata + shape.0 * f64_level_bytes * scratch_count
    };
    bytes = bytes
        .checked_add(
            boards.len()
                * (std::mem::size_of::<RankTable>()
                    + (n[0] + n[1])
                        * (std::mem::size_of::<SortedHand>() + std::mem::size_of::<RankGroup>()))
                + (n[0] + n[1]) * (64 + std::mem::size_of::<[Card; 2]>())
                + tree.nodes.len()
                    * (std::mem::size_of::<Option<NonZeroU32>>() + 2 * std::mem::size_of::<u32>())
                + shape.0 * (level_bytes + wide_level_bytes) * scratch_count
                + evaluation_bytes
                + 24 * (n[0] + n[1]) * 8
                + config.threads * 1024 * 1024,
        )
        .ok_or("memory estimate overflow")?;
    bytes
        .checked_add(bytes / 4)
        .ok_or_else(|| "memory estimate overflow".into())
}
fn compact_index(index: usize) -> Result<u32, String> {
    u32::try_from(index).map_err(|_| "tree metadata exceeds u32 index capacity".into())
}
fn validate(tree: &GameTree, ranges: &[DenseRange; 2]) -> Result<(), String> {
    // Validate before allocating compact node/table metadata. A table's encoded
    // index is one-based and there cannot be more tables than tree nodes.
    compact_index(tree.nodes.len())?;
    if tree.nodes.is_empty() || tree.root >= tree.nodes.len() || tree.config.starting_pot <= 0 {
        return Err("invalid root or starting pot".into());
    }
    cards::validate_board(
        &tree.nodes[tree.root]
            .board
            .iter()
            .map(|&c| Card(c))
            .collect::<Vec<_>>(),
    )?;
    for range in ranges {
        if range.combos.is_empty() {
            return Err("empty range".into());
        }
        let mut seen = BTreeSet::new();
        for c in &range.combos {
            if cards::Combo::new(c.combo.cards[0], c.combo.cards[1])? != c.combo {
                return Err("range combo must be canonical".into());
            }
            if !c.weight.is_finite() || c.weight < 0.0 || !seen.insert(c.combo.id()) {
                return Err("invalid range weights or duplicate combo".into());
            }
            if c.combo.mask() & board_mask(&tree.nodes[tree.root].board) != 0 {
                return Err("range overlaps starting board".into());
            }
        }
    }
    let mut seen = vec![false; tree.nodes.len()];
    let mut todo = vec![tree.root];
    while let Some(id) = todo.pop() {
        if id >= tree.nodes.len() || seen[id] {
            return Err("tree has invalid child, cycle or shared subtree".into());
        }
        seen[id] = true;
        let node = &tree.nodes[id];
        cards::validate_board(&node.board.iter().map(|&c| Card(c)).collect::<Vec<_>>())?;
        match &node.kind {
            NodeKind::Decision {
                player,
                actions,
                children,
            } => {
                if *player > 1 || actions.is_empty() || actions.len() != children.len() {
                    return Err("invalid decision".into());
                }
                todo.extend(children);
            }
            NodeKind::Chance { cards, children } => {
                if cards.is_empty()
                    || cards.len() != children.len()
                    || !(3..5).contains(&node.board.len())
                {
                    return Err("invalid chance node".into());
                }
                let mut mask = board_mask(&node.board);
                for (&card, &child) in cards.iter().zip(children) {
                    if card >= 52 || mask & (1u64 << card) != 0 || child >= tree.nodes.len() {
                        return Err("invalid chance card".into());
                    }
                    mask |= 1u64 << card;
                    if board_mask(&tree.nodes[child].board)
                        != board_mask(&node.board) | (1u64 << card)
                    {
                        return Err("chance child board mismatch".into());
                    }
                }
                // Full deck support is required: conditioning on BOTH private hands
                // cannot be replaced by weights normalized only by public board.
                if cards.len() != 52 - node.board.len() {
                    return Err("chance node must include every public-unblocked card".into());
                }
                todo.extend(children);
            }
            NodeKind::Terminal { winner } => {
                if winner.is_some_and(|p| p > 1) || (winner.is_none() && node.board.len() != 5) {
                    return Err("invalid terminal".into());
                }
            }
        }
    }
    if seen.iter().any(|&s| !s) {
        return Err("unreachable nodes".into());
    }
    Ok(())
}
fn board_mask(board: &[u8]) -> u64 {
    board.iter().fold(0, |m, &c| m | (1u64 << c))
}

impl<S: Scratch, T: Scratch> EngineCore<S, T> {
    pub fn new(
        tree: GameTree,
        ranges: [DenseRange; 2],
        config: EngineConfig,
    ) -> Result<Self, String> {
        if config.threads == 0 {
            return Err("threads must be positive".into());
        }
        if config.parallel_grain == 0 {
            return Err("parallel grain must be positive".into());
        }
        if ![config.dcfr_alpha, config.dcfr_beta, config.dcfr_gamma]
            .iter()
            .all(|x| x.is_finite())
            || config.dcfr_gamma < 0.0
        {
            return Err("invalid DCFR parameters".into());
        }
        let memory_bytes = estimate_memory_bytes_with_config(&tree, &ranges, &config)?;
        if memory_bytes > config.memory_limit_bytes {
            return Err(format!(
                "estimated memory {memory_bytes} exceeds limit {}",
                config.memory_limit_bytes
            ));
        }
        let masks =
            std::array::from_fn(|p| ranges[p].combos.iter().map(|c| c.combo.mask()).collect());
        let hand_cards: [Vec<[Card; 2]>; 2] =
            std::array::from_fn(|p| ranges[p].combos.iter().map(|c| c.combo.cards).collect());
        let same = std::array::from_fn(|p| {
            let mut index = [None; 1326];
            for (i, c) in ranges[1 - p].combos.iter().enumerate() {
                index[c.combo.id()] = Some(i);
            }
            ranges[p]
                .combos
                .iter()
                .map(|c| index[c.combo.id()])
                .collect::<Vec<_>>()
        });
        let initial_reach: [Vec<f64>; 2] = std::array::from_fn(|p| {
            let total: f64 = ranges[p].combos.iter().map(|c| c.weight).sum();
            ranges[p].combos.iter().map(|c| c.weight / total).collect()
        });
        // Initial own combo mass is a constant factor that cancels from each
        // normalized average policy. Remove it before i16 encoding, so a tiny
        // supplied range weight cannot erase that private hand's entire policy.
        let average_reach_scale = std::array::from_fn(|p| {
            initial_reach[p]
                .iter()
                .map(|&w| if w > 0.0 { 1.0 / w } else { 0.0 })
                .collect()
        });
        let totals = totals(&hand_cards[1], &initial_reach[1]);
        let joint_mass: f64 = ranges[0]
            .combos
            .iter()
            .enumerate()
            .map(|(i, c)| {
                initial_reach[0][i]
                    * compatible(
                        c.combo.cards,
                        &totals,
                        same[0][i].map_or(0.0, |j| initial_reach[1][j]),
                    )
            })
            .sum();
        if !joint_mass.is_finite() || joint_mass <= 0.0 {
            return Err("ranges have no compatible positive-weight deals".into());
        }
        let permutations = range_permutations(&tree, &ranges)?;
        // The checked node-count bound in validate makes every node ID fit.
        let mut aliases: Vec<u32> = (0..tree.nodes.len()).map(|id| id as u32).collect();
        let mut alias_perms = vec![0; tree.nodes.len()];
        let eligible = config.suit_isomorphism && tree.config.bet_size.is_suit_invariant();
        let size_invariance = if eligible {
            // Fixed seed: validation must not depend on run date or thread order.
            tree.config
                .bet_size
                .check_suit_invariance(0x726579736f6e, 64)
        } else {
            Ok(())
        };
        let merged_children = if let Err(reason) = size_invariance {
            eprintln!("warning: suit isomorphism disabled: {reason}");
            0
        } else if eligible {
            match prepare_iso(
                &tree,
                &permutations,
                tree.root,
                &mut aliases,
                &mut alias_perms,
            ) {
                Ok(merged) => merged,
                Err(reason) => {
                    // Sampling cannot prove arbitrary callback invariance. A
                    // structural mismatch is another safe fallback, including
                    // undoing aliases written before the mismatch was found.
                    eprintln!("warning: suit isomorphism disabled: {reason}");
                    for (id, alias) in aliases.iter_mut().enumerate() {
                        *alias = id as u32;
                    }
                    alias_perms.fill(0);
                    0
                }
            }
        } else {
            0
        };
        let mut tables = Vec::new();
        let mut table_ids = BTreeMap::new();
        let mut terminal_table = vec![None; tree.nodes.len()];
        for (id, node) in tree.nodes.iter().enumerate() {
            if aliases[id] as usize != id
                || !matches!(node.kind, NodeKind::Terminal { winner: None })
            {
                continue;
            }
            let key = board_mask(&node.board);
            let table_id = *table_ids.entry(key).or_insert_with(|| {
                let ranks: [Vec<u32>; 2] = std::array::from_fn(|p| {
                    ranges[p]
                        .combos
                        .iter()
                        .map(|c| {
                            if c.combo.mask() & key != 0 {
                                return 0;
                            }
                            let hand = [
                                Card(node.board[0]),
                                Card(node.board[1]),
                                Card(node.board[2]),
                                Card(node.board[3]),
                                Card(node.board[4]),
                                c.combo.cards[0],
                                c.combo.cards[1],
                            ];
                            evaluate7(&hand)
                        })
                        .collect()
                });
                let index = tables.len();
                tables.push(RankTable::new(ranks, &hand_cards, &same));
                index
            });
            let encoded = table_id
                .checked_add(1)
                .ok_or("showdown table index overflow")?;
            terminal_table[id] = NonZeroU32::new(compact_index(encoded)?);
        }
        let n = [ranges[0].combos.len(), ranges[1].combos.len()];
        let mut state = State::new(
            &tree,
            tree.root,
            n,
            0,
            &aliases,
            &alias_perms,
            config.threads > 1,
        );
        let shape = tree_shape(&tree, tree.root);
        let mut scratch = levels(shape.0, shape.1, n);
        if config.scratch_precision == ScratchPrecision::F32WideTotals {
            state.prepare_wide_values(n[0].max(n[1]));
            for level in &mut scratch {
                level.wide_value = vec![0.0; n[0].max(n[1])];
            }
        }
        let evaluation = (config.scratch_precision != ScratchPrecision::F64).then(|| Evaluation {
            state: State::new(
                &tree,
                tree.root,
                n,
                0,
                &aliases,
                &alias_perms,
                config.threads > 1,
            ),
            scratch: levels(shape.0, shape.1, n),
        });
        let regret = Slab::new(config.storage_mode, state.span);
        let average = Slab::new(config.storage_mode, state.span);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.threads)
            .stack_size(1024 * 1024)
            .build()
            .map_err(|e| e.to_string())?;
        let mut engine = Self {
            totals_precision: std::marker::PhantomData,
            tree,
            ranges,
            config,
            state,
            masks,
            hand_cards,
            same,
            tables,
            terminal_table,
            initial_reach,
            average_reach_scale,
            joint_mass,
            iterations: 0,
            memory_bytes,
            pool: Arc::new(pool),
            aliases,
            merged_children,
            regret,
            average,
            scratch,
            evaluation,
            permutations,
            alias_perms,
            wall_profile: WorkProfile::default(),
            kernel_diagnostics: None,
        };
        if engine.config.kernel_diagnostics {
            let [training, average, best_response] = diagnostics::workload(&engine);
            engine.kernel_diagnostics = Some(KernelDiagnostics {
                per_iteration_workload: training,
                per_average_evaluation_workload: average,
                per_best_response_evaluation_workload: best_response,
                ..Default::default()
            });
        }
        Ok(engine)
    }
    /// One iteration is an OOP update followed by an IP update. No engine-owned
    /// heap buffers are allocated here with kernel diagnostics disabled; Rayon
    /// uses its initialized worker pool. Opt-in diagnostics retain task records.
    pub fn iterate(&mut self) {
        let start = Instant::now();
        self.iterations += 1;
        if let Some(d) = &mut self.kernel_diagnostics {
            d.latest_training_tasks.clear();
            d.iterations.push(KernelIteration {
                iteration: self.iterations,
                ..Default::default()
            });
        }
        for player in 0..2 {
            self.run(player, Mode::Train);
        }
        self.wall_profile.training_seconds += start.elapsed().as_secs_f64();
    }
    fn run(&mut self, player: usize, mode: Mode) -> f64 {
        self.run_with_cache(player, mode, TotalsCache::default())
    }
    fn run_with_cache(
        &mut self,
        player: usize,
        mode: Mode,
        mut totals_cache: TotalsCache<T>,
    ) -> f64 {
        let start = Instant::now();
        for (out, &weight) in self.scratch[0].reach[0]
            .iter_mut()
            .zip(&self.initial_reach[0])
        {
            *out = S::from_f64(weight);
        }
        for (out, &weight) in self.scratch[0].reach[1]
            .iter_mut()
            .zip(&self.initial_reach[1])
        {
            *out = S::from_f64(weight);
        }
        let context = Context {
            tree: &self.tree,
            ranges: &self.ranges,
            masks: &self.masks,
            hand_cards: &self.hand_cards,
            same: &self.same,
            tables: &self.tables,
            terminal_table: &self.terminal_table,
            config: &self.config,
            permutations: &self.permutations,
            average_reach_scale: &self.average_reach_scale,
            initial_reach: &self.initial_reach,
            discounts: {
                let t = self.iterations.max(1) as f64;
                let pos = t.powf(self.config.dcfr_alpha);
                let neg = t.powf(self.config.dcfr_beta);
                [
                    pos / (pos + 1.0),
                    neg / (neg + 1.0),
                    (t / (t + 1.0)).powf(self.config.dcfr_gamma),
                ]
            },
            iteration: self.iterations,
            run_start: start,
        };
        let buffers = Buffers {
            regret: self.regret.slice(),
            average: self.average.slice(),
        };
        let mut diagnostics = RunDiagnostics::default();
        if self.config.threads == 1 || self.pool.current_thread_index().is_some() {
            traverse(
                &context,
                &mut self.state,
                &mut self.scratch,
                buffers,
                player,
                mode,
                true,
                &mut totals_cache,
                &mut diagnostics,
            );
        } else {
            self.pool.install(|| {
                traverse(
                    &context,
                    &mut self.state,
                    &mut self.scratch,
                    buffers,
                    player,
                    mode,
                    true,
                    &mut totals_cache,
                    &mut diagnostics,
                )
            });
        }
        if let Some(d) = &mut self.kernel_diagnostics {
            diagnostics.times.wall_ns = start.elapsed().as_nanos();
            diagnostics.times.traversals = 1;
            match mode {
                Mode::Train => {
                    d.training.add(diagnostics.times);
                    let iteration = d.iterations.last_mut().expect("training iteration");
                    iteration.times.add(diagnostics.times);
                    iteration.chance_batches.append(&mut diagnostics.batches);
                    d.latest_training_tasks.append(&mut diagnostics.tasks);
                }
                Mode::Average => {
                    d.average_evaluation.add(diagnostics.times);
                    d.latest_metrics_tasks.append(&mut diagnostics.tasks);
                }
                Mode::BestResponse => {
                    d.best_response_evaluation.add(diagnostics.times);
                    d.latest_metrics_tasks.append(&mut diagnostics.tasks);
                }
            }
        }
        if matches!(mode, Mode::BestResponse) {
            self.wall_profile.best_response_seconds += start.elapsed().as_secs_f64();
        }
        self.scratch[0].value[..self.initial_reach[player].len()]
            .iter()
            .zip(&self.initial_reach[player])
            .map(|(v, w)| v.to_f64() * w)
            .sum::<f64>()
            / self.joint_mass
    }
    fn run_metric(&mut self, player: usize, mode: Mode) -> f64 {
        let Some(evaluation) = &mut self.evaluation else {
            return self.run(player, mode);
        };
        let start = Instant::now();
        for (out, &weight) in evaluation.scratch[0].reach[0]
            .iter_mut()
            .zip(&self.initial_reach[0])
        {
            *out = weight;
        }
        for (out, &weight) in evaluation.scratch[0].reach[1]
            .iter_mut()
            .zip(&self.initial_reach[1])
        {
            *out = weight;
        }
        let context = Context {
            tree: &self.tree,
            ranges: &self.ranges,
            masks: &self.masks,
            hand_cards: &self.hand_cards,
            same: &self.same,
            tables: &self.tables,
            terminal_table: &self.terminal_table,
            config: &self.config,
            permutations: &self.permutations,
            average_reach_scale: &self.average_reach_scale,
            initial_reach: &self.initial_reach,
            discounts: {
                let t = self.iterations.max(1) as f64;
                let pos = t.powf(self.config.dcfr_alpha);
                let neg = t.powf(self.config.dcfr_beta);
                [
                    pos / (pos + 1.0),
                    neg / (neg + 1.0),
                    (t / (t + 1.0)).powf(self.config.dcfr_gamma),
                ]
            },
            iteration: self.iterations,
            run_start: start,
        };
        let buffers = Buffers {
            regret: self.regret.slice(),
            average: self.average.slice(),
        };
        let mut diagnostics = RunDiagnostics::default();
        if self.config.threads == 1 || self.pool.current_thread_index().is_some() {
            traverse(
                &context,
                &mut evaluation.state,
                &mut evaluation.scratch,
                buffers,
                player,
                mode,
                true,
                &mut TotalsCache::<f64>::default(),
                &mut diagnostics,
            );
        } else {
            self.pool.install(|| {
                traverse(
                    &context,
                    &mut evaluation.state,
                    &mut evaluation.scratch,
                    buffers,
                    player,
                    mode,
                    true,
                    &mut TotalsCache::<f64>::default(),
                    &mut diagnostics,
                )
            });
        }
        if let Some(d) = &mut self.kernel_diagnostics {
            diagnostics.times.wall_ns = start.elapsed().as_nanos();
            diagnostics.times.traversals = 1;
            match mode {
                Mode::Train => {
                    d.training.add(diagnostics.times);
                    let iteration = d.iterations.last_mut().expect("training iteration");
                    iteration.times.add(diagnostics.times);
                    iteration.chance_batches.append(&mut diagnostics.batches);
                    d.latest_training_tasks.append(&mut diagnostics.tasks);
                }
                Mode::Average => {
                    d.average_evaluation.add(diagnostics.times);
                    d.latest_metrics_tasks.append(&mut diagnostics.tasks);
                }
                Mode::BestResponse => {
                    d.best_response_evaluation.add(diagnostics.times);
                    d.latest_metrics_tasks.append(&mut diagnostics.tasks);
                }
            }
        }
        if matches!(mode, Mode::BestResponse) {
            self.wall_profile.best_response_seconds += start.elapsed().as_secs_f64();
        }
        evaluation.scratch[0].value[..self.initial_reach[player].len()]
            .iter()
            .zip(&self.initial_reach[player])
            .map(|(v, w)| v.to_f64() * w)
            .sum::<f64>()
            / self.joint_mass
    }
    pub fn metrics(&mut self) -> Metrics {
        if let Some(evaluation) = &mut self.evaluation {
            evaluation.state.copy_scales(&self.state);
        }
        if let Some(d) = &mut self.kernel_diagnostics {
            d.latest_metrics_tasks.clear();
        }
        let average_ev = [
            self.run_metric(0, Mode::Average),
            self.run_metric(1, Mode::Average),
        ];
        let best_response_ev = [
            self.run_metric(0, Mode::BestResponse),
            self.run_metric(1, Mode::BestResponse),
        ];
        let exploitability_chips = ((best_response_ev[0] - average_ev[0])
            + (best_response_ev[1] - average_ev[1]))
            .max(0.0)
            / 2.0;
        Metrics {
            average_ev,
            best_response_ev,
            exploitability_chips,
            exploitability_pct: 100.0 * exploitability_chips / self.tree.config.starting_pot as f64,
        }
    }
    /// Exact conditioned profile / BR metrics outside the allocation-free
    /// iteration loop. Input is OWN reach in original dense combo coordinates;
    /// aliases are permuted to the stored representative before traversal.
    /// Rejects own reach breaking any remaining merged chance-orbit symmetry;
    /// use a full-tree engine for arbitrary asymmetric conditioned ranges.
    /// EV retains the source game's sunk investment; percentage uses node pot.
    pub fn metrics_at_node(
        &mut self,
        node: usize,
        mut reach: [Vec<f64>; 2],
    ) -> Result<Metrics, String> {
        let public = self.tree.nodes.get(node).ok_or("unknown node")?;
        let pot = public.pot;
        let mask = board_mask(&public.board);
        for (p, weights) in reach.iter_mut().enumerate() {
            if weights.len() != self.ranges[p].len()
                || weights.iter().any(|w| !w.is_finite() || *w < 0.0)
            {
                return Err("invalid conditioned own reach".into());
            }
            for (i, w) in weights.iter_mut().enumerate() {
                if self.masks[p][i] & mask != 0 {
                    *w = 0.0;
                }
            }
            let total: f64 = weights.iter().sum();
            if !total.is_finite() || total <= 0.0 {
                return Err("conditioned range has no positive legal mass".into());
            }
            for w in weights {
                *w /= total;
            }
        }
        let mut source = node;
        while self.aliases[source] as usize != source {
            let permutation = &self.permutations[self.alias_perms[source] as usize];
            reach = std::array::from_fn(|p| {
                let mut out = vec![0.0; reach[p].len()];
                for (i, &w) in reach[p].iter().enumerate() {
                    out[permutation.indices[p][i]] = w;
                }
                out
            });
            source = self.aliases[source] as usize;
        }
        // Remaining merged orbits evaluate representatives once. That is exact
        // only when BOTH supplied reach vectors preserve their permutations.
        // Reject asymmetry, never silently average or accept approximate parity.
        fn mark_orbits<S: Scratch>(state: &State<S>, used: &mut [bool]) {
            for orbit in &state.orbits {
                for &p in orbit {
                    used[p] = true;
                }
            }
            for child in &state.children {
                mark_orbits(child, used);
            }
        }
        let mut used = vec![false; self.permutations.len()];
        mark_orbits(
            self.state.find(source).expect("alias representative state"),
            &mut used,
        );
        for (id, permutation) in self.permutations.iter().enumerate() {
            if !used[id] {
                continue;
            }
            for (p, weights) in reach.iter().enumerate() {
                if weights
                    .iter()
                    .enumerate()
                    .any(|(i, &w)| w != weights[permutation.indices[p][i]])
                {
                    return Err(
                        "conditioned reach breaks a remaining suit-isomorphism orbit".into(),
                    );
                }
            }
        }
        let other = totals(&self.hand_cards[1], &reach[1]);
        let joint_mass: f64 = reach[0]
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                w * compatible(
                    self.hand_cards[0][i],
                    &other,
                    self.same[0][i].map_or(0.0, |j| reach[1][j]),
                )
            })
            .sum();
        if !joint_mass.is_finite() || joint_mass <= 0.0 {
            return Err("conditioned ranges have no compatible deals".into());
        }
        if let Some(evaluation) = &mut self.evaluation {
            evaluation.state.copy_scales(&self.state);
        }
        let mut evaluate = |player, mode| {
            let context = Context {
                tree: &self.tree,
                ranges: &self.ranges,
                masks: &self.masks,
                hand_cards: &self.hand_cards,
                same: &self.same,
                tables: &self.tables,
                terminal_table: &self.terminal_table,
                config: &self.config,
                permutations: &self.permutations,
                average_reach_scale: &self.average_reach_scale,
                initial_reach: &self.initial_reach,
                discounts: [0.0; 3],
                iteration: self.iterations,
                run_start: Instant::now(),
            };
            let buffers = Buffers {
                regret: self.regret.slice(),
                average: self.average.slice(),
            };
            if let Some(evaluation) = &mut self.evaluation {
                conditioned_metric::<f64, f64>(
                    &context,
                    evaluation
                        .state
                        .find_mut(source)
                        .expect("alias representative state"),
                    &mut evaluation.scratch,
                    buffers,
                    &reach,
                    joint_mass,
                    player,
                    mode,
                )
            } else {
                conditioned_metric::<S, T>(
                    &context,
                    self.state
                        .find_mut(source)
                        .expect("alias representative state"),
                    &mut self.scratch,
                    buffers,
                    &reach,
                    joint_mass,
                    player,
                    mode,
                )
            }
        };
        let average_ev = [evaluate(0, Mode::Average), evaluate(1, Mode::Average)];
        let best_response_ev = [
            evaluate(0, Mode::BestResponse),
            evaluate(1, Mode::BestResponse),
        ];
        let exploitability_chips = ((best_response_ev[0] - average_ev[0])
            + (best_response_ev[1] - average_ev[1]))
            .max(0.0)
            / 2.0;
        Ok(Metrics {
            average_ev,
            best_response_ev,
            exploitability_chips,
            exploitability_pct: 100.0 * exploitability_chips / pot as f64,
        })
    }
    /// Average action-major probabilities; zero for future-board-blocked hands.
    /// The returned result allocation is outside the iteration loop.
    pub fn strategy(&self, node: usize) -> Result<Vec<f32>, String> {
        self.aliases.get(node).ok_or("unknown node")?;
        let NodeKind::Decision {
            player, actions, ..
        } = &self.tree.nodes[node].kind
        else {
            return Err("node is not a decision".into());
        };
        let n = self.ranges[*player].combos.len();
        let mut indices: Vec<usize> = (0..n).collect();
        let mut source = node;
        while self.aliases[source] as usize != source {
            let map = &self.permutations[self.alias_perms[source] as usize].indices[*player];
            for i in &mut indices {
                *i = map[*i];
            }
            source = self.aliases[source] as usize;
        }
        let canonical = source;
        let s = self.state.find(canonical).ok_or("unknown node")?;
        let mut norm = vec![0.0; s.own_len];
        self.average.normalize_policy(
            s.offset,
            s.own_len,
            &mut norm,
            n,
            actions.len(),
            &self.masks[*player],
            board_mask(&self.tree.nodes[canonical].board),
        );
        let mut out = vec![0.0; s.own_len];
        for a in 0..actions.len() {
            for i in 0..n {
                out[a * n + i] = norm[a * n + indices[i]];
            }
        }
        Ok(out)
    }
    pub fn iterations(&self) -> usize {
        self.iterations
    }
    pub fn memory_bytes(&self) -> usize {
        self.memory_bytes
    }
    pub fn tree(&self) -> &GameTree {
        &self.tree
    }
    pub fn ranges(&self) -> &[DenseRange; 2] {
        &self.ranges
    }
    /// Sum of measured terminal work across workers, not wall-clock elapsed time.
    pub fn terminal_seconds(&self) -> f64 {
        let mut w = self.state.work();
        if let Some(evaluation) = &self.evaluation {
            for (total, extra) in w.iter_mut().zip(evaluation.state.work()) {
                *total += extra;
            }
        }
        (w[0] + w[1]) as f64 * 1e-9
    }
    pub fn profile(&self) -> WorkProfile {
        let mut w = self.state.work();
        if let Some(evaluation) = &self.evaluation {
            for (total, extra) in w.iter_mut().zip(evaluation.state.work()) {
                *total += extra;
            }
        }
        let mut p = self.wall_profile;
        p.showdown_seconds = w[0] as f64 * 1e-9;
        p.fold_seconds = w[1] as f64 * 1e-9;
        p.regret_seconds = w[2] as f64 * 1e-9;
        p.chance_seconds = w[3] as f64 * 1e-9;
        p
    }
    pub fn kernel_diagnostics(&self) -> Option<&KernelDiagnostics> {
        self.kernel_diagnostics.as_ref()
    }
    pub fn persistent_storage_bytes(&self) -> usize {
        self.state.span
            * match self.config.storage_mode {
                StorageMode::F32 => 8,
                StorageMode::I16 => 4,
            }
    }
    pub fn isomorphism_used(&self) -> bool {
        self.merged_children > 0
    }
    pub fn merged_chance_children(&self) -> usize {
        self.merged_children
    }
    pub fn strategy_storage_offset(&self, node: usize) -> Option<usize> {
        self.state.find(node).map(|s| s.offset)
    }
}

/// Exact weight equality is intentional: close floating-point weights are NOT
/// evidence that a production range can safely be symmetrized.
fn range_permutations(
    tree: &GameTree,
    ranges: &[DenseRange; 2],
) -> Result<Vec<ComboPermutation>, String> {
    let board: Vec<Card> = tree.nodes[tree.root]
        .board
        .iter()
        .map(|&c| Card(c))
        .collect();
    let mut out = Vec::new();
    for suit in cards::board_preserving_permutations(&board)? {
        let mut indices: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        let mut valid = true;
        for player in 0..2 {
            let mut dense = [None; 1326];
            for (i, c) in ranges[player].combos.iter().enumerate() {
                dense[c.combo.id()] = Some(i);
            }
            for c in &ranges[player].combos {
                let Some(j) = dense[suit.map_combo(c.combo).id()] else {
                    valid = false;
                    break;
                };
                if c.weight.to_bits() != ranges[player].combos[j].weight.to_bits() {
                    valid = false;
                    break;
                }
                indices[player].push(j);
            }
            if !valid {
                break;
            }
        }
        if valid {
            out.push(ComboPermutation { suit, indices });
        }
    }
    // Enumeration starts with identity, hence permutation 0 is always identity.
    if out.first().is_none_or(|p| p.suit.0 != [0, 1, 2, 3]) {
        return Err("missing identity permutation".into());
    }
    Ok(out)
}
fn prepare_iso(
    tree: &GameTree,
    permutations: &[ComboPermutation],
    id: usize,
    aliases: &mut [u32],
    alias_perms: &mut [u32],
) -> Result<usize, String> {
    let mut merged = 0;
    let root_len = tree.nodes[tree.root].board.len();
    let children = match &tree.nodes[id].kind {
        NodeKind::Chance { children, .. } | NodeKind::Decision { children, .. } => {
            children.as_slice()
        }
        _ => &[][..],
    };
    if let NodeKind::Chance { cards, .. } = &tree.nodes[id].kind {
        // Preserve street history, not merely the final board's unordered set.
        let valid: Vec<_> = permutations
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                tree.nodes[id].board[root_len..]
                    .iter()
                    .all(|&c| p.suit.map_card(Card(c)).0 == c)
            })
            .collect();
        for member in 0..cards.len() {
            for rep in 0..member {
                if aliases[children[rep]] as usize != children[rep] {
                    continue;
                }
                if let Some(&(index, p)) = valid
                    .iter()
                    .find(|(_, p)| p.suit.map_card(Card(cards[member])).0 == cards[rep])
                {
                    alias_subtree(
                        tree,
                        children[member],
                        children[rep],
                        p.suit,
                        index,
                        aliases,
                        alias_perms,
                    )?;
                    merged += 1;
                    break;
                }
            }
        }
    }
    for &child in children {
        if aliases[child] as usize == child {
            merged += prepare_iso(tree, permutations, child, aliases, alias_perms)?;
        }
    }
    Ok(merged)
}
fn alias_subtree(
    tree: &GameTree,
    from: usize,
    to: usize,
    p: SuitPermutation,
    permutation: usize,
    aliases: &mut [u32],
    alias_perms: &mut [u32],
) -> Result<(), String> {
    let a = &tree.nodes[from];
    let b = &tree.nodes[to];
    let mapped = a
        .board
        .iter()
        .fold(0u64, |m, &c| m | p.map_card(Card(c)).mask());
    if mapped != board_mask(&b.board)
        || a.pot != b.pot
        || a.invested != b.invested
        || a.stacks != b.stacks
    {
        return Err("non-equivariant isomorphic subtree".into());
    }
    aliases[from] = compact_index(to)?;
    alias_perms[from] = compact_index(permutation)?;
    match (&a.kind, &b.kind) {
        (
            NodeKind::Decision {
                player: pa,
                actions: aa,
                children: ca,
            },
            NodeKind::Decision {
                player: pb,
                actions: ab,
                children: cb,
            },
        ) if pa == pb && aa == ab => {
            for (&x, &y) in ca.iter().zip(cb) {
                alias_subtree(tree, x, y, p, permutation, aliases, alias_perms)?;
            }
        }
        (
            NodeKind::Chance {
                cards: ca,
                children: aa,
            },
            NodeKind::Chance {
                cards: cb,
                children: ab,
            },
        ) => {
            for (&card, &x) in ca.iter().zip(aa) {
                let j = cb
                    .iter()
                    .position(|&c| c == p.map_card(Card(card)).0)
                    .ok_or("isomorphic chance mismatch")?;
                alias_subtree(tree, x, ab[j], p, permutation, aliases, alias_perms)?;
            }
        }
        (NodeKind::Terminal { winner: wa }, NodeKind::Terminal { winner: wb }) if wa == wb => {}
        _ => return Err("non-equivariant isomorphic actions".into()),
    }
    Ok(())
}

trait PolicyValue: Copy {
    fn positive(self) -> f64;
    #[cfg(target_arch = "aarch64")]
    fn positive_pair(input: &[Self]) -> float64x2_t;
}
impl PolicyValue for f32 {
    fn positive(self) -> f64 {
        self.max(0.0) as f64
    }
    #[cfg(target_arch = "aarch64")]
    fn positive_pair(input: &[Self]) -> float64x2_t {
        assert!(input.len() >= 2);
        // aarch64 provides NEON. The validated slice makes this exact two-f32
        // load safe, including the last even pair (no full-width overread).
        unsafe { vcvt_f64_f32(vmaxnm_f32(vld1_f32(input.as_ptr()), vdup_n_f32(0.0))) }
    }
}
impl PolicyValue for i16 {
    fn positive(self) -> f64 {
        self.max(0) as f64
    }
    #[cfg(target_arch = "aarch64")]
    fn positive_pair(input: &[Self]) -> float64x2_t {
        // Read exactly two checked scalar lanes. Packing their bit patterns
        // does not rely on alignment, pointer casts, or adjacent readable data.
        let bits = input[0] as u16 as u64 | ((input[1] as u16 as u64) << 16);
        unsafe {
            let signed = vget_low_s32(vmovl_s16(vcreate_s16(bits)));
            vcvtq_f64_s64(vmovl_s32(vmax_s32(signed, vdup_n_s32(0))))
        }
    }
}
fn normalize(
    input: &[f32],
    output: &mut [f32],
    n: usize,
    actions: usize,
    masks: &[u64],
    board: u64,
) {
    normalize_policy(input, output, n, actions, masks, board);
}
fn normalize_i16(
    input: &[i16],
    output: &mut [f32],
    n: usize,
    actions: usize,
    masks: &[u64],
    board: u64,
) {
    normalize_policy(input, output, n, actions, masks, board);
}
fn normalize_policy<T: PolicyValue>(
    input: &[T],
    output: &mut [f32],
    n: usize,
    actions: usize,
    masks: &[u64],
    board: u64,
) {
    match actions {
        2 => normalize_fixed::<T, 2>(input, output, n, masks, board),
        3 => normalize_fixed::<T, 3>(input, output, n, masks, board),
        _ => {
            for (i, &mask) in masks.iter().enumerate() {
                if mask & board != 0 {
                    for a in 0..actions {
                        output[a * n + i] = 0.0;
                    }
                    continue;
                }
                let sum: f64 = (0..actions).map(|a| input[a * n + i].positive()).sum();
                if sum > 0.0 {
                    let inverse = 1.0 / sum;
                    for a in 0..actions {
                        output[a * n + i] = (input[a * n + i].positive() * inverse) as f32;
                    }
                } else {
                    for a in 0..actions {
                        output[a * n + i] = 1.0 / actions as f32;
                    }
                }
            }
        }
    }
}
// NEON is part of the aarch64 baseline; all other architectures keep the
// portable scalar/vectorizable path. Neither changes the precision or order.
fn normalize_fixed<T: PolicyValue, const A: usize>(
    input: &[T],
    output: &mut [f32],
    n: usize,
    masks: &[u64],
    board: u64,
) {
    #[cfg(target_arch = "aarch64")]
    neon_normalize_fixed::<T, A>(input, output, n, masks, board);
    #[cfg(not(target_arch = "aarch64"))]
    normalize_fixed_portable::<T, A>(input, output, n, masks, board);
}
#[cfg(target_arch = "aarch64")]
fn neon_normalize_fixed<T: PolicyValue, const A: usize>(
    input: &[T],
    output: &mut [f32],
    n: usize,
    masks: &[u64],
    board: u64,
) {
    assert!(A == 2 || A == 3);
    assert_eq!(input.len(), A * n);
    assert_eq!(output.len(), A * n);
    assert_eq!(masks.len(), n);
    let pairs = n & !1;
    // All loads/stores below address two elements within already validated
    // action slices. Odd tails never reach these instructions.
    unsafe {
        let zero = vdupq_n_f64(0.0);
        let one = vdupq_n_f64(1.0);
        let uniform = vdupq_n_f64((1.0 / A as f32) as f64);
        let board = vdupq_n_u64(board);
        for i in (0..pairs).step_by(2) {
            let positive: [float64x2_t; A] =
                std::array::from_fn(|a| T::positive_pair(&input[a * n + i..(a + 1) * n]));
            let mut sum = zero;
            // No reassociation/FMA: same 0+x+y(+z) as the scalar oracle.
            for &x in &positive {
                sum = vaddq_f64(sum, x);
            }
            let nonzero = vcgtq_f64(sum, zero);
            // Avoid division by zero for uniform-policy lanes. Selecting the
            // denominator does not alter any nonzero-policy lane.
            let inverse = vdivq_f64(one, vbslq_f64(nonzero, sum, one));
            let active = vceqq_u64(
                vandq_u64(vld1q_u64(masks.as_ptr().add(i)), board),
                vdupq_n_u64(0),
            );
            for (a, &positive) in positive.iter().enumerate() {
                let normalized = vmulq_f64(positive, inverse);
                let policy = vbslq_f64(active, vbslq_f64(nonzero, normalized, uniform), zero);
                vst1_f32(output.as_mut_ptr().add(a * n + i), vcvt_f32_f64(policy));
            }
        }
    }
    for i in pairs..n {
        if masks[i] & board != 0 {
            for a in 0..A {
                output[a * n + i] = 0.0;
            }
            continue;
        }
        let positive: [f64; A] = std::array::from_fn(|a| input[a * n + i].positive());
        let sum: f64 = positive.iter().sum();
        let inverse = if sum > 0.0 { 1.0 / sum } else { 0.0 };
        for a in 0..A {
            output[a * n + i] = if sum > 0.0 {
                (positive[a] * inverse) as f32
            } else {
                1.0 / A as f32
            };
        }
    }
}
#[cfg(any(not(target_arch = "aarch64"), test))]
// Disjoint action slices and zipped exact-length streams remove per-combo
// dynamic indexing. LLVM can use packed operations without fast-math, FMA,
// architecture-specific intrinsics, or a changed reduction order.
fn normalize_fixed_portable<T: PolicyValue, const A: usize>(
    input: &[T],
    output: &mut [f32],
    n: usize,
    masks: &[u64],
    board: u64,
) {
    assert_eq!(input.len(), A * n);
    assert_eq!(output.len(), A * n);
    assert_eq!(masks.len(), n);
    let (i0, input) = input.split_at(n);
    let (o0, output) = output.split_at_mut(n);
    if A == 2 {
        let i1 = input;
        let o1 = output;
        for ((((&mask, &x), &y), o0), o1) in masks.iter().zip(i0).zip(i1).zip(o0).zip(o1) {
            let x = x.positive();
            let y = y.positive();
            // Keep the original Iterator::sum accumulation (0 + x + y).
            let sum = (0.0 + x) + y;
            let inverse = if sum > 0.0 { 1.0 / sum } else { 0.0 };
            *o0 = if mask & board != 0 {
                0.0
            } else if sum > 0.0 {
                (x * inverse) as f32
            } else {
                0.5
            };
            *o1 = if mask & board != 0 {
                0.0
            } else if sum > 0.0 {
                (y * inverse) as f32
            } else {
                0.5
            };
        }
    } else if A == 3 {
        let (i1, i2) = input.split_at(n);
        let (o1, o2) = output.split_at_mut(n);
        for ((((((&mask, &x), &y), &z), o0), o1), o2) in
            masks.iter().zip(i0).zip(i1).zip(i2).zip(o0).zip(o1).zip(o2)
        {
            let x = x.positive();
            let y = y.positive();
            let z = z.positive();
            let sum = ((0.0 + x) + y) + z;
            let inverse = if sum > 0.0 { 1.0 / sum } else { 0.0 };
            let uniform = 1.0 / 3.0;
            *o0 = if mask & board != 0 {
                0.0
            } else if sum > 0.0 {
                (x * inverse) as f32
            } else {
                uniform
            };
            *o1 = if mask & board != 0 {
                0.0
            } else if sum > 0.0 {
                (y * inverse) as f32
            } else {
                uniform
            };
            *o2 = if mask & board != 0 {
                0.0
            } else if sum > 0.0 {
                (z * inverse) as f32
            } else {
                uniform
            };
        }
    } else {
        unreachable!("fixed normalization is used for 2/3 actions only");
    }
}
#[cfg(test)]
fn scalar_normalize_fixed<T: PolicyValue, const A: usize>(
    input: &[T],
    output: &mut [f32],
    n: usize,
    masks: &[u64],
    board: u64,
) {
    for (i, &mask) in masks.iter().enumerate() {
        if mask & board != 0 {
            for a in 0..A {
                output[a * n + i] = 0.0;
            }
            continue;
        }
        let positive: [f64; A] = std::array::from_fn(|a| input[a * n + i].positive());
        let sum: f64 = positive.iter().sum();
        if sum > 0.0 {
            let inverse = 1.0 / sum;
            for a in 0..A {
                output[a * n + i] = (positive[a] * inverse) as f32;
            }
        } else {
            for a in 0..A {
                output[a * n + i] = 1.0 / A as f32;
            }
        }
    }
}
struct Buffers<'a> {
    regret: SlabSlice<'a>,
    average: SlabSlice<'a>,
}
impl<'a> Buffers<'a> {
    fn split(self, at: usize) -> (Self, Self) {
        let (ar, br) = self.regret.split(at);
        let (aa, ba) = self.average.split(at);
        (
            Self {
                regret: ar,
                average: aa,
            },
            Self {
                regret: br,
                average: ba,
            },
        )
    }
}
// Orbit members and children are still visited in their original order. The
// identity member avoids dense-index gathers and exposes a contiguous add loop.
fn chance_add<S: Scratch, T: Scratch>(
    output: &mut [T],
    child: &[S],
    permutation: usize,
    permutations: &[ComboPermutation],
    target: usize,
) {
    if permutation == 0 {
        for (out, &value) in output.iter_mut().zip(child) {
            *out += T::from_f64(value.to_f64());
        }
    } else {
        for (out, &j) in output
            .iter_mut()
            .zip(&permutations[permutation].indices[target])
        {
            *out += T::from_f64(child[j].to_f64());
        }
    }
}
// Conditioned evaluation uses the same traversal, policy slabs and summation
// order as root evaluation, with an explicitly normalized own reach.
#[allow(clippy::too_many_arguments)]
fn conditioned_metric<S: Scratch, T: Scratch>(
    ctx: &Context<'_>,
    state: &mut State<S>,
    scratch: &mut [Level<S>],
    buffers: Buffers<'_>,
    reach: &[Vec<f64>; 2],
    joint_mass: f64,
    player: usize,
    mode: Mode,
) -> f64 {
    for (out, &weight) in scratch[0].reach[0].iter_mut().zip(&reach[0]) {
        *out = S::from_f64(weight);
    }
    for (out, &weight) in scratch[0].reach[1].iter_mut().zip(&reach[1]) {
        *out = S::from_f64(weight);
    }
    let (_, buffers) = buffers.split(state.offset);
    let (buffers, _) = buffers.split(state.span);
    traverse(
        ctx,
        state,
        scratch,
        buffers,
        player,
        mode,
        false,
        &mut TotalsCache::<T>::default(),
        &mut RunDiagnostics::default(),
    );
    scratch[0].value[..reach[player].len()]
        .iter()
        .zip(&reach[player])
        .map(|(v, w)| v.to_f64() * w)
        .sum::<f64>()
        / joint_mass
}
// Traversal borrows independent slabs/scratch and a worker-local opt-in report;
// keeping these explicit avoids shared locks or additional per-node metadata.
#[allow(clippy::too_many_arguments)]
fn traverse<S: Scratch, T: Scratch>(
    ctx: &Context<'_>,
    state: &mut State<S>,
    scratch: &mut [Level<S>],
    buffers: Buffers<'_>,
    target: usize,
    mode: Mode,
    parallel: bool,
    totals_cache: &mut TotalsCache<T>,
    diagnostics: &mut RunDiagnostics,
) {
    let (level, desc_scratch) = scratch.split_first_mut().expect("depth scratch");
    let node = &ctx.tree.nodes[state.id];
    let n = ctx.ranges[target].combos.len();
    let (mut own, mut descendants) = buffers.split(state.own_len);
    match &node.kind {
        NodeKind::Terminal { winner } => {
            let start = (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
            terminal(
                ctx,
                state.id,
                level,
                target,
                *winner,
                totals_cache,
                ctx.config
                    .kernel_diagnostics
                    .then_some(&mut diagnostics.times),
            );
            if let Some(t) = start {
                let ns = t.elapsed().as_nanos();
                state.work[if winner.is_none() { 0 } else { 1 }] += ns as u64;
                if ctx.config.kernel_diagnostics {
                    diagnostics
                        .times
                        .section(if winner.is_none() { 0 } else { 1 }, ns);
                }
            }
        }
        NodeKind::Chance { .. } => {
            let start = (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
            if std::mem::size_of::<T>() > std::mem::size_of::<S>() {
                level.wide_value[..n].fill(0.0);
            } else {
                level.value[..n].fill(S::ZERO);
            }
            let probability = 1.0 / (52 - node.board.len() - 4) as f64;
            if let Some(t) = start {
                let ns = t.elapsed().as_nanos();
                state.work[3] += ns as u64;
                if ctx.config.kernel_diagnostics {
                    diagnostics.times.section(3, ns);
                }
            }
            if parallel && ctx.config.threads > 1 {
                let batch_start = ctx.config.kernel_diagnostics.then(Instant::now);
                let mut child_diagnostics = visit_parallel_chance(
                    ctx,
                    &mut state.children,
                    &mut state.parallel_scratch,
                    descendants,
                    &level.reach,
                    target,
                    mode,
                    probability,
                    state.id,
                    totals_cache,
                );
                if let Some(t) = batch_start {
                    child_diagnostics.batch(state.id, target, t.elapsed().as_nanos());
                }
                diagnostics.merge(child_diagnostics);
                let start =
                    (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
                for ((child_scratch, orbit), _) in state
                    .parallel_scratch
                    .iter()
                    .zip(&state.orbits)
                    .zip(&state.children)
                {
                    for &permutation in orbit {
                        if std::mem::size_of::<T>() > std::mem::size_of::<S>() {
                            chance_add(
                                &mut level.wide_value[..n],
                                &child_scratch[0].value[..n],
                                permutation,
                                ctx.permutations,
                                target,
                            );
                        } else {
                            chance_add(
                                &mut level.value[..n],
                                &child_scratch[0].value[..n],
                                permutation,
                                ctx.permutations,
                                target,
                            );
                        }
                    }
                }
                if let Some(t) = start {
                    let ns = t.elapsed().as_nanos();
                    state.work[3] += ns as u64;
                    if ctx.config.kernel_diagnostics {
                        diagnostics.times.section(3, ns);
                    }
                }
            } else {
                for (child, orbit) in state.children.iter_mut().zip(&state.orbits) {
                    let start =
                        (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
                    let card = *ctx.tree.nodes[child.id]
                        .board
                        .last()
                        .expect("chance child card");
                    chance_reach(
                        ctx,
                        &level.reach,
                        &mut desc_scratch[0].reach,
                        card,
                        target,
                        probability,
                    );
                    if let Some(t) = start {
                        let ns = t.elapsed().as_nanos();
                        state.work[3] += ns as u64;
                        if ctx.config.kernel_diagnostics {
                            diagnostics.times.section(3, ns);
                        }
                    }
                    let (child_buffers, rest) = descendants.split(child.span);
                    descendants = rest;
                    traverse(
                        ctx,
                        child,
                        desc_scratch,
                        child_buffers,
                        target,
                        mode,
                        false,
                        &mut totals_cache.fresh(),
                        diagnostics,
                    );
                    let start =
                        (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
                    for &permutation in orbit {
                        if std::mem::size_of::<T>() > std::mem::size_of::<S>() {
                            chance_add(
                                &mut level.wide_value[..n],
                                &desc_scratch[0].value[..n],
                                permutation,
                                ctx.permutations,
                                target,
                            );
                        } else {
                            chance_add(
                                &mut level.value[..n],
                                &desc_scratch[0].value[..n],
                                permutation,
                                ctx.permutations,
                                target,
                            );
                        }
                    }
                    if let Some(t) = start {
                        let ns = t.elapsed().as_nanos();
                        state.work[3] += ns as u64;
                        if ctx.config.kernel_diagnostics {
                            diagnostics.times.section(3, ns);
                        }
                    }
                }
            }
            if std::mem::size_of::<T>() > std::mem::size_of::<S>() {
                for (out, &value) in level.value[..n].iter_mut().zip(&level.wide_value[..n]) {
                    *out = S::from_f64(value);
                }
            }
        }
        NodeKind::Decision {
            player, actions, ..
        } => {
            let actor = *player;
            let na = ctx.ranges[actor].combos.len();
            let len = state.own_len;
            let start = (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
            let own_best_response = actor == target && matches!(mode, Mode::BestResponse);
            if !own_best_response {
                let source = if matches!(mode, Mode::Train) {
                    &own.regret
                } else {
                    &own.average
                };
                source.normalize_policy(
                    &mut level.strategy[..len],
                    na,
                    actions.len(),
                    &ctx.masks[actor],
                    board_mask(&node.board),
                );
            }
            if let Some(t) = start {
                let ns = t.elapsed().as_nanos();
                state.work[2] += ns as u64;
                if ctx.config.kernel_diagnostics {
                    diagnostics.times.section(2, ns);
                }
            }
            for (a, child) in state.children.iter_mut().enumerate() {
                let transport = ctx.config.kernel_diagnostics.then(Instant::now);
                decision_reach(
                    &level.reach,
                    &mut desc_scratch[0].reach,
                    actor,
                    (!own_best_response).then(|| &level.strategy[a * na..(a + 1) * na]),
                );
                if let Some(t) = transport {
                    diagnostics.times.decision_transport_ns += t.elapsed().as_nanos();
                }
                let (child_buffers, rest) = descendants.split(child.span);
                descendants = rest;
                // Only target-player actions leave opposing reach unchanged.
                // A different opponent action must never inherit the parent's totals.
                let mut changed_reach_cache = totals_cache.fresh();
                let child_cache = if actor == target {
                    &mut *totals_cache
                } else {
                    &mut changed_reach_cache
                };
                traverse(
                    ctx,
                    child,
                    desc_scratch,
                    child_buffers,
                    target,
                    mode,
                    parallel,
                    child_cache,
                    diagnostics,
                );
                let transport = ctx.config.kernel_diagnostics.then(Instant::now);
                level.action_values[a * n..(a + 1) * n]
                    .copy_from_slice(&desc_scratch[0].value[..n]);
                if let Some(t) = transport {
                    diagnostics.times.decision_transport_ns += t.elapsed().as_nanos();
                }
            }
            let start = (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
            // Action-major contiguous slices, retaining each hand's original
            // action order and float Sum's negative-zero identity exactly.
            let value = &mut level.value[..n];
            if actor == target && matches!(mode, Mode::BestResponse) {
                value.fill(S::NEG_INFINITY);
                for action in level.action_values[..actions.len() * n].chunks_exact(n) {
                    for (out, &child) in value.iter_mut().zip(action) {
                        *out = out.max(child);
                    }
                }
            } else {
                value.fill(S::from_f64(-0.0));
                for (a, action) in level.action_values[..actions.len() * n]
                    .chunks_exact(n)
                    .enumerate()
                {
                    if actor != target {
                        for (out, &child) in value.iter_mut().zip(action) {
                            *out += child;
                        }
                    } else {
                        let policy = &level.strategy[a * na..(a + 1) * na];
                        for ((out, &child), &probability) in
                            value.iter_mut().zip(action).zip(policy)
                        {
                            *out += S::from_f64(probability as f64) * child;
                        }
                    }
                }
            }
            if matches!(mode, Mode::Train) && actor == target {
                for i in 0..na {
                    let reach = level.reach[actor][i];
                    // Keep inverse-range normalization wide: its constant scale
                    // can overflow f32 even when the resulting path reach cannot.
                    let scale = ctx.average_reach_scale[actor][i];
                    level.average_path_reach[i] = if reach == S::ZERO {
                        S::ZERO
                    } else if scale.is_finite() {
                        S::from_f64(reach.to_f64() * scale)
                    } else {
                        S::from_f64(reach.to_f64() / ctx.initial_reach[actor][i])
                    };
                }
                match (&mut own.regret, &mut own.average) {
                    (SlabSlice::F32(regret), SlabSlice::F32(average)) => {
                        update_policy(
                            ctx,
                            &level.strategy[..len],
                            &level.action_values,
                            &level.value,
                            &level.average_path_reach[..na],
                            na,
                            actions.len(),
                            regret,
                            average,
                        );
                    }
                    (SlabSlice::I16(regret), SlabSlice::I16(average)) => {
                        update_policy_i16(
                            ctx,
                            &level.strategy[..len],
                            &level.action_values[..len],
                            &level.value[..na],
                            &level.average_path_reach[..na],
                            na,
                            regret,
                            average,
                            &mut state.scales,
                            &mut level.regret[..len],
                            &mut level.average[..len],
                        );
                    }
                    _ => unreachable!("regret and average use the same storage mode"),
                }
            }
            if let Some(t) = start {
                let ns = t.elapsed().as_nanos();
                state.work[2] += ns as u64;
                if ctx.config.kernel_diagnostics {
                    diagnostics.times.section(2, ns);
                }
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn update_policy<S: Scratch>(
    ctx: &Context<'_>,
    strategy: &[f32],
    action_values: &[S],
    value: &[S],
    path_reach: &[S],
    n: usize,
    actions: usize,
    regret: &mut [f32],
    average: &mut [f32],
) {
    // Persistent slab updates/discounts deliberately remain f64; traversal
    // differences are rounded in S before crossing this boundary.
    let t = ctx.iteration as f64;
    let [pos, neg, avg] = ctx.discounts;
    for a in 0..actions {
        for i in 0..n {
            let idx = a * n + i;
            let old = regret[idx] as f64;
            let delta = (action_values[idx] - value[i]).to_f64();
            match ctx.config.algorithm {
                Algorithm::Dcfr => {
                    let r = old + delta;
                    regret[idx] = (r * if r >= 0.0 { pos } else { neg }) as f32;
                    average[idx] = ((average[idx] as f64
                        + path_reach[i].to_f64() * strategy[idx] as f64)
                        * avg) as f32;
                }
                Algorithm::CfrPlus => {
                    regret[idx] = (old + delta).max(0.0) as f32;
                    average[idx] += (t * path_reach[i].to_f64() * strategy[idx] as f64) as f32;
                }
            }
        }
    }
}
// Action-major slices avoid a separate decode pass and a second maximum scan.
// Preserve the old f32 decode and rounding points before the f64 update; using
// q as f64 * scale as f64 here instead would silently change stored strategies.
#[allow(clippy::too_many_arguments)]
fn update_policy_i16<S: Scratch>(
    ctx: &Context<'_>,
    strategy: &[f32],
    action_values: &[S],
    value: &[S],
    path_reach: &[S],
    n: usize,
    regret: &mut [i16],
    average: &mut [i16],
    scales: &mut [f32; 2],
    regret_scratch: &mut [f32],
    average_scratch: &mut [f32],
) {
    let [regret_scale, average_scale] = *scales;
    let [pos, neg, avg] = ctx.discounts;
    let t = ctx.iteration as f64;
    let mut maxima = [0.0f32; 2];
    for (((((regret, average), strategy), action_values), regret_out), average_out) in regret
        .chunks_exact(n)
        .zip(average.chunks_exact(n))
        .zip(strategy.chunks_exact(n))
        .zip(action_values.chunks_exact(n))
        .zip(regret_scratch.chunks_exact_mut(n))
        .zip(average_scratch.chunks_exact_mut(n))
    {
        match ctx.config.algorithm {
            Algorithm::Dcfr => {
                for i in 0..n {
                    let old = (regret[i] as f32 * regret_scale) as f64;
                    let old_average = (average[i] as f32 * average_scale) as f64;
                    let delta = (action_values[i] - value[i]).to_f64();
                    let r = old + delta;
                    let r = (r * if r >= 0.0 { pos } else { neg }) as f32;
                    let a =
                        ((old_average + path_reach[i].to_f64() * strategy[i] as f64) * avg) as f32;
                    regret_out[i] = r;
                    average_out[i] = a;
                    maxima[0] = maxima[0].max(r.abs());
                    maxima[1] = maxima[1].max(a.abs());
                }
            }
            Algorithm::CfrPlus => {
                for i in 0..n {
                    let old = (regret[i] as f32 * regret_scale) as f64;
                    let old_average = average[i] as f32 * average_scale;
                    let delta = (action_values[i] - value[i]).to_f64();
                    let r = (old + delta).max(0.0) as f32;
                    let a = old_average + (t * path_reach[i].to_f64() * strategy[i] as f64) as f32;
                    regret_out[i] = r;
                    average_out[i] = a;
                    maxima[0] = maxima[0].max(r.abs());
                    maxima[1] = maxima[1].max(a.abs());
                }
            }
        }
    }
    encode_i16(regret_scratch, regret, &mut scales[0], maxima[0]);
    encode_i16(average_scratch, average, &mut scales[1], maxima[1]);
}

fn decision_reach<S: Scratch>(
    reach: &[Vec<S>; 2],
    out: &mut [Vec<S>; 2],
    actor: usize,
    policy: Option<&[f32]>,
) {
    for p in 0..2 {
        if let Some(policy) = policy.filter(|_| p == actor) {
            assert_eq!(out[p].len(), reach[p].len());
            assert_eq!(policy.len(), reach[p].len());
            for ((out, &reach), &probability) in out[p].iter_mut().zip(&reach[p]).zip(policy) {
                *out = reach * S::from_f64(probability as f64);
            }
        } else {
            out[p].copy_from_slice(&reach[p]);
        }
    }
}
fn chance_reach<S: Scratch>(
    ctx: &Context<'_>,
    reach: &[Vec<S>; 2],
    out: &mut [Vec<S>; 2],
    card: u8,
    target: usize,
    probability: f64,
) {
    let mask = 1u64 << card;
    for p in 0..2 {
        assert_eq!(out[p].len(), reach[p].len());
        assert_eq!(ctx.masks[p].len(), reach[p].len());
        let probability = S::from_f64(if p == target { 1.0 } else { probability });
        for ((out, &weight), &cards) in out[p].iter_mut().zip(&reach[p]).zip(&ctx.masks[p]) {
            *out = if cards & mask != 0 {
                S::ZERO
            } else {
                weight * probability
            };
        }
    }
}
// Contiguous partitions keep slab borrows disjoint and leave the later
// child/orbit accumulation order untouched. Slab span approximates subtree work;
// a terminal-only child still costs one unit. No extra nested scratch is allocated.
fn weighted_chance_split<S: Scratch>(children: &[State<S>]) -> usize {
    debug_assert!(children.len() > 1);
    let total: u128 = children.iter().map(|c| c.span.max(1) as u128).sum();
    let mut prefix = 0u128;
    let mut best = 1;
    let mut distance = u128::MAX;
    for (i, child) in children[..children.len() - 1].iter().enumerate() {
        prefix += child.span.max(1) as u128;
        let next_distance = (2 * prefix).abs_diff(total);
        if next_distance < distance {
            best = i + 1;
            distance = next_distance;
        }
        // Positive weights make all subsequent prefixes farther from halfway.
        if 2 * prefix >= total {
            break;
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn visit_parallel_chance<S: Scratch, T: Scratch>(
    ctx: &Context<'_>,
    children: &mut [State<S>],
    scratch: &mut [Vec<Level<S>>],
    buffers: Buffers<'_>,
    reach: &[Vec<S>; 2],
    target: usize,
    mode: Mode,
    probability: f64,
    boundary_node: usize,
    totals_cache: &TotalsCache<T>,
) -> RunDiagnostics {
    let mut diagnostics = RunDiagnostics::default();
    if children.len() > ctx.config.parallel_grain {
        let mid = weighted_chance_split(children);
        let span = children[..mid].iter().map(|s| s.span).sum();
        let (a, b) = buffers.split(span);
        let (left, right) = children.split_at_mut(mid);
        let (ls, rs) = scratch.split_at_mut(mid);
        let (left, right) = rayon::join(
            || {
                visit_parallel_chance(
                    ctx,
                    left,
                    ls,
                    a,
                    reach,
                    target,
                    mode,
                    probability,
                    boundary_node,
                    totals_cache,
                )
            },
            || {
                visit_parallel_chance(
                    ctx,
                    right,
                    rs,
                    b,
                    reach,
                    target,
                    mode,
                    probability,
                    boundary_node,
                    totals_cache,
                )
            },
        );
        diagnostics.merge(left);
        diagnostics.merge(right);
    } else {
        let task_start = ctx
            .config
            .kernel_diagnostics
            .then(|| ctx.run_start.elapsed().as_nanos());
        let first_child_node = children.first().map_or(0, |c| c.id);
        let child_count = children.len();
        let mut remaining = buffers;
        for (child, scratch) in children.iter_mut().zip(scratch) {
            let start = (ctx.config.profiling || ctx.config.kernel_diagnostics).then(Instant::now);
            let card = *ctx.tree.nodes[child.id]
                .board
                .last()
                .expect("chance child card");
            chance_reach(ctx, reach, &mut scratch[0].reach, card, target, probability);
            if let Some(t) = start {
                let ns = t.elapsed().as_nanos();
                child.work[3] += ns as u64;
                if ctx.config.kernel_diagnostics {
                    diagnostics.times.chance_ns += ns;
                }
            }
            let (child_buffers, rest) = remaining.split(child.span);
            remaining = rest;
            traverse(
                ctx,
                child,
                scratch,
                child_buffers,
                target,
                mode,
                false,
                &mut totals_cache.fresh(),
                &mut diagnostics,
            );
        }
        if let Some(start_ns) = task_start {
            diagnostics.tasks.push(diagnostics::ChanceTask {
                boundary_node,
                target,
                phase: match mode {
                    Mode::Train => "training",
                    Mode::Average => "average",
                    Mode::BestResponse => "best_response",
                },
                first_child_node,
                child_count,
                worker: rayon::current_thread_index(),
                start_ns,
                end_ns: ctx.run_start.elapsed().as_nanos(),
            });
        }
    }
    diagnostics
}

// Stack-local and reach-scoped: no cache is retained by State/Level, and no
// combo-sized scratch or memory-limit estimate changes. Only target-player
// decisions may share this cache; chance/opponent actions create a fresh one.
#[derive(Default)]
struct TotalsCache<T: Scratch = f64> {
    value: Option<Totals<T>>,
    #[cfg(test)]
    disabled: bool,
}
impl<T: Scratch> TotalsCache<T> {
    fn fresh(&self) -> Self {
        Self {
            value: None,
            #[cfg(test)]
            disabled: self.disabled,
        }
    }
    fn get<S: Scratch>(
        &mut self,
        cards: &[[Card; 2]],
        reach: &[S],
        mut diagnostics: Option<&mut diagnostics::KernelTimes>,
    ) -> &Totals<T> {
        #[cfg(test)]
        if self.disabled {
            self.value = None;
        }
        if self.value.is_none() {
            if let Some(d) = diagnostics.as_deref_mut() {
                d.terminal_totals_scanned_combos += reach.len() as u64;
            }
            self.value = Some(totals_nonzero(
                cards,
                reach,
                diagnostics
                    .as_deref_mut()
                    .map(|d| &mut d.terminal_totals_zero_combos),
            ));
        } else if let Some(d) = diagnostics {
            d.terminal_totals_cache_hits += 1;
        }
        self.value.as_ref().expect("initialized opposing totals")
    }
}

#[derive(Clone, Copy)]
struct Totals<T: Scratch = f64> {
    sum: T,
    cards: [T; 52],
}
fn totals<T: Scratch>(cards: &[[Card; 2]], reach: &[T]) -> Totals<T> {
    let mut out = Totals {
        sum: T::ZERO,
        cards: [T::ZERO; 52],
    };
    for (&cards, &w) in cards.iter().zip(reach) {
        add(&mut out, cards, w);
    }
    out
}
// Zero reach contributions may be omitted from nonnegative, +0-initialized
// sums without changing rounding/order of any nonzero contribution. Values and
// every Train update still execute, including target hands with zero own reach.
fn totals_nonzero<S: Scratch, T: Scratch>(
    cards: &[[Card; 2]],
    reach: &[S],
    mut skipped: Option<&mut u64>,
) -> Totals<T> {
    let mut out = Totals {
        sum: T::ZERO,
        cards: [T::ZERO; 52],
    };
    for (&cards, &weight) in cards.iter().zip(reach) {
        add_nonzero(
            &mut out,
            cards,
            T::from_f64(weight.to_f64()),
            skipped.as_deref_mut(),
        );
    }
    out
}
fn add_nonzero<T: Scratch>(
    t: &mut Totals<T>,
    cards: [Card; 2],
    weight: T,
    skipped: Option<&mut u64>,
) {
    if weight == T::ZERO {
        if let Some(skipped) = skipped {
            *skipped += 1;
        }
    } else {
        add(t, cards, weight);
    }
}
fn add<T: Scratch>(t: &mut Totals<T>, cards: [Card; 2], w: T) {
    t.sum += w;
    t.cards[cards[0].0 as usize] += w;
    t.cards[cards[1].0 as usize] += w;
}
fn compatible<T: Scratch>(cards: [Card; 2], t: &Totals<T>, same: T) -> T {
    (t.sum - t.cards[cards[0].0 as usize] - t.cards[cards[1].0 as usize] + same).max(T::ZERO)
}
fn terminal<S: Scratch, T: Scratch>(
    ctx: &Context<'_>,
    id: usize,
    level: &mut Level<S>,
    target: usize,
    winner: Option<usize>,
    totals_cache: &mut TotalsCache<T>,
    mut diagnostics: Option<&mut diagnostics::KernelTimes>,
) {
    let node = &ctx.tree.nodes[id];
    let reach = &level.reach[1 - target];
    let all = totals_cache.get(
        &ctx.hand_cards[1 - target],
        reach,
        diagnostics.as_deref_mut(),
    );
    let pot = S::from_f64(node.pot as f64 - ctx.tree.config.rake(node.pot));
    let base =
        S::from_f64(node.invested[target] as f64 + ctx.tree.config.starting_pot as f64 / 2.0);
    let board = board_mask(&node.board);
    if let Some(winner) = winner {
        let payoff = if winner == target { pot - base } else { -base };
        for (i, &cards) in ctx.hand_cards[target].iter().enumerate() {
            let same = ctx.same[target][i].map_or(T::ZERO, |j| T::from_f64(reach[j].to_f64()));
            level.value[i] = if ctx.masks[target][i] & board != 0 {
                S::ZERO
            } else {
                payoff * S::from_f64(compatible(cards, all, same).to_f64())
            };
        }
        return;
    }
    let table = &ctx.tables[ctx.terminal_table[id].expect("showdown table").get() as usize - 1];
    // Scratch is reused across boards. Reset only blocked outputs, not the
    // complete dense vector (active entries are overwritten by the pack).
    for &i in &table.blocked[target] {
        level.value[i as usize] = S::ZERO;
    }
    let mut weak = Totals {
        sum: T::ZERO,
        cards: [T::ZERO; 52],
    };
    let mut cursor = 0;
    for group in &table.groups[target] {
        // These boundaries use the original (rank, dense ID) ordering, so every
        // opposing weight enters each prefix in exactly the original order.
        for hand in &table.sorted[1 - target][cursor..group.opp_before as usize] {
            add_nonzero(
                &mut weak,
                hand.cards,
                T::from_f64(reach[hand.dense as usize].to_f64()),
                diagnostics
                    .as_deref_mut()
                    .map(|d| &mut d.showdown_prefix_zero_combos),
            );
        }
        let own = &table.sorted[target][group.own_begin as usize..group.own_end as usize];
        for hand in own {
            let before = compatible(hand.cards, &weak, T::ZERO);
            if std::mem::size_of::<T>() > std::mem::size_of::<S>() {
                level.wide_value[hand.dense as usize] = before.to_f64();
            } else {
                level.value[hand.dense as usize] = S::from_f64(before.to_f64());
            }
        }
        // E[receipt]/pot = 0.5 * (strictly weaker + weaker-or-equal).
        for hand in &table.sorted[1 - target][group.opp_before as usize..group.opp_after as usize] {
            add_nonzero(
                &mut weak,
                hand.cards,
                T::from_f64(reach[hand.dense as usize].to_f64()),
                diagnostics
                    .as_deref_mut()
                    .map(|d| &mut d.showdown_prefix_zero_combos),
            );
        }
        cursor = group.opp_after as usize;
        for hand in own {
            let i = hand.dense as usize;
            let same = if hand.same == u16::MAX {
                T::ZERO
            } else {
                T::from_f64(reach[hand.same as usize].to_f64())
            };
            if std::mem::size_of::<T>() > std::mem::size_of::<S>() {
                let total = compatible(hand.cards, all, same);
                let before = T::from_f64(level.wide_value[i]);
                // Keep prefixes, blocker subtraction and the entire payoff in
                // T; rounding before either subtraction defeats WideTotals.
                let after = compatible(hand.cards, &weak, same);
                let pot = T::from_f64(node.pot as f64 - ctx.tree.config.rake(node.pot));
                let base = T::from_f64(
                    node.invested[target] as f64 + ctx.tree.config.starting_pot as f64 / 2.0,
                );
                level.value[i] = S::from_f64(
                    (pot * T::from_f64(0.5) * (before + after) - base * total).to_f64(),
                );
            } else {
                let total = S::from_f64(compatible(hand.cards, all, same).to_f64());
                let before = level.value[i];
                // The identical opposing combo belongs to the equal prefix only.
                let after = S::from_f64(compatible(hand.cards, &weak, same).to_f64());
                level.value[i] = pot * S::from_f64(0.5) * (before + after) - base * total;
            }
        }
    }
}

/// Runtime-selected traversal precision with one monomorphized core per mode.
pub struct Engine {
    inner: EngineInner,
}
enum EngineInner {
    F64(EngineCore<f64, f64>),
    F32(EngineCore<f32, f32>),
    F32WideTotals(EngineCore<f32, f64>),
}
macro_rules! dispatch_engine {
    ($self:expr, $e:ident => $body:expr) => {
        match &$self.inner {
            EngineInner::F64($e) => $body,
            EngineInner::F32($e) => $body,
            EngineInner::F32WideTotals($e) => $body,
        }
    };
}
macro_rules! dispatch_engine_mut {
    ($self:expr, $e:ident => $body:expr) => {
        match &mut $self.inner {
            EngineInner::F64($e) => $body,
            EngineInner::F32($e) => $body,
            EngineInner::F32WideTotals($e) => $body,
        }
    };
}
impl Engine {
    pub fn new(
        tree: GameTree,
        ranges: [DenseRange; 2],
        config: EngineConfig,
    ) -> Result<Self, String> {
        let inner = match config.scratch_precision {
            ScratchPrecision::F64 => EngineInner::F64(EngineCore::new(tree, ranges, config)?),
            ScratchPrecision::F32 => EngineInner::F32(EngineCore::new(tree, ranges, config)?),
            ScratchPrecision::F32WideTotals => {
                EngineInner::F32WideTotals(EngineCore::new(tree, ranges, config)?)
            }
        };
        Ok(Self { inner })
    }
    pub fn iterate(&mut self) {
        dispatch_engine_mut!(self, e => e.iterate())
    }
    pub fn metrics(&mut self) -> Metrics {
        dispatch_engine_mut!(self, e => e.metrics())
    }
    /// Exact conditional root EV and full-runout showdown equity.
    pub fn root_hand_values(&mut self) -> [Vec<HandValue>; 2] {
        dispatch_engine_mut!(self, e => e.root_hand_values())
    }
    pub fn metrics_at_node(
        &mut self,
        node: usize,
        reach: [Vec<f64>; 2],
    ) -> Result<Metrics, String> {
        dispatch_engine_mut!(self, e => e.metrics_at_node(node, reach))
    }
    pub fn strategy(&self, node: usize) -> Result<Vec<f32>, String> {
        dispatch_engine!(self, e => e.strategy(node))
    }
    /// Install a complete solving loop once, avoiding Rayon external-injector
    /// queue allocations between calls to iterate/metrics. For threads=1 calls
    /// are already allocation-free and execute directly on the calling thread.
    pub fn with_pool<R: Send>(&mut self, f: impl FnOnce(&mut Self) -> R + Send) -> R {
        let pool = dispatch_engine!(self, e => (e.config.threads > 1).then(|| Arc::clone(&e.pool)));
        match pool {
            Some(pool) => pool.install(|| f(self)),
            None => f(self),
        }
    }
    pub fn iterations(&self) -> usize {
        dispatch_engine!(self, e => e.iterations())
    }
    pub fn memory_bytes(&self) -> usize {
        dispatch_engine!(self, e => e.memory_bytes())
    }
    pub fn tree(&self) -> &GameTree {
        dispatch_engine!(self, e => e.tree())
    }
    pub fn ranges(&self) -> &[DenseRange; 2] {
        dispatch_engine!(self, e => e.ranges())
    }
    pub fn terminal_seconds(&self) -> f64 {
        dispatch_engine!(self, e => e.terminal_seconds())
    }
    pub fn profile(&self) -> WorkProfile {
        dispatch_engine!(self, e => e.profile())
    }
    pub fn kernel_diagnostics(&self) -> Option<&KernelDiagnostics> {
        dispatch_engine!(self, e => e.kernel_diagnostics())
    }
    pub fn persistent_storage_bytes(&self) -> usize {
        dispatch_engine!(self, e => e.persistent_storage_bytes())
    }
    pub fn isomorphism_used(&self) -> bool {
        dispatch_engine!(self, e => e.isomorphism_used())
    }
    pub fn merged_chance_children(&self) -> usize {
        dispatch_engine!(self, e => e.merged_chance_children())
    }
    pub fn strategy_storage_offset(&self, node: usize) -> Option<usize> {
        dispatch_engine!(self, e => e.strategy_storage_offset(node))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    type Engine = EngineCore<f64, f64>;
    use cards::Combo;
    use ranges::Range;
    use tree::{Action, Node, SizeRule, TreeConfig};
    fn board(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|c| c.parse::<Card>().unwrap().0)
            .collect()
    }
    fn range(hands: &[(&str, f64)], b: &[u8]) -> DenseRange {
        Range::from_combos(
            hands
                .iter()
                .map(|(c, w)| (c.parse::<Combo>().unwrap(), *w))
                .collect(),
        )
        .unwrap()
        .blocked(&b.iter().map(|&c| Card(c)).collect::<Vec<_>>())
        .unwrap()
    }
    fn node(b: &[u8], pot: i64, invested: [i64; 2], kind: NodeKind) -> Node {
        Node {
            board: b.to_vec(),
            pot,
            invested,
            stacks: [0, 0],
            kind,
        }
    }
    fn toy() -> (GameTree, [DenseRange; 2]) {
        let b = board("2h 7d 9s Jc Qc");
        let p = 1000;
        let bet = 500;
        let config = TreeConfig {
            starting_pot: p,
            effective_stack: bet,
            rake_rate: 0.0,
            bet_size: SizeRule::Fixed(0.5),
            ..TreeConfig::default()
        };
        let nodes = vec![
            node(
                &b,
                p,
                [0, 0],
                NodeKind::Decision {
                    player: 0,
                    actions: vec![Action::Check, Action::Bet(bet)],
                    children: vec![1, 2],
                },
            ),
            node(&b, p, [0, 0], NodeKind::Terminal { winner: None }),
            node(
                &b,
                p + bet,
                [bet, 0],
                NodeKind::Decision {
                    player: 1,
                    actions: vec![Action::Fold, Action::Call],
                    children: vec![3, 4],
                },
            ),
            node(
                &b,
                p + bet,
                [bet, 0],
                NodeKind::Terminal { winner: Some(0) },
            ),
            node(
                &b,
                p + 2 * bet,
                [bet, bet],
                NodeKind::Terminal { winner: None },
            ),
        ];
        (
            GameTree::from_nodes(config, nodes, 0),
            [
                range(&[("AhAd", 1.0), ("3c4c", 1.0)], &b),
                range(&[("KhKd", 1.0)], &b),
            ],
        )
    }
    fn public_slab_bits(engine: &super::Engine) -> [Vec<u32>; 2] {
        match &engine.inner {
            EngineInner::F64(e) => [e.regret.bits(), e.average.bits()],
            EngineInner::F32(e) => [e.regret.bits(), e.average.bits()],
            EngineInner::F32WideTotals(e) => [e.regret.bits(), e.average.bits()],
        }
    }
    fn metric_bits(metrics: &Metrics) -> Vec<u64> {
        metrics
            .average_ev
            .iter()
            .chain(&metrics.best_response_ev)
            .chain([&metrics.exploitability_chips, &metrics.exploitability_pct])
            .map(|v| v.to_bits())
            .collect()
    }
    fn assert_f64_evaluation_matches_policy<S: Scratch, T: Scratch>(
        source: &mut EngineCore<S, T>,
        reference: &mut EngineCore,
    ) {
        fn copy_slab(source: &Slab) -> Slab {
            match source {
                Slab::F32(v) => Slab::F32(v.clone()),
                Slab::I16(v) => Slab::I16(v.clone()),
            }
        }
        reference.regret = copy_slab(&source.regret);
        reference.average = copy_slab(&source.average);
        reference.state.copy_scales(&source.state);
        reference.iterations = source.iterations;
        let policy = [source.regret.bits(), source.average.bits()];
        assert_eq!(
            metric_bits(&source.metrics()),
            metric_bits(&reference.metrics())
        );
        let mut nodes = vec![source.tree.root];
        if let Some(id) = source.tree.nodes.iter().enumerate().find_map(|(id, node)| {
            (id != source.tree.root && matches!(node.kind, NodeKind::Chance { .. })).then_some(id)
        }) {
            nodes.push(id);
        }
        if let Some(id) = source
            .aliases
            .iter()
            .enumerate()
            .find_map(|(id, &alias)| (alias as usize != id).then_some(id))
        {
            nodes.push(id);
        }
        if let Some(id) = source.tree.nodes.iter().enumerate().find_map(|(id, node)| {
            (node.board.len() == 5 && matches!(node.kind, NodeKind::Decision { .. })).then_some(id)
        }) {
            nodes.push(id);
        }
        for node in nodes {
            assert_eq!(
                metric_bits(
                    &source
                        .metrics_at_node(node, source.initial_reach.clone())
                        .unwrap()
                ),
                metric_bits(
                    &reference
                        .metrics_at_node(node, reference.initial_reach.clone())
                        .unwrap()
                ),
                "conditioned node {node}"
            );
        }
        assert_eq!(policy, [source.regret.bits(), source.average.bits()]);
    }
    #[test]
    fn scratch_precision_metrics_use_f64_for_identical_policy_slabs() {
        let b = board("2c 7c 9c Jc");
        let tree = GameTree::build(
            b.clone(),
            TreeConfig {
                starting_pot: 1001,
                effective_stack: 500,
                allow_raises: false,
                rake_rate: 0.05,
                ..Default::default()
            },
        )
        .unwrap();
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 0.3), ("KcKd", 0.0)], &b),
            range(&[("KcKd", 0.7), ("3c4d", 0.6), ("AcAd", 0.2)], &b),
        ];
        for storage_mode in [StorageMode::F32, StorageMode::I16] {
            for threads in [1, 2] {
                for suit_isomorphism in [false, true] {
                    for precision in [ScratchPrecision::F32, ScratchPrecision::F32WideTotals] {
                        let config = EngineConfig {
                            threads,
                            storage_mode,
                            suit_isomorphism,
                            scratch_precision: precision,
                            ..Default::default()
                        };
                        let mut source =
                            super::Engine::new(tree.clone(), rr.clone(), config.clone()).unwrap();
                        let mut reference = Engine::new(
                            tree.clone(),
                            rr.clone(),
                            EngineConfig {
                                scratch_precision: ScratchPrecision::F64,
                                ..config
                            },
                        )
                        .unwrap();
                        if suit_isomorphism {
                            assert!(source.merged_chance_children() > 0);
                        }
                        for count in [3, 13] {
                            for _ in 0..count {
                                source.iterate();
                            }
                            match &mut source.inner {
                                EngineInner::F32(e) => {
                                    assert_f64_evaluation_matches_policy(e, &mut reference)
                                }
                                EngineInner::F32WideTotals(e) => {
                                    assert_f64_evaluation_matches_policy(e, &mut reference)
                                }
                                EngineInner::F64(_) => unreachable!(),
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn scratch_precision_memory_accounts_for_evaluation_and_wide_values() {
        let (tree, rr) = toy();
        let mut estimates = Vec::new();
        for precision in [
            ScratchPrecision::F64,
            ScratchPrecision::F32,
            ScratchPrecision::F32WideTotals,
        ] {
            let config = EngineConfig {
                scratch_precision: precision,
                ..Default::default()
            };
            let estimate = estimate_memory_bytes_with_config(&tree, &rr, &config).unwrap();
            let e = super::Engine::new(tree.clone(), rr.clone(), config.clone()).unwrap();
            assert_eq!(estimate, e.memory_bytes());
            match &e.inner {
                EngineInner::F64(e) => assert!(e.evaluation.is_none()),
                EngineInner::F32(e) => {
                    let evaluation = e.evaluation.as_ref().unwrap();
                    assert_eq!(evaluation.scratch.len(), e.scratch.len());
                    assert!(e.scratch.iter().all(|level| level.wide_value.is_empty()));
                }
                EngineInner::F32WideTotals(e) => {
                    assert!(e.evaluation.is_some());
                    assert!(e
                        .scratch
                        .iter()
                        .all(|level| level.wide_value.len() == rr[0].len().max(rr[1].len())));
                }
            }
            assert!(super::Engine::new(
                tree.clone(),
                rr.clone(),
                EngineConfig {
                    memory_limit_bytes: estimate - 1,
                    ..config
                }
            )
            .is_err());
            estimates.push(estimate);
        }
        assert!(estimates[1] > estimates[0]);
        assert!(estimates[2] > estimates[1]);
    }
    #[test]
    fn chance_identity_zip_and_permuted_add_preserve_scalar_order() {
        fn check<S: Scratch>() {
            let permutations: Vec<_> = [[0, 1, 2, 3], [2, 0, 3, 1]]
                .into_iter()
                .map(|map| ComboPermutation {
                    suit: SuitPermutation([0, 1, 2, 3]),
                    indices: [map.to_vec(), map.to_vec()],
                })
                .collect();
            let children = [
                [16777216.0, -0.0, -16777216.0, 0.25],
                [1.0, 0.125, -1.0, -0.5],
                [-16777216.0, 0.0, 16777216.0, 1.0],
            ]
            .map(|values| values.map(S::from_f64));
            let mut actual = [S::ZERO; 4];
            let mut expected = actual;
            for (child, orbit) in children.iter().zip([[0, 1], [1, 0], [0, 1]]) {
                for permutation in orbit {
                    chance_add(&mut actual, child, permutation, &permutations, 0);
                    for (i, &j) in permutations[permutation].indices[0].iter().enumerate() {
                        expected[i] += child[j];
                    }
                }
            }
            assert_eq!(
                actual.map(|v| v.to_f64().to_bits()),
                expected.map(|v| v.to_f64().to_bits())
            );
        }
        check::<f64>();
        check::<f32>();
    }
    #[test]
    fn wide_totals_chance_sums_children_in_f64_before_final_cast() {
        let b = board("2c 7d 9h Js");
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 0.3), ("KcKd", 0.2)], &b),
            range(&[("KcKd", 0.7), ("3c4d", 0.6), ("AcAd", 0.2)], &b),
        ];
        let tree = GameTree::build(
            b,
            TreeConfig {
                starting_pot: 6001,
                effective_stack: 0,
                rake_rate: 0.05,
                ..Default::default()
            },
        )
        .unwrap();
        let mut parallel = EngineCore::<f32, f64>::new(
            tree.clone(),
            rr.clone(),
            EngineConfig {
                scratch_precision: ScratchPrecision::F32WideTotals,
                threads: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let mut serial = EngineCore::<f32, f64>::new(
            tree,
            rr,
            EngineConfig {
                scratch_precision: ScratchPrecision::F32WideTotals,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            parallel.tree.nodes[parallel.tree.root].kind,
            NodeKind::Chance { .. }
        ));
        let mut distinguishes_narrow_sum = false;
        for target in 0..2 {
            parallel.run(target, Mode::Average);
            serial.run(target, Mode::Average);
            let n = parallel.ranges[target].len();
            let mut wide = vec![0.0_f64; n];
            let mut narrow = vec![0.0_f32; n];
            for (scratch, orbit) in parallel
                .state
                .parallel_scratch
                .iter()
                .zip(&parallel.state.orbits)
            {
                for &permutation in orbit {
                    for (i, &j) in parallel.permutations[permutation].indices[target]
                        .iter()
                        .enumerate()
                    {
                        wide[i] += scratch[0].value[j] as f64;
                        narrow[i] += scratch[0].value[j];
                    }
                }
            }
            let expected: Vec<_> = wide.iter().map(|&v| (v as f32).to_bits()).collect();
            assert_eq!(
                parallel.scratch[0].value[..n]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                serial.scratch[0].value[..n]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected
            );
            distinguishes_narrow_sum |=
                narrow.iter().map(|v| v.to_bits()).collect::<Vec<_>>() != expected;
        }
        assert!(
            distinguishes_narrow_sum,
            "fixture must detect per-child f32 rounding"
        );
    }
    #[test]
    fn wide_totals_showdown_rounds_only_the_final_f64_payoff() {
        let b = board("2h 7d 9s Jc Qc");
        let hands: Vec<_> = (0..1326)
            .map(|id| Combo::from_id(id).unwrap())
            .filter(|c| c.mask() & board_mask(&b) == 0)
            .step_by(5)
            .collect();
        let rr = std::array::from_fn(|p| {
            Range::from_combos(
                hands
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| (c, ((i * 17 + p * 7) % 29 + 1) as f64 / 29.0))
                    .collect(),
            )
            .unwrap()
            .blocked(&b.iter().map(|&c| Card(c)).collect::<Vec<_>>())
            .unwrap()
        });
        let tree = GameTree::from_nodes(
            TreeConfig {
                starting_pot: 6001,
                effective_stack: 0,
                rake_rate: 0.05,
                ..Default::default()
            },
            vec![node(
                &b,
                8003,
                [1001, 1001],
                NodeKind::Terminal { winner: None },
            )],
            0,
        );
        let e = Engine::new(tree, rr, EngineConfig::default()).unwrap();
        let ctx = Context {
            tree: &e.tree,
            ranges: &e.ranges,
            masks: &e.masks,
            hand_cards: &e.hand_cards,
            same: &e.same,
            tables: &e.tables,
            terminal_table: &e.terminal_table,
            config: &e.config,
            permutations: &e.permutations,
            average_reach_scale: &e.average_reach_scale,
            initial_reach: &e.initial_reach,
            discounts: [1.0; 3],
            iteration: 0,
            run_start: Instant::now(),
        };
        let n = [e.ranges[0].len(), e.ranges[1].len()];
        let mut level = levels::<f32>(1, 0, n).remove(0);
        level.wide_value = vec![0.0; n[0].max(n[1])];
        for p in 0..2 {
            level.reach[p] = e.initial_reach[p].iter().map(|&w| w as f32).collect();
        }
        let mut detects_early_rounding = false;
        for (target, &count) in n.iter().enumerate() {
            let reach: Vec<_> = level.reach[1 - target].iter().map(|&w| w as f64).collect();
            let expected: Vec<_> = legacy_terminal_values(&e, 0, target, &reach)
                .iter()
                .map(|&v| (v as f32).to_bits())
                .collect();
            terminal(
                &ctx,
                0,
                &mut level,
                target,
                None,
                &mut TotalsCache::<f64>::default(),
                None,
            );
            assert_eq!(
                level.value[..count]
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected
            );
            terminal(
                &ctx,
                0,
                &mut level,
                target,
                None,
                &mut TotalsCache::<f32>::default(),
                None,
            );
            detects_early_rounding |= level.value[..count]
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
                != expected;
        }
        assert!(
            detects_early_rounding,
            "fixture must distinguish narrow showdown arithmetic"
        );
    }
    #[test]
    fn scratch_precision_public_dispatch_accuracy_and_thread_bits() {
        let b = board("2c 7d 9h Js");
        let tree = GameTree::build(
            b.clone(),
            TreeConfig {
                starting_pot: 1000,
                effective_stack: 500,
                allow_raises: false,
                rake_rate: 0.05,
                ..Default::default()
            },
        )
        .unwrap();
        // Asymmetric, overlapping ranges include zero reach and future blockers.
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 0.3), ("KcKd", 0.0)], &b),
            range(&[("KcKd", 0.7), ("3c4d", 0.6), ("AcAd", 0.2)], &b),
        ];
        for algorithm in [Algorithm::Dcfr, Algorithm::CfrPlus] {
            for storage_mode in [StorageMode::F32, StorageMode::I16] {
                let config = EngineConfig {
                    algorithm,
                    storage_mode,
                    ..Default::default()
                };
                let mut reference =
                    super::Engine::new(tree.clone(), rr.clone(), config.clone()).unwrap();
                let mut legacy = Engine::new(tree.clone(), rr.clone(), config.clone()).unwrap();
                for _ in 0..64 {
                    reference.iterate();
                    legacy.iterate();
                }
                let expected = reference.metrics();
                // Default runtime dispatch remains exactly the f64 core, including slabs.
                assert_eq!(
                    public_slab_bits(&reference),
                    [legacy.regret.bits(), legacy.average.bits()]
                );
                assert_eq!(metric_bits(&expected), metric_bits(&legacy.metrics()));
                for precision in [ScratchPrecision::F32, ScratchPrecision::F32WideTotals] {
                    let mut serial = super::Engine::new(
                        tree.clone(),
                        rr.clone(),
                        EngineConfig {
                            scratch_precision: precision,
                            ..config.clone()
                        },
                    )
                    .unwrap();
                    let mut parallel = super::Engine::new(
                        tree.clone(),
                        rr.clone(),
                        EngineConfig {
                            scratch_precision: precision,
                            threads: 4,
                            parallel_grain: 7,
                            ..config.clone()
                        },
                    )
                    .unwrap();
                    // f32 training retains independent f64 evaluation scratch;
                    // its complete payload is not smaller than the f64 engine.
                    assert!(serial.memory_bytes() > reference.memory_bytes());
                    assert_eq!(
                        serial.persistent_storage_bytes(),
                        reference.persistent_storage_bytes()
                    );
                    for _ in 0..64 {
                        serial.iterate();
                        parallel.iterate();
                    }
                    let actual = serial.metrics();
                    assert_eq!(public_slab_bits(&serial), public_slab_bits(&parallel));
                    assert_eq!(metric_bits(&actual), metric_bits(&parallel.metrics()));
                    // 0.05 chips is 5e-5 of the 1000-chip pot; allow the same
                    // bounded trajectory drift in EV and BR, not bit equivalence.
                    for p in 0..2 {
                        assert!(
                            (actual.average_ev[p] - expected.average_ev[p]).abs() < 0.05,
                            "{algorithm:?}/{storage_mode:?}/{precision:?}: {:?} != {:?}",
                            actual,
                            expected
                        );
                        assert!(
                            (actual.best_response_ev[p] - expected.best_response_ev[p]).abs()
                                < 0.05
                        );
                    }
                    assert!(
                        (actual.exploitability_pct - expected.exploitability_pct).abs() < 0.005
                    );
                    for (id, node) in tree.nodes.iter().enumerate() {
                        if matches!(node.kind, NodeKind::Decision { .. }) {
                            assert_eq!(
                                serial.strategy(id).unwrap(),
                                parallel.strategy(id).unwrap()
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn scratch_precision_f32_toy_matches_theory() {
        for algorithm in [Algorithm::Dcfr, Algorithm::CfrPlus] {
            for storage_mode in [StorageMode::F32, StorageMode::I16] {
                for scratch_precision in [ScratchPrecision::F32, ScratchPrecision::F32WideTotals] {
                    let (tree, ranges) = toy();
                    let nuts = ranges[0]
                        .combos
                        .iter()
                        .position(|c| c.combo == "AhAd".parse::<Combo>().unwrap())
                        .unwrap();
                    let mut engine = super::Engine::new(
                        tree,
                        ranges,
                        EngineConfig {
                            algorithm,
                            storage_mode,
                            scratch_precision,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    for _ in 0..20000 {
                        engine.iterate();
                    }
                    let bettor = engine.strategy(0).unwrap();
                    let call = engine.strategy(2).unwrap()[1] as f64;
                    let bluff_fraction =
                        bettor[2 + 1 - nuts] as f64 / (bettor[2] + bettor[3]) as f64;
                    assert!((bluff_fraction - 0.25).abs() < 0.01);
                    assert!((call - 2.0 / 3.0).abs() < 0.01);
                    assert!((bettor[2 + nuts] - 1.0).abs() < 0.01);
                    assert!(engine.metrics().exploitability_pct < 0.1);
                }
            }
        }
    }
    #[test]
    #[ignore = "opt-in subsecond-to-few-second isolated normalization throughput"]
    #[cfg(target_arch = "aarch64")]
    fn normalization_microbench() {
        use std::hint::black_box;
        fn run<const A: usize>(n: usize, masks: &[u64], board: u64) {
            let input: Vec<i16> = (0..n * A)
                .map(|i| ((i * 1777) % 65536) as u16 as i16)
                .collect();
            let mut output = vec![0.0; n * A];
            type Kernel = fn(&[i16], &mut [f32], usize, &[u64], u64);
            for (name, kernel) in [
                (
                    "scalar_reference",
                    scalar_normalize_fixed::<i16, A> as Kernel,
                ),
                ("portable_zip", normalize_fixed_portable::<i16, A> as Kernel),
                ("neon", neon_normalize_fixed::<i16, A> as Kernel),
            ] {
                let loops = 10_000;
                let start = Instant::now();
                for _ in 0..loops {
                    black_box(kernel)(
                        black_box(&input),
                        black_box(&mut output),
                        n,
                        black_box(masks),
                        black_box(board),
                    );
                }
                let ns = start.elapsed().as_nanos();
                let checksum: f64 = black_box(&output).iter().map(|&v| v as f64).sum();
                println!("{{\"kernel\":\"{name}\",\"storage\":\"i16\",\"actions\":{A},\"combos\":{n},\"loops\":{loops},\"elapsed_ns\":{ns},\"ns_per_actor_combo\":{},\"ns_per_action_combo\":{},\"checksum\":{checksum}}}",ns as f64/(n*loops) as f64,ns as f64/(n*A*loops) as f64);
            }
        }
        let root = board_mask(&board("Qs Jh 2h"));
        let masks: Vec<_> = (0..1326)
            .map(|id| Combo::from_id(id).unwrap().mask())
            .filter(|&m| m & root == 0)
            .collect();
        assert_eq!(masks.len(), 1176);
        // A future board blocks additional hands; this is synthetic policy data,
        // not a full-range solve or a convergence-speed measurement.
        let future =
            root | (1 << "3c".parse::<Card>().unwrap().0) | (1 << "4d".parse::<Card>().unwrap().0);
        for n in [1176, 690] {
            run::<2>(n, &masks[..n], future);
            run::<3>(n, &masks[..n], future);
        }
    }
    #[test]
    fn zipped_fixed_normalization_matches_scalar_bits_with_blockers_and_tails() {
        for n in [1, 2, 3, 7, 8, 9, 15, 16, 17, 69, 690, 1176, 1326] {
            let masks: Vec<_> = (0..n).map(|i| 1u64 << (i % 52)).collect();
            let board = (1 << 3) | (1 << 7) | (1 << 11);
            for actions in [2, 3] {
                let len = actions * n;
                let ints: Vec<_> = (0..len)
                    .map(|i| [-32768i16, -7, 0, 1, 101, 32767][i % 6])
                    .collect();
                let floats: Vec<_> = (0..len)
                    .map(|i| {
                        [
                            -1.0f32,
                            -0.0,
                            0.0,
                            f32::from_bits(1),
                            1e-37,
                            1.0,
                            32767.0,
                            f32::MAX,
                            f32::MIN_POSITIVE,
                        ][i % 9]
                    })
                    .collect();
                for zero in [false, true] {
                    let ints = if zero { vec![0; len] } else { ints.clone() };
                    let floats = if zero { vec![0.0; len] } else { floats.clone() };
                    let mut actual = vec![f32::NAN; len];
                    let mut expected = actual.clone();
                    macro_rules! check {
                        ($input:expr, $a:literal) => {
                            normalize_fixed::<_, $a>(&$input, &mut actual, n, &masks, board);
                            scalar_normalize_fixed::<_, $a>(
                                &$input,
                                &mut expected,
                                n,
                                &masks,
                                board,
                            );
                            assert_eq!(
                                actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                            );
                            normalize_fixed_portable::<_, $a>(
                                &$input,
                                &mut actual,
                                n,
                                &masks,
                                board,
                            );
                            assert_eq!(
                                actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                            );
                            assert!(actual.iter().all(|v| v.is_finite()));
                        };
                    }
                    if actions == 2 {
                        check!(ints, 2);
                        check!(floats, 2);
                    } else {
                        check!(ints, 3);
                        check!(floats, 3);
                    }
                }
            }
        }
    }
    #[test]
    fn zipped_reach_matches_scalar_bits_with_zero_subnormal_and_tails() {
        let (tree, ranges) = toy();
        let e = Engine::new(tree, ranges, EngineConfig::default()).unwrap();
        let ctx = Context {
            tree: &e.tree,
            ranges: &e.ranges,
            masks: &e.masks,
            hand_cards: &e.hand_cards,
            same: &e.same,
            tables: &e.tables,
            terminal_table: &e.terminal_table,
            config: &e.config,
            permutations: &e.permutations,
            average_reach_scale: &e.average_reach_scale,
            initial_reach: &e.initial_reach,
            discounts: [1.0; 3],
            iteration: 0,
            run_start: Instant::now(),
        };
        // Different player lengths exercise independent SIMD remainders.
        for n in [1, 2, 3, 7, 8, 9, 15, 16, 17, 690, 1176] {
            let reach: [Vec<f64>; 2] = std::array::from_fn(|p| {
                (0..n + p)
                    .map(|i| [0.0, -0.0, f64::from_bits(1), 1e-100, 0.7][i % 5])
                    .collect()
            });
            let mut actual: [Vec<f64>; 2] = std::array::from_fn(|p| vec![0.0; reach[p].len()]);
            for actor in 0..2 {
                let policy: Vec<_> = (0..reach[actor].len())
                    .map(|i| [0.0, 0.5, 1.0][i % 3])
                    .collect();
                for strategy in [None, Some(policy.as_slice())] {
                    decision_reach(&reach, &mut actual, actor, strategy);
                    for p in 0..2 {
                        for i in 0..reach[p].len() {
                            let expected = reach[p][i]
                                * if p == actor {
                                    strategy.map_or(1.0, |s| s[i] as f64)
                                } else {
                                    1.0
                                };
                            assert_eq!(actual[p][i].to_bits(), expected.to_bits());
                        }
                    }
                }
            }
            let masks: [Vec<u64>; 2] =
                std::array::from_fn(|p| (0..reach[p].len()).map(|i| 1u64 << (i % 52)).collect());
            let ctx = Context {
                masks: &masks,
                ..ctx
            };
            for target in 0..2 {
                chance_reach(&ctx, &reach, &mut actual, 3, target, 1.0 / 44.0);
                for p in 0..2 {
                    for i in 0..reach[p].len() {
                        let expected = if masks[p][i] & (1 << 3) != 0 {
                            0.0
                        } else {
                            reach[p][i] * if p == target { 1.0 } else { 1.0 / 44.0 }
                        };
                        assert_eq!(actual[p][i].to_bits(), expected.to_bits());
                    }
                }
            }
        }
    }
    #[test]
    fn kernel_diagnostics_exact_workload_and_metrics_isolation() {
        for storage_mode in [StorageMode::F32, StorageMode::I16] {
            let (tree, ranges) = toy();
            let mut baseline = Engine::new(
                tree.clone(),
                ranges.clone(),
                EngineConfig {
                    storage_mode,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut measured = Engine::new(
                tree,
                ranges,
                EngineConfig {
                    storage_mode,
                    kernel_diagnostics: true,
                    ..Default::default()
                },
            )
            .unwrap();
            let w = measured
                .kernel_diagnostics()
                .unwrap()
                .per_iteration_workload;
            assert_eq!(w.decision_visits, 4);
            assert_eq!(w.decision_dense_combos, 6);
            assert_eq!(w.decision_active_combos, 6);
            assert_eq!(w.action_dense_combos, 12);
            assert_eq!(w.normalization_dense_combos, 12);
            assert_eq!(w.update_dense_combos, 6);
            assert_eq!(w.fold_visits, 2);
            assert_eq!(w.fold_dense_combos, 3);
            assert_eq!(w.showdown_visits, 4);
            assert_eq!(w.showdown_dense_combos, 6);
            assert_eq!(w.terminal_totals_dense_combos, 9);
            assert_eq!(w.chance_visits, 0);
            assert_eq!(
                measured
                    .kernel_diagnostics()
                    .unwrap()
                    .per_best_response_evaluation_workload
                    .normalization_dense_combos,
                6
            );
            for _ in 0..8 {
                baseline.iterate();
                measured.iterate();
            }
            assert_eq!(baseline.regret.bits(), measured.regret.bits());
            assert_eq!(baseline.average.bits(), measured.average.bits());
            let training_ns = measured.kernel_diagnostics().unwrap().training.wall_ns;
            let bm = baseline.metrics();
            let mm = measured.metrics();
            assert_eq!(
                bm.exploitability_pct.to_bits(),
                mm.exploitability_pct.to_bits()
            );
            let d = measured.kernel_diagnostics().unwrap();
            assert_eq!(d.training.wall_ns, training_ns);
            assert_eq!(d.training.traversals, 16);
            assert_eq!(d.average_evaluation.traversals, 2);
            assert_eq!(d.best_response_evaluation.traversals, 2);
            assert_eq!(d.iterations.len(), 8);
            assert!(d.training.decision_ns > 0 && d.training.showdown_ns > 0);
            assert!(d.training.decision_transport_ns > 0);
            assert!(baseline.kernel_diagnostics().is_none());
        }
    }
    #[test]
    fn kernel_diagnostics_parallel_tasks_and_blocked_workload() {
        let b = board("2c 7d 9h Js");
        let tree = GameTree::build(
            b.clone(),
            TreeConfig {
                starting_pot: 1000,
                effective_stack: 500,
                allow_raises: false,
                rake_rate: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 0.7)], &b),
            range(&[("KcKd", 1.0)], &b),
        ];
        let mut measured = Engine::new(
            tree,
            rr,
            EngineConfig {
                threads: 2,
                kernel_diagnostics: true,
                ..Default::default()
            },
        )
        .unwrap();
        measured.iterate();
        let d = measured.kernel_diagnostics().unwrap();
        assert!(
            d.per_iteration_workload.decision_active_combos
                < d.per_iteration_workload.decision_dense_combos
        );
        assert!(d.per_iteration_workload.chance_representative_children > 0);
        assert!(!d.latest_training_tasks.is_empty());
        for t in &d.latest_training_tasks {
            assert!(t.end_ns >= t.start_ns);
            assert_eq!(t.child_count, 1);
            assert!(t.worker.is_some_and(|w| w < 2));
        }
        for batch in &d.iterations[0].chance_batches {
            assert!(batch.tasks > 1);
            assert!((1..=2).contains(&batch.workers_used));
            assert!(batch.task_min_ns <= batch.task_max_ns);
            assert!(batch.final_task_tail_ns <= batch.wall_ns);
        }
    }
    #[test]
    fn polarized_toy_dcfr_and_cfr_plus_match_theory() {
        for algorithm in [Algorithm::Dcfr, Algorithm::CfrPlus] {
            let (tree, ranges) = toy();
            let mut engine = Engine::new(
                tree,
                ranges,
                EngineConfig {
                    algorithm,
                    ..Default::default()
                },
            )
            .unwrap();
            for _ in 0..20000 {
                engine.iterate();
            }
            let bettor = engine.strategy(0).unwrap();
            let call = engine.strategy(2).unwrap()[1] as f64;
            let nuts = engine.ranges[0]
                .combos
                .iter()
                .position(|c| c.combo == "AhAd".parse::<Combo>().unwrap())
                .unwrap();
            let bluff = 1 - nuts;
            let bluff_fraction = bettor[2 + bluff] as f64 / (bettor[2] + bettor[3]) as f64;
            assert!(
                (bluff_fraction - 0.25).abs() < 0.01,
                "{algorithm:?} bluff fraction {bluff_fraction}"
            );
            assert!((call - 2.0 / 3.0).abs() < 0.01, "{algorithm:?} call {call}");
            assert!((bettor[2 + nuts] - 1.0).abs() < 0.01);
            assert!(engine.metrics().exploitability_pct < 0.1);
        }
    }
    fn random(seed: &mut u64) -> f64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }
    #[test]
    fn linear_terminals_match_quadratic_random_blockers_ties_and_rake() {
        for case in 0..20 {
            let b = if case % 4 == 0 {
                board("Ac Kc Qc Jc Tc")
            } else {
                board("2c 7d 9h Js Qd")
            };
            let bm = board_mask(&b);
            let mut seed = case + 1;
            let hands: Vec<_> = (0..1326)
                .map(|i| Combo::from_id(i).unwrap())
                .filter(|c| c.mask() & bm == 0)
                .collect();
            let rr: [DenseRange; 2] = std::array::from_fn(|p| {
                Range::from_combos(
                    hands
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % 3 != p)
                        .map(|(_, c)| {
                            (
                                *c,
                                if random(&mut seed) < 0.2 {
                                    0.0
                                } else {
                                    random(&mut seed)
                                },
                            )
                        })
                        .collect(),
                )
                .unwrap()
                .blocked(&b.iter().map(|&c| Card(c)).collect::<Vec<_>>())
                .unwrap()
            });
            let pot = if case % 2 == 0 { 120000 } else { 10000 };
            let config = TreeConfig {
                starting_pot: 6000,
                rake_rate: 0.05,
                rake_cap: 3000,
                ..Default::default()
            };
            let mut naive_ranks = vec![0; 1326];
            for &h in &hands {
                naive_ranks[h.id()] = evaluate7(&[
                    Card(b[0]),
                    Card(b[1]),
                    Card(b[2]),
                    Card(b[3]),
                    Card(b[4]),
                    h.cards[0],
                    h.cards[1],
                ]);
            }
            for winner in [None, Some(0), Some(1)] {
                let t = GameTree::from_nodes(
                    config.clone(),
                    vec![node(&b, pot, [1700, 2300], NodeKind::Terminal { winner })],
                    0,
                );
                let mut e = Engine::new(t, rr.clone(), EngineConfig::default()).unwrap();
                for target in 0..2 {
                    e.run(target, Mode::Average);
                    let p = pot as f64 - config.rake(pot);
                    let base = 1700.0 + 6000.0 / 2.0 + if target == 1 { 600.0 } else { 0.0 };
                    for (i, c) in e.ranges[target].combos.iter().enumerate() {
                        let mut naive = 0.0;
                        for (j, d) in e.ranges[1 - target].combos.iter().enumerate() {
                            if c.combo.mask() & d.combo.mask() != 0 {
                                continue;
                            }
                            let payout = if let Some(w) = winner {
                                if target == w {
                                    p
                                } else {
                                    0.0
                                }
                            } else {
                                let order =
                                    naive_ranks[c.combo.id()].cmp(&naive_ranks[d.combo.id()]);
                                match order {
                                    std::cmp::Ordering::Greater => p,
                                    std::cmp::Ordering::Equal => p / 2.0,
                                    std::cmp::Ordering::Less => 0.0,
                                }
                            };
                            naive += e.initial_reach[1 - target][j] * (payout - base);
                        }
                        assert!(
                            (naive - e.scratch[0].value[i]).abs() < 1e-7,
                            "case {case} target{target} combo{i}: {naive} != {}",
                            e.scratch[0].value[i]
                        );
                    }
                }
            }
        }
    }
    // Independent rank-comparison loop retained only as a bitwise regression
    // oracle for the cached group boundaries and compact card metadata.
    fn legacy_terminal_values(e: &Engine, id: usize, target: usize, reach: &[f64]) -> Vec<f64> {
        let node = &e.tree.nodes[id];
        let NodeKind::Terminal { winner } = node.kind else {
            panic!("terminal expected")
        };
        let own = &e.ranges[target];
        let opp = &e.ranges[1 - target];
        let mut all = Totals {
            sum: 0.0,
            cards: [0.0; 52],
        };
        for (c, &w) in opp.combos.iter().zip(reach) {
            add(&mut all, c.combo.cards, w);
        }
        let pot = node.pot as f64 - e.tree.config.rake(node.pot);
        let base = node.invested[target] as f64 + e.tree.config.starting_pot as f64 / 2.0;
        let board = board_mask(&node.board);
        let mut values = vec![0.0; own.combos.len()];
        if let Some(winner) = winner {
            let payoff = if winner == target { pot - base } else { -base };
            for (i, c) in own.combos.iter().enumerate() {
                let same = e.same[target][i].map_or(0.0, |j| reach[j]);
                values[i] = if e.masks[target][i] & board != 0 {
                    0.0
                } else {
                    payoff * compatible(c.combo.cards, &all, same)
                };
            }
            return values;
        }
        let ranks: [Vec<u32>; 2] = std::array::from_fn(|p| {
            e.ranges[p]
                .combos
                .iter()
                .map(|c| {
                    if c.combo.mask() & board != 0 {
                        return 0;
                    }
                    evaluate7(&[
                        Card(node.board[0]),
                        Card(node.board[1]),
                        Card(node.board[2]),
                        Card(node.board[3]),
                        Card(node.board[4]),
                        c.combo.cards[0],
                        c.combo.cards[1],
                    ])
                })
                .collect()
        });
        let sorted: [Vec<usize>; 2] = std::array::from_fn(|p| {
            let mut sorted: Vec<_> = (0..ranks[p].len()).collect();
            sorted.sort_unstable_by_key(|&i| (ranks[p][i], i));
            sorted
        });
        let mut weak = Totals {
            sum: 0.0,
            cards: [0.0; 52],
        };
        let mut cursor = 0;
        let mut own_cursor = 0;
        while own_cursor < sorted[target].len() {
            let rank = ranks[target][sorted[target][own_cursor]];
            while cursor < sorted[1 - target].len()
                && ranks[1 - target][sorted[1 - target][cursor]] < rank
            {
                let j = sorted[1 - target][cursor];
                add(&mut weak, opp.combos[j].combo.cards, reach[j]);
                cursor += 1;
            }
            let own_begin = own_cursor;
            while own_cursor < sorted[target].len()
                && ranks[target][sorted[target][own_cursor]] == rank
            {
                let i = sorted[target][own_cursor];
                values[i] = compatible(own.combos[i].combo.cards, &weak, 0.0);
                own_cursor += 1;
            }
            while cursor < sorted[1 - target].len()
                && ranks[1 - target][sorted[1 - target][cursor]] == rank
            {
                let j = sorted[1 - target][cursor];
                add(&mut weak, opp.combos[j].combo.cards, reach[j]);
                cursor += 1;
            }
            for &i in &sorted[target][own_begin..own_cursor] {
                let cards = own.combos[i].combo.cards;
                let same = e.same[target][i].map_or(0.0, |j| reach[j]);
                let total = compatible(cards, &all, same);
                let before = values[i];
                let after = compatible(cards, &weak, same);
                values[i] = if e.masks[target][i] & board != 0 {
                    0.0
                } else {
                    pot * 0.5 * (before + after) - base * total
                };
            }
        }
        values
    }
    #[test]
    fn zero_reach_terminal_contributions_preserve_bits_and_exact_skip_counts() {
        let (tree, ranges) = toy();
        let mut e = Engine::new(tree, ranges, EngineConfig::default()).unwrap();
        let id = 1;
        for target in 0..2 {
            for (all_zero, winner) in [false, true]
                .into_iter()
                .flat_map(|zero| [None, Some(0), Some(1)].map(move |winner| (zero, winner)))
            {
                e.tree.nodes[id].kind = NodeKind::Terminal { winner };
                let mut level = levels(1, 0, [e.masks[0].len(), e.masks[1].len()]).remove(0);
                for p in 0..2 {
                    for i in 0..level.reach[p].len() {
                        level.reach[p][i] = if all_zero || i % 2 == 0 {
                            -0.0
                        } else {
                            e.initial_reach[p][i]
                        };
                    }
                }
                let expected = legacy_terminal_values(&e, id, target, &level.reach[1 - target]);
                let mut diagnostic = diagnostics::KernelTimes::default();
                let ctx = Context {
                    tree: &e.tree,
                    ranges: &e.ranges,
                    masks: &e.masks,
                    hand_cards: &e.hand_cards,
                    same: &e.same,
                    tables: &e.tables,
                    terminal_table: &e.terminal_table,
                    config: &e.config,
                    permutations: &e.permutations,
                    average_reach_scale: &e.average_reach_scale,
                    initial_reach: &e.initial_reach,
                    discounts: [1.0; 3],
                    iteration: 0,
                    run_start: Instant::now(),
                };
                let table = &e.tables[e.terminal_table[id].unwrap().get() as usize - 1];
                let prefix = table.groups[target]
                    .last()
                    .map_or(0, |g| g.opp_after as usize);
                let expected_prefix_skips = if winner.is_some() {
                    0
                } else {
                    table.sorted[1 - target][..prefix]
                        .iter()
                        .filter(|h| level.reach[1 - target][h.dense as usize] == 0.0)
                        .count() as u64
                };
                let expected_total_skips = level.reach[1 - target]
                    .iter()
                    .filter(|&&r| r == 0.0)
                    .count() as u64;
                terminal(
                    &ctx,
                    id,
                    &mut level,
                    target,
                    winner,
                    &mut TotalsCache::<f64>::default(),
                    Some(&mut diagnostic),
                );
                assert_eq!(diagnostic.terminal_totals_zero_combos, expected_total_skips);
                assert_eq!(
                    diagnostic.showdown_prefix_zero_combos,
                    expected_prefix_skips
                );
                assert_eq!(
                    level.value[..expected.len()]
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                );
            }
        }
    }
    #[test]
    fn packed_terminals_match_dense_legacy_all_combos_and_empty_pack() {
        for sparse in [false, true] {
            let b = board("Qs Jh 2h 3c");
            let tree = GameTree::build(
                b.clone(),
                TreeConfig {
                    starting_pot: 6000,
                    effective_stack: 0,
                    rake_rate: 0.05,
                    rake_cap: 3000,
                    ..Default::default()
                },
            )
            .unwrap();
            let hands: Vec<_> = (0..1326)
                .map(|i| Combo::from_id(i).unwrap())
                .filter(|c| c.mask() & board_mask(&b) == 0)
                .collect();
            let rr = if sparse {
                [
                    range(&[("AcAh", 1.0), ("AcAd", 0.3)], &b),
                    range(&[("KcKh", 1.0), ("KdKs", 0.7)], &b),
                ]
            } else {
                std::array::from_fn(|p| {
                    Range::from_combos(
                        hands
                            .iter()
                            .take(if p == 0 { hands.len() } else { 690 })
                            .enumerate()
                            .map(|(i, &h)| (h, (i % 23 + 1) as f64 / 23.0))
                            .collect(),
                    )
                    .unwrap()
                    .blocked(&b.iter().map(|&c| Card(c)).collect::<Vec<_>>())
                    .unwrap()
                })
            };
            let mut e = Engine::new(tree, rr, EngineConfig::default()).unwrap();
            let ids: Vec<_> = e
                .tree
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n.kind, NodeKind::Terminal { winner: None }))
                .map(|(id, _)| id)
                .collect();
            let mut saw_empty = false;
            for id in ids.into_iter().step_by(if sparse { 1 } else { 7 }) {
                let table = &e.tables[e.terminal_table[id].unwrap().get() as usize - 1];
                for p in 0..2 {
                    assert_eq!(
                        table.sorted[p].len() + table.blocked[p].len(),
                        e.masks[p].len()
                    );
                    saw_empty |= table.sorted[p].is_empty();
                }
                for winner in [None, Some(0), Some(1)] {
                    e.tree.nodes[id].kind = NodeKind::Terminal { winner };
                    for target in 0..2 {
                        let mut level =
                            levels(1, 0, [e.masks[0].len(), e.masks[1].len()]).remove(0);
                        // Poison every reused output; blocked slots must be reset.
                        level.value.fill(f64::NAN);
                        for p in 0..2 {
                            for i in 0..level.reach[p].len() {
                                level.reach[p][i] =
                                    if e.masks[p][i] & board_mask(&e.tree.nodes[id].board) != 0 {
                                        0.0
                                    } else {
                                        e.initial_reach[p][i]
                                    };
                            }
                        }
                        let expected =
                            legacy_terminal_values(&e, id, target, &level.reach[1 - target]);
                        let ctx = Context {
                            tree: &e.tree,
                            ranges: &e.ranges,
                            masks: &e.masks,
                            hand_cards: &e.hand_cards,
                            same: &e.same,
                            tables: &e.tables,
                            terminal_table: &e.terminal_table,
                            config: &e.config,
                            permutations: &e.permutations,
                            average_reach_scale: &e.average_reach_scale,
                            initial_reach: &e.initial_reach,
                            discounts: [1.0; 3],
                            iteration: 0,
                            run_start: Instant::now(),
                        };
                        terminal(
                            &ctx,
                            id,
                            &mut level,
                            target,
                            winner,
                            &mut TotalsCache::<f64>::default(),
                            None,
                        );
                        assert_eq!(
                            level.value[..expected.len()]
                                .iter()
                                .map(|v| v.to_bits())
                                .collect::<Vec<_>>(),
                            expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                        );
                    }
                }
            }
            if sparse {
                assert!(saw_empty);
            }
        }
    }
    #[test]
    fn grouped_terminals_match_legacy_bits_with_future_blockers() {
        assert_eq!(std::mem::size_of::<SortedHand>(), 6);
        assert_eq!(std::mem::size_of::<RankGroup>(), 8);
        for b in [board("2c 7d 9h Js"), board("Ac Kc Qc Jc")] {
            let config = TreeConfig {
                starting_pot: 6000,
                effective_stack: 0,
                rake_rate: 0.05,
                rake_cap: 3000,
                ..Default::default()
            };
            let tree = GameTree::build(b.clone(), config).unwrap();
            let hands: Vec<_> = (0..1326)
                .map(|i| Combo::from_id(i).unwrap())
                .filter(|c| c.mask() & board_mask(&b) == 0)
                .collect();
            let rr = std::array::from_fn(|p| {
                Range::from_combos(
                    hands
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % (17 + p * 2) == 0)
                        .map(|(i, &c)| {
                            (
                                c,
                                if i % 5 == 0 {
                                    0.0
                                } else {
                                    (i % 23 + 1) as f64 / 23.0
                                },
                            )
                        })
                        .collect(),
                )
                .unwrap()
                .blocked(&b.iter().map(|&c| Card(c)).collect::<Vec<_>>())
                .unwrap()
            });
            let mut e = Engine::new(tree, rr, EngineConfig::default()).unwrap();
            let ids: Vec<_> = e
                .tree
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n.kind, NodeKind::Terminal { .. }))
                .map(|(id, _)| id)
                .step_by(5)
                .collect();
            for id in ids {
                for winner in [None, Some(0), Some(1)] {
                    e.tree.nodes[id].kind = NodeKind::Terminal { winner };
                    for target in 0..2 {
                        let mut level =
                            levels(1, 0, [e.ranges[0].combos.len(), e.ranges[1].combos.len()])
                                .remove(0);
                        for p in 0..2 {
                            for i in 0..level.reach[p].len() {
                                level.reach[p][i] =
                                    if e.masks[p][i] & board_mask(&e.tree.nodes[id].board) != 0 {
                                        0.0
                                    } else {
                                        e.initial_reach[p][i]
                                    };
                            }
                        }
                        let expected =
                            legacy_terminal_values(&e, id, target, &level.reach[1 - target]);
                        let ctx = Context {
                            tree: &e.tree,
                            ranges: &e.ranges,
                            masks: &e.masks,
                            hand_cards: &e.hand_cards,
                            same: &e.same,
                            tables: &e.tables,
                            terminal_table: &e.terminal_table,
                            config: &e.config,
                            permutations: &e.permutations,
                            average_reach_scale: &e.average_reach_scale,
                            initial_reach: &e.initial_reach,
                            discounts: [1.0; 3],
                            iteration: 0,
                            run_start: Instant::now(),
                        };
                        terminal(
                            &ctx,
                            id,
                            &mut level,
                            target,
                            winner,
                            &mut TotalsCache::<f64>::default(),
                            None,
                        );
                        assert_eq!(
                            level.value[..expected.len()]
                                .iter()
                                .map(|x| x.to_bits())
                                .collect::<Vec<_>>(),
                            expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                            "node {id},target {target},winner {winner:?}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn exact_best_response_matches_exhaustive_pure_strategies() {
        let (tree, ranges) = toy();
        let mut e = Engine::new(tree, ranges, EngineConfig::default()).unwrap();
        // Opponent always calls; bettor's independent pure choices have four profiles.
        let s = e.state.find(2).unwrap();
        let start = s.offset;
        e.average.f32_mut()[start..start + 2].copy_from_slice(&[0.0, 1.0]);
        let br = e.run(0, Mode::BestResponse);
        let mut exhaustive = f64::NEG_INFINITY;
        for first in 0..2 {
            for second in 0..2 {
                let choices = [first, second];
                let root = e.state.find(0).unwrap().offset;
                e.average.f32_mut()[root..root + 4].fill(0.0);
                for (hand, &choice) in choices.iter().enumerate() {
                    e.average.f32_mut()[root + choice * 2 + hand] = 1.0;
                }
                exhaustive = exhaustive.max(e.run(0, Mode::Average));
            }
        }
        assert!((br - exhaustive).abs() < 1e-10);
        assert!((br - 250.0).abs() < 1e-10);
        // Bettor pure bet with both hands: defender's exhaustive call/fold choices.
        let root = e.state.find(0).unwrap().offset;
        e.average.f32_mut()[root..root + 4].copy_from_slice(&[0.0, 0.0, 1.0, 1.0]);
        let br = e.run(1, Mode::BestResponse);
        let start = e.state.find(2).unwrap().offset;
        e.average.f32_mut()[start..start + 2].copy_from_slice(&[1.0, 0.0]);
        let fold = e.run(1, Mode::Average);
        e.average.f32_mut()[start..start + 2].copy_from_slice(&[0.0, 1.0]);
        let call = e.run(1, Mode::Average);
        assert!((br - fold.max(call)).abs() < 1e-10);
    }
    #[test]
    fn joint_conditioned_chance_preserves_mass_and_blocked_hands() {
        let b = board("2c 7c 9c Jc");
        let config = TreeConfig {
            starting_pot: 1000,
            effective_stack: 0,
            rake_rate: 0.0,
            ..Default::default()
        };
        let t = GameTree::build(b.clone(), config).unwrap();
        let rr = [range(&[("AcAd", 1.0)], &b), range(&[("KcKd", 1.0)], &b)];
        let mut e = Engine::new(t, rr, EngineConfig::default()).unwrap();
        let m = e.metrics();
        assert!((m.average_ev.iter().sum::<f64>()).abs() < 1e-8);
        // All river terminal counterfactual opponent reach mass totals to exactly 1.
        e.run(0, Mode::Average);
        let private_mask = e.masks[0][0] | e.masks[1][0];
        let NodeKind::Chance { cards, .. } = &e.tree.nodes[e.tree.root].kind else {
            panic!("chance root")
        };
        let legal = cards
            .iter()
            .filter(|&&card| private_mask & (1u64 << card) == 0)
            .count();
        assert!((legal as f64 / (52 - b.len() - 4) as f64 - 1.0).abs() < 1e-12);
    }
    #[test]
    fn reach_scoped_totals_match_uncached_serial_ev_br_and_strategy_bits() {
        let b = board("2c 7c 9c Jc");
        let tree = GameTree::build(
            b.clone(),
            TreeConfig {
                starting_pot: 1000,
                effective_stack: 500,
                allow_raises: false,
                rake_rate: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        // Nontrivial suit orbits, future blockers, and fractional opposing reach.
        let rr = [
            range(&[("AcAd", 0.7), ("AcAh", 0.7), ("AcAs", 0.7)], &b),
            range(&[("KcKd", 0.3), ("KcKh", 0.3), ("KcKs", 0.3)], &b),
        ];
        for storage_mode in [StorageMode::F32, StorageMode::I16] {
            for suit_isomorphism in [false, true] {
                let config = EngineConfig {
                    storage_mode,
                    suit_isomorphism,
                    ..Default::default()
                };
                let mut reference = Engine::new(tree.clone(), rr.clone(), config.clone()).unwrap();
                for _ in 0..12 {
                    reference.iterations += 1;
                    for target in 0..2 {
                        reference.run_with_cache(
                            target,
                            Mode::Train,
                            TotalsCache {
                                disabled: true,
                                ..Default::default()
                            },
                        );
                    }
                }
                let expected_average = std::array::from_fn::<_, 2, _>(|p| {
                    reference.run_with_cache(
                        p,
                        Mode::Average,
                        TotalsCache {
                            disabled: true,
                            ..Default::default()
                        },
                    )
                });
                let expected_br = std::array::from_fn::<_, 2, _>(|p| {
                    reference.run_with_cache(
                        p,
                        Mode::BestResponse,
                        TotalsCache {
                            disabled: true,
                            ..Default::default()
                        },
                    )
                });
                let expected_chips = ((expected_br[0] - expected_average[0])
                    + (expected_br[1] - expected_average[1]))
                    .max(0.0)
                    / 2.0;
                for threads in [1, 4] {
                    let mut actual = Engine::new(
                        tree.clone(),
                        rr.clone(),
                        EngineConfig {
                            threads,
                            kernel_diagnostics: true,
                            ..config.clone()
                        },
                    )
                    .unwrap();
                    for _ in 0..12 {
                        actual.iterate();
                    }
                    assert_eq!(actual.regret.bits(), reference.regret.bits());
                    assert_eq!(actual.average.bits(), reference.average.bits());
                    for (id, node) in tree.nodes.iter().enumerate() {
                        if matches!(node.kind, NodeKind::Decision { .. }) {
                            assert_eq!(
                                actual
                                    .strategy(id)
                                    .unwrap()
                                    .iter()
                                    .map(|v| v.to_bits())
                                    .collect::<Vec<_>>(),
                                reference
                                    .strategy(id)
                                    .unwrap()
                                    .iter()
                                    .map(|v| v.to_bits())
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                    let metrics = actual.metrics();
                    assert_eq!(
                        metrics.average_ev.map(f64::to_bits),
                        expected_average.map(f64::to_bits)
                    );
                    assert_eq!(
                        metrics.best_response_ev.map(f64::to_bits),
                        expected_br.map(f64::to_bits)
                    );
                    assert_eq!(
                        metrics.exploitability_chips.to_bits(),
                        expected_chips.to_bits()
                    );
                    assert_eq!(
                        metrics.exploitability_pct.to_bits(),
                        (100.0 * expected_chips / 1000.0).to_bits()
                    );
                    let d = actual.kernel_diagnostics().unwrap();
                    assert!(d.training.terminal_totals_cache_hits > 0);
                    assert!(
                        d.training.terminal_totals_scanned_combos
                            < 12 * d.per_iteration_workload.terminal_totals_dense_combos
                    );
                }
            }
        }
    }
    #[test]
    fn totals_cache_reuse_counts_actual_scans_and_preserves_sum_bits() {
        let cards = [[Card(0), Card(1)], [Card(2), Card(3)], [Card(0), Card(3)]];
        let reach = [0.7, -0.0, f64::from_bits(1)];
        let original = totals_nonzero::<f64, f64>(&cards, &reach, None);
        let mut cache = TotalsCache::<f64>::default();
        let mut diagnostic = diagnostics::KernelTimes::default();
        for _ in 0..3 {
            let actual = cache.get(&cards, &reach, Some(&mut diagnostic));
            assert_eq!(actual.sum.to_bits(), original.sum.to_bits());
            assert_eq!(
                actual.cards.map(f64::to_bits),
                original.cards.map(f64::to_bits)
            );
        }
        assert_eq!(diagnostic.terminal_totals_cache_hits, 2);
        assert_eq!(diagnostic.terminal_totals_scanned_combos, 3);
        assert_eq!(diagnostic.terminal_totals_zero_combos, 1);
        // Fresh scopes cannot reuse the previous reach, even at the same address.
        cache = cache.fresh();
        let next = [0.0, 0.9, 0.2];
        let actual = cache.get(&cards, &next, Some(&mut diagnostic));
        let expected = totals_nonzero::<f64, f64>(&cards, &next, None);
        assert_eq!(actual.sum.to_bits(), expected.sum.to_bits());
        assert_eq!(
            actual.cards.map(f64::to_bits),
            expected.cards.map(f64::to_bits)
        );
        assert_eq!(diagnostic.terminal_totals_scanned_combos, 6);
    }
    #[test]
    fn default_profiling_is_opt_in() {
        assert!(!EngineConfig::default().profiling);
        let (tree, ranges) = toy();
        let mut e = Engine::new(tree, ranges, EngineConfig::default()).unwrap();
        e.iterate();
        e.metrics();
        let p = e.profile();
        assert_eq!(
            [
                p.showdown_seconds,
                p.fold_seconds,
                p.regret_seconds,
                p.chance_seconds
            ],
            [0.0; 4]
        );
    }
    #[test]
    fn weighted_chance_split_balances_skew_and_terminal_only_children() {
        let state = |span| State::<f64> {
            id: 0,
            children: Vec::new(),
            offset: 0,
            own_len: 0,
            span,
            scales: [1.0; 2],
            work: [0; 4],
            orbits: Vec::new(),
            parallel_scratch: Vec::new(),
        };
        for (weights, expected) in [
            (vec![1, 1, 1, 9], 3),
            (vec![9, 1, 1, 1], 1),
            (vec![0, 0, 0, 0], 2),
            (vec![1, 1], 1),
            (vec![usize::MAX, usize::MAX, 1], 1),
        ] {
            let children: Vec<_> = weights.into_iter().map(state).collect();
            assert_eq!(weighted_chance_split(&children), expected);
        }
    }
    #[test]
    fn parallel_grain_preserves_storage_and_metrics_bits() {
        let b = board("2c 7d 9h Js");
        let tree = GameTree::build(
            b.clone(),
            TreeConfig {
                starting_pot: 1000,
                effective_stack: 500,
                allow_raises: false,
                rake_rate: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 0.3), ("KcKd", 0.0)], &b),
            range(&[("KcKd", 0.7), ("3c4d", 0.6), ("AcAd", 0.2)], &b),
        ];
        for storage_mode in [StorageMode::F32, StorageMode::I16] {
            let mut reference = Engine::new(
                tree.clone(),
                rr.clone(),
                EngineConfig {
                    storage_mode,
                    threads: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            for _ in 0..32 {
                reference.iterate();
            }
            let expected = reference.metrics();
            for (threads, grain) in [(1, 1), (4, 1), (4, 2), (4, 4), (4, 7), (4, 64)] {
                let mut actual = Engine::new(
                    tree.clone(),
                    rr.clone(),
                    EngineConfig {
                        storage_mode,
                        threads,
                        parallel_grain: grain,
                        ..Default::default()
                    },
                )
                .unwrap();
                for _ in 0..32 {
                    actual.iterate();
                }
                let metrics = actual.metrics();
                assert_eq!(actual.regret.bits(), reference.regret.bits());
                assert_eq!(actual.average.bits(), reference.average.bits());
                assert_eq!(
                    metrics.average_ev.map(f64::to_bits),
                    expected.average_ev.map(f64::to_bits)
                );
                assert_eq!(
                    metrics.best_response_ev.map(f64::to_bits),
                    expected.best_response_ev.map(f64::to_bits)
                );
                assert_eq!(
                    metrics.exploitability_pct.to_bits(),
                    expected.exploitability_pct.to_bits()
                );
            }
        }
        let invalid = Engine::new(
            tree,
            rr,
            EngineConfig {
                parallel_grain: 0,
                ..Default::default()
            },
        );
        assert!(invalid.err().unwrap().contains("grain"));
    }
    #[test]
    fn actual_isomorphism_matches_unmerged_strategies_and_br() {
        let b = board("2c 7c 9c Jc");
        let config = TreeConfig {
            starting_pot: 1000,
            effective_stack: 500,
            rake_rate: 0.0,
            allow_raises: false,
            ..Default::default()
        };
        let t = GameTree::build(b.clone(), config).unwrap();
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 1.0)], &b),
            range(&[("KcKd", 1.0)], &b),
        ];
        let mut plain = Engine::new(
            t.clone(),
            rr.clone(),
            EngineConfig {
                threads: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let mut iso = Engine::new(
            t,
            rr,
            EngineConfig {
                threads: 2,
                suit_isomorphism: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(iso.isomorphism_used());
        assert!(iso.merged_chance_children() > 0);
        for _ in 0..300 {
            plain.iterate();
            iso.iterate();
        }
        let a = plain.metrics();
        let b = iso.metrics();
        for p in 0..2 {
            assert!((a.average_ev[p] - b.average_ev[p]).abs() < 0.002);
            assert!((a.best_response_ev[p] - b.best_response_ev[p]).abs() < 0.002);
        }
        for (id, node) in plain.tree.nodes.iter().enumerate() {
            if matches!(node.kind, NodeKind::Decision { .. }) {
                for (a, b) in plain
                    .strategy(id)
                    .unwrap()
                    .iter()
                    .zip(iso.strategy(id).unwrap())
                {
                    assert!((*a - b).abs() < 0.0001, "node {id}: {a} {b}");
                }
            }
        }
    }
    #[test]
    fn asymmetric_ranges_and_custom_rules_are_not_merged() {
        let b = board("2c 7c 9c Jc");
        let config = TreeConfig {
            starting_pot: 1000,
            effective_stack: 0,
            ..Default::default()
        };
        let rr = [
            range(&[("AcAd", 1.0), ("3h4s", 1.0)], &b),
            range(&[("KcKh", 1.0)], &b),
        ];
        let t = GameTree::build(b.clone(), config.clone()).unwrap();
        let e = Engine::new(
            t,
            rr,
            EngineConfig {
                suit_isomorphism: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!e.isomorphism_used());
        let rr = [range(&[("AcAd", 1.0)], &b), range(&[("KcKd", 1.0)], &b)];
        let config = TreeConfig {
            bet_size: SizeRule::Custom(std::sync::Arc::new(|_| 0.5)),
            ..config
        };
        let e = Engine::new(
            GameTree::build(b, config).unwrap(),
            rr,
            EngineConfig {
                suit_isomorphism: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!e.isomorphism_used());
    }
    #[test]
    fn state_vectors_have_exact_capacity_with_and_without_aliases() {
        fn check(state: &State) {
            assert_eq!(state.children.capacity(), state.children.len());
            assert_eq!(state.orbits.capacity(), state.orbits.len());
            assert_eq!(
                state.parallel_scratch.capacity(),
                state.parallel_scratch.len()
            );
            for orbit in &state.orbits {
                assert_eq!(orbit.capacity(), orbit.len());
            }
            for child in &state.children {
                check(child);
            }
        }
        let b = board("2c 7c 9c Jc");
        let tree = GameTree::build(
            b.clone(),
            TreeConfig {
                starting_pot: 1000,
                effective_stack: 500,
                allow_raises: false,
                rake_rate: 0.0,
                ..Default::default()
            },
        )
        .unwrap();
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 1.0)], &b),
            range(&[("KcKd", 1.0)], &b),
        ];
        for suit_isomorphism in [false, true] {
            for threads in [1, 2] {
                let e = Engine::new(
                    tree.clone(),
                    rr.clone(),
                    EngineConfig {
                        suit_isomorphism,
                        threads,
                        ..Default::default()
                    },
                )
                .unwrap();
                if suit_isomorphism {
                    assert!(e.merged_chance_children() > 0);
                }
                check(&e.state);
            }
        }
    }
    #[test]
    fn compact_metadata_bounds_layout_and_profile_aggregation() {
        assert_eq!(compact_index(0).unwrap(), 0);
        assert_eq!(compact_index(u32::MAX as usize).unwrap(), u32::MAX);
        if let Some(too_large) = (u32::MAX as usize).checked_add(1) {
            assert!(compact_index(too_large).is_err());
        }
        assert_eq!(std::mem::size_of::<Option<NonZeroU32>>(), 4);
        #[allow(dead_code)]
        struct LegacyStateLayout {
            id: usize,
            children: Vec<State>,
            offset: usize,
            own_len: usize,
            span: usize,
            scales: [f32; 2],
            work: [u128; 4],
            orbits: Vec<Vec<usize>>,
            parallel_scratch: Vec<Vec<Level>>,
        }
        let old = std::mem::size_of::<LegacyStateLayout>();
        let new = std::mem::size_of::<State>();
        let arrays_saved = 2 * std::mem::size_of::<usize>() + std::mem::size_of::<Option<usize>>()
            - (2 * std::mem::size_of::<u32>() + std::mem::size_of::<Option<NonZeroU32>>());
        eprintln!("metadata bytes per node: State {old} -> {new}; arrays save {arrays_saved}; total save {}", old - new + arrays_saved);
        assert!(new < old);
        let (tree, ranges) = toy();
        let mut e = Engine::new(tree, ranges, EngineConfig::default()).unwrap();
        e.state.work[0] = u64::MAX;
        e.state.children[0].work[0] = u64::MAX;
        assert_eq!(e.state.work()[0], 2 * u64::MAX as u128);
    }
    #[test]
    fn deterministic_bit_patterns_and_memory_preflight() {
        let b = board("2c 7d 9h Js");
        let config = TreeConfig {
            starting_pot: 1000,
            effective_stack: 500,
            allow_raises: false,
            ..Default::default()
        };
        let t = GameTree::build(b.clone(), config).unwrap();
        let rr = [
            range(&[("AcAd", 1.0), ("3c4d", 0.7)], &b),
            range(&[("KcKd", 1.0)], &b),
        ];
        assert!(Engine::new(
            t.clone(),
            rr.clone(),
            EngineConfig {
                memory_limit_bytes: 1,
                ..Default::default()
            }
        )
        .is_err());
        let mut a = Engine::new(
            t.clone(),
            rr.clone(),
            EngineConfig {
                threads: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let mut b = Engine::new(
            t,
            rr,
            EngineConfig {
                threads: 2,
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..70 {
            a.iterate();
            b.iterate();
        }
        assert_eq!(a.regret.bits(), b.regret.bits());
        assert_eq!(a.average.bits(), b.average.bits());
        let ma = a.metrics();
        let mb = b.metrics();
        assert_eq!(
            ma.exploitability_pct.to_bits(),
            mb.exploitability_pct.to_bits()
        );
        assert_eq!(
            ma.average_ev.map(f64::to_bits),
            mb.average_ev.map(f64::to_bits)
        );
    }
    #[test]
    fn general_combo_permutations_match_physical_flop_trees() {
        fn all_pairs(rank: u8, b: &[u8]) -> DenseRange {
            let hands = (0..4)
                .flat_map(|a| {
                    ((a + 1)..4).map(move |z| {
                        (
                            cards::Combo::new(Card(rank * 4 + a), Card(rank * 4 + z)).unwrap(),
                            1.0,
                        )
                    })
                })
                .collect();
            Range::from_combos(hands)
                .unwrap()
                .blocked(&b.iter().map(|&c| Card(c)).collect::<Vec<_>>())
                .unwrap()
        }
        fn all_strategies(e: &Engine) -> Vec<Option<Vec<f32>>> {
            fn visit(e: &Engine, s: &State, out: &mut [Option<Vec<f32>>]) {
                if let NodeKind::Decision {
                    player, actions, ..
                } = &e.tree.nodes[s.id].kind
                {
                    let mut input = vec![0.0; s.own_len];
                    e.average.read(s.offset, &mut input, s.scales[1]);
                    let mut norm = vec![0.0; s.own_len];
                    normalize(
                        &input,
                        &mut norm,
                        e.ranges[*player].combos.len(),
                        actions.len(),
                        &e.masks[*player],
                        board_mask(&e.tree.nodes[s.id].board),
                    );
                    out[s.id] = Some(norm);
                }
                for child in &s.children {
                    visit(e, child, out);
                }
            }
            let mut out = vec![None; e.tree.nodes.len()];
            visit(e, &e.state, &mut out);
            out
        }
        for (text, expected_merge) in [
            ("Qc Jd 2h", false),
            ("Qc Jh 2h", true),
            ("Qh Jh 2h", true),
            ("Qc Qd 2h", true),
        ] {
            let b = board(text);
            let rr = [all_pairs(12, &b), all_pairs(11, &b)];
            let tree = GameTree::build(
                b,
                TreeConfig {
                    starting_pot: 1000,
                    effective_stack: 500,
                    allow_raises: false,
                    rake_rate: 0.05,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut physical =
                Engine::new(tree.clone(), rr.clone(), EngineConfig::default()).unwrap();
            let mut reduced = Engine::new(
                tree,
                rr,
                EngineConfig {
                    suit_isomorphism: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(reduced.isomorphism_used(), expected_merge, "{text}");
            if expected_merge {
                assert!(
                    reduced.permutations.iter().any(|p| p
                        .indices
                        .iter()
                        .any(|indices| indices.iter().enumerate().any(|(i, &j)| i != j))),
                    "nontrivial combo mapping required"
                );
                assert!(reduced.persistent_storage_bytes() < physical.persistent_storage_bytes());
            }
            for _ in 0..8 {
                physical.iterate();
                reduced.iterate();
            }
            let a = physical.metrics();
            let z = reduced.metrics();
            for player in 0..2 {
                assert!(
                    (a.average_ev[player] - z.average_ev[player]).abs() < 0.001,
                    "{text} average EV"
                );
                assert!(
                    (a.best_response_ev[player] - z.best_response_ev[player]).abs() < 0.001,
                    "{text} BR EV"
                );
            }
            let full = all_strategies(&physical);
            let reps = all_strategies(&reduced);
            let mut checked = 0;
            for (id, node) in physical.tree.nodes.iter().enumerate() {
                let NodeKind::Decision {
                    player, actions, ..
                } = &node.kind
                else {
                    continue;
                };
                let n = physical.ranges[*player].combos.len();
                let mut map: Vec<_> = (0..n).collect();
                let mut rep = id;
                while reduced.aliases[rep] as usize != rep {
                    let p =
                        &reduced.permutations[reduced.alias_perms[rep] as usize].indices[*player];
                    for i in &mut map {
                        *i = p[*i];
                    }
                    rep = reduced.aliases[rep] as usize;
                }
                let direct = full[id].as_ref().unwrap();
                let canonical = reps[rep].as_ref().unwrap();
                for action in 0..actions.len() {
                    for i in 0..n {
                        assert!(
                            (direct[action * n + i] - canonical[action * n + map[i]]).abs()
                                < 0.0001,
                            "{text} node {id} hand {i}"
                        );
                    }
                }
                if checked < 20 {
                    let public = reduced.strategy(id).unwrap();
                    for action in 0..actions.len() {
                        for i in 0..n {
                            assert_eq!(public[action * n + i], canonical[action * n + map[i]]);
                        }
                    }
                }
                checked += 1;
            }
            assert!(checked > 100);
        }
    }
    #[test]
    fn i16_storage_matches_f32_toy_accuracy_and_halves_payload() {
        for algorithm in [Algorithm::Dcfr, Algorithm::CfrPlus] {
            let (tree, ranges) = toy();
            let mut f32 = Engine::new(
                tree.clone(),
                ranges.clone(),
                EngineConfig {
                    algorithm,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut i16 = Engine::new(
                tree,
                ranges,
                EngineConfig {
                    algorithm,
                    storage_mode: StorageMode::I16,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                f32.persistent_storage_bytes(),
                i16.persistent_storage_bytes() * 2
            );
            for _ in 0..2000 {
                f32.iterate();
                i16.iterate();
            }
            let a = f32.metrics();
            let b = i16.metrics();
            for p in 0..2 {
                assert!((a.average_ev[p] - b.average_ev[p]).abs() < 0.3);
                assert!((a.best_response_ev[p] - b.best_response_ev[p]).abs() < 0.3);
            }
            assert!((a.exploitability_pct - b.exploitability_pct).abs() < 0.03);
            for node in [0, 2] {
                for (a, b) in f32
                    .strategy(node)
                    .unwrap()
                    .iter()
                    .zip(i16.strategy(node).unwrap())
                {
                    assert!((*a - b).abs() < 0.02);
                }
            }
        }
    }
    #[test]
    fn fused_i16_update_matches_decode_update_encode_bits() {
        for algorithm in [Algorithm::Dcfr, Algorithm::CfrPlus] {
            let (tree, ranges) = toy();
            let e = Engine::new(
                tree,
                ranges,
                EngineConfig {
                    algorithm,
                    ..Default::default()
                },
            )
            .unwrap();
            for iteration in [1, 9, 37] {
                let t = iteration as f64;
                let pos = t.powf(e.config.dcfr_alpha);
                let neg = t.powf(e.config.dcfr_beta);
                let ctx = Context {
                    tree: &e.tree,
                    ranges: &e.ranges,
                    masks: &e.masks,
                    hand_cards: &e.hand_cards,
                    same: &e.same,
                    tables: &e.tables,
                    terminal_table: &e.terminal_table,
                    config: &e.config,
                    permutations: &e.permutations,
                    average_reach_scale: &e.average_reach_scale,
                    initial_reach: &e.initial_reach,
                    discounts: [
                        pos / (pos + 1.0),
                        neg / (neg + 1.0),
                        (t / (t + 1.0)).powf(e.config.dcfr_gamma),
                    ],
                    iteration,
                    run_start: Instant::now(),
                };
                for actions in [1, 2, 3, 4] {
                    let n = 7;
                    let len = n * actions;
                    for (scales, magnitude) in [
                        ([1.0, 1.0], 1.0f32),
                        ([0.003, 17.0], 0.1),
                        ([1e-40, 1e-39], 1e-39),
                        ([f32::from_bits(1); 2], f32::from_bits(1)),
                        ([1.0, 1.0], 0.0),
                    ] {
                        let regret: Vec<i16> = (0..len)
                            .map(|i| [-32768, -32767, -7, 0, 1, 101, 32767][i % n])
                            .collect();
                        let average: Vec<i16> = (0..len)
                            .map(|i| [0, 1, 0, 32767, 7, 101, 13][i % n])
                            .collect();
                        let strategy: Vec<f32> = (0..len)
                            .map(|i| {
                                if i % n == 2 {
                                    0.0
                                } else {
                                    (i % 5) as f32 / 4.0
                                }
                            })
                            .collect();
                        let value: Vec<f64> = (0..n)
                            .map(|i| (i as f64 - 3.0) * magnitude as f64)
                            .collect();
                        let action_values: Vec<f64> = (0..len)
                            .map(|i| (i as f64 - 6.0) * magnitude as f64)
                            .collect();
                        let path_reach: Vec<f64> = (0..n)
                            .map(|i| {
                                if i == 2 {
                                    0.0
                                } else {
                                    i as f64 * magnitude as f64 / 7.0
                                }
                            })
                            .collect();
                        let mut reference_regret = vec![0.0f32; len];
                        let mut reference_average = vec![0.0f32; len];
                        let mut expected_regret = regret.clone();
                        let mut expected_average = average.clone();
                        SlabSlice::I16(&mut expected_regret).read(&mut reference_regret, scales[0]);
                        SlabSlice::I16(&mut expected_average)
                            .read(&mut reference_average, scales[1]);
                        update_policy(
                            &ctx,
                            &strategy,
                            &action_values,
                            &value,
                            &path_reach,
                            n,
                            actions,
                            &mut reference_regret,
                            &mut reference_average,
                        );
                        let mut expected_scales = scales;
                        SlabSlice::I16(&mut expected_regret)
                            .write(&reference_regret, &mut expected_scales[0]);
                        SlabSlice::I16(&mut expected_average)
                            .write(&reference_average, &mut expected_scales[1]);
                        let mut actual_regret = regret;
                        let mut actual_average = average;
                        let mut actual_scales = scales;
                        let mut regret_scratch = vec![0.0; len];
                        let mut average_scratch = vec![0.0; len];
                        update_policy_i16(
                            &ctx,
                            &strategy,
                            &action_values,
                            &value,
                            &path_reach,
                            n,
                            &mut actual_regret,
                            &mut actual_average,
                            &mut actual_scales,
                            &mut regret_scratch,
                            &mut average_scratch,
                        );
                        assert_eq!(regret_scratch, reference_regret);
                        assert_eq!(average_scratch, reference_average);
                        assert_eq!(actual_regret, expected_regret);
                        assert_eq!(actual_average, expected_average);
                        assert_eq!(
                            actual_scales.map(f32::to_bits),
                            expected_scales.map(f32::to_bits)
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn integer_policy_normalization_matches_scaled_reference() {
        for actions in [1, 2, 3, 4] {
            let n = 7;
            let input: Vec<i16> = (0..actions * n)
                .map(|i| match (i * 11 + i / n * 3) % 7 {
                    0 => -32767,
                    1 => 0,
                    2 => 1,
                    3 => 32767,
                    4 => -7,
                    5 => 101,
                    _ => 17,
                })
                .collect();
            let masks: Vec<u64> = (0..n).map(|i| 1u64 << i).collect();
            for scale in [0.001f32, 17.0, f32::from_bits(1)] {
                let decoded: Vec<f32> = input.iter().map(|&q| q as f32 * scale).collect();
                let mut reference = vec![0.0; input.len()];
                let mut direct = reference.clone();
                normalize(&decoded, &mut reference, n, actions, &masks, 2);
                normalize_i16(&input, &mut direct, n, actions, &masks, 2);
                for (&a, &b) in reference.iter().zip(&direct) {
                    assert!(
                        (a - b).abs() < 1e-6,
                        "{actions} actions,scale {scale}: {a} {b}"
                    );
                }
            }
        }
    }
    #[test]
    fn best_response_is_independent_of_own_average_policy() {
        let (tree, ranges) = toy();
        let mut e = Engine::new(tree, ranges, EngineConfig::default()).unwrap();
        let offset = e.state.find(0).unwrap().offset;
        e.average.f32_mut()[offset..offset + 4].copy_from_slice(&[1.0, 1.0, 0.0, 0.0]);
        let check = e.run(0, Mode::BestResponse);
        e.average.f32_mut()[offset..offset + 4].copy_from_slice(&[0.0, 0.0, 1.0, 1.0]);
        let bet = e.run(0, Mode::BestResponse);
        assert_eq!(check.to_bits(), bet.to_bits());
    }
    #[test]
    fn quantized_subnormal_scale_round_trip_is_finite_and_signed() {
        let mut cases = vec![1e-35f32, 1e-39f32, 1e-42f32, f32::from_bits(1)];
        cases.extend((1..100).map(|i| f32::from_bits(i * 12347)));
        for max in cases {
            let input = [-max, 0.0, max / 2.0, max];
            let mut encoded = [0i16; 4];
            let mut scale = 0.0;
            let mut slab = SlabSlice::I16(&mut encoded);
            slab.write(&input, &mut scale);
            assert!(scale.is_finite() && scale > 0.0);
            let mut decoded = [0.0; 4];
            slab.read(&mut decoded, scale);
            assert!(decoded.iter().all(|x| x.is_finite()));
            assert!(decoded[0] < 0.0 && decoded[3] > 0.0);
            for (&a, &b) in input.iter().zip(&decoded) {
                assert!((a as f64 - b as f64).abs() <= scale as f64 * 1.01 + max as f64 * 1e-6);
            }
        }
    }
    #[test]
    fn quantized_average_preserves_tiny_weight_private_hands() {
        let (tree, mut ranges) = toy();
        ranges[0].combos[1].weight = 1e-8;
        let mut exact = Engine::new(tree.clone(), ranges.clone(), EngineConfig::default()).unwrap();
        let mut packed = Engine::new(
            tree,
            ranges,
            EngineConfig {
                storage_mode: StorageMode::I16,
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..1000 {
            exact.iterate();
            packed.iterate();
        }
        let a = exact.strategy(0).unwrap();
        let b = packed.strategy(0).unwrap();
        for i in 0..a.len() {
            assert!(
                (a[i] - b[i]).abs() < 0.01,
                "private hand strategy {i}: f32={} i16={}",
                a[i],
                b[i]
            );
        }
    }
    #[test]
    fn range_weight_asymmetry_disables_nontrivial_combo_orbits() {
        let b = board("Qh Jh 2h");
        let rr = [
            range(&[("AcAd", 1.0), ("AcAs", 0.5), ("AdAs", 0.25)], &b),
            range(&[("KcKd", 1.0), ("KcKs", 1.0), ("KdKs", 1.0)], &b),
        ];
        let tree = GameTree::build(
            b,
            TreeConfig {
                effective_stack: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let e = Engine::new(
            tree,
            rr,
            EngineConfig {
                suit_isomorphism: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(e.permutations.len(), 1);
        assert!(!e.isomorphism_used());
    }
    #[test]
    fn raked_values_sum_to_negative_rake_and_cap() {
        for (pot, rake) in [(10000, 500.0), (120000, 3000.0)] {
            let b = board("Ac Kc Qc Jc Tc");
            let rr = [range(&[("2h3d", 1.0)], &b), range(&[("4h5d", 1.0)], &b)];
            let config = TreeConfig {
                starting_pot: pot,
                ..Default::default()
            };
            let t = GameTree::from_nodes(
                config,
                vec![node(&b, pot, [0, 0], NodeKind::Terminal { winner: None })],
                0,
            );
            let mut e = Engine::new(t, rr, Default::default()).unwrap();
            let m = e.metrics();
            assert_eq!(m.average_ev, [-rake / 2.0, -rake / 2.0]);
            assert_eq!(m.exploitability_pct, 0.0);
        }
    }
}
