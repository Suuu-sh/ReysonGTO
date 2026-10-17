//! Seeded, blocker-aware class equities. Class averages are not suit-specific
//! combo equities: callers must still exclude overlapping terminal combos.
use cards::{evaluate7, Card, Combo};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const CLASS_COUNT: usize = 169;
const SCHEMA_VERSION: u32 = 1;
const METHOD: &str = "uniform_legal_class_pairs_and_boards_splitmix64_v1";

/// Conventional 13 x 13 matrix, ranks A..2. Upper triangle is suited,
/// lower triangle offsuit, diagonal pairs (AA, AKs, ..., A2s, AKo, KK, ...).
pub fn class_names() -> Vec<String> {
    let ranks = b"AKQJT98765432";
    (0..CLASS_COUNT)
        .map(|i| {
            let (r, c) = (i / 13, i % 13);
            if r == c {
                format!("{}{}", ranks[r] as char, ranks[c] as char)
            } else {
                format!(
                    "{}{}{}",
                    ranks[r.min(c)] as char,
                    ranks[r.max(c)] as char,
                    if r < c { 's' } else { 'o' }
                )
            }
        })
        .collect()
}

/// Returns the conventional matrix index, or None for an invalid name.
pub fn class_index(name: &str) -> Option<usize> {
    class_names().iter().position(|n| n == name)
}

/// `combo` must contain two distinct valid cards, as ensured by Combo::new.
pub fn combo_class(combo: Combo) -> usize {
    let [a, b] = combo.cards;
    let (r, c) = (12 - a.rank() as usize, 12 - b.rank() as usize);
    let (high, low) = (r.min(c), r.max(c));
    if r == c || a.suit() == b.suit() {
        high * 13 + low
    } else {
        low * 13 + high
    }
}

