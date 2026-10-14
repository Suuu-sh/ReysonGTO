//! External weighted ranges; hand-class weights are total mass, not frequency.
use cards::{validate_board, Card, Combo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeightedCombo {
    pub combo: Combo,
    pub weight: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Range {
    pub player: Option<String>,
    pub combos: Vec<WeightedCombo>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DenseRange {
    pub player: Option<String>,
    pub combos: Vec<WeightedCombo>,
}
impl Range {
    pub fn from_combos(combos: Vec<(Combo, f64)>) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        for (c, w) in combos {
            let c = Combo::new(c.cards[0], c.cards[1])?;
            if !w.is_finite() || w < 0.0 {
                return Err("weight must be finite and nonnegative".into());
            }
            if map
                .insert(
                    c.id(),
                    WeightedCombo {
                        combo: c,
                        weight: w,
                    },
                )
                .is_some()
            {
                return Err("duplicate combo".into());
            }
        }
        let combos = map.into_values().filter(|c| c.weight > 0.0).collect();
        Ok(Self {
            player: None,
            combos,
        })
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let obj = v.as_object().ok_or("range must be an object")?;
        let (player, entries) = if obj.contains_key("combos") {
            if obj.keys().any(|k| k != "combos" && k != "player") {
                return Err("unknown range metadata".into());
            }
            let player = match obj.get("player") {
                Some(p) => Some(p.as_str().ok_or("player must be a string")?.to_owned()),
                None => None,
            };
            (
                player,
                obj["combos"]
                    .as_object()
                    .ok_or("combos must be an object")?,
            )
        } else {
            (None, obj)
        };
        let mut combos = Vec::new();
        for (key, value) in entries {
            let w = value.as_f64().ok_or("weight must be numeric")?;
            let expanded = expand_key(key)?;
            let part = w / expanded.len() as f64;
            for c in expanded {
                combos.push((c, part))
            }
        }
        let mut range = Self::from_combos(combos)?;
        range.player = player;
        Ok(range)
    }
    pub fn blocked(&self, board: &[Card]) -> Result<DenseRange, String> {
        let mask = validate_board(board)?;
        let checked = Self::from_combos(self.combos.iter().map(|c| (c.combo, c.weight)).collect())?;
        Ok(DenseRange {
            player: self.player.clone(),
            combos: checked
                .combos
                .into_iter()
                .filter(|c| c.combo.mask() & mask == 0)
                .collect(),
        })
    }
}
impl DenseRange {
    pub fn len(&self) -> usize {
        self.combos.len()
    }
    pub fn is_empty(&self) -> bool {
        self.combos.is_empty()
    }
    pub fn weights(&self) -> Vec<f64> {
        self.combos.iter().map(|c| c.weight).collect()
    }
}
pub fn expand_key(key: &str) -> Result<Vec<Combo>, String> {
    if key.len() == 4 {
        return Ok(vec![key.parse()?]);
    }
    let b = key.as_bytes();
    if !key.is_ascii() || !(b.len() == 2 || b.len() == 3) {
        return Err("invalid hand class".into());
    }
    let r1 = b"23456789TJQKA"
        .iter()
        .position(|&r| r == b[0])
        .ok_or("invalid class rank")? as u8;
    let r2 = b"23456789TJQKA"
        .iter()
        .position(|&r| r == b[1])
        .ok_or("invalid class rank")? as u8;
    if r1 == r2 && b.len() != 2 {
        return Err("pair cannot have a suitedness suffix".into());
    }
    if r1 != r2 && (b.len() != 3 || !(b[2] == b's' || b[2] == b'o')) {
        return Err("nonpair class requires s or o".into());
    }
    let mut out = Vec::new();
    for s in 0..4 {
        for t in 0..4 {
            if r1 == r2 {
                if s >= t {
                    continue;
                }
            } else if (b[2] == b's') != (s == t) {
                continue;
            }
            out.push(Combo::new(Card(r1 * 4 + s), Card(r2 * 4 + t))?)
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classes_and_blockers() {
        let r =
            Range::from_json(r#"{"player":"synthetic","combos":{"AKs":0.8,"22":0.6}}"#).unwrap();
        assert_eq!(r.combos.len(), 10);
        assert!((r.combos.iter().map(|x| x.weight).sum::<f64>() - 1.4).abs() < 1e-12);
        let d = r.blocked(&["Ah".parse().unwrap()]).unwrap();
        assert_eq!(d.len(), 9);
        assert!(d
            .combos
            .windows(2)
            .all(|c| c[0].combo.id() < c[1].combo.id()));
    }
    #[test]
    fn normalizes_raw_combo_order() {
        let canonical: Combo = "AhKd".parse().unwrap();
        let reversed = Combo {
            cards: [canonical.cards[1], canonical.cards[0]],
        };
        let range = Range::from_combos(vec![(reversed, 0.5)]).unwrap();
        assert_eq!(range.combos[0].combo, canonical);
        assert_eq!(range.combos[0].combo.id(), canonical.id());
        assert!(Range::from_combos(vec![(canonical, 0.5), (reversed, 0.5)]).is_err());
    }
    #[test]
    fn invalid() {
        for s in [
            r#"{"AhAh":1}"#,
            r#"{"AKs":-1}"#,
            r#"{"AK":1}"#,
            r#"{"AhKd":1,"KdAh":2}"#,
            r#"{"AKs":1,"AhKh":1}"#,
        ] {
            assert!(Range::from_json(s).is_err(), "{s}")
        }
    }
}
