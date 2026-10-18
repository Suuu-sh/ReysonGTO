//! Readable decision table from board features to one bet size (design 12.3–12.4).
use crate::features::{
    board_pairing, flop_features, runout_card, BoardPairing, Connectivity, Height, Pairing,
    RunoutCard, SuitPattern,
};
use cards::Card;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tree::{SizeContext, SizeRule, Street};

/// One row. `None` fields match anything; the row with the most matching fields wins,
/// and earlier rows win ties, so general rows go last as fallbacks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub street: Option<Street>,
    /// 0 = OOP, 1 = IP.
    pub player: Option<usize>,
    /// The first bet of the street (not a raise). Rules only size first bets; raises use the multiplier.
    pub after_check: Option<bool>,
    pub previous_street_line: Option<tree::PreviousStreetLine>,
    pub height: Option<Height>,
    pub pairing: Option<Pairing>,
    /// Rank multiplicities of the entire current board; omitted means wildcard.
    pub board_pairing: Option<BoardPairing>,
    pub suits: Option<SuitPattern>,
    pub connectivity: Option<Connectivity>,
    /// Turn/river only: what the new card changed.
    pub runout: Option<RunoutCard>,
    /// Bet size as a fraction of the pot.
    pub fraction: f64,
    /// Situations that produced this row (for tracing a rule back to its evidence).
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleTable {
    pub version: u32,
    /// Candidate sizes that were compared, e.g. SRP [0.33, 0.66, 1.0, 1.25].
    pub candidates: Vec<f64>,
    /// Used when no row matches.
    pub default_fraction: f64,
    pub rules: Vec<Rule>,
}

/// Facts about one bet decision that rules can match on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Situation {
    pub street: Street,
    pub player: usize,
    pub after_check: bool,
    pub previous_street_line: Option<tree::PreviousStreetLine>,
    pub height: Height,
    pub pairing: Pairing,
    pub board_pairing: Option<BoardPairing>,
    pub suits: SuitPattern,
    pub connectivity: Connectivity,
    pub runout: Option<RunoutCard>,
}

impl Situation {
    pub fn from_context(c: &SizeContext<'_>) -> Self {
        let board: Vec<Card> = c.board.iter().map(|&id| Card(id)).collect();
        let flop = flop_features(&board[..3]);
        let runout = match board.len() {
            4 => Some(runout_card(&board[..3], board[3])),
            5 => Some(runout_card(&board[..4], board[4])),
            _ => None,
        };
        Self {
            street: c.street,
            player: c.player,
            after_check: c.after_check,
            previous_street_line: c.previous_street_line,
            height: flop.height,
            pairing: flop.pairing,
            board_pairing: Some(board_pairing(&board)),
            suits: flop.suits,
            connectivity: flop.connectivity,
            runout,
        }
    }
}

impl Rule {
    /// Number of constrained fields if the rule matches, otherwise `None`.
    fn specificity(&self, s: &Situation) -> Option<u32> {
        let mut n = 0;
        macro_rules! check {
            ($field:ident, $value:expr) => {
                if let Some(v) = self.$field {
                    if v != $value {
                        return None;
                    }
                    n += 1;
                }
            };
        }
        check!(street, s.street);
        check!(player, s.player);
        check!(after_check, s.after_check);
        if let Some(value) = self.previous_street_line {
            if s.previous_street_line != Some(value) {
                return None;
            }
            n += 1;
        }
        check!(height, s.height);
        check!(pairing, s.pairing);
        if let Some(value) = self.board_pairing {
            if s.board_pairing != Some(value) {
                return None;
            }
            n += 1;
        }
        check!(suits, s.suits);
        check!(connectivity, s.connectivity);
        if let Some(r) = self.runout {
            if Some(r) != s.runout {
                return None;
            }
            n += 1;
        }
        Some(n)
    }
}