#[derive(Clone, Debug)]
pub struct HandClasses {
    pub names: Vec<String>,
    pub combos: Vec<Vec<Combo>>,
    /// Index by Combo::id().
    pub combo_classes: Vec<usize>,
}
impl Default for HandClasses {
    fn default() -> Self {
        Self::new()
    }
}
impl HandClasses {
    pub fn new() -> Self {
        let mut combos = vec![Vec::new(); CLASS_COUNT];
        let mut combo_classes = Vec::with_capacity(1326);
        for id in 0..1326 {
            let combo = Combo::from_id(id).expect("valid enumerated combo");
            let class = combo_class(combo);
            combos[class].push(combo);
            combo_classes.push(class);
        }
        Self {
            names: class_names(),
            combos,
            combo_classes,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquityMetadata {
    pub schema_version: u32,
    pub method: String,
    pub seed: u64,
    /// Samples per unordered matchup. For the heads-up method, distinct
    /// pairs are sampled and diagonal equity is exactly 0.5; for the
    /// three-way method, each legal unordered triple is sampled.
    pub samples_per_matchup: u64,
    /// Distribution-free upper bound on one estimate's standard error.
    pub standard_error_upper_bound: f64,
    /// Normal-approximation 95% pointwise half-width, NOT a simultaneous
    /// guarantee over all 14,196 sampled pairs, nor a rigorous error bound.
    pub pointwise_95_half_width_upper_bound: f64,
    pub class_averaged_suits: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquityTable {
    pub metadata: EquityMetadata,
    pub classes: Vec<String>,
    /// Row-major full matrix. The reverse cell is set to exactly 1 - forward.
    pub values: Vec<f64>,
}
impl EquityTable {
    /// Generation is deterministic for a fixed seed and sample count,
    /// independent of rayon worker count or scheduling. Uses the existing
    /// cards::evaluate7 evaluator; no external poker implementation.
    pub fn generate(samples_per_matchup: u64, seed: u64) -> Result<Self, String> {
        if samples_per_matchup == 0 {
            return Err("samples_per_matchup must be positive".into());
        }
        let classes = HandClasses::new();
        let pairs: Vec<_> = (0..CLASS_COUNT)
            .flat_map(|a| (a + 1..CLASS_COUNT).map(move |b| (a, b)))
            .collect();
        let results: Vec<_> = pairs
            .par_iter()
            .map(|&(a, b)| {
                let mut rng = Rng::new(matchup_seed(seed, a, b));
                let equity = sample_pair(
                    &classes.combos[a],
                    &classes.combos[b],
                    samples_per_matchup,
                    &mut rng,
                );
                (a, b, equity)
            })
            .collect();
        let mut values = vec![0.5; CLASS_COUNT * CLASS_COUNT];
        for (a, b, equity) in results {
            values[a * CLASS_COUNT + b] = equity;
            values[b * CLASS_COUNT + a] = 1.0 - equity;
        }
        let se = 0.5 / (samples_per_matchup as f64).sqrt();
        Ok(Self {
            metadata: EquityMetadata {
                schema_version: SCHEMA_VERSION,
                method: METHOD.into(),
                seed,
                samples_per_matchup,
                standard_error_upper_bound: se,
                pointwise_95_half_width_upper_bound: 1.96 * se,
                class_averaged_suits: true,
            },
            classes: classes.names,
            values,
        })
    }
    pub fn get(&self, a: usize, b: usize) -> f64 {
        self.values[a * CLASS_COUNT + b]
    }
    pub fn validate(&self) -> Result<(), String> {
        let m = &self.metadata;
        if m.schema_version != SCHEMA_VERSION || m.method != METHOD || !m.class_averaged_suits {
            return Err("unsupported equity table schema or sampling method".into());
        }
        if self.classes != class_names() || self.values.len() != CLASS_COUNT * CLASS_COUNT {
            return Err("invalid equity table class order or matrix size".into());
        }
        if m.samples_per_matchup == 0 {
            return Err("missing equity samples".into());
        }
        let se = 0.5 / (m.samples_per_matchup as f64).sqrt();
        if !m.standard_error_upper_bound.is_finite()
            || !m.pointwise_95_half_width_upper_bound.is_finite()
            || (m.standard_error_upper_bound - se).abs() > 1e-14
            || (m.pointwise_95_half_width_upper_bound - 1.96 * se).abs() > 1e-14
        {
            return Err("invalid equity uncertainty metadata".into());
        }
        for a in 0..CLASS_COUNT {
            if self.get(a, a) != 0.5 {
                return Err("equity diagonal must equal 0.5".into());
            }
            for b in 0..CLASS_COUNT {
                let e = self.get(a, b);
                if !e.is_finite()
                    || !(0.0..=1.0).contains(&e)
                    || (e + self.get(b, a) - 1.0).abs() > 1e-12
                {
                    return Err("invalid equity probability or reverse symmetry".into());
                }
            }
        }
        Ok(())
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let table: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        table.validate()?;
        Ok(table)
    }
}

const THREE_WAY_METHOD: &str = "uniform_legal_class_triples_and_boards_splitmix64_v1";
const THREE_WAY_COUNT: usize = CLASS_COUNT * (CLASS_COUNT + 1) * (CLASS_COUNT + 2) / 6;

/// Class-averaged three-player showdown shares, indexed by unordered triples
/// with repetition in lexicographic order. None means no disjoint private deal.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreeWayEquityTable {
    pub metadata: EquityMetadata,
    pub classes: Vec<String>,
    pub values: Vec<Option<[f64; 3]>>,
    /// FNV-1a-64 accidental-corruption checksum, not an authenticity proof.
    pub content_hash: String,
}
impl ThreeWayEquityTable {
    pub fn generate(samples: u64, seed: u64) -> Result<Self, String> {
        if samples == 0 || samples > u64::MAX / 6 {
            return Err("three-way samples must be positive and at most u64::MAX / 6".into());
        }
        let classes = HandClasses::new();
        let triples: Vec<_> = (0..CLASS_COUNT)
            .flat_map(|a| {
                (a..CLASS_COUNT).flat_map(move |b| (b..CLASS_COUNT).map(move |c| [a, b, c]))
            })
            .collect();
        let completed = std::sync::atomic::AtomicUsize::new(0);
        eprintln!(
            "three-way equity: {} unordered triples, {samples} samples each",
            triples.len()
        );
        let values = triples
            .par_iter()
            .map(|&ids| {
                let result = sample_three_way(&classes, ids, samples, seed);
                let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if done.is_multiple_of(100_000) || done == THREE_WAY_COUNT {
                    eprintln!("three-way equity: {done}/{THREE_WAY_COUNT} triples complete");
                }
                result
            })
            .collect();
        let se = 0.5 / (samples as f64).sqrt();
        let mut table = Self {
            metadata: EquityMetadata {
                schema_version: SCHEMA_VERSION,
                method: THREE_WAY_METHOD.into(),
                seed,
                samples_per_matchup: samples,
                standard_error_upper_bound: se,
                pointwise_95_half_width_upper_bound: 1.96 * se,
                class_averaged_suits: true,
            },
            classes: classes.names,
            values,
            content_hash: String::new(),
        };
        table.content_hash = table.compute_hash()?;
        Ok(table)
    }
    /// Shares follow the caller's player order. Invalid classes and impossible
    /// deals return None. Identical classes have equal shares by exchangeability.
    pub fn get(&self, a: usize, b: usize, c: usize) -> Option<[f64; 3]> {
        if [a, b, c].iter().any(|&id| id >= CLASS_COUNT) {
            return None;
        }
        let mut ordered = [(a, 0), (b, 1), (c, 2)];
        ordered.sort_unstable();
        let value = self
            .values
            .get(triple_index(ordered[0].0, ordered[1].0, ordered[2].0))?
            .as_ref()?;
        let mut result = [0.0; 3];
        for i in 0..3 {
            result[ordered[i].1] = value[i];
        }
        Some(result)
    }
    fn compute_hash(&self) -> Result<String, String> {
        // Hash the values' bit patterns, not their JSON text: serde_json's default
        // float parser is not guaranteed to round-trip every f64 bit-exactly, so a
        // text hash computed before saving could mismatch after loading.
        let mut bytes =
            serde_json::to_vec(&(&self.metadata, &self.classes)).map_err(|e| e.to_string())?;
        for value in &self.values {
            match value {
                None => bytes.push(0),
                Some(e) => {
                    bytes.push(1);
                    for v in e {
                        bytes.extend_from_slice(&v.to_bits().to_le_bytes());
                    }
                }
            }
        }
        Ok(fnv1a64(&bytes))
    }
    fn legacy_json_hash(&self) -> Result<String, String> {
        let bytes = serde_json::to_vec(&(&self.metadata, &self.classes, &self.values))
            .map_err(|e| e.to_string())?;
        Ok(fnv1a64(&bytes))
    }
    pub fn validate(&self) -> Result<(), String> {
        let m = &self.metadata;
        if m.schema_version != SCHEMA_VERSION
            || m.method != THREE_WAY_METHOD
            || !m.class_averaged_suits
            || m.samples_per_matchup == 0
            || m.samples_per_matchup > u64::MAX / 6
            || self.classes != class_names()
            || self.values.len() != THREE_WAY_COUNT
        {
            return Err("invalid three-way schema, classes, size or samples".into());
        }
        let se = 0.5 / (m.samples_per_matchup as f64).sqrt();
        if !m.standard_error_upper_bound.is_finite()
            || !m.pointwise_95_half_width_upper_bound.is_finite()
            || (m.standard_error_upper_bound - se).abs() > 1e-14
            || (m.pointwise_95_half_width_upper_bound - 1.96 * se).abs() > 1e-14
        {
            return Err("invalid three-way uncertainty metadata".into());
        }
        let classes = HandClasses::new();
        let mut index = 0;
        for a in 0..CLASS_COUNT {
            for b in a..CLASS_COUNT {
                for c in b..CLASS_COUNT {
                    let groups = [
                        classes.combos[a].as_slice(),
                        classes.combos[b].as_slice(),
                        classes.combos[c].as_slice(),
                    ];
                    let value = self.values[index];
                    index += 1;
                    if value.is_some() != joint_deal_exists(&groups, 0, 0) {
                        return Err("invalid three-way legal-deal marker".into());
                    }
                    if let Some(e) = value {
                        if e.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                            || (e.iter().sum::<f64>() - 1.0).abs() > 1e-12
                            || (a == b && e[0] != e[1])
                            || (b == c && e[1] != e[2])
                        {
                            return Err("invalid three-way shares or exchangeability".into());
                        }
                    }
                }
            }
        }
        // Tables saved before the bit-pattern hash carry a hash of their JSON text;
        // with float_roundtrip they reload bit-exactly, so that hash still verifies.
        if self.content_hash != self.compute_hash()?
            && self.content_hash != self.legacy_json_hash()?
        {
            return Err("three-way content hash mismatch".into());
        }
        Ok(())
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(path, serde_json::to_vec(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let table: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        table.validate()?;
        Ok(table)
    }
}
fn fnv1a64(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    });
    format!("fnv1a64:{hash:016x}")
}
fn triple_index(a: usize, b: usize, c: usize) -> usize {
    // Subtract all triples whose first class is at least a, then skip the
    // rows with second class below b. Constant-time canonical indexing.
    let n = CLASS_COUNT - a;
    let first = THREE_WAY_COUNT - n * (n + 1) * (n + 2) / 6;
    let rows = b - a;
    first + rows * CLASS_COUNT - rows * (2 * a + rows.saturating_sub(1)) / 2 + c - b
}
fn sample_three_way(
    classes: &HandClasses,
    ids: [usize; 3],
    samples: u64,
    seed: u64,
) -> Option<[f64; 3]> {
    let groups = ids.map(|id| classes.combos[id].as_slice());
    if !joint_deal_exists(&groups, 0, 0) {
        return None;
    }
    let key = (ids[0] * CLASS_COUNT * CLASS_COUNT + ids[1] * CLASS_COUNT + ids[2]) as u64;
    let mut rng = Rng::new(seed ^ key.wrapping_mul(0xd6e8_feb8_6659_fd93));
    let mut points = [0u64; 3];
    for _ in 0..samples {
        // Independent class-combo draws followed by whole-deal rejection are
        // uniform over legal joint assignments (sequential conditioning is not).
        let hands = loop {
            let hands = groups.map(|g| g[rng.below(g.len() as u64) as usize]);
            if hands[0].mask() & hands[1].mask() == 0
                && (hands[0].mask() | hands[1].mask()) & hands[2].mask() == 0
            {
                break hands;
            }
        };
        let board = sample_board(hands.iter().fold(0, |m, h| m | h.mask()), &mut rng);
        let ranks = hands.map(|h| score(h, board));
        let best = *ranks.iter().max().unwrap();
        let winners = ranks.iter().filter(|&&r| r == best).count() as u64;
        for p in 0..3 {
            if ranks[p] == best {
                points[p] += 6 / winners;
            }
        }
    }
    let mut result = points.map(|p| p as f64 / (6.0 * samples as f64));
    // Rao-Blackwellize seat permutations of repeated classes. This preserves
    // unbiasedness while making query ordering unambiguous for repeated IDs.
    if ids[0] == ids[2] {
        result = [1.0 / 3.0; 3];
    } else if ids[0] == ids[1] {
        let e = (result[0] + result[1]) / 2.0;
        result[0] = e;
        result[1] = e;
    } else if ids[1] == ids[2] {
        let e = (result[1] + result[2]) / 2.0;
        result[1] = e;
        result[2] = e;
    }
    Some(result)
}

/// On-demand approximation for 2..6 players: samples all class combos
/// independently, rejects the entire deal on overlap, then samples a legal
/// board. This is uniform over legal joint suit assignments, not a biased
/// sequential choice. Splits ties equally among ALL winning players.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MultiwayEstimate {
    pub equities: Vec<f64>,
    pub samples: u64,
    pub seed: u64,
    pub standard_error_upper_bound: f64,
    pub pointwise_95_half_width_upper_bound: f64,
    pub class_averaged_suits: bool,
}
pub fn estimate_multiway(
    class_ids: &[usize],
    samples: u64,
    seed: u64,
) -> Result<MultiwayEstimate, String> {
    if !(2..=6).contains(&class_ids.len())
        || samples == 0
        || class_ids.iter().any(|&c| c >= CLASS_COUNT)
    {
        return Err("multiway requires 2..6 valid classes and positive samples".into());
    }
    let classes = HandClasses::new();
    let groups: Vec<_> = class_ids
        .iter()
        .map(|&c| classes.combos[c].as_slice())
        .collect();
    if !joint_deal_exists(&groups, 0, 0) {
        return Err("class combination has no legal disjoint deal".into());
    }
    let mut rng = Rng::new(seed);
    let mut sums = vec![0.0; class_ids.len()];
    let mut hands = [Combo {
        cards: [Card(0), Card(1)],
    }; 6];
    let mut ranks = [0; 6];
    for _ in 0..samples {
        let mut attempts = 0;
        let used = loop {
            attempts += 1;
            if attempts > 1_000_000 {
                return Err("multiway legal-deal rejection limit exceeded".into());
            }
            let mut used = 0;
            let mut legal = true;
            for (p, choices) in groups.iter().enumerate() {
                let hand = choices[rng.below(choices.len() as u64) as usize];
                if used & hand.mask() != 0 {
                    legal = false;
                    break;
                }
                used |= hand.mask();
                hands[p] = hand;
            }
            if legal {
                break used;
            }
        };
        let board = sample_board(used, &mut rng);
        for p in 0..groups.len() {
            ranks[p] = score(hands[p], board);
        }
        let best = *ranks[..groups.len()].iter().max().unwrap();
        let winners = ranks[..groups.len()].iter().filter(|&&r| r == best).count();
        for p in 0..groups.len() {
            if ranks[p] == best {
                sums[p] += 1.0 / winners as f64;
            }
        }
    }
    for e in &mut sums {
        *e /= samples as f64;
    }
    let se = 0.5 / (samples as f64).sqrt();
    Ok(MultiwayEstimate {
        equities: sums,
        samples,
        seed,
        standard_error_upper_bound: se,
        pointwise_95_half_width_upper_bound: 1.96 * se,
        class_averaged_suits: true,
    })
}
fn joint_deal_exists(groups: &[&[Combo]], p: usize, used: u64) -> bool {
    p == groups.len()
        || groups[p]
            .iter()
            .any(|h| h.mask() & used == 0 && joint_deal_exists(groups, p + 1, used | h.mask()))
}

/// A focused matchup without generating the entire matrix. Same stream as
/// generation for distinct classes; reverse and self matchups are canonical.
pub fn estimate_heads_up(a: usize, b: usize, samples: u64, seed: u64) -> Result<f64, String> {
    if a >= CLASS_COUNT || b >= CLASS_COUNT || samples == 0 {
        return Err("heads-up requires valid classes and positive samples".into());
    }
    if a == b {
        return Ok(0.5);
    }
    let classes = HandClasses::new();
    let (lo, hi) = (a.min(b), a.max(b));
    let mut rng = Rng::new(matchup_seed(seed, lo, hi));
    let equity = sample_pair(&classes.combos[lo], &classes.combos[hi], samples, &mut rng);
    Ok(if a == lo { equity } else { 1.0 - equity })
}
fn sample_pair(a: &[Combo], b: &[Combo], samples: u64, rng: &mut Rng) -> f64 {
    let legal: Vec<_> = a
        .iter()
        .flat_map(|&a| {
            b.iter()
                .filter_map(move |&b| (a.mask() & b.mask() == 0).then_some((a, b)))
        })
        .collect();
    let mut points = 0u64;
    for _ in 0..samples {
        let (a, b) = legal[rng.below(legal.len() as u64) as usize];
        let board = sample_board(a.mask() | b.mask(), rng);
        points += match score(a, board).cmp(&score(b, board)) {
            std::cmp::Ordering::Greater => 2,
            std::cmp::Ordering::Equal => 1,
            std::cmp::Ordering::Less => 0,
        };
    }
    points as f64 / (2.0 * samples as f64)
}
fn sample_board(mut used: u64, rng: &mut Rng) -> [Card; 5] {
    let mut board = [Card(0); 5];
    for card in &mut board {
        loop {
            let id = rng.below(52) as u8;
            if used & (1u64 << id) == 0 {
                used |= 1u64 << id;
                *card = Card(id);
                break;
            }
        }
    }
    board
}
#[inline]
fn score(hand: Combo, board: [Card; 5]) -> u32 {
    evaluate7(&[
        hand.cards[0],
        hand.cards[1],
        board[0],
        board[1],
        board[2],
        board[3],
        board[4],
    ])
}
fn matchup_seed(seed: u64, a: usize, b: usize) -> u64 {
    seed ^ ((a * CLASS_COUNT + b) as u64).wrapping_mul(0xd6e8_feb8_6659_fd93)
}
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Rejection removes modulo bias, including for legal-pair counts.
    fn below(&mut self, n: u64) -> u64 {
        let threshold = n.wrapping_neg() % n;
        loop {
            let x = self.next();
            if x >= threshold {
                return x % n;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn class(name: &str) -> usize {
        class_names().iter().position(|n| n == name).unwrap()
    }
    #[test]
    fn classes_cover_every_combo_once() {
        let c = HandClasses::new();
        assert_eq!(c.combos.iter().map(Vec::len).sum::<usize>(), 1326);
        for (id, combos) in c.combos.iter().enumerate() {
            assert_eq!(
                combos.len(),
                if id / 13 == id % 13 {
                    6
                } else if id / 13 < id % 13 {
                    4
                } else {
                    12
                }
            );
            for &h in combos {
                assert_eq!(combo_class(h), id);
                assert_eq!(c.combo_classes[h.id()], id);
            }
        }
        assert_eq!(combo_class("AcKd".parse().unwrap()), class("AKo"));
        assert_eq!(combo_class("AcKc".parse().unwrap()), class("AKs"));
    }
    #[test]
    fn known_matchups_and_reproducibility() {
        let aa_kk = estimate_heads_up(class("AA"), class("KK"), 20_000, 42).unwrap();
        let aks_22 = estimate_heads_up(class("AKs"), class("22"), 20_000, 42).unwrap();
        assert!((aa_kk - 0.82).abs() < 0.015, "AA/KK={aa_kk}");
        assert!((aks_22 - 0.50).abs() < 0.02, "AKs/22={aks_22}");
        let ako_qq = estimate_heads_up(class("AKo"), class("QQ"), 20_000, 42).unwrap();
        assert!((ako_qq - 0.43).abs() < 0.02, "AKo/QQ={ako_qq}");
        assert_eq!(
            aa_kk,
            estimate_heads_up(class("AA"), class("KK"), 20_000, 42).unwrap()
        );
        assert_eq!(
            aa_kk + estimate_heads_up(class("KK"), class("AA"), 20_000, 42).unwrap(),
            1.0
        );
    }
    #[test]
    fn matrix_symmetry_roundtrip_and_corruption() {
        let table = EquityTable::generate(2, 123).unwrap();
        table.validate().unwrap();
        let single_threaded = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| EquityTable::generate(2, 123).unwrap());
        assert_eq!(table.values, single_threaded.values);
        for a in 0..CLASS_COUNT {
            for b in 0..CLASS_COUNT {
                assert_eq!(table.get(a, b) + table.get(b, a), 1.0);
            }
        }
        let path =
            std::env::temp_dir().join(format!("reyson-equity-test-{}.json", std::process::id()));
        table.save(&path).unwrap();
        let loaded = EquityTable::load(&path).unwrap();
        fs::write(&path, b"{truncated").unwrap();
        assert!(EquityTable::load(&path).is_err());
        fs::remove_file(&path).unwrap();
        assert_eq!(loaded.values, table.values);
        assert_eq!(loaded.metadata.seed, 123);
        let mut bad = table.clone();
        bad.values[1] = -0.1;
        assert!(bad.validate().is_err());
        let mut bad = table.clone();
        bad.classes.swap(0, 1);
        assert!(bad.validate().is_err());
        let mut bad = table.clone();
        bad.metadata.samples_per_matchup = 0;
        assert!(bad.validate().is_err());
        let mut bad = table.clone();
        bad.values.pop();
        assert!(bad.validate().is_err());
        let mut bad = table.clone();
        bad.values[0] = f64::NAN;
        assert!(bad.validate().is_err());
        let mut bad = table.clone();
        bad.metadata.standard_error_upper_bound = 0.0;
        assert!(bad.validate().is_err());
        assert!(EquityTable::generate(0, 1).is_err());
    }
    #[test]
    fn multiway_conserves_pot_and_rejects_impossible_deals() {
        let ids = [class("AA"), class("KK"), class("QQ")];
        let result = estimate_multiway(&ids, 2_000, 42).unwrap();
        assert!((result.equities.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(result.equities[0] > result.equities[1]);
        assert_eq!(
            result.equities,
            estimate_multiway(&ids, 2_000, 42).unwrap().equities
        );
        assert!(estimate_multiway(&[class("AA"); 3], 1, 1).is_err());
        assert!(estimate_multiway(
            &[
                class("AKs"),
                class("AKs"),
                class("KQs"),
                class("KQs"),
                class("AQs"),
                class("AQs")
            ],
            1,
            1
        )
        .is_err());
    }
    /// Small throughput probe only; never creates or saves a production table.
    #[test]
    #[ignore]
    fn three_way_generation_estimate() {
        let classes = HandClasses::new();
        let triples: Vec<_> = (0..CLASS_COUNT)
            .flat_map(|a| {
                (a..CLASS_COUNT).flat_map(move |b| (b..CLASS_COUNT).map(move |c| [a, b, c]))
            })
            .collect();
        let subset: Vec<_> = (0..100).map(|i| triples[i * triples.len() / 100]).collect();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let measure = |samples| {
            let start = std::time::Instant::now();
            let values: Vec<_> = pool.install(|| {
                subset
                    .par_iter()
                    .map(|&ids| sample_three_way(&classes, ids, samples, 123))
                    .collect()
            });
            std::hint::black_box(values);
            start.elapsed().as_secs_f64()
        };
        let tiny = measure(1);
        let larger = measure(1000);
        let sampling_seconds = (larger - tiny).max(0.0) / 999.0;
        let projected = sampling_seconds * 1000.0 * THREE_WAY_COUNT as f64 / subset.len() as f64;
        eprintln!("three-way probe: 100 evenly spaced triples, threads=4, sample1={tiny:.6}s sample1000={larger:.6}s; setup-subtracted projected full sample1000={projected:.3}s (approximate, no IO/hash; suit rejection mix and scaling may vary)");
    }
    #[test]
    fn three_way_known_matchup_and_indexing() {
        let classes = HandClasses::new();
        let ids = [class("AA"), class("KK"), class("QQ")];
        let e = sample_three_way(&classes, ids, 20_000, 42).unwrap();
        assert!((e[0] - 0.69).abs() < 0.025, "AA/KK/QQ={e:?}");
        assert!((e.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(sample_three_way(&classes, [class("AA"); 3], 1, 42).is_none());
        let mut index = 0;
        for a in 0..CLASS_COUNT {
            for b in a..CLASS_COUNT {
                for c in b..CLASS_COUNT {
                    assert_eq!(triple_index(a, b, c), index);
                    index += 1;
                }
            }
        }
        assert_eq!(index, THREE_WAY_COUNT);
    }
    #[test]
    fn three_way_roundtrip_order_corruption_and_threads() {
        let make = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| ThreeWayEquityTable::generate(1, 123).unwrap())
        };
        let table = make(2);
        let single = make(1);
        assert_eq!(table.values, single.values);
        assert_eq!(table.content_hash, single.content_hash);
        let [a, b, c] = [class("AA"), class("KK"), class("QQ")];
        let e = table.get(a, b, c).unwrap();
        assert_eq!(table.get(c, a, b), Some([e[2], e[0], e[1]]));
        assert_eq!(table.get(b, c, a), Some([e[1], e[2], e[0]]));
        assert!(table.get(a, a, a).is_none());
        assert!(table.get(CLASS_COUNT, b, c).is_none());
        let repeated = table.get(a, a, b).unwrap();
        assert_eq!(repeated[0], repeated[1]);
        let path =
            std::env::temp_dir().join(format!("reyson-three-way-test-{}.json", std::process::id()));
        table.save(&path).unwrap();
        let loaded = ThreeWayEquityTable::load(&path).unwrap();
        assert_eq!(loaded.values, table.values);
        // A table saved with the earlier JSON-text hash still validates.
        let mut legacy = table.clone();
        legacy.content_hash = legacy.legacy_json_hash().unwrap();
        assert_ne!(legacy.content_hash, table.content_hash);
        legacy.validate().unwrap();
        // Preserve valid probability sums: checksum still detects altered data.
        let mut bad = table.clone();
        let i = triple_index(a, b, c);
        bad.values[i] = Some(if e == [1.0, 0.0, 0.0] {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        });
        assert!(bad.validate().is_err());
        let mut bad = table.clone();
        bad.metadata.seed += 1;
        assert!(bad.validate().is_err());
        fs::write(&path, b"{truncated").unwrap();
        assert!(ThreeWayEquityTable::load(&path).is_err());
        assert!(ThreeWayEquityTable::generate(0, 1).is_err());
    }
    #[test]
    fn sampled_deals_have_no_duplicate_cards() {
        let mut rng = Rng::new(42);
        let hand: Combo = "AcKd".parse().unwrap();
        for _ in 0..1000 {
            let board = sample_board(hand.mask(), &mut rng);
            assert_eq!(cards::validate_board(&board).unwrap() & hand.mask(), 0);
        }
    }
}
