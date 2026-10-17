//! Explicit v1 assumptions, not fitted postflop equilibrium values.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandRealization {
    pub pair: f64,
    pub suited_connector: f64,
    pub suited: f64,
    pub offsuit_broadway: f64,
    pub offsuit: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PositionRealization {
    pub ip: HandRealization,
    pub oop: HandRealization,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealizationConfig {
    #[serde(default = "default_multiway")]
    pub multiway: f64,
    pub limp: PositionRealization,
    pub srp: PositionRealization,
    pub three_bet: PositionRealization,
    pub four_bet: PositionRealization,
}

fn default_multiway() -> f64 {
    0.8
}
impl Default for RealizationConfig {
    fn default() -> Self {
        fn row(ip: f64, oop: f64) -> PositionRealization {
            fn hands(base: f64) -> HandRealization {
                HandRealization {
                    pair: base + 0.02,
                    suited_connector: base + 0.05,
                    suited: base + 0.01,
                    offsuit_broadway: base - 0.01,
                    offsuit: base - 0.09,
                }
            }
            PositionRealization {
                ip: hands(ip),
                oop: hands(oop),
            }
        }
        Self {
            multiway: default_multiway(),
            limp: row(0.98, 0.83),
            srp: row(1.0, 0.85),
            three_bet: row(1.0, 0.88),
            four_bet: row(1.0, 0.94),
        }
    }
}
impl RealizationConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.multiway.is_finite() || !(0.0..=1.5).contains(&self.multiway) {
            return Err("invalid multiway realization".into());
        }
        for row in [&self.limp, &self.srp, &self.three_bet, &self.four_bet] {
            for h in [&row.ip, &row.oop] {
                for v in [
                    h.pair,
                    h.suited_connector,
                    h.suited,
                    h.offsuit_broadway,
                    h.offsuit,
                ] {
                    if !v.is_finite() || !(0.0..=1.5).contains(&v) {
                        return Err("realization values must be finite and in [0, 1.5]".into());
                    }
                }
            }
        }
        Ok(())
    }
    pub fn get(&self, stage: u8, ip: bool, class_name: &str) -> f64 {
        let row = match stage {
            0 => &self.limp,
            1 => &self.srp,
            2 => &self.three_bet,
            _ => &self.four_bet,
        };
        let h = if ip { &row.ip } else { &row.oop };
        let b = class_name.as_bytes();
        if b.len() == 2 {
            return h.pair;
        }
        if b[2] == b's' {
            let ranks = b"23456789TJQKA";
            let a = ranks.iter().position(|&r| r == b[0]).unwrap_or(0);
            let z = ranks.iter().position(|&r| r == b[1]).unwrap_or(0);
            if a.abs_diff(z) == 1 {
                h.suited_connector
            } else {
                h.suited
            }
        } else if b"TJQKA".contains(&b[0]) && b"TJQKA".contains(&b[1]) {
            h.offsuit_broadway
        } else {
            h.offsuit
        }
    }
}
