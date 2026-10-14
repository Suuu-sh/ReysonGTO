//! Clean-room vector alternating CFR. Values are net future receipts minus future
//! investments and half the starting pot (so zero-rake values sum to zero).
//! Dense combo IDs stay stable across streets; newly blocked hands have zero strategy.
//! Optional suit reduction uses the subgroup fixing every supplied combo pointwise
//! and a suit-invariant size rule. This conservative subgroup needs identity combo
//! remapping; arbitrary asymmetric ranges fall back to the unmerged exact tree.
use cards::{evaluate7, Card, SuitPermutation};
use ranges::DenseRange;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    pub memory_limit_bytes: usize,
    pub suit_isomorphism: bool,
    pub profiling: bool,
}
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::Dcfr,
            threads: 1,
            memory_limit_bytes: 2 * 1024 * 1024 * 1024,
            suit_isomorphism: false,
            profiling: false,
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

// Each node's three action-major buffers are contiguous; all traversal scratch
// is allocated at construction, including scratch for every parallel chance child.
struct State {
    id: usize,
    children: Vec<State>,
    reach: [Vec<f64>; 2],
    value: Vec<f64>,
    offset: usize,
    own_len: usize,
    span: usize,
    terminal_nanos: u128,
    chance_weights: Vec<usize>,
}
impl State {
    fn new(tree: &GameTree, id: usize, n: [usize; 2], offset: usize) -> Self {
        let (children, count) = match &tree.nodes[id].kind {
            NodeKind::Decision {
                player,
                children,
                actions,
            } => (children.as_slice(), actions.len() * n[*player]),
            NodeKind::Chance { children, .. } => (children.as_slice(), 0),
            NodeKind::Terminal { .. } => (&[][..], 0),
        };
        let mut end = offset + count;
        let children: Vec<_> = children
            .iter()
            .map(|&c| {
                let child = Self::new(tree, c, n, end);
                end += child.span;
                child
            })
            .collect();
        let child_count = children.len();
        Self {
            id,
            children,
            reach: [vec![0.0; n[0]], vec![0.0; n[1]]],
            value: vec![0.0; n[0].max(n[1])],
            offset,
            own_len: count,
            span: end - offset,
            terminal_nanos: 0,
            chance_weights: vec![1; child_count],
        }
    }
    fn find(&self, id: usize) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|s| s.find(id))
    }
    fn terminal_nanos(&self) -> u128 {
        self.terminal_nanos + self.children.iter().map(Self::terminal_nanos).sum::<u128>()
    }
}
struct RankTable {
    ranks: [Vec<u32>; 2],
    sorted: [Vec<usize>; 2],
}
struct Context<'a> {
    tree: &'a GameTree,
    ranges: &'a [DenseRange; 2],
    masks: &'a [Vec<u64>; 2],
    same: &'a [Vec<Option<usize>>; 2],
    tables: &'a [RankTable],
    terminal_table: &'a [Option<usize>],
    config: &'a EngineConfig,
    iteration: usize,
}
#[derive(Clone, Copy)]
enum Mode {
    Train,
    Average,
    BestResponse,
}

pub struct Engine {
    tree: GameTree,
    ranges: [DenseRange; 2],
    config: EngineConfig,
    state: State,
    masks: [Vec<u64>; 2],
    same: [Vec<Option<usize>>; 2],
    tables: Vec<RankTable>,
    terminal_table: Vec<Option<usize>>,
    initial_reach: [Vec<f64>; 2],
    joint_mass: f64,
    iterations: usize,
    memory_bytes: usize,
    pool: Arc<rayon::ThreadPool>,
    aliases: Vec<usize>,
    merged_children: usize,
    regret: Vec<f32>,
    average: Vec<f32>,
    strategy: Vec<f32>,
}

