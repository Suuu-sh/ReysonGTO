//! Clean-room public betting tree. Chip amounts use 1 BB = 1,000 units.
use serde::{Deserialize, Serialize};
use std::sync::Arc;
pub const BB: i64 = 1_000;
pub type NodeId = usize;
/// Default protects direct callers from unbounded arena construction.
pub const DEFAULT_NODE_LIMIT: usize = 1_000_000;
fn default_node_limit() -> usize {
    DEFAULT_NODE_LIMIT
}
fn default_allin_merge_enabled() -> bool {
    true
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Street {
    Flop,
    Turn,
    River,
}
/// Completed action line on the immediately preceding street.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviousStreetLine {
    CheckedThrough,
    IpBetCalled,
    LeadCalled,
    Raised,
}

/// Legacy true only says a wager occurred, so its exact line stays unknown.
pub fn deserialize_previous_street_line<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PreviousStreetLine>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum History {
        Line(PreviousStreetLine),
        Legacy(bool),
    }
    Ok(match Option::<History>::deserialize(deserializer)? {
        Some(History::Line(line)) => Some(line),
        Some(History::Legacy(false)) => Some(PreviousStreetLine::CheckedThrough),
        Some(History::Legacy(true)) | None => None,
    })
}

impl PreviousStreetLine {
    fn after_action(self, player: usize, action: &Action) -> Self {
        match action {
            Action::Raise(_) => Self::Raised,
            Action::Bet(_) if player == 1 => Self::IpBetCalled,
            Action::Bet(_) => Self::LeadCalled,
            _ => self,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SizeContext<'a> {
    pub board: &'a [u8],
    pub street: Street,
    pub player: usize,
    pub first_bet: bool,
    pub after_check: bool,
    /// Exact preceding street line; unknown at standalone roots.
    pub previous_street_line: Option<PreviousStreetLine>,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum SizeRule {
    Fixed(f64),
    /// Serializable, suit-independent fractions for each street.
    PerStreet {
        flop: f64,
        turn: f64,
        river: f64,
    },
    #[serde(skip)]
    Custom(Arc<dyn Fn(&SizeContext<'_>) -> f64 + Send + Sync>),
    /// Explicit opt-in for a pure callback invariant under suit relabeling.
    /// Engine construction checks deterministic samples and the actual alias
    /// subtrees; a failed check disables isomorphism, not the sizing rule.
    #[serde(skip)]
    SuitInvariant(Arc<dyn Fn(&SizeContext<'_>) -> f64 + Send + Sync>),
}
impl std::fmt::Debug for SizeRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(v) => f.debug_tuple("Fixed").field(v).finish(),
            Self::PerStreet { flop, turn, river } => f
                .debug_struct("PerStreet")
                .field("flop", flop)
                .field("turn", turn)
                .field("river", river)
                .finish(),
            Self::Custom(_) => f.write_str("Custom"),
            Self::SuitInvariant(_) => f.write_str("SuitInvariant"),
        }
    }
}
impl SizeRule {
    /// Whether the rule is eligible for checked suit isomorphism. A callback's
    /// declaration alone is not proof: also call check_suit_invariance.
    pub fn is_suit_invariant(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }
    pub fn fraction(&self, c: &SizeContext<'_>) -> f64 {
        match self {
            Self::Fixed(v) => *v,
            Self::PerStreet { flop, turn, river } => match c.street {
                Street::Flop => *flop,
                Street::Turn => *turn,
                Street::River => *river,
            },
            Self::Custom(f) | Self::SuitInvariant(f) => f(c),
        }
    }
    /// Deterministic random-board validation, using all 24 suit permutations
    /// and every player/check/first-bet context on every sampled street.
    /// This is a falsification test, not a proof over all boards. The engine
    /// additionally verifies actual action/amount equality before aliasing.
    pub fn check_suit_invariance(&self, seed: u64, samples: usize) -> Result<(), String> {
        if !self.is_suit_invariant() {
            return Err("custom sizing rule did not declare suit invariance".into());
        }
        if samples == 0 {
            return Err("suit invariance validation needs at least one sample".into());
        }
        let mut random = seed.max(1);
        for sample in 0..samples {
            let len = 3 + sample % 3;
            let street = match len {
                3 => Street::Flop,
                4 => Street::Turn,
                _ => Street::River,
            };
            let mut deck: [u8; 52] = std::array::from_fn(|i| i as u8);
            for i in 0..len {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                deck.swap(i, i + random as usize % (52 - i));
            }
            let board = &deck[..len];
            for player in 0..2 {
                for first_bet in [false, true] {
                    for after_check in [false, true] {
                        for previous_street_line in [
                            None,
                            Some(PreviousStreetLine::CheckedThrough),
                            Some(PreviousStreetLine::IpBetCalled),
                            Some(PreviousStreetLine::LeadCalled),
                            Some(PreviousStreetLine::Raised),
                        ] {
                            let context = SizeContext {
                                board,
                                street,
                                player,
                                first_bet,
                                after_check,
                                previous_street_line,
                            };
                            let expected = self.fraction(&context);
                            if !expected.is_finite() || expected <= 0.0 {
                                return Err(format!("invalid sizing fraction in sample {sample}"));
                            }
                            for a in 0..4 {
                                for b in 0..4 {
                                    for c in 0..4 {
                                        if a == b || a == c || b == c {
                                            continue;
                                        }
                                        let d = 6 - a - b - c;
                                        let permutation = [a, b, c, d];
                                        let mapped = board
                                            .iter()
                                            .map(|&card| {
                                                card / 4 * 4 + permutation[card as usize % 4]
                                            })
                                            .collect::<Vec<_>>();
                                        let actual = self.fraction(&SizeContext {
                                            board: &mapped,
                                            ..context.clone()
                                        });
                                        if actual != expected {
                                            return Err(format!(
                                            "sizing rule changes under suit permutation in sample {sample} ({street:?}, player {player}, first_bet {first_bet}, after_check {after_check})"
                                        ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeConfig {
    #[serde(
        default,
        alias = "previous_street_bet",
        deserialize_with = "deserialize_previous_street_line",
        skip_serializing_if = "Option::is_none"
    )]
    pub previous_street_line: Option<PreviousStreetLine>,
    pub starting_pot: i64,
    pub effective_stack: i64,
    pub oop_can_lead_flop: bool,
    pub bet_size: SizeRule,
    pub raise_multiplier: f64,
    /// Disable for reference trees that cap wagers only at the stack limit.
    /// Missing fields in M1 JSON preserve the original merge behavior.
    #[serde(default = "default_allin_merge_enabled")]
    pub allin_merge_enabled: bool,
    pub allin_merge_ratio: f64,
    pub rake_rate: f64,
    pub rake_cap: i64,
    pub allow_raises: bool,
}
impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            previous_street_line: None,
            starting_pot: 5_500,
            effective_stack: 97_500,
            oop_can_lead_flop: false,
            bet_size: SizeRule::PerStreet {
                flop: 0.33,
                turn: 0.66,
                river: 0.66,
            },
            raise_multiplier: 2.5,
            allin_merge_enabled: true,
            allin_merge_ratio: 0.67,
            rake_rate: 0.05,
            rake_cap: 3 * BB,
            allow_raises: true,
        }
    }
}
impl TreeConfig {
    pub fn rake(&self, pot: i64) -> f64 {
        (pot as f64 * self.rake_rate).min(self.rake_cap as f64)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Check,
    Fold,
    Call,
    Bet(i64),
    /// Additional chips paid, not the total wager on this street.
    Raise(i64),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeKind {
    Decision {
        player: usize,
        actions: Vec<Action>,
        children: Vec<NodeId>,
    },
    Chance {
        cards: Vec<u8>,
        children: Vec<NodeId>,
    },
    Terminal {
        winner: Option<usize>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub board: Vec<u8>,
    pub pot: i64,
    pub stacks: [i64; 2],
    pub invested: [i64; 2],
    pub kind: NodeKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameTree {
    pub config: TreeConfig,
    pub nodes: Vec<Node>,
    pub root: NodeId,
    #[serde(skip, default = "default_node_limit")]
    max_nodes: usize,
}
#[derive(Clone)]
struct State {
    board: Vec<u8>,
    pot: i64,
    stacks: [i64; 2],
    invested: [i64; 2],
    street_paid: [i64; 2],
    street_line: PreviousStreetLine,
    previous_street_line: Option<PreviousStreetLine>,
}
impl GameTree {
    /// Constructs an explicit arena for synthetic games.
    /// This does not validate edges or game semantics; Engine::new validates
    /// the supplied arena before solving. Prefer build for poker trees.
    pub fn from_nodes(config: TreeConfig, nodes: Vec<Node>, root: NodeId) -> Self {
        Self {
            config,
            nodes,
            root,
            max_nodes: DEFAULT_NODE_LIMIT,
        }
    }
    pub fn build(board: Vec<u8>, config: TreeConfig) -> Result<Self, String> {
        Self::build_with_node_limit(board, config, DEFAULT_NODE_LIMIT)
    }
    /// Stops before adding a node beyond the caller's memory-derived bound.
    pub fn build_with_node_limit(
        board: Vec<u8>,
        config: TreeConfig,
        max_nodes: usize,
    ) -> Result<Self, String> {
        if !(3..=5).contains(&board.len())
            || board.iter().any(|&c| c >= 52)
            || board
                .iter()
                .enumerate()
                .any(|(i, c)| board[..i].contains(c))
        {
            return Err("board must contain 3, 4, or 5 distinct cards in 0..52".into());
        }
        if config.starting_pot <= 0
            || config.effective_stack < 0
            || !config.raise_multiplier.is_finite()
            || config.raise_multiplier < 2.0
            || !config.allin_merge_ratio.is_finite()
            || config.allin_merge_ratio <= 0.0
            || config.allin_merge_ratio > 1.0
            || !config.rake_rate.is_finite()
            || !(0.0..=1.0).contains(&config.rake_rate)
            || config.rake_cap < 0
            || config
                .effective_stack
                .checked_mul(2)
                .and_then(|v| config.starting_pot.checked_add(v))
                .is_none()
        {
            return Err("invalid tree configuration".into());
        }
        let state = State {
            board: board.clone(),
            pot: config.starting_pot,
            stacks: [config.effective_stack; 2],
            invested: [0; 2],
            street_paid: [0; 2],
            street_line: PreviousStreetLine::CheckedThrough,
            previous_street_line: if board.len() == 3 {
                None
            } else {
                config.previous_street_line
            },
        };
        let mut tree = Self {
            config,
            nodes: Vec::new(),
            root: 0,
            max_nodes,
        };
        tree.root = if state.stacks[0] == 0 {
            tree.finish(state)?
        } else {
            tree.decision(state, 0, false)?
        };
        Ok(tree)
    }
    /// Recover preceding-street action history, including pre-deal chance nodes.
    pub fn previous_street_line_at(&self, target: NodeId) -> Option<PreviousStreetLine> {
        let previous = if self.nodes.get(self.root)?.board.len() == 3 {
            None
        } else {
            self.config.previous_street_line
        };
        let mut pending = vec![(self.root, previous, PreviousStreetLine::CheckedThrough)];
        while let Some((id, previous, bet)) = pending.pop() {
            let node = self.nodes.get(id)?;
            let boundary = matches!(node.kind, NodeKind::Chance { .. });
            if id == target {
                return if boundary { Some(bet) } else { previous };
            }
            match &node.kind {
                NodeKind::Decision {
                    actions,
                    children,
                    player,
                } => {
                    for (action, &child) in actions.iter().zip(children) {
                        pending.push((child, previous, bet.after_action(*player, action)));
                    }
                }
                NodeKind::Chance { children, .. } => {
                    for &child in children {
                        pending.push((child, Some(bet), PreviousStreetLine::CheckedThrough));
                    }
                }
                NodeKind::Terminal { .. } => {}
            }
        }
        None
    }
    fn push(&mut self, s: &State, kind: NodeKind) -> Result<NodeId, String> {
        if self.nodes.len() >= self.max_nodes {
            return Err(format!("tree node limit exceeded ({})", self.max_nodes));
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            board: s.board.clone(),
            pot: s.pot,
            stacks: s.stacks,
            invested: s.invested,
            kind,
        });
        Ok(id)
    }
    fn spend(s: &mut State, p: usize, n: i64) {
        s.stacks[p] -= n;
        s.invested[p] += n;
        s.street_paid[p] += n;
        s.pot += n;
    }
    fn merged(&self, n: i64, stack: i64) -> i64 {
        if self.config.allin_merge_enabled
            && n as f64 >= stack as f64 * self.config.allin_merge_ratio
        {
            stack
        } else {
            n.min(stack)
        }
    }
    fn finish(&mut self, s: State) -> Result<NodeId, String> {
        if s.board.len() == 5 {
            return self.push(&s, NodeKind::Terminal { winner: None });
        }
        let id = self.push(
            &s,
            NodeKind::Chance {
                cards: vec![],
                children: vec![],
            },
        )?;
        let mut cards = Vec::new();
        let mut children = Vec::new();
        for card in 0..52 {
            if s.board.contains(&card) {
                continue;
            }
            let mut next = s.clone();
            next.board.push(card);
            next.previous_street_line = Some(s.street_line);
            next.street_paid = [0; 2];
            next.street_line = PreviousStreetLine::CheckedThrough;
            cards.push(card);
            children.push(if next.stacks[0] == 0 || next.stacks[1] == 0 {
                self.finish(next)?
            } else {
                self.decision(next, 0, false)?
            });
        }
        self.nodes[id].kind = NodeKind::Chance { cards, children };
        Ok(id)
    }
    fn decision(&mut self, s: State, p: usize, checked: bool) -> Result<NodeId, String> {
        let q = 1 - p;
        let due = s.street_paid[q] - s.street_paid[p];
        let id = self.push(&s, NodeKind::Terminal { winner: None })?;
        let mut actions = Vec::new();
        let mut children = Vec::new();
        if due > 0 {
            actions.push(Action::Fold);
            children.push(self.push(&s, NodeKind::Terminal { winner: Some(q) })?);
            let mut call = s.clone();
            let call_amount = due.min(call.stacks[p]);
            Self::spend(&mut call, p, call_amount);
            actions.push(Action::Call);
            children.push(self.finish(call)?);
            if self.config.allow_raises && s.stacks[p] > due && s.stacks[q] > 0 {
                let target =
                    (s.street_paid[q] as f64 * self.config.raise_multiplier).round() as i64;
                let spend = self.merged((target - s.street_paid[p]).max(due + 1), s.stacks[p]);
                let mut raise = s.clone();
                raise.street_line = PreviousStreetLine::Raised;
                Self::spend(&mut raise, p, spend);
                actions.push(Action::Raise(spend));
                children.push(self.decision(raise, q, false)?);
            }
        } else {
            actions.push(Action::Check);
            children.push(if checked {
                self.finish(s.clone())?
            } else {
                self.decision(s.clone(), q, true)?
            });
            if !(s.board.len() == 3 && p == 0 && !checked && !self.config.oop_can_lead_flop)
                && s.stacks[p] > 0
                && s.stacks[q] > 0
            {
                let street = match s.board.len() {
                    3 => Street::Flop,
                    4 => Street::Turn,
                    _ => Street::River,
                };
                let fraction = self.config.bet_size.fraction(&SizeContext {
                    board: &s.board,
                    street,
                    player: p,
                    first_bet: true,
                    after_check: checked,
                    previous_street_line: s.previous_street_line,
                });
                if !fraction.is_finite() || fraction <= 0.0 {
                    return Err("bet size must be finite and positive".into());
                }
                let n = self.merged(
                    ((s.pot as f64 * fraction).round() as i64).max(1),
                    s.stacks[p],
                );
                let mut bet = s.clone();
                bet.street_line = s.street_line.after_action(p, &Action::Bet(n));
                Self::spend(&mut bet, p, n);
                actions.push(Action::Bet(n));
                children.push(self.decision(bet, q, false)?);
            }
        }
        self.nodes[id].kind = NodeKind::Decision {
            player: p,
            actions,
            children,
        };
        Ok(id)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> TreeConfig {
        TreeConfig {
            starting_pot: 1_000,
            effective_stack: 10_000,
            oop_can_lead_flop: true,
            bet_size: SizeRule::Fixed(0.5),
            rake_rate: 0.0,
            ..Default::default()
        }
    }
    fn actions(t: &GameTree, id: usize) -> (&[Action], &[usize]) {
        match &t.nodes[id].kind {
            NodeKind::Decision {
                actions, children, ..
            } => (actions, children),
            _ => panic!("not decision"),
        }
    }
    #[test]
    fn previous_street_history_tracks_checks_bets_and_raises() {
        let cfg = TreeConfig {
            previous_street_line: Some(PreviousStreetLine::IpBetCalled),
            allow_raises: false,
            bet_size: SizeRule::SuitInvariant(Arc::new(|c| {
                if c.previous_street_line == Some(PreviousStreetLine::IpBetCalled) {
                    1.0
                } else {
                    0.5
                }
            })),
            ..config()
        };
        let tree = GameTree::build(vec![0, 5, 10], cfg).unwrap();
        assert_eq!(tree.previous_street_line_at(tree.root), None);
        let follow = |id, index| actions(&tree, id).1[index];
        let xx = follow(follow(tree.root, 0), 0);
        let bc = follow(follow(follow(tree.root, 0), 1), 1);
        for (chance, history) in [
            (xx, PreviousStreetLine::CheckedThrough),
            (bc, PreviousStreetLine::IpBetCalled),
        ] {
            assert_eq!(tree.previous_street_line_at(chance), Some(history));
            let turn = match &tree.nodes[chance].kind {
                NodeKind::Chance { children, .. } => children[0],
                _ => panic!("chance"),
            };
            assert_eq!(tree.previous_street_line_at(turn), Some(history));
            let bet = actions(&tree, turn).0[1];
            assert_eq!(
                bet,
                Action::Bet(
                    (tree.nodes[turn].pot as f64
                        * if history == PreviousStreetLine::IpBetCalled {
                            1.0
                        } else {
                            0.5
                        })
                    .round() as i64
                )
            );
            let river_chance = follow(follow(turn, 0), 0);
            assert_eq!(
                tree.previous_street_line_at(river_chance),
                Some(PreviousStreetLine::CheckedThrough)
            );
        }
        let tree = GameTree::build(vec![0, 5, 10, 15], config()).unwrap();
        let bet = actions(&tree, tree.root).1[1];
        let raise = actions(&tree, bet).1[2];
        let call = actions(&tree, raise).1[1];
        assert_eq!(
            tree.previous_street_line_at(call),
            Some(PreviousStreetLine::Raised)
        );
        let lead_call = actions(&tree, bet).1[1];
        assert_eq!(
            tree.previous_street_line_at(lead_call),
            Some(PreviousStreetLine::LeadCalled)
        );
        let ip_bet = actions(&tree, actions(&tree, tree.root).1[0]).1[1];
        let ip_call = actions(&tree, ip_bet).1[1];
        assert_eq!(
            tree.previous_street_line_at(ip_call),
            Some(PreviousStreetLine::IpBetCalled)
        );
        let check_raise = actions(&tree, ip_bet).1[2];
        let check_raise_call = actions(&tree, check_raise).1[1];
        assert_eq!(
            tree.previous_street_line_at(check_raise_call),
            Some(PreviousStreetLine::Raised)
        );
        for (chance, line) in [
            (lead_call, PreviousStreetLine::LeadCalled),
            (check_raise_call, PreviousStreetLine::Raised),
        ] {
            let NodeKind::Chance { children, .. } = &tree.nodes[chance].kind else {
                panic!("chance")
            };
            assert_eq!(tree.previous_street_line_at(children[0]), Some(line));
        }
    }

    #[test]
    fn fixed_sizing_is_unchanged_by_previous_street_line() {
        let baseline = GameTree::build(vec![0, 5, 10, 15], config()).unwrap();
        for line in [
            PreviousStreetLine::CheckedThrough,
            PreviousStreetLine::IpBetCalled,
            PreviousStreetLine::LeadCalled,
            PreviousStreetLine::Raised,
        ] {
            let tree = GameTree::build(
                vec![0, 5, 10, 15],
                TreeConfig {
                    previous_street_line: Some(line),
                    ..config()
                },
            )
            .unwrap();
            assert_eq!(format!("{:?}", tree.nodes), format!("{:?}", baseline.nodes));
        }
    }

    #[test]
    fn raise_targets_street_total_and_merge_is_incremental() {
        let t = GameTree::build(vec![0, 5, 10, 15, 20], config()).unwrap();
        let (a, c) = actions(&t, t.root);
        assert_eq!(a, &[Action::Check, Action::Bet(500)]);
        let (a, c) = actions(&t, c[1]);
        assert_eq!(a, &[Action::Fold, Action::Call, Action::Raise(1250)]);
        let (a, _) = actions(&t, c[2]);
        assert_eq!(a[2], Action::Raise(2625));
        let mut cfg = config();
        cfg.effective_stack = 700;
        let t = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
        assert_eq!(actions(&t, 0).0[1], Action::Bet(700));
    }
    #[test]
    fn reference_merge_disabled_preserves_raise_totals_until_stack_cap() {
        let cfg = TreeConfig {
            starting_pot: 360_448,      // 5.5 BB at the reference's 65,536 units/BB
            effective_stack: 6_389_760, // 97.5 BB
            allin_merge_enabled: false,
            ..config()
        };
        let t = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
        let mut id = actions(&t, t.root).1[1];
        let mut totals = vec![180_224];
        loop {
            let (a, c) = actions(&t, id);
            let Some(i) = a.iter().position(|a| matches!(a, Action::Raise(_))) else {
                break;
            };
            id = c[i];
            totals.push(t.nodes[id].invested.iter().copied().max().unwrap());
        }
        assert_eq!(
            totals,
            vec![180_224, 450_560, 1_126_400, 2_816_000, 6_389_760]
        );
        // Disabling merging retains wagers below the stack limit, even when
        // their incremental payment would cross the default 67% threshold.
        let mut cfg = config();
        cfg.effective_stack = 700;
        cfg.allin_merge_enabled = false;
        let t = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
        assert_eq!(actions(&t, t.root).0[1], Action::Bet(500));
    }
    #[test]
    fn no_donk_and_fold_only_facing_wager() {
        let mut cfg = config();
        cfg.effective_stack = 1;
        cfg.oop_can_lead_flop = false;
        let t = GameTree::build(vec![0, 5, 10], cfg).unwrap();
        assert_eq!(actions(&t, 0).0, &[Action::Check]);
        for n in &t.nodes {
            if let NodeKind::Decision { actions, .. } = &n.kind {
                assert!(!(actions.contains(&Action::Check) && actions.contains(&Action::Fold)));
            }
        }
    }
    #[test]
    fn allin_runout_has_no_decisions_and_all_public_cards() {
        let mut cfg = config();
        cfg.effective_stack = 0;
        let t = GameTree::build(vec![0, 5, 10, 15], cfg).unwrap();
        assert_eq!(t.nodes.len(), 49);
        match &t.nodes[0].kind {
            NodeKind::Chance { cards, children } => {
                assert_eq!(cards.len(), 48);
                assert_eq!(children.len(), 48);
                assert!(!cards.contains(&0));
            }
            _ => panic!(),
        };
        assert!(t.nodes[1..]
            .iter()
            .all(|n| matches!(n.kind, NodeKind::Terminal { winner: None })));
    }
    #[test]
    fn rake_cap_and_invalid_board() {
        let c = TreeConfig::default();
        assert_eq!(c.rake(10_000), 500.0);
        assert_eq!(c.rake(100_000), 3_000.0);
        assert!(GameTree::build(vec![0, 0, 1], c).is_err());
    }
    #[test]
    fn board_dependent_size_and_raises_disabled() {
        let mut cfg = config();
        cfg.allow_raises = false;
        cfg.bet_size = SizeRule::Custom(Arc::new(|c| {
            if c.board.len() == 5 && c.player == 0 {
                0.25
            } else {
                0.5
            }
        }));
        let t = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
        let (a, c) = actions(&t, 0);
        assert_eq!(a[1], Action::Bet(250));
        assert_eq!(actions(&t, c[1]).0, &[Action::Fold, Action::Call]);
    }
    #[test]
    fn production_defaults_round_bets_and_raise_without_a_count_cap() {
        let cfg = TreeConfig::default();
        assert_eq!(cfg.starting_pot, 5_500);
        assert_eq!(cfg.effective_stack, 97_500);
        assert!(!cfg.oop_can_lead_flop);
        assert_eq!(cfg.raise_multiplier, 2.5);
        assert!(cfg.allow_raises && cfg.allin_merge_enabled);
        assert_eq!(cfg.allin_merge_ratio, 0.67);
        assert_eq!(cfg.rake_rate, 0.05);
        assert_eq!(cfg.rake_cap, 3 * BB);
        for (street, fraction) in [
            (Street::Flop, 0.33),
            (Street::Turn, 0.66),
            (Street::River, 0.66),
        ] {
            assert_eq!(
                cfg.bet_size.fraction(&SizeContext {
                    board: &[],
                    street,
                    player: 0,
                    first_bet: true,
                    after_check: false,
                    previous_street_line: None,
                }),
                fraction
            );
        }
        let tree = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
        let (root_actions, root_children) = actions(&tree, tree.root);
        assert_eq!(root_actions, &[Action::Check, Action::Bet(3_630)]);
        let mut id = root_children[1];
        let mut totals = vec![3_630];
        while let Some(index) = actions(&tree, id)
            .0
            .iter()
            .position(|a| matches!(a, Action::Raise(_)))
        {
            id = actions(&tree, id).1[index];
            totals.push(*tree.nodes[id].invested.iter().max().unwrap());
        }
        assert_eq!(totals, [3_630, 9_075, 22_688, 56_720, 97_500]);
        assert_eq!(actions(&tree, id).0, &[Action::Fold, Action::Call]);
    }
    #[test]
    fn allin_merge_is_inclusive_and_uses_remaining_incremental_payment() {
        for (fraction, expected) in [(0.669, 669), (0.67, 1_000)] {
            let cfg = TreeConfig {
                starting_pot: 1_000,
                effective_stack: 1_000,
                bet_size: SizeRule::Fixed(fraction),
                ..config()
            };
            let tree = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
            assert_eq!(actions(&tree, tree.root).0[1], Action::Bet(expected));
        }
        for (stack, expected) in [(746, 746), (747, 500)] {
            let cfg = TreeConfig {
                starting_pot: 400,
                effective_stack: stack,
                ..config()
            };
            let tree = GameTree::build(vec![0, 5, 10, 15, 20], cfg).unwrap();
            let response = actions(&tree, tree.root).1[1];
            assert_eq!(actions(&tree, response).0[2], Action::Raise(expected));
        }
    }
    #[test]
    fn suit_invariant_checker_covers_board_features_and_contexts() {
        let rule = SizeRule::SuitInvariant(Arc::new(|c| {
            let paired = c
                .board
                .iter()
                .enumerate()
                .any(|(i, card)| c.board[..i].iter().any(|earlier| earlier / 4 == card / 4));
            let monotone = c.board.iter().all(|card| card % 4 == c.board[0] % 4);
            0.25 + 0.1 * c.player as f64
                + if paired || monotone || c.after_check {
                    0.25
                } else {
                    0.0
                }
        }));
        assert!(rule.check_suit_invariance(12345, 64).is_ok());
        for rule in [
            SizeRule::SuitInvariant(Arc::new(|c| if c.board[0] % 4 == 0 { 0.25 } else { 0.5 })),
            SizeRule::SuitInvariant(Arc::new(|c| {
                if c.player == 1 && !c.first_bet && c.after_check {
                    c.board[0] as f64 + 1.0
                } else {
                    0.5
                }
            })),
        ] {
            let first = rule.check_suit_invariance(12345, 64).unwrap_err();
            assert!(first.contains("suit permutation"));
            assert_eq!(first, rule.check_suit_invariance(12345, 64).unwrap_err());
        }
        assert!(SizeRule::Custom(Arc::new(|_| 0.5))
            .check_suit_invariance(12345, 64)
            .is_err());
        assert!(SizeRule::Fixed(0.5)
            .check_suit_invariance(12345, 0)
            .is_err());
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    #[test]
    fn memory_node_bound_stops_construction() {
        let cfg = TreeConfig {
            effective_stack: 0,
            ..Default::default()
        };
        assert!(
            GameTree::build_with_node_limit(vec![0, 1, 2, 3], cfg.clone(), 48)
                .unwrap_err()
                .contains("node limit")
        );
        assert_eq!(
            GameTree::build_with_node_limit(vec![0, 1, 2, 3], cfg, 49)
                .unwrap()
                .nodes
                .len(),
            49
        );
    }
}
