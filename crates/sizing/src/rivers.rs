//! River-card categories and seeded representatives under the four-card board's suit stabilizer.
use crate::features::{runout_card, RunoutCard};
use crate::flops::{mix, PERMUTATIONS};
use cards::Card;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct RiverGroup {
    pub kind: RunoutCard,
    pub cards: Vec<String>,
}

/// Without a limit, returns every remaining card. With a limit, collapses suit
/// orbits first, then selects up to that many representatives per category.
/// The stabilizer preserves the entire unordered board, including paired ranks.
pub fn river_cards(
    board: [Card; 4],
    per_kind: Option<usize>,
    seed: u64,
) -> Result<Vec<RiverGroup>, String> {
    if board.iter().any(|c| c.0 >= 52) || board.into_iter().collect::<BTreeSet<_>>().len() != 4 {
        return Err("board must contain four distinct valid cards".into());
    }
    if per_kind == Some(0) {
        return Err("per-kind must be positive".into());
    }
    let mut sorted = board;
    sorted.sort_unstable();
    let stabilizer: Vec<_> = PERMUTATIONS
        .iter()
        .filter(|p| {
            let mut mapped = board.map(|c| Card(c.rank() * 4 + p[c.suit() as usize]));
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
                .filter(|c| !board.contains(c))
                .filter(|c| runout_card(&board, *c) == kind)
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
            RiverGroup {
                kind,
                cards: cards.into_iter().map(|c| c.to_string()).collect(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(text: &str) -> [Card; 4] {
        std::array::from_fn(|i| text[i * 2..i * 2 + 2].parse().unwrap())
    }

    #[test]
    fn all_48_are_classified_disjoint_and_never_flush_draw() {
        for text in ["7c6d5dKs", "Qh8h2c3s", "7c7d5h5s"] {
            let b = board(text);
            let groups = river_cards(b, None, 42).unwrap();
            let mut seen = BTreeSet::new();
            assert_eq!(groups.len(), 6);
            for group in groups {
                if group.kind == RunoutCard::FlushDraw {
                    assert!(group.cards.is_empty());
                }
                for text in group.cards {
                    let c: Card = text.parse().unwrap();
                    assert!(!b.contains(&c));
                    assert_eq!(runout_card(&b, c), group.kind);
                    assert!(seen.insert(c));
                }
            }
            assert_eq!(seen.len(), 48);
        }
    }

    #[test]
    fn representatives_are_deterministic_and_collapse_suit_orbits() {
        let b = board("7c6d5dKd");
        let groups = river_cards(b, Some(2), 7).unwrap();
        assert_eq!(groups, river_cards(b, Some(2), 7).unwrap());
        assert!(groups.iter().all(|g| g.cards.len() <= 2));
        for group in groups {
            for text in group.cards {
                let c: Card = text.parse().unwrap();
                assert!(!b.contains(&c));
                assert_eq!(runout_card(&b, c), group.kind);
            }
        }
        let all = river_cards(b, Some(52), 7).unwrap();
        assert_eq!(all.iter().map(|g| g.cards.len()).sum::<usize>(), 35);
        assert!(all.iter().flat_map(|g| &g.cards).all(|s| !s.ends_with('s')));
        let mut reordered = b;
        reordered.reverse();
        assert_eq!(all, river_cards(reordered, Some(52), 7).unwrap());
    }

    #[test]
    fn paired_board_stabilizer_can_swap_occupied_suits() {
        let all = river_cards(board("7c7d5h5s"), Some(52), 0).unwrap();
        assert_eq!(all.iter().map(|g| g.cards.len()).sum::<usize>(), 24);
    }

    #[test]
    fn classification_uses_all_four_previous_cards() {
        let b = board("Qh8h2c3s");
        for (text, expected) in [
            ("3d", RunoutCard::BoardPair),
            ("4h", RunoutCard::FlushComplete),
            ("Tc", RunoutCard::StraightComplete),
            ("Ks", RunoutCard::Overcard),
            ("7d", RunoutCard::Blank),
        ] {
            let groups = river_cards(b, None, 0).unwrap();
            assert!(groups
                .iter()
                .any(|g| g.kind == expected && g.cards.iter().any(|c| c == text)));
        }
    }

    #[test]
    fn invalid_inputs_fail() {
        assert!(river_cards(board("7c7c5dKs"), None, 0).is_err());
        assert!(river_cards(board("7c6d5dKs"), Some(0), 0).is_err());
        let mut b = board("7c6d5dKs");
        b[3] = Card(52);
        assert!(river_cards(b, None, 0).is_err());
    }
}