/// Conservative owned solver payload estimate, including per-node traversal
/// buffers, rank caches and tree/ranges. Thread stacks and allocator overhead are
/// covered by a 25% margin plus 1 MiB per configured worker in Engine::new.
pub fn estimate_memory_bytes(tree: &GameTree, ranges: &[DenseRange; 2]) -> Result<usize, String> {
    validate(tree, ranges)?;
    let n = [ranges[0].combos.len(), ranges[1].combos.len()];
    let mut bytes = 0usize;
    let mut boards = BTreeSet::new();
    for node in &tree.nodes {
        let action = match &node.kind {
            NodeKind::Decision {
                player, actions, ..
            } => actions
                .len()
                .checked_mul(n[*player])
                .ok_or("memory estimate overflow")?,
            _ => 0,
        };
        let payload = action
            .checked_mul(12)
            .and_then(|v| v.checked_add((n[0] + n[1] + n[0].max(n[1])) * 8))
            .ok_or("memory estimate overflow")?;
        bytes = bytes
            .checked_add(
                payload
                    + std::mem::size_of::<State>()
                    + std::mem::size_of::<tree::Node>()
                    + node.board.len(),
            )
            .ok_or("memory estimate overflow")?;
        match &node.kind {
            NodeKind::Terminal { winner: None } => {
                boards.insert(board_mask(&node.board));
            }
            NodeKind::Decision {
                actions, children, ..
            } => {
                bytes = bytes
                    .checked_add(
                        actions.len() * std::mem::size_of::<tree::Action>() + children.len() * 16,
                    )
                    .ok_or("memory estimate overflow")?;
            }
            NodeKind::Chance { cards, children } => {
                bytes = bytes
                    .checked_add(cards.len() + children.len() * 16)
                    .ok_or("memory estimate overflow")?;
            }
            _ => {}
        }
    }
    bytes = bytes
        .checked_add(boards.len() * (n[0] + n[1]) * 12 + (n[0] + n[1]) * 64 + tree.nodes.len() * 24)
        .ok_or("memory estimate overflow")?;
    bytes
        .checked_add(bytes / 4)
        .ok_or_else(|| "memory estimate overflow".into())
}
fn validate(tree: &GameTree, ranges: &[DenseRange; 2]) -> Result<(), String> {
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

impl Engine {
    pub fn new(
        tree: GameTree,
        ranges: [DenseRange; 2],
        config: EngineConfig,
    ) -> Result<Self, String> {
        if config.threads == 0 {
            return Err("threads must be positive".into());
        }
        let memory_bytes = estimate_memory_bytes(&tree, &ranges)?
            .checked_add(
                config
                    .threads
                    .checked_mul(1024 * 1024)
                    .ok_or("memory estimate overflow")?,
            )
            .ok_or("memory estimate overflow")?;
        if memory_bytes > config.memory_limit_bytes {
            return Err(format!(
                "estimated memory {memory_bytes} exceeds limit {}",
                config.memory_limit_bytes
            ));
        }
        let masks =
            std::array::from_fn(|p| ranges[p].combos.iter().map(|c| c.combo.mask()).collect());
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
        let totals = totals(&ranges[1], &initial_reach[1]);
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
        let mut tables = Vec::new();
        let mut table_ids = BTreeMap::new();
        let mut terminal_table = vec![None; tree.nodes.len()];
        for (id, node) in tree.nodes.iter().enumerate() {
            if !matches!(node.kind, NodeKind::Terminal { winner: None }) {
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
                let sorted = std::array::from_fn(|p| {
                    let mut order: Vec<_> = (0..ranks[p].len()).collect();
                    order.sort_unstable_by_key(|&i| (ranks[p][i], i));
                    order
                });
                let index = tables.len();
                tables.push(RankTable { ranks, sorted });
                index
            });
            terminal_table[id] = Some(table_id);
        }
        let n = [ranges[0].combos.len(), ranges[1].combos.len()];
        let mut state = State::new(&tree, tree.root, n, 0);
        let mut aliases: Vec<usize> = (0..tree.nodes.len()).collect();
        let merged_children = if config.suit_isomorphism && tree.config.bet_size.is_suit_invariant()
        {
            prepare_iso(&tree, &ranges, &mut state, &mut aliases)?
        } else {
            0
        };
        let regret = vec![0.0; state.span];
        let average = vec![0.0; state.span];
        let strategy = vec![0.0; state.span];
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.threads)
            .stack_size(1024 * 1024)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tree,
            ranges,
            config,
            state,
            masks,
            same,
            tables,
            terminal_table,
            initial_reach,
            joint_mass,
            iterations: 0,
            memory_bytes,
            pool: Arc::new(pool),
            aliases,
            merged_children,
            regret,
            average,
            strategy,
        })
    }
    /// One iteration is an OOP update followed by an IP update. No engine-owned
    /// heap buffers are allocated here; Rayon uses its initialized worker pool.
    pub fn iterate(&mut self) {
        self.iterations += 1;
        for player in 0..2 {
            self.run(player, Mode::Train);
        }
    }
    fn run(&mut self, player: usize, mode: Mode) -> f64 {
        self.state.reach[0].copy_from_slice(&self.initial_reach[0]);
        self.state.reach[1].copy_from_slice(&self.initial_reach[1]);
        let context = Context {
            tree: &self.tree,
            ranges: &self.ranges,
            masks: &self.masks,
            same: &self.same,
            tables: &self.tables,
            terminal_table: &self.terminal_table,
            config: &self.config,
            iteration: self.iterations,
        };
        let buffers = Buffers {
            regret: &mut self.regret,
            average: &mut self.average,
            strategy: &mut self.strategy,
        };
        if self.config.threads == 1 || self.pool.current_thread_index().is_some() {
            traverse(&context, &mut self.state, buffers, player, mode, true);
        } else {
            self.pool
                .install(|| traverse(&context, &mut self.state, buffers, player, mode, true));
        }
        self.state.value[..self.initial_reach[player].len()]
            .iter()
            .zip(&self.initial_reach[player])
            .map(|(v, w)| v * w)
            .sum::<f64>()
            / self.joint_mass
    }
    pub fn metrics(&mut self) -> Metrics {
        let average_ev = [self.run(0, Mode::Average), self.run(1, Mode::Average)];
        let best_response_ev = [
            self.run(0, Mode::BestResponse),
            self.run(1, Mode::BestResponse),
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
    /// Average action-major probabilities; zero for future-board-blocked hands.
    /// The returned result allocation is outside the iteration loop.
    pub fn strategy(&self, node: usize) -> Result<Vec<f32>, String> {
        let mut canonical = *self.aliases.get(node).ok_or("unknown node")?;
        while self.aliases[canonical] != canonical {
            canonical = self.aliases[canonical];
        }
        let s = self.state.find(canonical).ok_or("unknown node")?;
        let NodeKind::Decision {
            player, actions, ..
        } = &self.tree.nodes[node].kind
        else {
            return Err("node is not a decision".into());
        };
        let n = self.ranges[*player].combos.len();
        let mut out = vec![0.0; s.own_len];
        normalize(
            &self.average[s.offset..s.offset + s.own_len],
            &mut out,
            n,
            actions.len(),
            &self.masks[*player],
            board_mask(&self.tree.nodes[node].board),
        );
        Ok(out)
    }
    /// Install a complete solving loop once, avoiding Rayon external-injector
    /// queue allocations between calls to iterate/metrics. For threads=1 calls
    /// are already allocation-free and execute directly on the calling thread.
    pub fn with_pool<R: Send>(&mut self, f: impl FnOnce(&mut Self) -> R + Send) -> R {
        if self.config.threads == 1 {
            return f(self);
        }
        let pool = Arc::clone(&self.pool);
        pool.install(|| f(self))
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
        self.state.terminal_nanos() as f64 * 1e-9
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

// Conservative exact orbit reduction. Every private combo maps to itself, so
// both reach vectors and all decision vectors have the identity permutation.
// Aliases map skipped public subtrees back to the computed representative.
fn prepare_iso(
    tree: &GameTree,
    ranges: &[DenseRange; 2],
    state: &mut State,
    aliases: &mut [usize],
) -> Result<usize, String> {
    let mut merged = 0;
    if let NodeKind::Chance { cards, children } = &tree.nodes[state.id].kind {
        let board: Vec<Card> = tree.nodes[state.id]
            .board
            .iter()
            .map(|&c| Card(c))
            .collect();
        let permutations: Vec<_> = cards::board_preserving_permutations(&board)?
            .into_iter()
            .filter(|p| {
                ranges.iter().all(|r| {
                    r.combos
                        .iter()
                        .all(|c| p.map_combo(c.combo).id() == c.combo.id())
                })
            })
            .collect();
        for member in 0..cards.len() {
            for representative in 0..member {
                if state.chance_weights[representative] == 0 {
                    continue;
                }
                if let Some(&permutation) = permutations
                    .iter()
                    .find(|p| p.map_card(Card(cards[member])).0 == cards[representative])
                {
                    alias_subtree(
                        tree,
                        children[member],
                        children[representative],
                        permutation,
                        aliases,
                    )?;
                    state.chance_weights[member] = 0;
                    state.chance_weights[representative] += 1;
                    merged += 1;
                    break;
                }
            }
        }
    }
    for (child, &weight) in state.children.iter_mut().zip(&state.chance_weights) {
        if weight > 0 {
            merged += prepare_iso(tree, ranges, child, aliases)?;
        }
    }
    Ok(merged)
}
fn alias_subtree(
    tree: &GameTree,
    from: usize,
    to: usize,
    p: SuitPermutation,
    aliases: &mut [usize],
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
    aliases[from] = to;
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
                alias_subtree(tree, x, y, p, aliases)?;
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
                alias_subtree(tree, x, ab[j], p, aliases)?;
            }
        }
        (NodeKind::Terminal { winner: wa }, NodeKind::Terminal { winner: wb }) if wa == wb => {}
        _ => return Err("non-equivariant isomorphic actions".into()),
    }
    Ok(())
}

