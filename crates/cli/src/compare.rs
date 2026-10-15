//! Compare only locally supplied strategy outputs; never read a solver source.
use super::*;
use cards::Combo;
use serde::Deserialize;
use solver::NodeStrategy;
use std::collections::BTreeMap;
use tree::Action;

#[derive(Deserialize)]
struct Report {
    game_spec: serde_json::Value,
    root_strategy: Option<NodeStrategy>,
    after_check_strategy: Option<NodeStrategy>,
}
#[derive(Clone)]
struct Row {
    class: String,
    count: usize,
    weight: f64,
    probabilities: Vec<f64>,
}
struct Table {
    actions: Vec<Action>,
    rows: BTreeMap<String, Row>,
}
#[derive(Serialize)]
pub struct Comparison {
    pub weighting: &'static str,
    pub nodes: Vec<NodeComparison>,
}
#[derive(Serialize)]
pub struct NodeComparison {
    pub node: &'static str,
    pub combos: usize,
    pub classes: usize,
    pub combo_differences: Vec<ActionDifference>,
    pub class_differences: Vec<ActionDifference>,
    pub own_reach_weighted_frequencies: Vec<f64>,
}
#[derive(Serialize)]
pub struct ActionDifference {
    pub action: Action,
    pub weighted_mean_absolute_difference: f64,
    pub maximum_absolute_difference: f64,
    pub reference_frequency: f64,
    pub solution_frequency_common_weight: f64,
    pub overall_frequency_absolute_difference: f64,
}

// Minimal RFC4180-style reader, without a dependency; quoted embedded delimiters
// and doubled quotes work, malformed quoting fails rather than shifting columns.
fn csv(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut closed = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() && !closed => quoted = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                    closed = false;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    closed = false;
                }
                '\r' if chars.peek() == Some(&'\n') => {}
                _ if closed || c == '"' => bail!("invalid CSV quoting"),
                _ => field.push(c),
            }
        }
    }
    if quoted {
        bail!("unterminated CSV quote");
    }
    if closed || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}
