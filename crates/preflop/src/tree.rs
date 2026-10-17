//! Finite preflop betting tree. All state transitions use integer 0.001BB chips.
use crate::config::{bb, chips, PreflopConfig};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Fold,
    Check,
    Call,
    RaiseTo(f64),
    AllIn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalKind {
    Fold,
    Flop,
    AllIn,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Terminal {
    pub kind: TerminalKind,
    pub contributions: Vec<f64>,
    pub active: Vec<bool>,
    pub winner: Option<usize>,
    /// 0 = limp, 1 = open/iso, 2 = 3bet, 3 = 4bet, 4 = 5bet, 5 = 6bet jam.
    pub stage: u8,
    pub dead_blind_bb: f64,
}
impl Terminal {
    pub fn pot_bb(&self) -> f64 {
        self.contributions.iter().sum::<f64>() + self.dead_blind_bb
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Node {
    Decision {
        actor: usize,
        actions: Vec<Action>,
        children: Vec<usize>,
    },
    Terminal(Terminal),
}
#[derive(Clone, Debug)]
pub struct PreflopTree {
    pub config: PreflopConfig,
    pub nodes: Vec<Node>,
    pub root: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TreeStats {
    pub nodes: usize,
    pub decisions: usize,
    pub terminals: usize,
    pub fold_terminals: usize,
    pub flop_terminals: usize,
    pub all_in_terminals: usize,
    pub max_depth: usize,
    pub decisions_by_seat: Vec<usize>,
    pub max_stage: u8,
}
#[derive(Clone)]
struct State {
    contributions: Vec<i64>,
    active: u8,
    pending: u8,
    entered: u8,
    actor: usize,
    target: i64,
    last_increment: i64,
    stage: u8,
    aggressor: Option<usize>,
    opener: Option<usize>,
    callers: usize,
    sb_limp: bool,
}
impl PreflopTree {
    pub fn build(config: PreflopConfig) -> Result<Self> {
        Self::build_mode(config, false)
    }
    /// Frozen P1-P3 betting abstraction for explicit historical HU regression only.
    pub fn build_legacy(config: PreflopConfig) -> Result<Self> {
        Self::build_mode(config, true)
    }
    fn build_mode(config: PreflopConfig, legacy: bool) -> Result<Self> {
        config.validate()?;
        let state = Self::initial_state(&config);
        let mut tree = Self {
            config,
            nodes: Vec::new(),
            root: 0,
        };
        tree.root = tree.expand(state, legacy)?;
        Ok(tree)
    }
    /// Synthetic shove-or-fold / call-or-fold toy, supporting 2..=6 seats.
    /// HU preserves the historical five-node game exactly.
    pub fn push_fold(config: PreflopConfig) -> Result<Self> {
        config.validate()?;
        if config.players == 2 {
            return Self::push_fold_legacy_hu(config);
        }
        let state = Self::initial_state(&config);
        let mut tree = Self {
            config,
            nodes: Vec::new(),
            root: 0,
        };
        tree.root = tree.expand_push_fold(state)?;
        Ok(tree)
    }
    fn push_fold_legacy_hu(config: PreflopConfig) -> Result<Self> {
        config.validate()?;
        if config.node_limit < 5 {
            bail!("push/fold requires node_limit >= 5");
        }
        if config.positions != ["BTN", "BB"] {
            bail!("push/fold requires BTN and BB seats");
        }
        let mut tree = Self {
            config,
            nodes: Vec::new(),
            root: 0,
        };
        let initial = Self::initial_state(&tree.config);
        let mut fold = initial.clone();
        fold.active &= !1;
        let fold_id = tree.add_terminal(&fold, TerminalKind::Fold);
        let mut shove = initial;
        shove.contributions[0] = chips(tree.config.stack_bb);
        shove.stage = 4;
        let mut bb_fold = shove.clone();
        bb_fold.active &= !2;
        let bb_fold_id = tree.add_terminal(&bb_fold, TerminalKind::Fold);
        let mut call = shove;
        call.contributions[1] = chips(tree.config.stack_bb);
        let call_id = tree.add_terminal(&call, TerminalKind::AllIn);
        let response = tree.nodes.len();
        tree.nodes.push(Node::Decision {
            actor: 1,
            actions: vec![Action::Fold, Action::Call],
            children: vec![bb_fold_id, call_id],
        });
        tree.root = tree.nodes.len();
        tree.nodes.push(Node::Decision {
            actor: 0,
            actions: vec![Action::Fold, Action::AllIn],
            children: vec![fold_id, response],
        });
        Ok(tree)
    }
    fn expand_push_fold(&mut self, state: State) -> Result<usize> {
        if self.nodes.len() >= self.config.node_limit {
            bail!("preflop node limit {} exceeded", self.config.node_limit);
        }
        if state.active.count_ones() == 1 {
            return Ok(self.add_terminal(&state, TerminalKind::Fold));
        }
        if state.pending == 0 {
            return Ok(self.add_terminal(&state, TerminalKind::AllIn));
        }
        let actor = state.actor;
        let actions = if state.target == chips(self.config.stack_bb) {
            vec![Action::Fold, Action::Call]
        } else {
            vec![Action::Fold, Action::AllIn]
        };
        let id = self.nodes.len();
        self.nodes.push(Node::Decision {
            actor,
            actions: actions.clone(),
            children: Vec::new(),
        });
        let mut children = Vec::new();
        for &action in &actions {
            let mut next = self.apply(&state, action);
            // Match the historical toy's terminal stage metadata.
            if action == Action::AllIn {
                next.stage = 4;
            }
            children.push(self.expand_push_fold(next)?);
        }
        if let Node::Decision {
            children: stored, ..
        } = &mut self.nodes[id]
        {
            *stored = children;
        }
        Ok(id)
    }
    fn initial_state(config: &PreflopConfig) -> State {
        let contributions: Vec<i64> = config
            .positions
            .iter()
            .map(|p| match p.as_str() {
                "SB" => chips(config.ante_bb + 0.5),
                "BB" => chips(config.ante_bb + 1.0),
                _ => chips(config.ante_bb),
            })
            .collect();
        let mask = (1 << config.players) - 1;
        let pending = (0..config.players).fold(mask, |mask, p| {
            if contributions[p] == chips(config.stack_bb) {
                mask & !(1 << p)
            } else {
                mask
            }
        });
        State {
            contributions,
            active: mask,
            pending,
            entered: 0,
            actor: 0,
            target: chips(config.ante_bb + 1.0),
            last_increment: chips(1.0),
            stage: 0,
            aggressor: None,
            opener: None,
            callers: 0,
            sb_limp: false,
        }
    }
    fn add_terminal(&mut self, state: &State, kind: TerminalKind) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::Terminal(Terminal {
            kind,
            contributions: state.contributions.iter().map(|&x| bb(x)).collect(),
            active: (0..self.config.players)
                .map(|p| state.active & (1 << p) != 0)
                .collect(),
            winner: (kind == TerminalKind::Fold).then(|| state.active.trailing_zeros() as usize),
            stage: state.stage,
            dead_blind_bb: self.config.dead_blind_bb,
        }));
        id
    }
    fn expand(&mut self, state: State, legacy: bool) -> Result<usize> {
        if self.nodes.len() >= self.config.node_limit {
            bail!("preflop node limit {} exceeded", self.config.node_limit);
        }
        if state.active.count_ones() == 1 {
            return Ok(self.add_terminal(&state, TerminalKind::Fold));
        }
        if state.pending == 0 {
            let stack = chips(self.config.stack_bb);
            let all_in = (0..self.config.players)
                .any(|p| state.active & (1 << p) != 0 && state.contributions[p] == stack);
            return Ok(self.add_terminal(
                &state,
                if all_in {
                    TerminalKind::AllIn
                } else {
                    TerminalKind::Flop
                },
            ));
        }
        let actor = state.actor;
        let actions = self.actions(&state, legacy);
        let id = self.nodes.len();
        // Reserve the parent before recursion so node limits cover every node.
        self.nodes.push(Node::Decision {
            actor,
            actions: actions.clone(),
            children: Vec::new(),
        });
        let mut children = Vec::with_capacity(actions.len());
        for &action in &actions {
            children.push(self.expand(self.apply(&state, action), legacy)?);
        }
        if let Node::Decision {
            children: stored, ..
        } = &mut self.nodes[id]
        {
            *stored = children;
        }
        Ok(id)
    }
    fn actions(&self, state: &State, legacy: bool) -> Vec<Action> {
        let actor = state.actor;
        let owed = state.target - state.contributions[actor];
        let position = self.config.positions[actor].as_str();
        let mut actions = Vec::with_capacity(4);
        if owed > 0 {
            actions.push(Action::Fold);
        } else {
            actions.push(Action::Check);
        }
        let can_limp = position == "SB" && state.stage == 0;
        let can_call = state.stage > 0 && (state.stage < 3 || state.entered & (1 << actor) != 0);
        if owed > 0
            && (can_limp
                || can_call
                || (state.stage == 0 && state.target == chips(self.config.stack_bb)))
        {
            actions.push(Action::Call);
        }
        let stack = chips(self.config.stack_bb);
        if state.target >= stack || (legacy && state.stage >= 4) {
            return actions;
        }
        let f = &self.config.sizing.fixed_raise_to_bb;
        let targets: Vec<f64> = match state.stage {
            0 if state.sb_limp => vec![f.iso_vs_limp],
            0 => self
                .config
                .sizing
                .open_sizes_by_position
                .get(position)
                .map_or_else(|| self.config.sizing.open_sizes_bb.clone(), |&s| vec![s]),
            1 => {
                let opener = state.opener.expect("a raised state has an opener");
                let ip = self.config.is_ip(actor, opener);
                vec![if state.sb_limp && position == "SB" {
                    f.limp_reraise
                } else if state.callers > 0 {
                    (if ip { f.squeeze.ip } else { f.squeeze.oop })
                        + (state.callers - 1) as f64 * f.squeeze.per_additional_caller
                } else if self.config.positions[opener] == "SB" {
                    f.three_bet_vs_sb_open
                } else if ip {
                    f.three_bet.ip
                } else {
                    f.three_bet.oop
                }]
            }
            2 => {
                let aggressor = state.aggressor.expect("3bet has an aggressor");
                vec![if position == "SB"
                    && self.config.positions[aggressor] == "BB"
                    && state.opener == Some(actor)
                    && !state.sb_limp
                {
                    f.four_bet_vs_bb_three_bet_from_sb
                } else if self.config.is_ip(actor, aggressor) {
                    f.four_bet.ip
                } else {
                    f.four_bet.oop
                }]
            }
            3 if !legacy => vec![
                bb(state.target - chips(self.config.ante_bb))
                    * self.config.sizing.five_bet_multiplier,
            ],
            _ => vec![self.config.stack_bb],
        };
        for target in targets {
            // A squeeze can make a nominal fixed 4bet too small. Keep it legal.
            let mut target = chips(target + self.config.ante_bb)
                .max(state.target + state.last_increment)
                .min(stack);
            if !legacy
                && state.stage == 3
                && (target - state.contributions[actor]) as f64
                    >= self.config.sizing.five_bet_merge_remaining_fraction
                        * (stack - state.contributions[actor]) as f64
            {
                target = stack;
            }
            if target > state.target {
                let action = if target == stack {
                    Action::AllIn
                } else {
                    Action::RaiseTo(bb(target - chips(self.config.ante_bb)))
                };
                if !actions.contains(&action) {
                    actions.push(action);
                }
            }
        }
        if !legacy && state.stage >= 2 && !actions.contains(&Action::AllIn) {
            actions.push(Action::AllIn);
        }
        actions
    }
    fn apply(&self, state: &State, action: Action) -> State {
        let mut next = state.clone();
        let p = state.actor;
        let bit = 1 << p;
        next.pending &= !bit;
        match action {
            Action::Fold => next.active &= !bit,
            Action::Check => (),
            Action::Call => {
                next.contributions[p] = state.target.min(chips(self.config.stack_bb));
                next.entered |= bit;
                if state.stage == 0 {
                    next.sb_limp = true;
                } else {
                    next.callers += 1;
                }
            }
            Action::RaiseTo(target) => {
                self.apply_raise(&mut next, p, chips(target + self.config.ante_bb))
            }
            Action::AllIn => self.apply_raise(&mut next, p, chips(self.config.stack_bb)),
        }
        // All-in players never act again; everyone else owes action after a raise.
        let stack = chips(self.config.stack_bb);
        for seat in 0..self.config.players {
            if next.contributions[seat] == stack {
                next.pending &= !(1 << seat);
            }
        }
        if next.pending != 0 {
            next.actor = (1..=self.config.players)
                .map(|offset| (p + offset) % self.config.players)
                .find(|&seat| next.pending & (1 << seat) != 0)
                .unwrap();
        }
        next
    }
    fn apply_raise(&self, state: &mut State, actor: usize, target: i64) {
        state.last_increment = target - state.target;
        state.target = target;
        state.contributions[actor] = target;
        state.stage += 1;
        state.aggressor = Some(actor);
        if state.opener.is_none() {
            state.opener = Some(actor);
        }
        state.entered |= 1 << actor;
        state.callers = 0;
        state.pending = state.active & !(1 << actor);
    }
    pub fn stats(&self) -> TreeStats {
        let mut stats = TreeStats {
            nodes: self.nodes.len(),
            decisions_by_seat: vec![0; self.config.players],
            ..TreeStats::default()
        };
        let mut work = vec![(self.root, 0)];
        while let Some((id, depth)) = work.pop() {
            stats.max_depth = stats.max_depth.max(depth);
            match &self.nodes[id] {
                Node::Decision {
                    actor, children, ..
                } => {
                    stats.decisions += 1;
                    stats.decisions_by_seat[*actor] += 1;
                    work.extend(children.iter().map(|&id| (id, depth + 1)));
                }
                Node::Terminal(t) => {
                    stats.terminals += 1;
                    stats.max_stage = stats.max_stage.max(t.stage);
                    match t.kind {
                        TerminalKind::Fold => stats.fold_terminals += 1,
                        TerminalKind::Flop => stats.flop_terminals += 1,
                        TerminalKind::AllIn => stats.all_in_terminals += 1,
                    }
                }
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> PreflopConfig {
        serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap()
    }
    fn six_max() -> PreflopConfig {
        let mut c = config();
        c.players = 6;
        c.positions = ["UTG", "HJ", "CO", "BTN", "SB", "BB"]
            .map(String::from)
            .to_vec();
        c.dead_blind_bb = 0.0;
        c
    }
    fn follow(tree: &PreflopTree, path: &[Action]) -> usize {
        let mut id = tree.root;
        for action in path {
            match &tree.nodes[id] {
                Node::Decision {
                    actions, children, ..
                } => {
                    id = children[actions
                        .iter()
                        .position(|a| a == action)
                        .unwrap_or_else(|| panic!("missing {action:?} from {actions:?}"))]
                }
                _ => panic!("terminal before path ends"),
            }
        }
        id
    }
    fn actions_at(tree: &PreflopTree, path: &[Action]) -> Vec<Action> {
        match &tree.nodes[follow(tree, path)] {
            Node::Decision { actions, .. } => actions.clone(),
            _ => panic!("expected decision"),
        }
    }
    fn terminal_at<'a>(tree: &'a PreflopTree, path: &[Action]) -> &'a Terminal {
        match &tree.nodes[follow(tree, path)] {
            Node::Terminal(t) => t,
            _ => panic!("expected terminal"),
        }
    }
    #[test]
    fn hu_full_call_path_and_dead_money() {
        let tree = PreflopTree::build(config()).unwrap();
        assert_eq!(
            actions_at(&tree, &[]),
            vec![Action::Fold, Action::RaiseTo(2.0)]
        );
        let t = terminal_at(&tree, &[Action::RaiseTo(2.0), Action::Call]);
        assert_eq!(t.kind, TerminalKind::Flop);
        assert_eq!(t.contributions, [2.0, 2.0]);
        assert_eq!(t.pot_bb(), 4.5);
        assert_eq!(t.stage, 1);
        let t = terminal_at(&tree, &[Action::Fold]);
        assert_eq!(t.winner, Some(1));
        assert_eq!(t.pot_bb(), 1.5);
    }
    #[test]
    fn hu_raise_schedule_and_five_bet_closure() {
        let tree = PreflopTree::build(config()).unwrap();
        let path = [
            Action::RaiseTo(2.0),
            Action::RaiseTo(9.0),
            Action::RaiseTo(21.0),
            Action::AllIn,
        ];
        assert_eq!(actions_at(&tree, &path), vec![Action::Fold, Action::Call]);
        let mut called = path.to_vec();
        called.push(Action::Call);
        let t = terminal_at(&tree, &called);
        assert_eq!(t.kind, TerminalKind::AllIn);
        assert_eq!(t.contributions, [40.0, 40.0]);
        assert_eq!(t.stage, 4);
    }
    #[test]
    fn sb_complete_bb_option_iso_and_limp_reraise() {
        let mut c = config();
        c.positions = vec!["SB".into(), "BB".into()];
        c.dead_blind_bb = 0.0;
        let tree = PreflopTree::build(c).unwrap();
        assert_eq!(
            actions_at(&tree, &[Action::Call]),
            vec![Action::Check, Action::RaiseTo(3.0)]
        );
        let t = terminal_at(&tree, &[Action::Call, Action::Check]);
        assert_eq!(t.kind, TerminalKind::Flop);
        assert_eq!(t.contributions, [1.0, 1.0]);
        assert_eq!(t.stage, 0);
        assert!(actions_at(&tree, &[Action::Call, Action::RaiseTo(3.0)])
            .contains(&Action::RaiseTo(9.0)));
    }
    #[test]
    fn sb_open_special_three_and_four_bet() {
        let mut c = config();
        c.positions = vec!["SB".into(), "BB".into()];
        let tree = PreflopTree::build(c).unwrap();
        assert!(actions_at(&tree, &[Action::RaiseTo(3.0)]).contains(&Action::RaiseTo(9.0)));
        assert!(
            actions_at(&tree, &[Action::RaiseTo(3.0), Action::RaiseTo(9.0)])
                .contains(&Action::RaiseTo(20.0))
        );
    }
    #[test]
    fn calls_before_four_bet_allowed_but_new_cold_calls_after_forbidden() {
        let tree = PreflopTree::build(six_max()).unwrap();
        assert!(actions_at(&tree, &[Action::RaiseTo(2.0)]).contains(&Action::Call));
        assert!(
            actions_at(&tree, &[Action::RaiseTo(2.0), Action::RaiseTo(7.0)])
                .contains(&Action::Call)
        );
        let path = [
            Action::RaiseTo(2.0),
            Action::RaiseTo(7.0),
            Action::RaiseTo(21.0),
        ];
        assert!(!actions_at(&tree, &path).contains(&Action::Call));
    }
    #[test]
    fn squeeze_caller_increment_and_ip_oop_selection() {
        let tree = PreflopTree::build(six_max()).unwrap();
        assert!(actions_at(&tree, &[Action::RaiseTo(2.0), Action::Call])
            .contains(&Action::RaiseTo(10.0)));
        assert!(
            actions_at(&tree, &[Action::RaiseTo(2.0), Action::Call, Action::Call])
                .contains(&Action::RaiseTo(12.0))
        );
        let path = [
            Action::RaiseTo(2.0),
            Action::Call,
            Action::Call,
            Action::Call,
        ];
        assert!(actions_at(&tree, &path).contains(&Action::RaiseTo(15.0)));
    }
    #[test]
    fn minimum_raise_clamp_and_stack_cap() {
        let mut c = six_max();
        c.sizing.fixed_raise_to_bb.squeeze.ip = 12.0;
        c.sizing.fixed_raise_to_bb.four_bet.oop = 20.0;
        c.sizing.open_sizes_bb = vec![2.5];
        let tree = PreflopTree::build(c).unwrap();
        // UTG open, HJ call, CO squeeze, BTN/SB/BB fold; nominal 20 must become 21.5.
        let path = [
            Action::RaiseTo(2.5),
            Action::Call,
            Action::RaiseTo(12.0),
            Action::Fold,
            Action::Fold,
            Action::Fold,
        ];
        assert!(actions_at(&tree, &path).contains(&Action::RaiseTo(21.5)));
        let mut c = config();
        c.stack_bb = 8.0;
        let tree = PreflopTree::build(c).unwrap();
        assert_eq!(
            actions_at(&tree, &[Action::RaiseTo(2.0)]),
            vec![Action::Fold, Action::Call, Action::AllIn]
        );
        assert_eq!(
            actions_at(&tree, &[Action::RaiseTo(2.0), Action::AllIn]),
            vec![Action::Fold, Action::Call]
        );
    }
    #[test]
    fn all_six_max_terminals_conserve_chips_and_close_action() {
        let tree = PreflopTree::build(six_max()).unwrap();
        let stats = tree.stats();
        assert_eq!(stats.nodes, stats.decisions + stats.terminals);
        assert!(stats.flop_terminals > 0 && stats.all_in_terminals > 0 && stats.fold_terminals > 0);
        for node in &tree.nodes {
            match node {
                Node::Terminal(t) => {
                    let live = t.active.iter().filter(|&&x| x).count();
                    assert_eq!(t.contributions.len(), 6);
                    assert!(t.contributions.iter().all(|&x| (0.0..=40.0).contains(&x)));
                    if t.kind == TerminalKind::Fold {
                        assert_eq!(live, 1);
                    } else {
                        assert!(live >= 2);
                        let amounts: Vec<_> = t
                            .contributions
                            .iter()
                            .zip(&t.active)
                            .filter_map(|(&x, &active)| active.then_some(x))
                            .collect();
                        assert!(amounts.iter().all(|&x| x == amounts[0]));
                        assert_eq!(t.kind == TerminalKind::AllIn, amounts[0] == 40.0);
                    }
                }
                Node::Decision {
                    actions, children, ..
                } => {
                    assert_eq!(actions.len(), children.len());
                    assert!(!actions.is_empty());
                }
            }
        }
    }
    #[test]
    fn node_limit_fails_explicitly() {
        let mut c = config();
        c.node_limit = 2;
        assert!(PreflopTree::build(c)
            .unwrap_err()
            .to_string()
            .contains("node limit"));
    }
    #[test]
    fn push_fold_is_tiny_and_has_all_in_call_terminal() {
        let tree = PreflopTree::push_fold(config()).unwrap();
        assert_eq!(tree.nodes.len(), 5);
        assert_eq!(
            terminal_at(&tree, &[Action::AllIn, Action::Call]).kind,
            TerminalKind::AllIn
        );
    }
}

#[cfg(test)]
mod ante_tests {
    use super::*;
    #[test]
    fn ante_is_in_contributions_not_raise_labels_and_short_blind_call_is_legal() {
        let mut c: PreflopConfig =
            serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap();
        c.ante_bb = 0.1;
        let tree = PreflopTree::build(c.clone()).unwrap();
        let children = match &tree.nodes[tree.root] {
            Node::Decision {
                actions, children, ..
            } => {
                assert_eq!(actions, &[Action::Fold, Action::RaiseTo(2.0)]);
                children
            }
            _ => panic!("root decision"),
        };
        if let Node::Decision { children, .. } = &tree.nodes[children[1]] {
            if let Node::Terminal(t) = &tree.nodes[children[1]] {
                assert_eq!(t.contributions, [2.1, 2.1]);
                assert_eq!(t.pot_bb(), 4.7);
            } else {
                panic!("call closes betting");
            }
        } else {
            panic!("BB response");
        }
        c.stack_bb = 1.1;
        let tree = PreflopTree::build(c).unwrap();
        if let Node::Decision {
            actions, children, ..
        } = &tree.nodes[tree.root]
        {
            assert_eq!(actions, &[Action::Fold, Action::Call]);
            if let Node::Terminal(t) = &tree.nodes[children[1]] {
                assert_eq!(t.kind, TerminalKind::AllIn);
            } else {
                panic!("no action for all-in BB");
            }
        }
    }
}

#[cfg(test)]
mod p4_tests {
    use super::*;
    fn config() -> PreflopConfig {
        let mut c: PreflopConfig =
            serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap();
        c.stack_bb = 100.0;
        c
    }
    fn actions(tree: &PreflopTree, path: &[Action]) -> Vec<Action> {
        let mut id = tree.root;
        for a in path {
            if let Node::Decision {
                actions, children, ..
            } = &tree.nodes[id]
            {
                id = children[actions.iter().position(|x| x == a).expect("path action")];
            } else {
                panic!("early terminal");
            }
        }
        if let Node::Decision { actions, .. } = &tree.nodes[id] {
            actions.clone()
        } else {
            panic!("terminal")
        }
    }
    #[test]
    fn four_bet_fixed_and_jam_five_bet_multiplier_then_six_bet_jam() {
        let t = PreflopTree::build(config()).unwrap();
        let p = [Action::RaiseTo(2.0), Action::RaiseTo(9.0)];
        assert_eq!(
            actions(&t, &p),
            vec![
                Action::Fold,
                Action::Call,
                Action::RaiseTo(21.0),
                Action::AllIn
            ]
        );
        let mut p = p.to_vec();
        p.push(Action::RaiseTo(21.0));
        assert_eq!(
            actions(&t, &p),
            vec![
                Action::Fold,
                Action::Call,
                Action::RaiseTo(46.2),
                Action::AllIn
            ]
        );
        p.push(Action::RaiseTo(46.2));
        assert_eq!(
            actions(&t, &p),
            vec![Action::Fold, Action::Call, Action::AllIn]
        );
        assert_eq!(PreflopTree::build_legacy(config()).unwrap().nodes.len(), 14);
    }
    #[test]
    fn five_bet_merge_uses_additional_chips_and_remaining_stack() {
        let mut c = config();
        c.sizing.five_bet_multiplier = 3.0;
        let p = [
            Action::RaiseTo(2.0),
            Action::RaiseTo(9.0),
            Action::RaiseTo(21.0),
        ];
        // Additional54 / remaining91 is below .67, despite target63 exceeding .67*91.
        let t = PreflopTree::build(c.clone()).unwrap();
        assert!(actions(&t, &p).contains(&Action::RaiseTo(63.0)));
        c.sizing.five_bet_merge_remaining_fraction = 54.0 / 91.0;
        assert_eq!(
            actions(&PreflopTree::build(c).unwrap(), &p),
            vec![Action::Fold, Action::Call, Action::AllIn]
        );
    }
    #[test]
    fn four_bet_stack_cap_deduplicates_jam_and_node_limit_applies() {
        let mut c = config();
        c.stack_bb = 20.0;
        let t = PreflopTree::build(c).unwrap();
        assert_eq!(
            actions(&t, &[Action::RaiseTo(2.0), Action::RaiseTo(9.0)]),
            vec![Action::Fold, Action::Call, Action::AllIn]
        );
        let mut c = config();
        c.node_limit = 14;
        assert!(PreflopTree::build(c).is_err());
    }
    #[test]
    fn shallow_multiway_push_fold_never_reacts_with_allin_seat() {
        let mut c = config();
        c.players = 3;
        c.positions = vec!["BTN".into(), "SB".into(), "BB".into()];
        c.stack_bb = 1.0;
        let t = PreflopTree::push_fold(c).unwrap();
        assert!(t
            .nodes
            .iter()
            .all(|n| !matches!(n, Node::Decision { actor: 2, .. })));
        assert!(t
            .nodes
            .iter()
            .any(|n| matches!(n, Node::Terminal(x) if x.kind == TerminalKind::AllIn)));
    }
    #[test]
    fn three_player_push_fold_has_joint_allin_and_limit() {
        let mut c = config();
        c.players = 3;
        c.positions = vec!["BTN".into(), "SB".into(), "BB".into()];
        let t = PreflopTree::push_fold(c.clone()).unwrap();
        assert!(t.nodes.iter().any(|n| matches!(n, Node::Terminal(x) if x.kind == TerminalKind::AllIn && x.active.iter().all(|a| *a))));
        c.node_limit = t.nodes.len() - 1;
        assert!(PreflopTree::push_fold(c).is_err());
    }
}
