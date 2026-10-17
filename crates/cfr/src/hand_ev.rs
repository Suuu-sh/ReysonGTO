use super::*;

/// Root conditional values against the average policy. Zero compatible mass is
/// represented by None, never NaN. reach_weight is the blocker-aware joint marginal.
#[derive(Clone, Debug, Serialize)]
pub struct HandValue {
    pub own_reach_weight: f64,
    pub reach_weight: f64,
    pub ev_chips: Option<f64>,
    pub equity: Option<f64>,
}

impl<S: Scratch, T: Scratch> EngineCore<S, T> {
    pub(super) fn root_hand_values(&mut self) -> [Vec<HandValue>; 2] {
        if let Some(evaluation) = &mut self.evaluation {
            evaluation.state.copy_scales(&self.state);
        }
        let equity = equities(
            &self.tree.nodes[self.tree.root]
                .board
                .iter()
                .copied()
                .map(Card)
                .collect::<Vec<_>>(),
            &self.ranges,
            &self.initial_reach,
        );
        let diagnostics = self.kernel_diagnostics.take();
        let profiling = self.config.profiling;
        let kernel_diagnostics = self.config.kernel_diagnostics;
        self.config.profiling = false;
        self.config.kernel_diagnostics = false;
        let result = std::array::from_fn(|p| {
            self.run_metric(p, Mode::Average);
            let values: Vec<f64> = if let Some(e) = &self.evaluation {
                e.scratch[0].value[..self.ranges[p].len()].to_vec()
            } else {
                self.scratch[0].value[..self.ranges[p].len()]
                    .iter()
                    .map(|v| v.to_f64())
                    .collect()
            };
            let other = totals(&self.hand_cards[1 - p], &self.initial_reach[1 - p]);
            values
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let mass = compatible(
                        self.hand_cards[p][i],
                        &other,
                        self.same[p][i].map_or(0.0, |j| self.initial_reach[1 - p][j]),
                    );
                    HandValue {
                        own_reach_weight: self.initial_reach[p][i],
                        reach_weight: self.initial_reach[p][i] * mass,
                        ev_chips: (mass > 0.0).then(|| v / mass),
                        equity: (mass > 0.0).then(|| equity[p][i] / mass),
                    }
                })
                .collect()
        });
        self.kernel_diagnostics = diagnostics;
        self.config.profiling = profiling;
        self.config.kernel_diagnostics = kernel_diagnostics;
        result
    }
}

#[derive(Clone)]
struct Mass {
    total: f64,
    cards: [f64; 52],
}
impl Mass {
    fn new() -> Self {
        Self {
            total: 0.0,
            cards: [0.0; 52],
        }
    }
    fn add(&mut self, c: cards::Combo, w: f64) {
        self.total += w;
        for card in c.cards {
            self.cards[card.0 as usize] += w;
        }
    }
    fn excluding(&self, c: cards::Combo) -> f64 {
        self.total - self.cards[c.cards[0].0 as usize] - self.cards[c.cards[1].0 as usize]
    }
}