fn action(text: &str) -> Result<Action> {
    Ok(match text {
        "Check" => Action::Check,
        "Call" => Action::Call,
        "Fold" => Action::Fold,
        _ => {
            let (kind, amount) = text
                .split_once('(')
                .ok_or_else(|| anyhow!("invalid action {text}"))?;
            let amount: i64 = amount
                .strip_suffix(')')
                .ok_or_else(|| anyhow!("invalid action {text}"))?
                .parse()?;
            if amount <= 0 {
                bail!("nonpositive action amount");
            }
            match kind {
                "Bet" => Action::Bet(amount),
                "Raise" => Action::Raise(amount),
                _ => bail!("unknown action {text}"),
            }
        }
    })
}
fn load_table(path: &Path, combos: bool) -> Result<Table> {
    let records =
        csv(&fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?)?;
    let header = records.first().ok_or_else(|| anyhow!("empty CSV"))?;
    let prefix = if combos {
        vec!["combo", "class", "reach_weight"]
    } else {
        vec!["class", "combo_count", "reach_weight"]
    };
    if header.len() < 4 || !header.iter().take(3).map(String::as_str).eq(prefix) {
        bail!("unexpected CSV columns: {}", path.display());
    }
    let actions = header[3..]
        .iter()
        .map(|v| action(v))
        .collect::<Result<Vec<_>>>()?;
    if actions
        .iter()
        .enumerate()
        .any(|(i, a)| actions[..i].contains(a))
    {
        bail!("duplicate CSV action");
    }
    let mut rows = BTreeMap::new();
    for record in &records[1..] {
        if record.len() != header.len() {
            bail!("CSV column count mismatch");
        }
        let (key, class, count) = if combos {
            let combo: Combo = record[0].parse().map_err(|e: String| anyhow!(e))?;
            let class = solver::hand_class(combo);
            if class != record[1] {
                bail!("combo/class mismatch");
            }
            (combo.to_string(), class, 1)
        } else {
            (
                record[0].clone(),
                record[0].clone(),
                record[1].parse::<usize>()?,
            )
        };
        let weight: f64 = record[2].parse()?;
        if !weight.is_finite() || weight < 0.0 || count == 0 {
            bail!("invalid CSV reach weight/count");
        }
        let probabilities = record[3..]
            .iter()
            .map(|v| v.parse::<f64>().map_err(Into::into))
            .collect::<Result<Vec<_>>>()?;
        valid_probabilities(&probabilities)?;
        if rows
            .insert(
                key,
                Row {
                    class,
                    count,
                    weight,
                    probabilities,
                },
            )
            .is_some()
        {
            bail!("duplicate CSV row");
        }
    }
    if rows.is_empty() {
        bail!("empty strategy CSV");
    }
    Ok(Table { actions, rows })
}
fn valid_probabilities(p: &[f64]) -> Result<()> {
    if p.iter()
        .any(|x| !x.is_finite() || !(0.0..=1.000001).contains(x))
        || (p.iter().sum::<f64>() - 1.0).abs() > 1e-5
    {
        bail!("invalid strategy probabilities");
    }
    Ok(())
}
fn compatible_actions(a: &[Action], au: i64, b: &[Action], bu: i64) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Action::Bet(x), Action::Bet(y)) | (Action::Raise(x), Action::Raise(y)) => {
                (*x as i128) * bu as i128 == (*y as i128) * au as i128
            }
            _ => x == y,
        })
}
fn aggregate(
    rows: &BTreeMap<String, Row>,
    probabilities: &BTreeMap<String, Vec<f64>>,
) -> BTreeMap<String, Row> {
    let mut classes: BTreeMap<String, Row> = BTreeMap::new();
    for (key, row) in rows {
        let class = classes.entry(row.class.clone()).or_insert_with(|| Row {
            class: row.class.clone(),
            count: 0,
            weight: 0.0,
            probabilities: vec![0.0; row.probabilities.len()],
        });
        class.count += 1;
        class.weight += row.weight;
        for (sum, p) in class.probabilities.iter_mut().zip(&probabilities[key]) {
            *sum += row.weight * p;
        }
    }
    for row in classes.values_mut() {
        if row.weight > 0.0 {
            for p in &mut row.probabilities {
                *p /= row.weight;
            }
        }
    }
    classes
}
fn differences(
    reference: &BTreeMap<String, Row>,
    own: &BTreeMap<String, Vec<f64>>,
    actions: &[Action],
) -> Result<Vec<ActionDifference>> {
    let total = reference.values().map(|r| r.weight).sum::<f64>();
    if total <= 0.0 {
        bail!("zero total reference reach weight");
    }
    Ok(actions
        .iter()
        .enumerate()
        .map(|(a, action)| {
            let mut mean = 0.0f64;
            let mut max = 0.0f64;
            let mut rf = 0.0;
            let mut sf = 0.0;
            for (key, row) in reference {
                let diff = (row.probabilities[a] - own[key][a]).abs();
                mean += row.weight * diff;
                max = max.max(diff);
                rf += row.weight * row.probabilities[a];
                sf += row.weight * own[key][a];
            }
            ActionDifference {
                action: *action,
                weighted_mean_absolute_difference: mean / total,
                maximum_absolute_difference: max,
                reference_frequency: rf / total,
                solution_frequency_common_weight: sf / total,
                overall_frequency_absolute_difference: (sf - rf).abs() / total,
            }
        })
        .collect())
}
fn compare_node(
    name: &'static str,
    strategy: &NodeStrategy,
    combos: Table,
    classes: Table,
    units: i64,
    reference_units: i64,
    player: usize,
) -> Result<NodeComparison> {
    if strategy.player != player
        || !compatible_actions(&strategy.actions, units, &combos.actions, reference_units)
        || combos.actions != classes.actions
    {
        bail!("{name}: incompatible player/actions or chip precision");
    }
    let mut own = BTreeMap::new();
    let mut own_total = 0.0;
    let mut own_freq = vec![0.0; strategy.actions.len()];
    for row in &strategy.combos {
        let combo: Combo = row.combo.parse().map_err(|e: String| anyhow!(e))?;
        if combo.id() != row.combo_id || !row.weight.is_finite() || row.weight <= 0.0 {
            bail!("invalid solution combo id/input weight");
        }
        let p = row
            .probabilities
            .iter()
            .map(|&x| x as f64)
            .collect::<Vec<_>>();
        if p.len() != strategy.actions.len()
            || !row.reach_weight.is_finite()
            || row.reach_weight < 0.0
        {
            bail!("invalid solution row");
        }
        valid_probabilities(&p)?;
        for (sum, &v) in own_freq.iter_mut().zip(&p) {
            *sum += row.reach_weight * v;
        }
        own_total += row.reach_weight;
        if own.insert(combo.to_string(), p).is_some() {
            bail!("duplicate solution combo");
        }
    }
    if !own.keys().eq(combos.rows.keys()) {
        bail!("{name}: combo sets differ; comparison requires identical blocked inputs");
    }
    if player == 0 {
        let reference_total: f64 = combos.rows.values().map(|r| r.weight).sum();
        if own_total <= 0.0 || reference_total <= 0.0 {
            bail!("zero root reach weight");
        }
        let drift: f64 = strategy
            .combos
            .iter()
            .map(|row| {
                let combo: Combo = row.combo.parse().unwrap();
                (row.reach_weight / own_total
                    - combos.rows[&combo.to_string()].weight / reference_total)
                    .abs()
            })
            .sum();
        if drift > 1e-4 {
            bail!("root normalized compatible-joint range weights differ (L1={drift}); input ranges are not equivalent");
        }
    }
    let reference_probabilities = combos
        .rows
        .iter()
        .map(|(k, r)| (k.clone(), r.probabilities.clone()))
        .collect();
    let reference_aggregate = aggregate(&combos.rows, &reference_probabilities);
    if !reference_aggregate.keys().eq(classes.rows.keys())
        || !strategy.classes.keys().eq(classes.rows.keys())
    {
        bail!("{name}: class sets differ");
    }
    for p in strategy.classes.values() {
        if p.len() != strategy.actions.len()
            || p.iter()
                .any(|v| !v.is_finite() || !(0.0..=1.000001).contains(v))
        {
            bail!("invalid solution class probabilities");
        }
    }
    for (key, row) in &reference_aggregate {
        let class = &classes.rows[key];
        if row.count != class.count
            || (row.weight - class.weight).abs() > 1e-5 * class.weight.max(1.0)
            || row.weight > 0.0
                && row
                    .probabilities
                    .iter()
                    .zip(&class.probabilities)
                    .any(|(a, b)| (a - b).abs() > 1e-5)
        {
            bail!("{name}: inconsistent combo/class reference CSV");
        }
    }
    let own_classes = aggregate(&combos.rows, &own)
        .into_iter()
        .map(|(k, r)| (k, r.probabilities))
        .collect();
    for f in &mut own_freq {
        if own_total > 0.0 {
            *f /= own_total;
        }
    }
    Ok(NodeComparison {
        node: name,
        combos: own.len(),
        classes: classes.rows.len(),
        combo_differences: differences(&combos.rows, &own, &strategy.actions)?,
        class_differences: differences(&classes.rows, &own_classes, &strategy.actions)?,
        own_reach_weighted_frequencies: own_freq,
    })
}
fn number(v: &serde_json::Value, key: &str) -> Result<f64> {
    v[key]
        .as_f64()
        .ok_or_else(|| anyhow!("missing numeric setting {key}"))
}
fn check_settings(spec: &serde_json::Value, reference: &serde_json::Value) -> Result<(i64, i64)> {
    let units = spec["units_per_bb"].as_i64().unwrap_or(BB);
    let ru = reference["units_per_bb"]
        .as_i64()
        .ok_or_else(|| anyhow!("missing reference chip precision"))?;
    if units <= 0 || ru <= 0 {
        bail!("invalid chip precision");
    }
    if units != ru {
        bail!("chip precision mismatch: root amounts can agree while downstream rounded raises differ");
    }
    let board: Vec<Card> = serde_json::from_value(spec["board"].clone())?;
    if board
        != parse_board(
            reference["board"]
                .as_str()
                .ok_or_else(|| anyhow!("missing reference board"))?,
        )?
    {
        bail!("board mismatch");
    }
    let t = &spec["tree"];
    for (local, remote) in [
        ("starting_pot", "starting_pot_bb"),
        ("effective_stack", "effective_stack_bb"),
    ] {
        if (number(t, local)? / units as f64 - number(reference, remote)?).abs() > 1e-9 {
            bail!("incompatible {local}");
        }
    }
    if t["bet_size"]["Fixed"].as_f64() != Some(0.5)
        || reference["bet_sizes"]["all_streets"] != "50% pot"
        || t["raise_multiplier"].as_f64() != Some(2.5)
        || reference["bet_sizes"]["raises"] != "2.5x previous bet"
        || reference["bet_sizes"]["raise_cap_count"].as_u64() != Some(0)
        || t["allow_raises"] != true
        || t["allin_merge_enabled"] != false
        || t["oop_can_lead_flop"] != true
    {
        bail!("incompatible betting/all-in/donk settings");
    }
    for key in [
        "add_allin_threshold",
        "force_allin_threshold",
        "merging_threshold",
    ] {
        if number(reference, key)? != 0.0 {
            bail!("unsupported reference threshold {key}");
        }
    }
    if number(t, "rake_rate")? != number(&reference["rake"], "rate")?
        || (number(t, "rake_cap")? / units as f64 - number(&reference["rake"], "cap")? / ru as f64)
            .abs()
            > 1e-9
    {
        bail!("rake mismatch");
    }
    Ok((units, ru))
}
pub fn run(report: &Path, reference_dir: &Path) -> Result<Comparison> {
    let report: Report = serde_json::from_slice(&fs::read(report)?)?;
    let preflight = serde_json::from_slice(&fs::read(reference_dir.join("tree-preflight.json"))?)?;
    let (units, ru) = check_settings(&report.game_spec, &preflight)?;
    let mut nodes = Vec::new();
    for (name, stem, strategy, player) in [
        (
            "root_oop_bb",
            "strategy-root-oop-bb",
            report.root_strategy,
            0,
        ),
        (
            "after_check_ip_btn",
            "strategy-after-check-ip-btn",
            report.after_check_strategy,
            1,
        ),
    ] {
        let strategy = strategy.ok_or_else(|| anyhow!("missing {name} strategy"))?;
        nodes.push(compare_node(
            name,
            &strategy,
            load_table(&reference_dir.join(format!("{stem}-combos.csv")), true)?,
            load_table(&reference_dir.join(format!("{stem}-classes.csv")), false)?,
            units,
            ru,
            player,
        )?);
    }
    Ok(Comparison { weighting: "Reference CSV compatible-joint reach_weight is the common weight for both strategies; class solution probabilities are reaggregated from combos with those same weights. Differences are probability units, not percentage points.", nodes })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_and_errors() {
        assert_eq!(
            csv("a,\"b,c\",\"d\"\"e\"\r\n").unwrap(),
            vec![vec!["a", "b,c", "d\"e"]]
        );
        assert!(csv("\"x").is_err());
        assert!(csv("\"x\"z").is_err());
    }
    #[test]
    fn action_units_and_order() {
        assert!(compatible_actions(
            &[Action::Check, Action::Bet(2750)],
            1000,
            &[Action::Check, Action::Bet(180224)],
            65536
        ));
        assert!(!compatible_actions(
            &[Action::Bet(2750), Action::Check],
            1000,
            &[Action::Check, Action::Bet(180224)],
            65536
        ));
    }
    fn one_row() -> (NodeStrategy, Table, Table) {
        let actions = vec![Action::Check, Action::Bet(2750)];
        let combo: Combo = "AcAd".parse().unwrap();
        let strategy = NodeStrategy {
            node: 0,
            player: 0,
            actions: actions.clone(),
            combos: vec![solver::ComboStrategy {
                dense_id: 0,
                combo_id: combo.id(),
                combo: combo.to_string(),
                weight: 1.0,
                reach_weight: 2.0,
                probabilities: vec![0.25, 0.75],
            }],
            classes: BTreeMap::from([("AA".into(), vec![0.25, 0.75])]),
        };
        let reference_actions = vec![Action::Check, Action::Bet(180224)];
        let row = Row {
            class: "AA".into(),
            count: 1,
            weight: 2.0,
            probabilities: vec![0.5, 0.5],
        };
        (
            strategy,
            Table {
                actions: reference_actions.clone(),
                rows: BTreeMap::from([(combo.to_string(), row.clone())]),
            },
            Table {
                actions: reference_actions,
                rows: BTreeMap::from([("AA".into(), row)]),
            },
        )
    }
    #[test]
    fn node_comparison_and_rejections() {
        let (strategy, combos, classes) = one_row();
        let result = compare_node("synthetic", &strategy, combos, classes, 1000, 65536, 0).unwrap();
        assert_eq!(
            result.combo_differences[0].weighted_mean_absolute_difference,
            0.25
        );
        assert_eq!(
            result.class_differences[0].maximum_absolute_difference,
            0.25
        );
        assert_eq!(result.own_reach_weighted_frequencies, vec![0.25, 0.75]);
        let (mut strategy, combos, classes) = one_row();
        strategy.combos.clear();
        assert!(compare_node("synthetic", &strategy, combos, classes, 1000, 65536, 0).is_err());
        let (strategy, combos, mut classes) = one_row();
        classes.rows.get_mut("AA").unwrap().probabilities = vec![0.0, 1.0];
        assert!(compare_node("synthetic", &strategy, combos, classes, 1000, 65536, 0).is_err());
        let (mut strategy, combos, classes) = one_row();
        strategy.actions[1] = Action::Bet(2751);
        assert!(compare_node("synthetic", &strategy, combos, classes, 1000, 65536, 0).is_err());
    }
    #[test]
    fn settings_reject_different_board_and_merge() {
        let mut spec = serde_json::json!({"board":parse_board("QsJh2h").unwrap(),"units_per_bb":65536,"tree":{"starting_pot":360448,"effective_stack":6389760,"bet_size":{"Fixed":0.5},"raise_multiplier":2.5,"allow_raises":true,"allin_merge_enabled":false,"oop_can_lead_flop":true,"rake_rate":0.0,"rake_cap":0}});
        let reference = serde_json::json!({"board":"QsJh2h","units_per_bb":65536,"starting_pot_bb":5.5,"effective_stack_bb":97.5,"bet_sizes":{"all_streets":"50% pot","raises":"2.5x previous bet","raise_cap_count":0},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"rake":{"rate":0.0,"cap":0}});
        assert_eq!(check_settings(&spec, &reference).unwrap(), (65536, 65536));
        let mut coarse = spec.clone();
        coarse["units_per_bb"] = serde_json::json!(1000);
        coarse["tree"]["starting_pot"] = serde_json::json!(5500);
        coarse["tree"]["effective_stack"] = serde_json::json!(97500);
        assert!(check_settings(&coarse, &reference).is_err());
        spec["tree"]["allin_merge_enabled"] = serde_json::json!(true);
        assert!(check_settings(&spec, &reference).is_err());
        spec["tree"]["allin_merge_enabled"] = serde_json::json!(false);
        spec["board"] = serde_json::json!(parse_board("AsJh2h").unwrap());
        assert!(check_settings(&spec, &reference).is_err());
    }
    #[test]
    fn weighted_differences() {
        let reference = BTreeMap::from([
            (
                "a".into(),
                Row {
                    class: "AA".into(),
                    count: 1,
                    weight: 1.0,
                    probabilities: vec![0.0, 1.0],
                },
            ),
            (
                "b".into(),
                Row {
                    class: "KK".into(),
                    count: 1,
                    weight: 3.0,
                    probabilities: vec![1.0, 0.0],
                },
            ),
        ]);
        let own = BTreeMap::from([("a".into(), vec![1.0, 0.0]), ("b".into(), vec![0.5, 0.5])]);
        let d = differences(&reference, &own, &[Action::Check, Action::Bet(1)]).unwrap();
        assert_eq!(d[0].weighted_mean_absolute_difference, 0.625);
        assert_eq!(d[0].maximum_absolute_difference, 1.0);
        assert_eq!(d[0].overall_frequency_absolute_difference, 0.125);
    }
}
