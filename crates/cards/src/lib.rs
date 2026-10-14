//! Clean-room card encoding, hand evaluation and suit symmetry.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Card(pub u8);
impl Card {
    pub fn new(id: u8) -> Result<Self, String> {
        if id < 52 {
            Ok(Self(id))
        } else {
            Err("card id must be below 52".into())
        }
    }
    pub fn rank(self) -> u8 {
        self.0 / 4
    }
    pub fn suit(self) -> u8 {
        self.0 % 4
    }
    pub fn mask(self) -> u64 {
        1u64 << self.0
    }
}
impl FromStr for Card {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let b = s.as_bytes();
        if b.len() != 2 {
            return Err("card must have two characters".into());
        }
        let rank = b"23456789TJQKA"
            .iter()
            .position(|&x| x == b[0].to_ascii_uppercase())
            .ok_or("invalid rank")?;
        let suit = b"cdhs"
            .iter()
            .position(|&x| x == b[1].to_ascii_lowercase())
            .ok_or("invalid suit")?;
        Ok(Self((rank * 4 + suit) as u8))
    }
}
impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            b"23456789TJQKA"[self.rank() as usize] as char,
            b"cdhs"[self.suit() as usize] as char
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Combo {
    pub cards: [Card; 2],
}
impl Combo {
    pub fn new(a: Card, b: Card) -> Result<Self, String> {
        if a.0 >= 52 || b.0 >= 52 || a == b {
            return Err("invalid or repeated combo card".into());
        }
        Ok(Self {
            cards: if a < b { [a, b] } else { [b, a] },
        })
    }
    pub fn id(self) -> usize {
        let [a, b] = self.cards;
        let a = a.0 as usize;
        let b = b.0 as usize;
        a * (103 - a) / 2 + b - a - 1
    }
    pub fn from_id(id: usize) -> Result<Self, String> {
        if id >= 1326 {
            return Err("combo id must be below 1326".into());
        }
        let mut n = id;
        for a in 0..51 {
            let count = 51 - a;
            if n < count {
                return Self::new(Card(a as u8), Card((a + 1 + n) as u8));
            }
            n -= count;
        }
        unreachable!()
    }
    pub fn mask(self) -> u64 {
        self.cards[0].mask() | self.cards[1].mask()
    }
}
impl FromStr for Combo {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s.len() != 4 || !s.is_ascii() {
            return Err("combo must contain two cards".into());
        }
        Self::new(s[..2].parse()?, s[2..].parse()?)
    }
}
impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.cards[0], self.cards[1])
    }
}
pub fn validate_board(board: &[Card]) -> Result<u64, String> {
    if board.len() > 5 {
        return Err("board contains more than five cards".into());
    }
    let mut mask = 0;
    for &c in board {
        if c.0 >= 52 {
            return Err("invalid card".into());
        }
        if mask & c.mask() != 0 {
            return Err("duplicate board card".into());
        }
        mask |= c.mask();
    }
    Ok(mask)
}
fn encode(category: u32, ranks: &[u8]) -> u32 {
    let mut value = category << 20;
    for (i, &r) in ranks.iter().enumerate() {
        value |= (r as u32 + 2) << (16 - 4 * i);
    }
    value
}
fn straight(mask: u16) -> Option<u8> {
    for high in (4..13).rev() {
        if mask & (31 << (high - 4)) == 31 << (high - 4) {
            return Some(high);
        }
    }
    if mask & 0x100f == 0x100f {
        Some(3)
    } else {
        None
    }
}
fn top(mask: u16, n: usize) -> Vec<u8> {
    (0..13)
        .rev()
        .filter(|&r| mask & (1 << r) != 0)
        .take(n)
        .map(|r| r as u8)
        .collect()
}
/// Seven distinct valid cards. Larger scores are stronger; equal scores tie.
pub fn evaluate7(cards: &[Card; 7]) -> u32 {
    let mut counts = [0u8; 13];
    let mut suits = [0u16; 4];
    let mut mask = 0u16;
    for &c in cards {
        let r = c.rank() as usize;
        counts[r] += 1;
        suits[c.suit() as usize] |= 1 << r;
        mask |= 1 << r;
    }
    let flush = suits.iter().find(|m| m.count_ones() >= 5).copied();
    if let Some(m) = flush {
        if let Some(r) = straight(m) {
            return encode(8, &[r]);
        }
    }
    let quads = (0..13).rev().find(|&r| counts[r] == 4);
    if let Some(r) = quads {
        return encode(7, &[r as u8, top(mask & !(1 << r), 1)[0]]);
    }
    let trips: Vec<_> = (0..13).rev().filter(|&r| counts[r] >= 3).collect();
    if let Some(&t) = trips.first() {
        if let Some(p) = (0..13).rev().find(|&r| r != t && counts[r] >= 2) {
            return encode(6, &[t as u8, p as u8]);
        }
    }
    if let Some(m) = flush {
        return encode(5, &top(m, 5));
    }
    if let Some(r) = straight(mask) {
        return encode(4, &[r]);
    }
    if let Some(&t) = trips.first() {
        let mut rs = vec![t as u8];
        rs.extend(top(mask & !(1 << t), 2));
        return encode(3, &rs);
    }
    let pairs: Vec<_> = (0..13).rev().filter(|&r| counts[r] >= 2).collect();
    if pairs.len() >= 2 {
        let a = pairs[0];
        let b = pairs[1];
        return encode(
            2,
            &[a as u8, b as u8, top(mask & !(1 << a) & !(1 << b), 1)[0]],
        );
    }
    if let Some(&p) = pairs.first() {
        let mut rs = vec![p as u8];
        rs.extend(top(mask & !(1 << p), 3));
        return encode(1, &rs);
    }
    encode(0, &top(mask, 5))
}
/// Independent reference: enumerate every five-card subset and sort rank groups.
pub fn evaluate7_naive(cards: &[Card; 7]) -> u32 {
    let mut best = 0;
    for a in 0..7 {
        for b in a + 1..7 {
            let mut five = [Card(0); 5];
            let mut n = 0;
            for (i, &c) in cards.iter().enumerate() {
                if i != a && i != b {
                    five[n] = c;
                    n += 1
                }
            }
            best = best.max(evaluate5_naive(&five));
        }
    }
    best
}
fn evaluate5_naive(cards: &[Card; 5]) -> u32 {
    let mut ranks = cards.map(|c| c.rank());
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let flush = cards.iter().all(|c| c.suit() == cards[0].suit());
    let straight_high = if ranks == [12, 3, 2, 1, 0] {
        Some(3)
    } else if ranks.windows(2).all(|w| w[0] == w[1] + 1) {
        Some(ranks[0])
    } else {
        None
    };
    let mut groups = Vec::<(u8, u8)>::new();
    for r in ranks {
        if let Some(g) = groups.iter_mut().find(|g| g.1 == r) {
            g.0 += 1
        } else {
            groups.push((1, r))
        }
    }
    groups.sort_unstable_by(|a, b| b.cmp(a));
    let rs: Vec<_> = groups.iter().map(|g| g.1).collect();
    if flush {
        if let Some(high) = straight_high {
            return encode(8, &[high]);
        }
    }
    if groups[0].0 == 4 {
        return encode(7, &rs);
    }
    if groups[0].0 == 3 && groups[1].0 == 2 {
        return encode(6, &rs);
    }
    if flush {
        return encode(5, &ranks);
    }
    if let Some(r) = straight_high {
        return encode(4, &[r]);
    }
    let cat = if groups[0].0 == 3 {
        3
    } else if groups[0].0 == 2 && groups[1].0 == 2 {
        2
    } else if groups[0].0 == 2 {
        1
    } else {
        0
    };
    encode(cat, &rs)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuitPermutation(pub [u8; 4]);
impl SuitPermutation {
    pub fn map_card(self, c: Card) -> Card {
        Card(c.rank() * 4 + self.0[c.suit() as usize])
    }
    pub fn map_combo(self, c: Combo) -> Combo {
        Combo::new(self.map_card(c.cards[0]), self.map_card(c.cards[1])).unwrap()
    }
}
pub fn board_preserving_permutations(board: &[Card]) -> Result<Vec<SuitPermutation>, String> {
    let mask = validate_board(board)?;
    let mut out = Vec::new();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let p = SuitPermutation([a, b, c, d]);
                    if a == b || a == c || a == d || b == c || b == d || c == d {
                        continue;
                    }
                    let mapped = board.iter().fold(0, |m, &x| m | p.map_card(x).mask());
                    if mapped == mask {
                        out.push(p)
                    }
                }
            }
        }
    }
    Ok(out)
}
#[derive(Clone, Debug)]
pub struct ChanceClass {
    pub representative: Card,
    pub weight: usize,
    pub members: Vec<Card>,
}
/// Board-only symmetry. Range invariance must also be checked before solver reduction.
pub fn chance_classes(board: &[Card]) -> Result<Vec<ChanceClass>, String> {
    let mask = validate_board(board)?;
    let perms = board_preserving_permutations(board)?;
    let mut seen = mask;
    let mut out = Vec::new();
    for id in 0..52 {
        let card = Card(id);
        if seen & card.mask() != 0 {
            continue;
        }
        let mut members: Vec<_> = perms.iter().map(|p| p.map_card(card)).collect();
        members.sort_unstable();
        members.dedup();
        for c in &members {
            seen |= c.mask()
        }
        out.push(ChanceClass {
            representative: card,
            weight: members.len(),
            members,
        });
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_and_symmetry() {
        for i in 0..1326 {
            let c = Combo::from_id(i).unwrap();
            assert_eq!(c.id(), i);
            assert_eq!(c.to_string().parse::<Combo>().unwrap(), c)
        }
        let board = [
            "Ah".parse().unwrap(),
            "Kh".parse().unwrap(),
            "2h".parse().unwrap(),
        ];
        let classes = chance_classes(&board).unwrap();
        assert_eq!(classes.iter().map(|c| c.weight).sum::<usize>(), 49);
        assert_eq!(board_preserving_permutations(&board).unwrap().len(), 6);
        for p in board_preserving_permutations(&board).unwrap() {
            for id in 0..1326 {
                assert!(p.map_combo(Combo::from_id(id).unwrap()).id() < 1326)
            }
        }
    }
    #[test]
    fn categories() {
        let cases = [
            ("AhKhQhJhTh2c3d", 8),
            ("AhAdAsAcKh2c3d", 7),
            ("AhAdAsKhKd2c3d", 6),
            ("AhJh8h4h2h3c5d", 5),
            ("Ah2d3s4c5h8c9d", 4),
            ("AhAdAsKhQd2c3d", 3),
            ("AhAdKhKdQh2c3d", 2),
            ("AhAdKhQdJh2c3d", 1),
            ("AhKdQsJc9h2c3d", 0),
        ];
        for (s, cat) in cases {
            let cards = std::array::from_fn(|i| s[i * 2..i * 2 + 2].parse().unwrap());
            assert_eq!(evaluate7(&cards) >> 20, cat);
            assert_eq!(evaluate7(&cards), evaluate7_naive(&cards))
        }
    }
    #[test]
    fn million_random_hands() {
        let mut state = 0x5ab123987654321u64;
        for _ in 0..1_000_000 {
            let mut cards = [Card(0); 7];
            let mut mask = 0;
            for c in &mut cards {
                loop {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    let id = (state % 52) as u8;
                    if mask & (1u64 << id) == 0 {
                        *c = Card(id);
                        mask |= 1u64 << id;
                        break;
                    }
                }
            }
            assert_eq!(evaluate7(&cards), evaluate7_naive(&cards), "{cards:?}")
        }
    }
    #[test]
    #[ignore = "133,784,560 hands: run explicitly in release mode"]
    fn exhaustive_seven_card_reference() {
        use rayon::prelude::*;
        let start = std::time::Instant::now();
        // A cache of independently evaluated five-card hands makes the exhaustive
        // 21-subset reference affordable without sharing the fast evaluator logic.
        let mut choose = [[0usize; 6]; 53];
        for n in 0..53 {
            choose[n][0] = 1;
            for k in 1..=5 {
                choose[n][k] = if n == 0 {
                    0
                } else {
                    choose[n - 1][k] + choose[n - 1][k - 1]
                }
            }
        }
        let mut table = vec![0u32; 2_598_960];
        for a in 0..48 {
            for b in a + 1..49 {
                for c in b + 1..50 {
                    for d in c + 1..51 {
                        for e in d + 1..52 {
                            let id = choose[a][1]
                                + choose[b][2]
                                + choose[c][3]
                                + choose[d][4]
                                + choose[e][5];
                            table[id] = evaluate5_naive(&[
                                Card(a as u8),
                                Card(b as u8),
                                Card(c as u8),
                                Card(d as u8),
                                Card(e as u8),
                            ]);
                        }
                    }
                }
            }
        }
        let count: u64 = (0..46usize)
            .into_par_iter()
            .map(|a| {
                let mut count = 0;
                for b in a + 1..47 {
                    for c in b + 1..48 {
                        for d in c + 1..49 {
                            for e in d + 1..50 {
                                for f in e + 1..51 {
                                    for g in f + 1..52 {
                                        let ids = [a, b, c, d, e, f, g];
                                        let cards = ids.map(|i| Card(i as u8));
                                        let mut reference = 0;
                                        for x in 0..7 {
                                            for y in x + 1..7 {
                                                let mut id = 0;
                                                let mut k = 1;
                                                for (i, &v) in ids.iter().enumerate() {
                                                    if i != x && i != y {
                                                        id += choose[v][k];
                                                        k += 1
                                                    }
                                                }
                                                reference = reference.max(table[id]);
                                            }
                                        }
                                        assert_eq!(evaluate7(&cards), reference, "{cards:?}");
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                count
            })
            .sum();
        assert_eq!(count, 133_784_560);
        println!(
            "verified {count} hands in {:.3}s",
            start.elapsed().as_secs_f64()
        );
    }
}
