//! External configuration schema. Product configurations are read, not bundled.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const CHIPS_PER_BB: i64 = 1_000;
fn default_game() -> String {
    "Cash".to_owned()
}
fn default_node_limit() -> usize {
    2_000_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreflopConfig {
    #[serde(default = "default_game")]
    pub game: String,
    pub players: usize,
    pub stack_bb: f64,
    #[serde(default)]
    pub ante_bb: f64,
    pub rake: RakeConfig,
    pub positions: Vec<String>,
    pub sizing: SizingConfig,
    #[serde(default)]
    pub solver: SolverConfig,
    /// An already-folded seat's dead money, useful for BTN-versus-BB tests.
    #[serde(default)]
    pub dead_blind_bb: f64,
    #[serde(default = "default_node_limit")]
    pub node_limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RakeConfig {
    pub rate: f64,
    pub cap_bb: f64,
    pub no_flop_no_drop: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
impl RakeConfig {
    pub fn amount_bb(&self, pot_bb: f64, saw_flop: bool) -> f64 {
        if !saw_flop && self.no_flop_no_drop {
            0.0
        } else {
            (pot_bb * self.rate).min(self.cap_bb)
        }
    }
}
fn default_five_bet_multiplier() -> f64 {
    2.2
}
fn default_five_bet_merge_remaining_fraction() -> f64 {
    0.67
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SizingConfig {
    pub open_sizes_bb: Vec<f64>,
    #[serde(default)]
    pub open_sizes_by_position: BTreeMap<String, f64>,
    pub limp: LimpConfig,
    /// Legacy metadata accepted from the product; fixed targets below are authoritative.
    #[serde(default)]
    pub three_bet_ip_multiplier: Option<f64>,
    #[serde(default)]
    pub three_bet_oop_multiplier: Option<f64>,
    #[serde(default)]
    pub four_bet_multiplier: Option<f64>,
    pub fixed_raise_to_bb: FixedRaiseConfig,
    /// Legacy metadata; the current tree always offers a 5bet jam.
    pub five_bet_all_in: bool,
    #[serde(default = "default_five_bet_multiplier")]
    pub five_bet_multiplier: f64,
    /// Merge the fixed 5bet when its additional chips reach this fraction of remaining chips.
    #[serde(default = "default_five_bet_merge_remaining_fraction")]
    pub five_bet_merge_remaining_fraction: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LimpConfig {
    pub sb_complete_to_bb: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PositionSizes {
    pub ip: f64,
    pub oop: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SqueezeSizes {
    pub ip: f64,
    pub oop: f64,
    pub per_additional_caller: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixedRaiseConfig {
    pub three_bet: PositionSizes,
    pub three_bet_vs_sb_open: f64,
    pub iso_vs_limp: f64,
    pub limp_reraise: f64,
    pub squeeze: SqueezeSizes,
    pub four_bet: PositionSizes,
    pub four_bet_vs_bb_three_bet_from_sb: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SolverConfig {
    #[serde(default)]
    pub iterations: usize,
    #[serde(default)]
    pub strategy: String,
}

impl PreflopConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path.as_ref())
            .with_context(|| format!("reading {}", path.as_ref().display()))?;
        let config: Self =
            serde_json::from_slice(&bytes).context("parsing preflop configuration")?;
        config.validate()?;
        Ok(config)
    }
    /// Keep only BTN and BB; the folded SB remains dead money.
    pub fn heads_up_btn_bb(&self) -> Result<Self> {
        self.validate()?;
        if !self.positions.iter().any(|p| p == "BTN") {
            bail!("BTN seat is required");
        }
        let mut config = self.clone();
        if self.positions.iter().any(|p| p == "SB") {
            config.dead_blind_bb += 0.5;
        }
        config.dead_blind_bb += (self.players - 2) as f64 * self.ante_bb;
        config.players = 2;
        config.positions = vec!["BTN".into(), "BB".into()];
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !(2..=6).contains(&self.players) || self.positions.len() != self.players {
            bail!("players and positions must describe 2..=6 seats");
        }
        let canonical = ["UTG", "HJ", "CO", "BTN", "SB", "BB"];
        let mut previous = None;
        for position in &self.positions {
            let index = canonical
                .iter()
                .position(|p| *p == position)
                .context("unknown preflop position")?;
            if previous.is_some_and(|p| p >= index) {
                bail!("positions must be unique and in preflop action order");
            }
            previous = Some(index);
        }
        if self.positions.last().map(String::as_str) != Some("BB") {
            bail!("BB must be the final seat");
        }
        nonnegative_chips(self.ante_bb, "ante_bb")?;
        positive_chips(self.stack_bb, "stack_bb")?;
        if self.stack_bb < self.ante_bb + 1.0 {
            bail!("stack must cover the ante and big blind");
        }
        nonnegative_chips(self.dead_blind_bb, "dead_blind_bb")?;
        if !(self.rake.rate.is_finite() && (0.0..=1.0).contains(&self.rake.rate)) {
            bail!("rake rate must be in [0,1]");
        }
        nonnegative_chips(self.rake.cap_bb, "rake cap")?;
        if !self.rake.no_flop_no_drop {
            bail!("P1 requires no_flop_no_drop=true");
        }
        if self.node_limit == 0 {
            bail!("node_limit must be positive");
        }
        if self.sizing.open_sizes_bb.is_empty() {
            bail!("at least one open size is required");
        }
        for &size in &self.sizing.open_sizes_bb {
            positive_chips(size, "open size")?;
            if size < 2.0 {
                bail!("open size must be at least 2BB");
            }
        }
        for (position, &size) in &self.sizing.open_sizes_by_position {
            if !canonical.contains(&position.as_str()) {
                bail!("unknown open-size position");
            }
            positive_chips(size, "position open")?;
            if size < 2.0 {
                bail!("position open must be at least 2BB");
            }
        }
        if self.sizing.limp.sb_complete_to_bb != 1.0 {
            bail!("SB complete must be to 1BB");
        }
        let f = &self.sizing.fixed_raise_to_bb;
        for size in [
            f.three_bet.ip,
            f.three_bet.oop,
            f.three_bet_vs_sb_open,
            f.iso_vs_limp,
            f.limp_reraise,
            f.squeeze.ip,
            f.squeeze.oop,
            f.four_bet.ip,
            f.four_bet.oop,
            f.four_bet_vs_bb_three_bet_from_sb,
        ] {
            positive_chips(size, "fixed raise target")?;
        }
        nonnegative_chips(f.squeeze.per_additional_caller, "squeeze increment")?;
        if !self.sizing.five_bet_multiplier.is_finite() || self.sizing.five_bet_multiplier <= 1.0 {
            bail!("five_bet_multiplier must be finite and greater than one");
        }
        if !self.sizing.five_bet_merge_remaining_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.sizing.five_bet_merge_remaining_fraction)
        {
            bail!("five_bet_merge_remaining_fraction must be in [0,1]");
        }
        if !self.sizing.five_bet_all_in {
            bail!("P1 requires five_bet_all_in=true");
        }
        Ok(())
    }
    pub fn is_ip(&self, player: usize, opponent: usize) -> bool {
        // Postflop action order: SB, BB, UTG, HJ, CO, BTN.
        let rank = |p: &str| match p {
            "SB" => 0,
            "BB" => 1,
            "UTG" => 2,
            "HJ" => 3,
            "CO" => 4,
            "BTN" => 5,
            _ => unreachable!(),
        };
        rank(&self.positions[player]) > rank(&self.positions[opponent])
    }
}
pub(crate) fn chips(bb: f64) -> i64 {
    (bb * CHIPS_PER_BB as f64).round() as i64
}
pub(crate) fn bb(chips: i64) -> f64 {
    chips as f64 / CHIPS_PER_BB as f64
}
fn positive_chips(value: f64, name: &str) -> Result<()> {
    nonnegative_chips(value, name)?;
    if value <= 0.0 {
        bail!("{name} must be positive");
    }
    Ok(())
}
fn nonnegative_chips(value: f64, name: &str) -> Result<()> {
    if !value.is_finite()
        || value < 0.0
        || value * CHIPS_PER_BB as f64 >= i64::MAX as f64 / 12.0
        || (value * CHIPS_PER_BB as f64 - (value * CHIPS_PER_BB as f64).round()).abs() > 1e-7
    {
        bail!("{name} must be finite, nonnegative and representable in 0.001BB chips");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> PreflopConfig {
        serde_json::from_str(include_str!("../../../fixtures/preflop-hu.json")).unwrap()
    }
    #[test]
    fn fixture_loads_and_product_metadata_is_accepted() {
        let mut json = serde_json::to_value(config()).unwrap();
        json["sizing"]["three_bet_ip_multiplier"] = 3.0.into();
        json["sizing"]["three_bet_oop_multiplier"] = 4.0.into();
        json["sizing"]["four_bet_multiplier"] = 2.2.into();
        json["rake"]["label"] = "external label".into();
        serde_json::from_value::<PreflopConfig>(json)
            .unwrap()
            .validate()
            .unwrap();
    }
    #[test]
    fn rejects_bad_seats_and_amounts() {
        let mut c = config();
        c.positions.reverse();
        assert!(c.validate().is_err());
        let mut c = config();
        c.stack_bb = f64::NAN;
        assert!(c.validate().is_err());
        let mut c = config();
        c.stack_bb = 40.0001;
        assert!(c.validate().is_err());
        let mut c = config();
        c.ante_bb = 40.0;
        assert!(c.validate().is_err());
        let mut c = config();
        c.node_limit = 0;
        assert!(c.validate().is_err());
    }
    #[test]
    fn heads_up_conversion_preserves_folded_sb_once() {
        let mut c = config();
        c.players = 6;
        c.dead_blind_bb = 0.0;
        c.positions = ["UTG", "HJ", "CO", "BTN", "SB", "BB"]
            .map(String::from)
            .to_vec();
        let hu = c.heads_up_btn_bb().unwrap();
        assert_eq!(hu.positions, ["BTN", "BB"]);
        assert_eq!(hu.dead_blind_bb, 0.5);
        assert_eq!(hu.heads_up_btn_bb().unwrap().dead_blind_bb, 0.5);
        assert!(hu.is_ip(0, 1));
        assert!(c.is_ip(0, 5));
        assert!(!c.is_ip(4, 5));
    }
    #[test]
    fn five_bet_defaults_and_parameter_validation() {
        let mut c = config();
        assert_eq!(c.sizing.five_bet_multiplier, 2.2);
        assert_eq!(c.sizing.five_bet_merge_remaining_fraction, 0.67);
        c.sizing.five_bet_multiplier = 1.0;
        assert!(c.validate().is_err());
        c.sizing.five_bet_multiplier = f64::INFINITY;
        assert!(c.validate().is_err());
        c.sizing.five_bet_multiplier = 2.0;
        for bad in [-0.01, 1.01, f64::NAN] {
            c.sizing.five_bet_merge_remaining_fraction = bad;
            assert!(c.validate().is_err());
        }
    }
    #[test]
    fn rake_no_flop_no_drop_and_cap() {
        let c = config();
        assert_eq!(c.rake.amount_bb(100.0, false), 0.0);
        assert_eq!(c.rake.amount_bb(10.0, true), 0.5);
        assert_eq!(c.rake.amount_bb(100.0, true), 2.0);
    }
}