// Enumerate physical boards once, rank each legal combo once per board, then
// sorted rank prefixes with per-card blocker subtraction. No H0*H1 pair table.
fn equities(board: &[Card], ranges: &[DenseRange; 2], reach: &[Vec<f64>; 2]) -> [Vec<f64>; 2] {
    let mut out = std::array::from_fn(|p| vec![0.0; ranges[p].len()]);
    let mask = card_mask(board);
    let mut complete = [Card(0); 5];
    complete[..board.len()].copy_from_slice(board);
    let runouts = match board.len() {
        3 => 990.0,
        4 => 44.0,
        5 => 1.0,
        _ => unreachable!(),
    };
    let mut visit = |b: &[Card; 5]| {
        let bm = card_mask(b);
        let ranks: [Vec<(u32, usize)>; 2] = std::array::from_fn(|p| {
            let mut v: Vec<_> = ranges[p]
                .combos
                .iter()
                .enumerate()
                .filter(|(_, c)| c.combo.mask() & bm == 0)
                .map(|(i, c)| {
                    (
                        evaluate7(&[
                            b[0],
                            b[1],
                            b[2],
                            b[3],
                            b[4],
                            c.combo.cards[0],
                            c.combo.cards[1],
                        ]),
                        i,
                    )
                })
                .collect();
            v.sort_unstable();
            v
        });
        for p in 0..2 {
            let mut less = Mass::new();
            let mut leq = Mass::new();
            let mut lo = 0;
            let mut hi = 0;
            for &(rank, i) in &ranks[p] {
                while lo < ranks[1 - p].len() && ranks[1 - p][lo].0 < rank {
                    let j = ranks[1 - p][lo].1;
                    less.add(ranges[1 - p].combos[j].combo, reach[1 - p][j]);
                    lo += 1;
                }
                while hi < ranks[1 - p].len() && ranks[1 - p][hi].0 <= rank {
                    let j = ranks[1 - p][hi].1;
                    leq.add(ranges[1 - p].combos[j].combo, reach[1 - p][j]);
                    hi += 1;
                }
                let combo = ranges[p].combos[i].combo;
                let same = ranges[1 - p]
                    .combos
                    .binary_search_by_key(&combo.id(), |c| c.combo.id())
                    .ok()
                    .map_or(0.0, |j| reach[1 - p][j]);
                out[p][i] += (0.5 * (less.excluding(combo) + leq.excluding(combo) + same)).max(0.0)
                    / runouts;
            }
        }
    };
    match board.len() {
        5 => visit(&complete),
        4 => {
            for a in 0..52 {
                if mask & Card(a).mask() == 0 {
                    complete[4] = Card(a);
                    visit(&complete);
                }
            }
        }
        3 => {
            for a in 0..52 {
                if mask & Card(a).mask() != 0 {
                    continue;
                }
                for b in a + 1..52 {
                    if mask & Card(b).mask() == 0 {
                        complete[3] = Card(a);
                        complete[4] = Card(b);
                        visit(&complete);
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    out
}

fn card_mask(board: &[Card]) -> u64 {
    board.iter().fold(0, |mask, c| mask | c.mask())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "manual small synthetic equity timing, no solve"]
    fn synthetic_equity_timing() {
        let board: Vec<Card> = ["2c", "7d", "Th"].map(|s| s.parse().unwrap()).to_vec();
        let combos = (0..1326)
            .map(|i| cards::Combo::from_id(i).unwrap())
            .filter(|c| c.mask() & card_mask(&board) == 0)
            .take(64)
            .map(|c| (c, 1.0))
            .collect();
        let range = ranges::Range::from_combos(combos)
            .unwrap()
            .blocked(&board)
            .unwrap();
        let ranges = [range.clone(), range];
        let reach = ranges.each_ref().map(|r| r.weights());
        let start = Instant::now();
        for _ in 0..10 {
            std::hint::black_box(equities(&board, &ranges, &reach));
        }
        eprintln!(
            "64+64 synthetic flop equity: {:.6} seconds per call",
            start.elapsed().as_secs_f64() / 10.0
        );
    }
    #[test]
    fn equity_matches_independent_naive_runout_enumeration() {
        let board: Vec<Card> = ["2c", "7d", "Th"].map(|s| s.parse().unwrap()).to_vec();
        let ranges = [
            ranges::Range::from_json(r#"{"AsAh":2,"KsKh":1}"#)
                .unwrap()
                .blocked(&board)
                .unwrap(),
            ranges::Range::from_json(r#"{"AsAd":1,"QcQd":3}"#)
                .unwrap()
                .blocked(&board)
                .unwrap(),
        ];
        let reach = ranges.each_ref().map(|r| r.weights());
        let actual = equities(&board, &ranges, &reach);
        for p in 0..2 {
            for (i, hand) in ranges[p].combos.iter().enumerate() {
                let mut expected = 0.0;
                for (j, other) in ranges[1 - p].combos.iter().enumerate() {
                    if hand.combo.mask() & other.combo.mask() != 0 {
                        continue;
                    }
                    let mask = card_mask(&board) | hand.combo.mask() | other.combo.mask();
                    let mut wins = 0.0;
                    let mut count = 0;
                    for a in 0..52 {
                        for b in a + 1..52 {
                            if mask & (Card(a).mask() | Card(b).mask()) != 0 {
                                continue;
                            }
                            let rank = |c: cards::Combo| {
                                cards::evaluate7_naive(&[
                                    board[0],
                                    board[1],
                                    board[2],
                                    Card(a),
                                    Card(b),
                                    c.cards[0],
                                    c.cards[1],
                                ])
                            };
                            wins += match rank(hand.combo).cmp(&rank(other.combo)) {
                                std::cmp::Ordering::Greater => 1.0,
                                std::cmp::Ordering::Equal => 0.5,
                                _ => 0.0,
                            };
                            count += 1;
                        }
                    }
                    expected += reach[1 - p][j] * wins / count as f64;
                }
                assert!(
                    (actual[p][i] - expected).abs() < 1e-11,
                    "{} {}",
                    actual[p][i],
                    expected
                );
            }
        }
    }
}
