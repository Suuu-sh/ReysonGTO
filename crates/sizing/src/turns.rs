//! Turn-card categories and seeded representatives under the flop's suit stabilizer.
use crate::features::{runout_card, RunoutCard};
use crate::flops::{mix, PERMUTATIONS};
use cards::Card;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TurnGroup {
    pub kind: RunoutCard,
    pub cards: Vec<String>,
}

/// Without a limit, returns every remaining card. With a limit, collapses suit
/// orbits first, then selects up to that many representatives per category.
/// The stabilizer preserves the entire unordered flop, including paired ranks.
pub fn turn_cards(
    flop: [Card; 3],
    per_kind: Option<usize>,
    seed: u64,
) -> Result<Vec<TurnGroup>, String> {
    if flop.iter().any(|c| c.0 >= 52) || flop.into_iter().collect::<BTreeSet<_>>().len() != 3 {
        return Err("flop must contain three distinct valid cards".into());
    }
    if per_kind == Some(0) {
        return Err("per-kind must be positive".into());
    }
    let mut sorted = flop;
    sorted.sort_unstable();
    let stabilizer: Vec<_> = PERMUTATIONS
        .iter()
        .filter(|p| {
            let mut mapped = flop.map(|c| Card(c.rank() * 4 + p[c.suit() as usize]));
            mapped.sort_unstable();
            mapped == sorted
        })
        .collect();
    let kinds = [
        RunoutCard::BoardPair,
        RunoutCard::FlushComplete,
        RunoutCard::StraightComplete,
        RunoutCard::Overcard,
        RunoutCard::FlushDraw,
        RunoutCard::Blank,
    ];
    Ok(kinds
        .into_iter()
        .map(|kind| {
            let mut cards: Vec<_> = (0..52)
                .map(Card)
                .filter(|c| !flop.contains(c))
                .filter(|c| runout_card(&flop, *c) == kind)
                .filter(|c| {
                    per_kind.is_none()
                        || stabilizer
                            .iter()
                            .map(|p| Card(c.rank() * 4 + p[c.suit() as usize]))
                            .min()
                            == Some(*c)
                })
                .collect();
            if let Some(n) = per_kind {
                cards.sort_by_key(|c| (mix(seed ^ c.0 as u64), c.0));
                cards.truncate(n);
                cards.sort_unstable();
            }
            TurnGroup {
                kind,
                cards: cards.into_iter().map(|c| c.to_string()).collect(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn flop(text: &str) -> [Card; 3] {
        [
            text[0..2].parse().unwrap(),
            text[2..4].parse().unwrap(),
            text[4..6].parse().unwrap(),
        ]
    }
    #[test]
    fn all_49_are_classified_and_disjoint() {
        let f = flop("7c6d5d");
        let groups = turn_cards(f, None, 42).unwrap();
        let mut seen = BTreeSet::new();
        for group in groups {
            for text in group.cards {
                let c: Card = text.parse().unwrap();
                assert!(!f.contains(&c));
                assert_eq!(runout_card(&f, c), group.kind);
                assert!(seen.insert(c));
            }
        }
        assert_eq!(seen.len(), 49);
    }
    #[test]
    fn representatives_are_deterministic_valid_and_collapse_unused_suits() {
        let f = flop("7c6d5d");
        let a = turn_cards(f, Some(2), 7).unwrap();
        assert_eq!(a, turn_cards(f, Some(2), 7).unwrap());
        assert!(a.iter().all(|g| g.cards.len() <= 2));
        for g in a {
            for s in g.cards {
                let c = s.parse().unwrap();
                assert!(!f.contains(&c));
                assert_eq!(runout_card(&f, c), g.kind);
            }
        }
        let all = turn_cards(f, Some(52), 7).unwrap();
        assert_eq!(all.iter().map(|g| g.cards.len()).sum::<usize>(), 36);
        assert!(all.iter().flat_map(|g| &g.cards).all(|s| !s.ends_with('s')));
    }
    #[test]
    fn paired_flop_stabilizer_can_swap_occupied_suits() {
        let all = turn_cards(flop("7c7d5h"), Some(52), 0).unwrap();
        assert_eq!(all.iter().map(|g| g.cards.len()).sum::<usize>(), 37);
    }
    #[test]
    fn invalid_inputs_fail() {
        assert!(turn_cards(flop("7c7c5d"), None, 0).is_err());
        assert!(turn_cards(flop("7c6d5d"), Some(0), 0).is_err());
    }
}
