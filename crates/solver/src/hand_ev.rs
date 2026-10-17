use super::*;

#[derive(Debug, Serialize)]
pub struct ComboValue {
    pub combo: String,
    pub hand_class: String,
    #[serde(flatten)]
    pub value: cfr::HandValue,
    pub ev_bb: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct ClassValue {
    pub hand_class: String,
    pub reach_weight: f64,
    pub ev_chips: Option<f64>,
    pub ev_bb: Option<f64>,
    pub equity: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct PlayerHandValues {
    pub player: usize,
    pub combos: Vec<ComboValue>,
    pub classes: Vec<ClassValue>,
}
#[derive(Debug, Serialize)]
pub struct HandEvReport {
    pub schema_version: u32,
    pub board: Vec<String>,
    pub pot_chips: i64,
    pub pot_definition: &'static str,
    pub ev_definition: &'static str,
    pub weight_definition: &'static str,
    pub units_per_bb: i64,
    pub root_ev_chips: [f64; 2],
    pub players: [PlayerHandValues; 2],
}
impl Solution {
    /// Optional post-solve analysis. Does not modify the learned policy or metrics.
    pub fn hand_ev_report(&mut self) -> HandEvReport {
        let values = self.engine.root_hand_values();
        let bb = self.game_spec.units_per_bb as f64;
        let players = values
            .into_iter()
            .enumerate()
            .map(|(p, values)| {
                let combos: Vec<_> = values
                    .into_iter()
                    .zip(&self.engine.ranges()[p].combos)
                    .map(|(value, c)| {
                        let hand_class = hand_class(c.combo);
                        ComboValue {
                            combo: c.combo.to_string(),
                            hand_class,
                            ev_bb: value.ev_chips.map(|v| v / bb),
                            value,
                        }
                    })
                    .collect();
                let mut groups: BTreeMap<String, [f64; 3]> = BTreeMap::new();
                for c in &combos {
                    let g = groups.entry(c.hand_class.clone()).or_default();
                    g[0] += c.value.reach_weight;
                    g[1] += c.value.reach_weight * c.value.ev_chips.unwrap_or(0.0);
                    g[2] += c.value.reach_weight * c.value.equity.unwrap_or(0.0);
                }
                let classes = groups
                    .into_iter()
                    .map(|(hand_class, g)| ClassValue {
                        hand_class,
                        reach_weight: g[0],
                        ev_chips: (g[0] > 0.0).then(|| g[1] / g[0]),
                        ev_bb: (g[0] > 0.0).then(|| g[1] / g[0] / bb),
                        equity: (g[0] > 0.0).then(|| g[2] / g[0]),
                    })
                    .collect();
                PlayerHandValues {
                    player: p,
                    combos,
                    classes,
                }
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        HandEvReport {
            schema_version: 1, board: self.game_spec.board.iter().map(ToString::to_string).collect(),
            pot_chips: self.game_spec.tree.starting_pot,
            pot_definition: "root starting pot before rake",
            ev_definition: "same as root_ev_chips: terminal receipts after rake minus future investments minus half starting pot; gross continuation value = ev_chips + pot_chips/2",
            weight_definition: "own_reach_weight is normalized own range; reach_weight = own reach times compatible opponent mass; normalize reach_weight for averages; null denotes no compatible opponent",
            units_per_bb: self.game_spec.units_per_bb, root_ev_chips: self.root_ev(), players,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn game(board: &[&str], a: &str, b: &str) -> Solution {
        solve(
            GameSpec {
                board: board.iter().map(|s| s.parse().unwrap()).collect(),
                units_per_bb: 100,
                tree: TreeConfig {
                    starting_pot: 200,
                    effective_stack: if board.len() == 5 { 0 } else { 10 },
                    rake_rate: 0.0,
                    ..TreeConfig::default()
                },
                engine: EngineConfig {
                    threads: 1,
                    kernel_diagnostics: true,
                    ..EngineConfig::default()
                },
            },
            [Range::from_json(a).unwrap(), Range::from_json(b).unwrap()],
            StopConditions {
                max_iterations: 2,
                target_exploitability_pct: 0.0,
                ..Default::default()
            },
        )
        .unwrap()
    }
    #[test]
    fn hand_ev_weighted_average_matches_root_and_export_preserves_report_bits() {
        let mut s = game(
            &["2c", "7d", "Th"],
            r#"{"AsAh":2,"KsKh":1}"#,
            r#"{"AsAd":1,"QcQd":3}"#,
        );
        let before = serde_json::to_vec(&s.report()).unwrap();
        let report = s.hand_ev_report();
        for p in 0..2 {
            let c = &report.players[p].combos;
            let mass: f64 = c.iter().map(|v| v.value.reach_weight).sum();
            let ev: f64 = c
                .iter()
                .map(|v| v.value.reach_weight * v.value.ev_chips.unwrap())
                .sum::<f64>()
                / mass;
            assert!((ev - s.root_ev()[p]).abs() < 1e-10);
            let classes = &report.players[p].classes;
            assert!(
                (classes
                    .iter()
                    .map(|v| v.reach_weight * v.ev_chips.unwrap())
                    .sum::<f64>()
                    / mass
                    - ev)
                    .abs()
                    < 1e-10
            );
        }
        assert_eq!(before, serde_json::to_vec(&s.report()).unwrap());
    }
    #[test]
    fn symmetric_river_board_nuts_have_half_equity_and_zero_net_ev() {
        let mut s = game(
            &["As", "Ks", "Qs", "Js", "Ts"],
            r#"{"2c3c":1,"2d3d":2}"#,
            r#"{"2c3c":1,"2d3d":2}"#,
        );
        let report = s.hand_ev_report();
        for p in &report.players {
            for c in &p.combos {
                assert!((c.value.equity.unwrap() - 0.5).abs() < 1e-12);
                assert!(c.value.ev_chips.unwrap().abs() < 1e-10);
            }
        }
    }
    #[test]
    fn unreachable_combo_has_null_values() {
        let mut s = game(
            &["2c", "7d", "Th", "9s", "Jc"],
            r#"{"AsAh":1,"KsKh":1}"#,
            r#"{"AsAd":1}"#,
        );
        let report = s.hand_ev_report();
        let blocked = report.players[0]
            .combos
            .iter()
            .find(|c| c.hand_class == "AA")
            .unwrap();
        assert_eq!(blocked.value.reach_weight, 0.0);
        assert_eq!(blocked.value.ev_chips, None);
        assert_eq!(blocked.value.equity, None);
    }
}
