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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Street {
    Flop,
    Turn,
    River,
}
#[derive(Clone, Debug)]
pub struct SizeContext<'a> {
    pub board: &'a [u8],
    pub street: Street,
    pub player: usize,
    pub first_bet: bool,
    pub after_check: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum SizeRule {
    Fixed(f64),
    #[serde(skip)]
    Custom(Arc<dyn Fn(&SizeContext<'_>) -> f64 + Send + Sync>),
}
impl std::fmt::Debug for SizeRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(v) => f.debug_tuple("Fixed").field(v).finish(),
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}
impl SizeRule {
    /// Only fixed rules guarantee invariance under every suit permutation.
    pub fn is_suit_invariant(&self) -> bool {
        matches!(self, Self::Fixed(_))
    }
    pub fn fraction(&self, c: &SizeContext<'_>) -> f64 {
        match self {
            Self::Fixed(v) => *v,
            Self::Custom(f) => f(c),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeConfig {
    pub starting_pot: i64,
    pub effective_stack: i64,
    pub oop_can_lead_flop: bool,
    pub bet_size: SizeRule,
    pub raise_multiplier: f64,
    pub allin_merge_ratio: f64,
    pub rake_rate: f64,
    pub rake_cap: i64,
    pub allow_raises: bool,
}
impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            starting_pot: 5_000,
            effective_stack: 97_500,
            oop_can_lead_flop: false,
            bet_size: SizeRule::Fixed(0.33),
            raise_multiplier: 2.5,
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
            board,
            pot: config.starting_pot,
            stacks: [config.effective_stack; 2],
            invested: [0; 2],
            street_paid: [0; 2],
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
        if n as f64 >= stack as f64 * self.config.allin_merge_ratio {
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
            next.street_paid = [0; 2];
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
                });
                if !fraction.is_finite() || fraction <= 0.0 {
                    return Err("bet size must be finite and positive".into());
                }
                let n = self.merged(
                    ((s.pot as f64 * fraction).round() as i64).max(1),
                    s.stacks[p],
                );
                let mut bet = s.clone();
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
