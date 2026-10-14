//! Method 1 (design 12.1): solve one single-size tree per candidate and keep the size
//! with the highest bettor EV, eliminating clearly worse sizes on coarse solves first.
use ranges::Range;
use serde::Serialize;
use solver::{solve, GameSpec, StopConditions};
use std::sync::Arc;
use tree::{SizeContext, SizeRule, Street};

/// One elimination round: solve the remaining sizes to this accuracy, keep the best `keep`.
#[derive(Clone, Debug, Serialize)]
pub struct Stage {
    pub stop: StopConditions,
    pub keep: usize,
}

/// Coarse-then-finer schedule: 4% for everyone, 1.5% for the best two.
pub fn default_stages() -> Vec<Stage> {
    vec![
        Stage { stop: StopConditions { target_exploitability_pct: 4.0, max_iterations: 400, max_seconds: 600.0 }, keep: 2 },
        Stage { stop: StopConditions { target_exploitability_pct: 1.5, max_iterations: 1_000, max_seconds: 1_800.0 }, keep: 1 },
    ]
}

#[derive(Clone, Debug, Serialize)]
pub struct SizeResult {
    pub fraction: f64,
    /// Equilibrium value of the bettor at the root, in BB.
    pub bettor_ev_bb: f64,
    pub exploitability_pct: f64,
    pub iterations: usize,
    pub seconds: f64,
    /// Last round this size took part in (0-based).
    pub round: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    pub street: Street,
    pub bettor: usize,
    pub best: f64,
    /// Every size with its result from the last round it reached, best first.
    pub results: Vec<SizeResult>,
}

/// Use `fraction` for first bets on `street` and keep `base` everywhere else.
pub fn override_street(base: SizeRule, street: Street, fraction: f64) -> SizeRule {
    SizeRule::Custom(Arc::new(move |c: &SizeContext<'_>| if c.street == street { fraction } else { base.fraction(c) }))
}

/// Compare `candidates` for the first bets of `street` (the root street of `spec`).
/// `bettor` is whose EV decides (0 = OOP, 1 = IP), e.g. the preflop raiser.
pub fn compare_sizes(spec: &GameSpec, ranges: &[Range; 2], street: Street, candidates: &[f64], bettor: usize, stages: &[Stage]) -> Result<Comparison, String> {
    if candidates.is_empty() || stages.is_empty() || bettor > 1 {
        return Err("need candidates, stages and a bettor of 0 or 1".into());
    }
    let mut alive: Vec<f64> = candidates.to_vec();
    let mut finished: Vec<SizeResult> = Vec::new();
    for (round, stage) in stages.iter().enumerate() {
        let mut results = Vec::new();
        for &fraction in &alive {
            let mut s = spec.clone();
            s.tree.bet_size = override_street(spec.tree.bet_size.clone(), street, fraction);
            let solution = solve(s, ranges.clone(), stage.stop.clone())?;
            results.push(SizeResult {
                fraction,
                bettor_ev_bb: solution.root_ev()[bettor] / tree::BB as f64,
                exploitability_pct: solution.metrics.exploitability_pct,
                iterations: solution.history.last().map_or(0, |r| r.iteration),
                seconds: solution.setup_seconds + solution.solve_seconds,
                round,
            });
        }
        results.sort_by(|a, b| b.bettor_ev_bb.total_cmp(&a.bettor_ev_bb));
        let keep = stage.keep.clamp(1, results.len());
        alive = results[..keep].iter().map(|r| r.fraction).collect();
        // Sizes dropped this round keep their (coarser) result for the record.
        finished.extend(results.drain(keep..));
        if round + 1 == stages.len() {
            finished.extend(results);
        }
    }
    finished.sort_by(|a, b| b.round.cmp(&a.round).then(b.bettor_ev_bb.total_cmp(&a.bettor_ev_bb)));
    Ok(Comparison { street, bettor, best: finished[0].fraction, results: finished })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfr::EngineConfig;
    use tree::{TreeConfig, BB};

    #[test]
    fn river_comparison_ranks_every_candidate() {
        // Polarized bettor (nuts or air) against a bluff catcher: bigger bets earn more.
        let board: Vec<cards::Card> = ["Kh", "9d", "4c", "2s", "3h"].iter().map(|c| c.parse().unwrap()).collect();
        let oop = Range::from_json(r#"{"KdKc":1,"7c6c":1}"#).unwrap();
        let ip = Range::from_json(r#"{"QdQc":1}"#).unwrap();
        let spec = GameSpec {
            board,
            tree: TreeConfig { starting_pot: 10 * BB, effective_stack: 50 * BB, oop_can_lead_flop: true, rake_rate: 0.0, ..TreeConfig::default() },
            engine: EngineConfig { threads: 1, ..EngineConfig::default() },
        };
        let stages = vec![
            Stage { stop: StopConditions { target_exploitability_pct: 1.0, max_iterations: 2_000, max_seconds: 30.0 }, keep: 2 },
            Stage { stop: StopConditions { target_exploitability_pct: 0.2, max_iterations: 5_000, max_seconds: 30.0 }, keep: 1 },
        ];
        let c = compare_sizes(&spec, &[oop, ip], Street::River, &[0.33, 0.66, 1.25], 0, &stages).unwrap();
        assert_eq!(c.results.len(), 3);
        assert_eq!(c.best, c.results[0].fraction);
        assert!(c.results[0].round == 1, "the winner survived to the last round");
        assert_eq!(c.best, 1.25, "a polarized range prefers the largest bet: {:?}", c.results);
    }
}