fn normalize(
    input: &[f32],
    output: &mut [f32],
    n: usize,
    actions: usize,
    masks: &[u64],
    board: u64,
) {
    for (i, &mask) in masks.iter().enumerate() {
        let sum: f64 = (0..actions).map(|a| input[a * n + i].max(0.0) as f64).sum();
        for a in 0..actions {
            output[a * n + i] = if mask & board != 0 {
                0.0
            } else if sum > 0.0 {
                (input[a * n + i].max(0.0) as f64 / sum) as f32
            } else {
                1.0 / actions as f32
            };
        }
    }
}
struct Buffers<'a> {
    regret: &'a mut [f32],
    average: &'a mut [f32],
    strategy: &'a mut [f32],
}
impl<'a> Buffers<'a> {
    fn split(self, at: usize) -> (Self, Self) {
        let (ar, br) = self.regret.split_at_mut(at);
        let (aa, ba) = self.average.split_at_mut(at);
        let (as_, bs) = self.strategy.split_at_mut(at);
        (
            Self {
                regret: ar,
                average: aa,
                strategy: as_,
            },
            Self {
                regret: br,
                average: ba,
                strategy: bs,
            },
        )
    }
}
fn traverse(
    ctx: &Context<'_>,
    state: &mut State,
    buffers: Buffers<'_>,
    target: usize,
    mode: Mode,
    parallel: bool,
) {
    let node = &ctx.tree.nodes[state.id];
    let n = ctx.ranges[target].combos.len();
    let (own, mut descendants) = buffers.split(state.own_len);
    match &node.kind {
        NodeKind::Terminal { winner } => {
            let start = ctx.config.profiling.then(Instant::now);
            terminal(ctx, state, target, *winner);
            if let Some(start) = start {
                state.terminal_nanos += start.elapsed().as_nanos();
            }
        }
        NodeKind::Chance { cards, .. } => {
            let probability = 1.0 / (52 - node.board.len() - 4) as f64;
            visit_chance(
                ctx,
                &mut state.children,
                cards,
                &state.chance_weights,
                &state.reach,
                descendants,
                target,
                mode,
                probability,
                parallel && ctx.config.threads > 1,
            );
            state.value[..n].fill(0.0);
            // Fixed public-card order, regardless of worker completion order.
            for (child, &weight) in state.children.iter().zip(&state.chance_weights) {
                if weight > 0 {
                    for i in 0..n {
                        state.value[i] += child.value[i] * weight as f64;
                    }
                }
            }
        }
        NodeKind::Decision {
            player, actions, ..
        } => {
            let actor = *player;
            let na = ctx.ranges[actor].combos.len();
            let strategy_source = if matches!(mode, Mode::Train) {
                &*own.regret
            } else {
                &*own.average
            };
            normalize(
                strategy_source,
                own.strategy,
                na,
                actions.len(),
                &ctx.masks[actor],
                board_mask(&node.board),
            );
            for (a, child) in state.children.iter_mut().enumerate() {
                for p in 0..2 {
                    for i in 0..state.reach[p].len() {
                        child.reach[p][i] = state.reach[p][i]
                            * if p == actor {
                                own.strategy[a * na + i] as f64
                            } else {
                                1.0
                            };
                    }
                }
                let (child_buffers, rest) = descendants.split(child.span);
                descendants = rest;
                traverse(ctx, child, child_buffers, target, mode, parallel);
            }
            for i in 0..n {
                state.value[i] = if actor != target {
                    state.children.iter().map(|c| c.value[i]).sum()
                } else if matches!(mode, Mode::BestResponse) {
                    state
                        .children
                        .iter()
                        .map(|c| c.value[i])
                        .fold(f64::NEG_INFINITY, f64::max)
                } else {
                    state
                        .children
                        .iter()
                        .enumerate()
                        .map(|(a, c)| own.strategy[a * na + i] as f64 * c.value[i])
                        .sum()
                };
            }
            if matches!(mode, Mode::Train) && actor == target {
                let t = ctx.iteration as f64;
                let positive_discount = t.powf(1.5) / (t.powf(1.5) + 1.0);
                let average_discount = (t / (t + 1.0)).powi(2);
                for (a, child) in state.children.iter().enumerate() {
                    for i in 0..na {
                        let idx = a * na + i;
                        let old = own.regret[idx] as f64;
                        let delta = child.value[i] - state.value[i];
                        match ctx.config.algorithm {
                            Algorithm::Dcfr => {
                                let r = old + delta;
                                own.regret[idx] =
                                    (r * if r >= 0.0 { positive_discount } else { 0.5 }) as f32;
                                own.average[idx] = ((own.average[idx] as f64
                                    + state.reach[actor][i] * own.strategy[idx] as f64)
                                    * average_discount)
                                    as f32;
                            }
                            Algorithm::CfrPlus => {
                                own.regret[idx] = (old + delta).max(0.0) as f32;
                                own.average[idx] +=
                                    (t * state.reach[actor][i] * own.strategy[idx] as f64) as f32;
                            }
                        }
                    }
                }
            }
        }
    }
}
// Recursive disjoint slice partitioning avoids allocating a vector of mutable
// work descriptors and guarantees no races in globally contiguous f32 slabs.
#[allow(clippy::too_many_arguments)]
fn visit_chance(
    ctx: &Context<'_>,
    children: &mut [State],
    cards: &[u8],
    weights: &[usize],
    reach: &[Vec<f64>; 2],
    buffers: Buffers<'_>,
    target: usize,
    mode: Mode,
    probability: f64,
    parallel: bool,
) {
    if children.len() > 1 {
        let midpoint = children.len() / 2;
        let span = children[..midpoint].iter().map(|s| s.span).sum();
        let (left_buf, right_buf) = buffers.split(span);
        let (left, right) = children.split_at_mut(midpoint);
        let a = || {
            visit_chance(
                ctx,
                left,
                &cards[..midpoint],
                &weights[..midpoint],
                reach,
                left_buf,
                target,
                mode,
                probability,
                parallel,
            )
        };
        let b = || {
            visit_chance(
                ctx,
                right,
                &cards[midpoint..],
                &weights[midpoint..],
                reach,
                right_buf,
                target,
                mode,
                probability,
                parallel,
            )
        };
        if parallel {
            rayon::join(a, b);
        } else {
            a();
            b();
        }
    } else if let Some(child) = children.first_mut() {
        if weights[0] == 0 {
            return;
        }
        let mask = 1u64 << cards[0];
        for (p, player_reach) in reach.iter().enumerate() {
            for (i, &weight) in player_reach.iter().enumerate() {
                child.reach[p][i] = if ctx.masks[p][i] & mask != 0 {
                    0.0
                } else {
                    weight * if p == target { 1.0 } else { probability }
                };
            }
        }
        traverse(ctx, child, buffers, target, mode, false);
    }
}

