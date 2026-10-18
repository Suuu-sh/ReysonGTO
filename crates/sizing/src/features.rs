//! Suit-invariant board features used as keys of the bet-size rules (design 12.2).
use cards::Card;
use serde::{Deserialize, Serialize};

/// Height of the highest board card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Height {
    Ace,
    King,
    QueenJack,
    TenEight,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Pairing {
    Unpaired,
    Paired,
    Trips,
}

/// Rank multiplicities of the entire current board, independent of suit labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BoardPairing {
    Unpaired,
    OnePair,
    TwoPair,
    Trips,
    FullHouse,
    Quads,
}

pub fn board_pairing(board: &[Card]) -> BoardPairing {
    assert!(
        (3..=5).contains(&board.len()),
        "a current board has 3 to 5 cards"
    );
    let mut counts = [0u8; 13];
    for card in board {
        counts[card.rank() as usize] += 1;
    }
    let pairs = counts.iter().filter(|&&count| count == 2).count();
    if counts.contains(&4) {
        BoardPairing::Quads
    } else if counts.contains(&3) {
        if pairs > 0 {
            BoardPairing::FullHouse
        } else {
            BoardPairing::Trips
        }
    } else {
        match pairs {
            0 => BoardPairing::Unpaired,
            1 => BoardPairing::OnePair,
            _ => BoardPairing::TwoPair,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SuitPattern {
    Rainbow,
    TwoTone,
    Monotone,
}

/// How many distinct board ranks fit in one five-rank straight window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Connectivity {
    /// No window holds two ranks: no straight draws from the board.
    Disconnected,
    /// Two ranks share a window: straight draws exist.
    Gapped,
    /// Three ranks share a window: a straight is possible with two hole cards.
    Connected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FlopFeatures {
    pub height: Height,
    pub pairing: Pairing,
    pub suits: SuitPattern,
    pub connectivity: Connectivity,
}

/// What the newly dealt turn or river card changed. Ordered by precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RunoutCard {
    /// Matches a rank already on the board.
    BoardPair,
    /// Third card of one suit: a flush is possible.
    FlushComplete,
    /// A straight became possible with two hole cards (or one, for four in a window).
    StraightComplete,
    /// Higher than every earlier board card.
    Overcard,
    /// Turn only: second card of a suit on a rainbow flop.
    FlushDraw,
    Blank,
}

fn height(top_rank: u8) -> Height {
    match top_rank {
        12 => Height::Ace,
        11 => Height::King,
        9 | 10 => Height::QueenJack,
        6..=8 => Height::TenEight,
        _ => Height::Low,
    }
}

/// Distinct ranks as a bit set where the ace also counts as the lowest straight card.
fn rank_bits(cards: &[Card]) -> u16 {
    let mut bits = 0u16;
    for c in cards {
        bits |= 1 << (c.rank() + 1);
        if c.rank() == 12 {
            bits |= 1;
        }
    }
    bits
}

/// Largest number of distinct ranks inside any five-rank straight window.
pub fn max_straight_window(cards: &[Card]) -> u32 {
    let bits = rank_bits(cards);
    (0..=9)
        .map(|low| (bits >> low & 0b1_1111).count_ones())
        .max()
        .unwrap_or(0)
}

fn max_suit_count(cards: &[Card]) -> usize {
    (0..4)
        .map(|s| cards.iter().filter(|c| c.suit() == s).count())
        .max()
        .unwrap_or(0)
}

pub fn flop_features(flop: &[Card]) -> FlopFeatures {
    assert_eq!(flop.len(), 3, "a flop has three cards");
    let mut ranks: Vec<u8> = flop.iter().map(|c| c.rank()).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let distinct = {
        let mut d = ranks.clone();
        d.dedup();
        d.len()
    };
    let pairing = match distinct {
        3 => Pairing::Unpaired,
        2 => Pairing::Paired,
        _ => Pairing::Trips,
    };
    let suits = match max_suit_count(flop) {
        1 => SuitPattern::Rainbow,
        2 => SuitPattern::TwoTone,
        _ => SuitPattern::Monotone,
    };
    let connectivity = match max_straight_window(flop) {
        0 | 1 => Connectivity::Disconnected,
        2 => Connectivity::Gapped,
        _ => Connectivity::Connected,
    };
    FlopFeatures {
        height: height(ranks[0]),
        pairing,
        suits,
        connectivity,
    }
}

/// Category of `card` dealt onto `before` (the flop, or flop + turn).
pub fn runout_card(before: &[Card], card: Card) -> RunoutCard {
    let after: Vec<Card> = before.iter().copied().chain([card]).collect();
    if before.iter().any(|c| c.rank() == card.rank()) {
        return RunoutCard::BoardPair;
    }
    if max_suit_count(&after) >= 3 && max_suit_count(before) < 3 {
        return RunoutCard::FlushComplete;
    }
    let (was, now) = (max_straight_window(before), max_straight_window(&after));
    if now >= 3 && now > was {
        return RunoutCard::StraightComplete;
    }
    if before.iter().all(|c| c.rank() < card.rank()) {
        return RunoutCard::Overcard;
    }
    if before.len() == 3 && max_suit_count(before) == 1 && max_suit_count(&after) == 2 {
        return RunoutCard::FlushDraw;
    }
    RunoutCard::Blank
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cards(s: &str) -> Vec<Card> {
        s.as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap().parse().unwrap())
            .collect()
    }

    #[test]
    fn whole_board_pairing_all_shapes_and_streets() {
        for (board, expected) in [
            ("As7d2c", BoardPairing::Unpaired),
            ("AsAd2c", BoardPairing::OnePair),
            ("AsAdAc", BoardPairing::Trips),
            ("As7d2c9h", BoardPairing::Unpaired),
            ("As7d2c7h", BoardPairing::OnePair),
            ("AsAd2c2h", BoardPairing::TwoPair),
            ("AsAdAc2h", BoardPairing::Trips),
            ("AsAdAcAh", BoardPairing::Quads),
            ("As7d2c9hTs", BoardPairing::Unpaired),
            ("As7d2c7hTs", BoardPairing::OnePair),
            ("AsAd2c2hTs", BoardPairing::TwoPair),
            ("AsAdAc2hTs", BoardPairing::Trips),
            ("AsAdAc2h2s", BoardPairing::FullHouse),
            ("AsAdAcAhTs", BoardPairing::Quads),
        ] {
            let original = cards(board);
            assert_eq!(board_pairing(&original), expected, "{board}");
            for shift in 0..4 {
                let relabeled: Vec<_> = original
                    .iter()
                    .map(|c| Card(c.rank() * 4 + (c.suit() + shift) % 4))
                    .collect();
                assert_eq!(board_pairing(&relabeled), expected);
            }
        }
    }

    #[test]
    fn flop_feature_examples() {
        let f = flop_features(&cards("As7d2c"));
        assert_eq!(
            (f.height, f.pairing, f.suits, f.connectivity),
            (
                Height::Ace,
                Pairing::Unpaired,
                SuitPattern::Rainbow,
                Connectivity::Gapped
            )
        );
        let f = flop_features(&cards("Th9h8c"));
        assert_eq!(
            (f.height, f.suits, f.connectivity),
            (
                Height::TenEight,
                SuitPattern::TwoTone,
                Connectivity::Connected
            )
        );
        let f = flop_features(&cards("KcKd4h"));
        assert_eq!((f.height, f.pairing), (Height::King, Pairing::Paired));
        let f = flop_features(&cards("Ks7s2s"));
        assert_eq!(
            (f.height, f.suits, f.connectivity),
            (
                Height::King,
                SuitPattern::Monotone,
                Connectivity::Disconnected
            )
        );
        // Q and 8 share the 8-Q window, so gutshots exist.
        assert_eq!(
            flop_features(&cards("Qs8s3s")).connectivity,
            Connectivity::Gapped
        );
        // The wheel counts: A-3-5 share a window.
        assert_eq!(
            flop_features(&cards("As5d3c")).connectivity,
            Connectivity::Connected
        );
    }

    #[test]
    fn runout_examples() {
        let flop = cards("Qh8d2c");
        assert_eq!(
            runout_card(&flop, "8s".parse().unwrap()),
            RunoutCard::BoardPair
        );
        assert_eq!(
            runout_card(&flop, "As".parse().unwrap()),
            RunoutCard::Overcard
        );
        assert_eq!(
            runout_card(&flop, "5h".parse().unwrap()),
            RunoutCard::FlushDraw
        );
        assert_eq!(
            runout_card(&flop, "Tc".parse().unwrap()),
            RunoutCard::StraightComplete
        );
        assert_eq!(runout_card(&flop, "4s".parse().unwrap()), RunoutCard::Blank);
        let two_tone = cards("Kh9h4c");
        assert_eq!(
            runout_card(&two_tone, "2h".parse().unwrap()),
            RunoutCard::FlushComplete
        );
    }

    #[test]
    fn features_ignore_suit_relabeling() {
        let a = flop_features(&cards("Ah7h2c"));
        let b = flop_features(&cards("As7s2d"));
        assert_eq!(a, b);
    }
}
