//! The 1,755 strategically distinct flops and the choice of representative and check flops (design 12.3).
use crate::features::{flop_features, FlopFeatures};
use cards::Card;
use serde::Serialize;
use std::collections::BTreeMap;

/// One suit-isomorphism class of flops.
#[derive(Clone, Debug, Serialize)]
pub struct CanonicalFlop {
    /// Smallest member under card-id order, cards in descending order.
    pub cards: [Card; 3],
    /// Number of the 22,100 raw flops in this class.
    pub weight: u32,
    pub features: FlopFeatures,
}

impl CanonicalFlop {
    pub fn text(&self) -> String {
        self.cards.iter().map(|c| c.to_string()).collect()
    }
}

pub(crate) const PERMUTATIONS: [[u8; 4]; 24] = {
    let mut out = [[0u8; 4]; 24];
    let mut n = 0;
    let mut a = 0;
    while a < 4 {
        let mut b = 0;
        while b < 4 {
            let mut c = 0;
            while c < 4 {
                let d = 6 - a - b - c;
                if a != b && a != c && b != c && d < 4 && d != a && d != b && d != c {
                    out[n] = [a as u8, b as u8, c as u8, d as u8];
                    n += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }
    out
};

fn sorted_desc(mut cards: [Card; 3]) -> [Card; 3] {
    cards.sort_unstable_by(|a, b| b.cmp(a));
    cards
}

fn canonical(cards: [Card; 3]) -> [Card; 3] {
    PERMUTATIONS
        .iter()
        .map(|p| sorted_desc(cards.map(|c| Card(c.rank() * 4 + p[c.suit() as usize]))))
        .min()
        .unwrap()
}

/// All suit-isomorphism classes of flops, in a stable order.
pub fn canonical_flops() -> Vec<CanonicalFlop> {
    let mut classes: BTreeMap<[Card; 3], u32> = BTreeMap::new();
    for a in 0..52u8 {
        for b in (a + 1)..52 {
            for c in (b + 1)..52 {
                *classes
                    .entry(canonical([Card(a), Card(b), Card(c)]))
                    .or_default() += 1;
            }
        }
    }
    classes
        .into_iter()
        .map(|(cards, weight)| CanonicalFlop {
            cards,
            weight,
            features: flop_features(&cards),
        })
        .collect()
}

/// Deterministic pseudo-random order (splitmix64) so selections are reproducible.
pub(crate) fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn order_key(flop: &CanonicalFlop, seed: u64) -> u64 {
    let id = flop.cards.iter().fold(0u64, |a, c| a * 52 + c.0 as u64);
    mix(id ^ seed)
}

/// Representative flops for building rules and a disjoint set for checking them.
#[derive(Clone, Debug, Serialize)]
pub struct Selection {
    pub training: Vec<CanonicalFlop>,
    pub validation: Vec<CanonicalFlop>,
}

/// Stratified selection by feature bucket: every bucket gets at least one training flop,
/// the rest are spread in proportion to how often the bucket is dealt (raw-flop weight).
/// Validation flops are drawn the same way from the flops not used for training.
pub fn select_flops(training: usize, validation: usize, seed: u64) -> Selection {
    let flops = canonical_flops();
    let mut buckets: BTreeMap<FlopFeatures, Vec<CanonicalFlop>> = BTreeMap::new();
    for f in flops {
        buckets.entry(f.features).or_default().push(f);
    }
    for list in buckets.values_mut() {
        list.sort_by_key(|f| order_key(f, seed));
    }
    let take =
        |buckets: &mut BTreeMap<FlopFeatures, Vec<CanonicalFlop>>, count: usize, salt: u64| {
            let total: u32 = buckets.values().flatten().map(|f| f.weight).sum();
            let nonempty = buckets.values().filter(|l| !l.is_empty()).count();
            let mut picked = Vec::new();
            // One per non-empty bucket first (if the budget allows), then by largest remaining share.
            let mut quotas: Vec<(FlopFeatures, f64, usize)> = buckets
                .iter()
                .filter(|(_, l)| !l.is_empty())
                .map(|(k, l)| {
                    let w: u32 = l.iter().map(|f| f.weight).sum();
                    (
                        *k,
                        count as f64 * w as f64 / total as f64,
                        usize::from(count >= nonempty),
                    )
                })
                .collect();
            let mut assigned: usize = quotas.iter().map(|q| q.2).sum();
            while assigned < count {
                let best = quotas
                    .iter_mut()
                    .filter(|q| q.2 < buckets[&q.0].len())
                    .max_by(|a, b| {
                        (a.1 - a.2 as f64)
                            .total_cmp(&(b.1 - b.2 as f64))
                            .then(order_key_bucket(&b.0, salt).cmp(&order_key_bucket(&a.0, salt)))
                    });
                match best {
                    Some(q) => {
                        q.2 += 1;
                        assigned += 1;
                    }
                    None => break,
                }
            }
            for (key, _, n) in quotas {
                let list = buckets.get_mut(&key).unwrap();
                picked.extend(list.drain(..n.min(list.len())));
            }
            picked.sort_by_key(|f| f.cards);
            picked
        };
    let training = take(&mut buckets, training, seed);
    let validation = take(&mut buckets, validation, seed ^ 0xA5A5);
    Selection {
        training,
        validation,
    }
}

fn order_key_bucket(key: &FlopFeatures, salt: u64) -> u64 {
    let id = (key.height as u64) << 12
        | (key.pairing as u64) << 8
        | (key.suits as u64) << 4
        | key.connectivity as u64;
    mix(id ^ salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_1755_flops_covering_every_raw_flop() {
        let flops = canonical_flops();
        assert_eq!(flops.len(), 1755);
        assert_eq!(flops.iter().map(|f| f.weight).sum::<u32>(), 22_100);
    }

    #[test]
    fn selection_is_disjoint_reproducible_and_covers_buckets() {
        let a = select_flops(200, 80, 7);
        let b = select_flops(200, 80, 7);
        assert_eq!(a.training.len(), 200);
        assert_eq!(a.validation.len(), 80);
        let ids = |s: &[CanonicalFlop]| s.iter().map(|f| f.text()).collect::<Vec<_>>();
        assert_eq!(ids(&a.training), ids(&b.training));
        let train: std::collections::HashSet<_> = ids(&a.training).into_iter().collect();
        assert!(ids(&a.validation).iter().all(|f| !train.contains(f)));
        let buckets: std::collections::HashSet<_> =
            canonical_flops().iter().map(|f| f.features).collect();
        let covered: std::collections::HashSet<_> = a.training.iter().map(|f| f.features).collect();
        if buckets.len() <= 200 {
            assert_eq!(
                covered.len(),
                buckets.len(),
                "every bucket has a training flop"
            );
        }
    }
}