#[derive(Clone, Copy)]
struct Totals {
    sum: f64,
    cards: [f64; 52],
}
fn totals(range: &DenseRange, reach: &[f64]) -> Totals {
    let mut out = Totals {
        sum: 0.0,
        cards: [0.0; 52],
    };
    for (c, &w) in range.combos.iter().zip(reach) {
        add(&mut out, c.combo.cards, w);
    }
    out
}
fn add(t: &mut Totals, cards: [Card; 2], w: f64) {
    t.sum += w;
    t.cards[cards[0].0 as usize] += w;
    t.cards[cards[1].0 as usize] += w;
}
fn compatible(cards: [Card; 2], t: &Totals, same: f64) -> f64 {
    (t.sum - t.cards[cards[0].0 as usize] - t.cards[cards[1].0 as usize] + same).max(0.0)
}
fn terminal(ctx: &Context<'_>, state: &mut State, target: usize, winner: Option<usize>) {
    let node = &ctx.tree.nodes[state.id];
    let own = &ctx.ranges[target];
    let opp = &ctx.ranges[1 - target];
    let reach = &state.reach[1 - target];
    let all = totals(opp, reach);
    let pot = node.pot as f64 - ctx.tree.config.rake(node.pot);
    let base = node.invested[target] as f64 + ctx.tree.config.starting_pot as f64 / 2.0;
    let board = board_mask(&node.board);
    if let Some(winner) = winner {
        let payoff = if winner == target { pot - base } else { -base };
        for (i, c) in own.combos.iter().enumerate() {
            let same = ctx.same[target][i].map_or(0.0, |j| reach[j]);
            state.value[i] = if ctx.masks[target][i] & board != 0 {
                0.0
            } else {
                payoff * compatible(c.combo.cards, &all, same)
            };
        }
        return;
    }
    let table = &ctx.tables[ctx.terminal_table[state.id].expect("showdown table")];
    let mut weak = Totals {
        sum: 0.0,
        cards: [0.0; 52],
    };
    let mut cursor = 0;
    let mut own_cursor = 0;
    while own_cursor < table.sorted[target].len() {
        let first = table.sorted[target][own_cursor];
        let rank = table.ranks[target][first];
        while cursor < table.sorted[1 - target].len()
            && table.ranks[1 - target][table.sorted[1 - target][cursor]] < rank
        {
            let j = table.sorted[1 - target][cursor];
            add(&mut weak, opp.combos[j].combo.cards, reach[j]);
            cursor += 1;
        }
        let mut equal = Totals {
            sum: 0.0,
            cards: [0.0; 52],
        };
        let mut end = cursor;
        while end < table.sorted[1 - target].len()
            && table.ranks[1 - target][table.sorted[1 - target][end]] == rank
        {
            let j = table.sorted[1 - target][end];
            add(&mut equal, opp.combos[j].combo.cards, reach[j]);
            end += 1;
        }
        while own_cursor < table.sorted[target].len()
            && table.ranks[target][table.sorted[target][own_cursor]] == rank
        {
            let i = table.sorted[target][own_cursor];
            let c = own.combos[i].combo.cards;
            let same = ctx.same[target][i].map_or(0.0, |j| reach[j]);
            let total = compatible(c, &all, same);
            let wins = compatible(c, &weak, 0.0);
            let ties = compatible(c, &equal, same);
            state.value[i] = if ctx.masks[target][i] & board != 0 {
                0.0
            } else {
                pot * (wins + 0.5 * ties) - base * total
            };
            own_cursor += 1;
        }
        // Incremental card-prefix totals: each opposing hand enters once.
        for k in cursor..end {
            let j = table.sorted[1 - target][k];
            add(&mut weak, opp.combos[j].combo.cards, reach[j]);
        }
        cursor = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                            (naive - e.state.value[i]).abs() < 1e-7,
                            "case {case} target{target} combo{i}: {naive} != {}",
                            e.state.value[i]
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
        e.average[start..start + 2].copy_from_slice(&[0.0, 1.0]);
        let br = e.run(0, Mode::BestResponse);
        let mut exhaustive = f64::NEG_INFINITY;
        for first in 0..2 {
            for second in 0..2 {
                let choices = [first, second];
                let root = e.state.find(0).unwrap().offset;
                e.average[root..root + 4].fill(0.0);
                for (hand, &choice) in choices.iter().enumerate() {
                    e.average[root + choice * 2 + hand] = 1.0;
                }
                exhaustive = exhaustive.max(e.run(0, Mode::Average));
            }
        }
        assert!((br - exhaustive).abs() < 1e-10);
        assert!((br - 250.0).abs() < 1e-10);
        // Bettor pure bet with both hands: defender's exhaustive call/fold choices.
        let root = e.state.find(0).unwrap().offset;
        e.average[root..root + 4].copy_from_slice(&[0.0, 0.0, 1.0, 1.0]);
        let br = e.run(1, Mode::BestResponse);
        let start = e.state.find(2).unwrap().offset;
        e.average[start..start + 2].copy_from_slice(&[1.0, 0.0]);
        let fold = e.run(1, Mode::Average);
        e.average[start..start + 2].copy_from_slice(&[0.0, 1.0]);
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
        fn mass(s: &State) -> f64 {
            if s.children.is_empty() {
                s.reach[0][0] * s.reach[1][0]
            } else {
                s.children.iter().map(mass).sum()
            }
        }
        assert!((mass(&e.state) - 1.0).abs() < 1e-12);
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
        assert_eq!(
            a.regret.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            b.regret.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
        assert_eq!(
            a.average.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            b.average.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
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