impl RuleTable {
    pub fn validate(&self) -> Result<(), String> {
        let ok = |f: f64| f.is_finite() && f > 0.0;
        if !ok(self.default_fraction) || self.rules.iter().any(|r| !ok(r.fraction)) {
            return Err("bet fractions must be finite and positive".into());
        }
        Ok(())
    }

    pub fn fraction(&self, s: &Situation) -> f64 {
        let mut best: Option<(u32, f64)> = None;
        for rule in &self.rules {
            if let Some(n) = rule.specificity(s) {
                if best.is_none_or(|(b, _)| n > b) {
                    best = Some((n, rule.fraction));
                }
            }
        }
        best.map_or(self.default_fraction, |(_, f)| f)
    }

    /// The table as a tree size rule. Features ignore suit labels, so the result is
    /// explicitly declared suit-invariant.
    pub fn into_size_rule(self) -> SizeRule {
        let table = Arc::new(self);
        SizeRule::SuitInvariant(Arc::new(move |c: &SizeContext<'_>| {
            table.fraction(&Situation::from_context(c))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> RuleTable {
        RuleTable {
            version: 1,
            candidates: vec![0.33, 0.66, 1.0, 1.25],
            default_fraction: 0.66,
            rules: vec![
                Rule {
                    street: Some(Street::Flop),
                    height: Some(Height::Ace),
                    suits: Some(SuitPattern::Rainbow),
                    fraction: 0.33,
                    ..Default::default()
                },
                Rule {
                    street: Some(Street::Flop),
                    connectivity: Some(Connectivity::Connected),
                    suits: Some(SuitPattern::TwoTone),
                    fraction: 1.0,
                    ..Default::default()
                },
                Rule {
                    street: Some(Street::River),
                    runout: Some(RunoutCard::FlushComplete),
                    fraction: 1.25,
                    ..Default::default()
                },
                Rule {
                    street: Some(Street::Flop),
                    fraction: 0.66,
                    ..Default::default()
                },
            ],
        }
    }

    fn ctx(board: &str, street: Street) -> f64 {
        fraction_on(&table(), board, street)
    }

    fn fraction_on(table: &RuleTable, board: &str, street: Street) -> f64 {
        let ids: Vec<u8> = board
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap().parse::<Card>().unwrap().0)
            .collect();
        table.fraction(&Situation::from_context(&SizeContext {
            board: &ids,
            street,
            player: 1,
            first_bet: true,
            after_check: true,
            previous_street_line: None,
        }))
    }

    #[test]
    fn situation_uses_entire_current_board_on_each_street() {
        let ids: Vec<_> = ["As", "Ad", "2c", "2h", "Ac"]
            .iter()
            .map(|c| c.parse::<Card>().unwrap().0)
            .collect();
        for (len, street, expected) in [
            (3, Street::Flop, BoardPairing::OnePair),
            (4, Street::Turn, BoardPairing::TwoPair),
            (5, Street::River, BoardPairing::FullHouse),
        ] {
            let situation = Situation::from_context(&SizeContext {
                board: &ids[..len],
                street,
                player: 0,
                first_bet: true,
                after_check: false,
                previous_street_line: None,
            });
            assert_eq!(situation.pairing, Pairing::Paired);
            assert_eq!(situation.board_pairing, Some(expected));
        }
    }

    #[test]
    fn river_rules_cover_pairings_players_and_turn_histories() {
        let table: RuleTable =
            serde_json::from_str(include_str!("../../../configs/sizing/srp-btn-vs-bb.json"))
                .unwrap();
        for (board, pairing, expected) in [
            (
                "As7d2c9hTs",
                BoardPairing::Unpaired,
                [0.33, 0.33, 1.0, 0.66],
            ),
            ("As7d2c7hTs", BoardPairing::OnePair, [0.33, 0.33, 1.0, 0.66]),
            ("AsAd2c2hTs", BoardPairing::TwoPair, [0.66, 0.33, 1.0, 0.66]),
            ("AsAdAc2hTs", BoardPairing::Trips, [0.33, 0.33, 1.25, 0.66]),
            (
                "AsAdAc2h2s",
                BoardPairing::FullHouse,
                [0.66, 1.25, 1.25, 1.0],
            ),
            ("AsAdAcAhTs", BoardPairing::Quads, [0.33, 0.33, 0.33, 0.66]),
        ] {
            let ids: Vec<_> = board
                .as_bytes()
                .chunks(2)
                .map(|c| std::str::from_utf8(c).unwrap().parse::<Card>().unwrap().0)
                .collect();
            for (i, (player, history)) in [(0, true), (0, false), (1, true), (1, false)]
                .into_iter()
                .enumerate()
            {
                let situation = Situation::from_context(&SizeContext {
                    board: &ids,
                    street: Street::River,
                    player,
                    first_bet: true,
                    after_check: player == 1,
                    previous_street_line: Some(if history {
                        tree::PreviousStreetLine::IpBetCalled
                    } else {
                        tree::PreviousStreetLine::CheckedThrough
                    }),
                });
                assert_eq!(situation.board_pairing, Some(pairing));
                assert_eq!(
                    table.fraction(&situation),
                    expected[i],
                    "{board} p{player} history={history}"
                );
                // Both unknown history and the opposite check context are unmatched.
                for history in [
                    None,
                    Some(tree::PreviousStreetLine::LeadCalled),
                    Some(tree::PreviousStreetLine::Raised),
                ] {
                    assert_eq!(
                        table.fraction(&Situation {
                            previous_street_line: history,
                            ..situation
                        }),
                        0.66
                    );
                }
                assert_eq!(
                    table.fraction(&Situation {
                        after_check: player != 1,
                        ..situation
                    }),
                    0.66
                );
            }
        }
        for row in table
            .rules
            .iter()
            .filter(|r| r.street == Some(Street::River))
        {
            assert_eq!(
                row.sources,
                ["data: M4 river 20 flops, 1824 spots, all sizes at 0.1%"]
            );
        }
        table
            .into_size_rule()
            .check_suit_invariance(20261019, 128)
            .unwrap();
    }

    #[test]
    fn board_pairing_is_optional_and_more_specific_than_wildcard() {
        let mut table: RuleTable = serde_json::from_str(
            r#"{
            "version":1,"candidates":[0.33,0.66],"default_fraction":0.66,
            "rules":[{"street":"River","fraction":0.33}]}"#,
        )
        .unwrap();
        assert_eq!(table.rules[0].board_pairing, None);
        assert_eq!(fraction_on(&table, "AsAdAc2h2s", Street::River), 0.33);
        table.rules.push(Rule {
            street: Some(Street::River),
            board_pairing: Some(BoardPairing::FullHouse),
            fraction: 1.25,
            ..Default::default()
        });
        assert_eq!(fraction_on(&table, "AsAdAc2h2s", Street::River), 1.25);
        assert_eq!(fraction_on(&table, "AsAdAc2hTs", Street::River), 0.33);
        table.rules.clear();
        assert_eq!(fraction_on(&table, "AsAdAc2h2s", Street::River), 0.66);
    }

    #[test]
    fn most_specific_row_wins_and_default_applies() {
        assert_eq!(ctx("As7d2c", Street::Flop), 0.33);
        assert_eq!(ctx("Th9h8c", Street::Flop), 1.0);
        assert_eq!(ctx("Kd8c3s", Street::Flop), 0.66);
        assert_eq!(ctx("Kh9h4c2s3h", Street::River), 1.25);
        assert_eq!(ctx("Kh9h4c2s", Street::Turn), 0.66); // default
    }

    #[test]
    fn btn_vs_bb_flop_rule_from_m4() {
        let t: RuleTable =
            serde_json::from_str(include_str!("../../../configs/sizing/srp-btn-vs-bb.json"))
                .unwrap();
        t.validate().unwrap();
        // Connected, unpaired, 8-high or lower: 66%. Everything else: 33%.
        assert_eq!(fraction_on(&t, "7c6d5d", Street::Flop), 0.66);
        assert_eq!(fraction_on(&t, "8c5d4h", Street::Flop), 0.66);
        assert_eq!(fraction_on(&t, "KcQdTh", Street::Flop), 0.33);
        assert_eq!(fraction_on(&t, "7c7d6h", Street::Flop), 0.33);
        assert_eq!(fraction_on(&t, "AcKd4h", Street::Flop), 0.33);
        // Unknown turn and river histories retain the 66% fallback.
        assert_eq!(fraction_on(&t, "7c6d5dKs", Street::Turn), 0.66);
        assert_eq!(fraction_on(&t, "7c6d5dKs2h", Street::River), 0.66);
        t.into_size_rule()
            .check_suit_invariance(20261017, 64)
            .unwrap();
    }

    #[test]
    fn turn_rules_cover_runouts_players_and_previous_street_histories() {
        let table: RuleTable =
            serde_json::from_str(include_str!("../../../configs/sizing/srp-btn-vs-bb.json"))
                .unwrap();
        let cases = [
            ("Qh8d2c8s", RunoutCard::BoardPair, [0.33, 0.66, 0.66, 0.33]),
            (
                "Qh8h2c3h",
                RunoutCard::FlushComplete,
                [1.25, 0.33, 0.66, 0.66],
            ),
            ("Qh8d2cAs", RunoutCard::Overcard, [1.25, 0.33, 1.0, 0.33]),
            (
                "Qh8d2cTc",
                RunoutCard::StraightComplete,
                [1.0, 0.33, 0.66, 0.66],
            ),
            ("Qh8d2c3h", RunoutCard::FlushDraw, [0.33, 0.33, 1.25, 0.33]),
            ("Qh8h2c3s", RunoutCard::Blank, [0.33, 0.33, 1.25, 0.33]),
        ];
        for (board, kind, expected) in cases {
            let ids: Vec<u8> = board
                .as_bytes()
                .chunks(2)
                .map(|c| std::str::from_utf8(c).unwrap().parse::<Card>().unwrap().0)
                .collect();
            for (i, (player, history)) in [(0, true), (0, false), (1, true), (1, false)]
                .into_iter()
                .enumerate()
            {
                let context = SizeContext {
                    board: &ids,
                    street: Street::Turn,
                    player,
                    first_bet: true,
                    after_check: player == 1,
                    previous_street_line: Some(if history {
                        tree::PreviousStreetLine::IpBetCalled
                    } else {
                        tree::PreviousStreetLine::CheckedThrough
                    }),
                };
                let situation = Situation::from_context(&context);
                assert_eq!(situation.runout, Some(kind), "{board}");
                assert_eq!(
                    table.fraction(&situation),
                    expected[i],
                    "{board} p{player} history={history}"
                );
                for history in [
                    None,
                    Some(tree::PreviousStreetLine::LeadCalled),
                    Some(tree::PreviousStreetLine::Raised),
                ] {
                    assert_eq!(
                        table.fraction(&Situation {
                            previous_street_line: history,
                            ..situation
                        }),
                        0.66
                    );
                }
            }
        }
        for row in table
            .rules
            .iter()
            .filter(|r| r.street == Some(Street::Turn))
        {
            assert!(row
                .sources
                .iter()
                .any(|s| s == "data: M4 turn 20 flops, all sizes at 0.1%"));
        }
        table
            .into_size_rule()
            .check_suit_invariance(20261018, 64)
            .unwrap();
    }

    #[test]
    fn compiled_table_is_suit_invariant() {
        let rule = table().into_size_rule();
        assert!(matches!(rule, SizeRule::SuitInvariant(_)));
        rule.check_suit_invariance(20261015, 64).unwrap();
    }

    #[test]
    fn table_round_trips_through_json() {
        let t = table();
        let back: RuleTable =
            serde_json::from_str(&serde_json::to_string_pretty(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        back.validate().unwrap();
    }
}
