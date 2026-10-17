use anyhow::{anyhow, bail, Context, Result};
use cards::Card;
use cfr::{Algorithm, EngineConfig, ScratchPrecision};
use clap::{Args, Parser, Subcommand, ValueEnum};
use ranges::Range;
use serde::Serialize;
use solver::{solve, solve_with_progress, AdaptiveMetrics, GameSpec, StopConditions};
mod compare;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tree::{SizeRule, TreeConfig, BB};

#[derive(Parser)]
#[command(
    name = "reyson-gto",
    version,
    about = "Clean-room heads-up postflop solver"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Method {
    Dcfr,
    CfrPlus,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Storage {
    F32,
    I16,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Scratch {
    F32,
    F64,
}
// Parsed once at startup, not stored per node or iteration; boxing Solve would
// add indirection/allocation without reducing the solver's working set.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    Solve(SolveArgs),
    /// Solve a turn/river from external ranges or an extracted reach snapshot.
    SolveSubgame(SolveArgs),
    /// Compare local output reports against copied benchmark CSVs only.
    Compare {
        #[arg(long, alias = "report")]
        solution: PathBuf,
        #[arg(long, alias = "reference", default_value = "data/benchmark")]
        reference_dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Three small synthetic cases, not production-range throughput claims.
    Bench {
        #[arg(long, default_value_t = 10)]
        threads: usize,
        #[arg(long, default_value_t = 20)]
        max_iters: usize,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Preset {
    Production,
    Benchmark,
}
#[derive(Args)]
#[group(id = "solve_args", multiple = true)]
struct SolveArgs {
    /// Three cards for solve; four or five for solve-subgame.
    #[arg(long, required_unless_present = "state", conflicts_with = "state")]
    board: Option<String>,
    /// Extracted reach snapshot (solve-subgame only).
    #[arg(long)]
    state: Option<PathBuf>,
    /// Deal this public card into an extracted pre-turn/pre-river snapshot.
    #[arg(long, requires = "state")]
    card: Option<Card>,
    /// Default production; benchmark exactly reproduces the M2c game/settings.
    #[arg(long, value_enum)]
    preset: Option<Preset>,
    #[arg(long, conflicts_with = "state")]
    pot: Option<f64>,
    #[arg(long, conflicts_with = "state")]
    stack: Option<f64>,
    #[arg(long, required_unless_present = "state", conflicts_with = "state")]
    oop_range: Option<PathBuf>,
    #[arg(long, required_unless_present = "state", conflicts_with = "state")]
    ip_range: Option<PathBuf>,
    /// Percentage of the pot, e.g. 50 for a half-pot bet.
    #[arg(long)]
    bet: Option<f64>,
    /// JSON RuleTable; mutually exclusive with explicit bet percentages.
    #[arg(long, conflicts_with_all = ["bet", "bet_flop", "bet_turn", "bet_river"])]
    size_rules: Option<PathBuf>,
    #[arg(long)]
    bet_flop: Option<f64>,
    #[arg(long)]
    bet_turn: Option<f64>,
    #[arg(long)]
    bet_river: Option<f64>,
    #[arg(long)]
    oop_leads: bool,
    /// Explicitly disable flop leading, including for benchmark preset.
    #[arg(long, conflicts_with = "oop_leads")]
    no_oop_leads: bool,
    #[arg(long, conflicts_with = "state")]
    units_per_bb: Option<i64>,
    #[arg(long)]
    raise_multiplier: Option<f64>,
    #[arg(long)]
    allin_merge_ratio: Option<f64>,
    #[arg(long)]
    no_allin_merge: bool,
    /// Explicitly enable merging, including for benchmark preset.
    #[arg(long, conflicts_with = "no_allin_merge")]
    allin_merge: bool,
    #[arg(long, default_value_t = 10)]
    metrics_every: usize,
    /// Enable sparse early exact metrics (already enabled for production).
    #[arg(long, conflicts_with = "no_adaptive_metrics")]
    adaptive_metrics: bool,
    /// Use fixed --metrics-every cadence, including in production.
    #[arg(long)]
    no_adaptive_metrics: bool,
    #[arg(long, default_value_t = 25)]
    metrics_early_every: usize,
    /// Near-target cadence; defaults to --metrics-every.
    #[arg(long)]
    metrics_near_every: Option<usize>,
    /// Switch permanently to near cadence at target times this multiplier.
    #[arg(long, default_value_t = 2.0)]
    metrics_near_target_multiplier: f64,
    /// Flush setup and fresh metric checkpoint JSONL while solving.
    #[arg(long)]
    progress_jsonl: Option<PathBuf>,
    /// Export average-policy own-reach ranges and exact node chip state.
    #[arg(
        long,
        requires = "reach_out",
        conflicts_with = "reach_line",
        group = "reach_selector"
    )]
    reach_node: Option<usize>,
    /// Root action path, e.g. X,X or X,B,C.
    #[arg(long, requires = "reach_out", group = "reach_selector")]
    reach_line: Option<String>,
    /// JSON array of {line, card?, out} exports from this one solve.
    #[arg(long)]
    reach_requests: Option<PathBuf>,
    #[arg(long, requires = "reach_selector")]
    reach_out: Option<PathBuf>,
    /// Optionally deal one card before exporting the snapshot.
    #[arg(long, requires = "reach_selector")]
    reach_card: Option<Card>,
    #[arg(long, value_enum)]
    storage: Option<Storage>,
    /// Precision of iteration scratch buffers; independent of regret storage.
    #[arg(long, value_enum, default_value_t = Scratch::F64)]
    scratch_precision: Scratch,
    /// Diagnostic only: retain f64 total accumulation with f32 scratch.
    #[arg(long, hide = true)]
    wide_totals: bool,
    #[arg(long)]
    dcfr_alpha: Option<f64>,
    #[arg(long)]
    dcfr_beta: Option<f64>,
    #[arg(long)]
    dcfr_gamma: Option<f64>,
    #[arg(long)]
    rake: Option<f64>,
    #[arg(long)]
    rake_cap: Option<f64>,
    /// Exploitability percentage of this game's starting pot.
    #[arg(long, default_value_t = 0.3)]
    target: f64,
    #[arg(long, default_value_t = 1000)]
    max_iters: usize,
    #[arg(long, default_value_t = 60.0)]
    max_seconds: f64,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 10)]
    threads: usize,
    /// Enable fine-grained per-node worker timers (off by default).
    #[arg(long, conflicts_with = "no_profiling")]
    profiling: bool,
    /// Compatibility flag: fine-grained worker timers are already off by default.
    #[arg(long)]
    no_profiling: bool,
    /// Opt-in phase-isolated kernel workload and chance-task diagnostics.
    #[arg(long)]
    kernel_diagnostics: bool,
    /// Minimum number of chance children per parallel task (must be positive).
    #[arg(long, default_value_t = 1)]
    parallel_grain: usize,
    #[arg(long, value_enum, default_value_t = Method::Dcfr)]
    algorithm: Method,
    #[arg(long)]
    no_isomorphism: bool,
    /// Conservative memory budget in MiB; also bounds arena construction.
    #[arg(long, default_value_t = 8192)]
    memory_mb: usize,
}
fn parse_board(text: &str) -> Result<Vec<Card>> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    if !compact.is_ascii() || ![6, 8, 10].contains(&compact.len()) {
        bail!("board must contain 3, 4, or 5 cards, e.g. AhKd2c");
    }
    let board = (0..compact.len())
        .step_by(2)
        .map(|i| compact[i..i + 2].parse().map_err(|e: String| anyhow!(e)))
        .collect::<Result<Vec<Card>>>()?;
    cards::validate_board(&board).map_err(|e| anyhow!(e))?;
    Ok(board)
}
#[cfg(test)]
fn chips(bb: f64) -> Result<i64> {
    chips_at(bb, BB)
}
fn chips_at(bb: f64, units: i64) -> Result<i64> {
    if units <= 0 || !bb.is_finite() || bb < 0.0 || bb * units as f64 >= i64::MAX as f64 {
        bail!("BB amount must be finite, nonnegative and representable");
    }
    Ok((bb * units as f64).round() as i64)
}
fn read_range(path: PathBuf) -> Result<Range> {
    let text = fs::read_to_string(&path)
        .with_context(|| format!("cannot read range {}", path.display()))?;
    Range::from_json(&text).map_err(|e| anyhow!("{}: {e}", path.display()))
}
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("output must name a file"))?;
    let mut temporary_name = name.to_os_string();
    temporary_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let temporary = path.with_file_name(temporary_name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("cannot create output near {}", path.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        // Only remove the uniquely named temporary file we created above.
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("cannot save {}", path.display()))
}
impl SolveArgs {
    fn tree_config(&self, units: i64, inherited: Option<&TreeConfig>) -> Result<TreeConfig> {
        let preset = self.preset.unwrap_or(Preset::Production);
        let mut config = if let Some(previous) = inherited.filter(|_| self.preset.is_none()) {
            previous.clone()
        } else {
            TreeConfig {
                starting_pot: chips_at(5.5, units)?,
                effective_stack: chips_at(97.5, units)?,
                oop_can_lead_flop: preset == Preset::Benchmark,
                bet_size: match preset {
                    Preset::Production => SizeRule::PerStreet {
                        flop: 0.33,
                        turn: 0.66,
                        river: 0.66,
                    },
                    Preset::Benchmark => SizeRule::Fixed(0.5),
                },
                rake_rate: if preset == Preset::Production {
                    0.05
                } else {
                    0.0
                },
                rake_cap: chips_at(
                    if preset == Preset::Production {
                        3.0
                    } else {
                        0.0
                    },
                    units,
                )?,
                raise_multiplier: 2.5,
                allin_merge_enabled: preset == Preset::Production,
                allin_merge_ratio: 0.67,
                allow_raises: true,
            }
        };
        // A preset changes rules, never the exact extracted pot/stack state.
        if let Some(previous) = inherited {
            config.starting_pot = previous.starting_pot;
            config.effective_stack = previous.effective_stack;
        }
        if let Some(pot) = self.pot {
            config.starting_pot = chips_at(pot, units)?;
        }
        if let Some(stack) = self.stack {
            config.effective_stack = chips_at(stack, units)?;
        }
        if self.oop_leads {
            config.oop_can_lead_flop = true;
        }
        if self.no_oop_leads {
            config.oop_can_lead_flop = false;
        }
        if self.no_allin_merge {
            config.allin_merge_enabled = false;
        }
        if self.allin_merge {
            config.allin_merge_enabled = true;
        }
        if let Some(ratio) = self.allin_merge_ratio {
            config.allin_merge_ratio = ratio;
        }
        if let Some(multiplier) = self.raise_multiplier {
            config.raise_multiplier = multiplier;
        }
        if let Some(rate) = self.rake {
            config.rake_rate = rate;
        }
        if let Some(cap) = self.rake_cap {
            config.rake_cap = chips_at(cap, units)?;
        }
        if let Some(bet) = self.bet {
            config.bet_size = SizeRule::Fixed(bet_fraction(bet)?);
        }
        if self.bet_flop.is_some() || self.bet_turn.is_some() || self.bet_river.is_some() {
            let fraction = |street| {
                config.bet_size.fraction(&tree::SizeContext {
                    board: &[],
                    street,
                    player: 0,
                    first_bet: true,
                    after_check: false,
                })
            };
            config.bet_size = SizeRule::PerStreet {
                flop: self
                    .bet_flop
                    .map(bet_fraction)
                    .transpose()?
                    .unwrap_or_else(|| fraction(tree::Street::Flop)),
                turn: self
                    .bet_turn
                    .map(bet_fraction)
                    .transpose()?
                    .unwrap_or_else(|| fraction(tree::Street::Turn)),
                river: self
                    .bet_river
                    .map(bet_fraction)
                    .transpose()?
                    .unwrap_or_else(|| fraction(tree::Street::River)),
            };
        }
        if let Some(table) = self.load_size_rules()? {
            config.bet_size = table.into_size_rule();
        }
        Ok(config)
    }
    fn load_size_rules(&self) -> Result<Option<sizing::rules::RuleTable>> {
        self.size_rules
            .as_ref()
            .map(|path| {
                let table: sizing::rules::RuleTable = serde_json::from_slice(
                    &fs::read(path)
                        .with_context(|| format!("cannot read size rules {}", path.display()))?,
                )
                .with_context(|| format!("invalid size rules {}", path.display()))?;
                table.validate().map_err(|e| anyhow!(e))?;
                Ok(table)
            })
            .transpose()
    }
    fn engine_config(&self) -> Result<EngineConfig> {
        let benchmark = self.preset == Some(Preset::Benchmark);
        if self.wide_totals && !matches!(self.scratch_precision, Scratch::F32) {
            bail!("--wide-totals requires --scratch-precision f32");
        }
        Ok(EngineConfig {
            threads: self.threads,
            parallel_grain: self.parallel_grain,
            algorithm: match self.algorithm {
                Method::Dcfr => Algorithm::Dcfr,
                Method::CfrPlus => Algorithm::CfrPlus,
            },
            suit_isomorphism: !self.no_isomorphism,
            memory_limit_bytes: self
                .memory_mb
                .checked_mul(1024 * 1024)
                .ok_or_else(|| anyhow!("memory limit overflow"))?,
            profiling: self.profiling,
            kernel_diagnostics: self.kernel_diagnostics,
            scratch_precision: match self.scratch_precision {
                Scratch::F32 if self.wide_totals => ScratchPrecision::F32WideTotals,
                Scratch::F32 => ScratchPrecision::F32,
                Scratch::F64 => ScratchPrecision::F64,
            },
            storage_mode: match self.storage.unwrap_or(if benchmark {
                Storage::I16
            } else {
                Storage::F32
            }) {
                Storage::F32 => cfr::StorageMode::F32,
                Storage::I16 => cfr::StorageMode::I16,
            },
            dcfr_alpha: self.dcfr_alpha.unwrap_or(1.5),
            dcfr_beta: self.dcfr_beta.unwrap_or(0.0),
            dcfr_gamma: self.dcfr_gamma.unwrap_or(if benchmark { 4.0 } else { 2.0 }),
        })
    }
    fn game_spec(&self, subgame: bool) -> Result<GameSpec> {
        let board = parse_board(
            self.board
                .as_deref()
                .ok_or_else(|| anyhow!("--board is required"))?,
        )?;
        if subgame && ![4, 5].contains(&board.len()) {
            bail!("solve-subgame requires a turn or river board (4 or 5 cards)");
        }
        if subgame && (self.pot.is_none() || self.stack.is_none()) {
            bail!("direct solve-subgame requires explicit --pot and --stack");
        }
        let units_per_bb = self
            .units_per_bb
            .unwrap_or(if self.preset == Some(Preset::Benchmark) {
                65_536
            } else {
                BB
            });
        Ok(GameSpec {
            board,
            units_per_bb,
            tree: self.tree_config(units_per_bb, None)?,
            engine: self.engine_config()?,
        })
    }
    fn stop_conditions(&self) -> StopConditions {
        StopConditions {
            target_exploitability_pct: self.target,
            max_iterations: self.max_iters,
            max_seconds: self.max_seconds,
            metrics_every: self.metrics_every,
            adaptive_metrics: (!self.no_adaptive_metrics
                && (self.adaptive_metrics || self.preset != Some(Preset::Benchmark)))
            .then_some(AdaptiveMetrics {
                early_every: self.metrics_early_every,
                near_every: self.metrics_near_every.unwrap_or(self.metrics_every),
                near_target_multiplier: self.metrics_near_target_multiplier,
            }),
        }
    }
}
fn bet_fraction(percentage: f64) -> Result<f64> {
    if !percentage.is_finite() || percentage <= 0.0 {
        bail!("bet percentage must be finite and positive");
    }
    Ok(percentage / 100.0)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReachRequest {
    line: String,
    card: Option<String>,
    out: PathBuf,
}

fn resolve_reach_line(tree: &tree::GameTree, line: &str) -> Result<usize> {
    let tokens: Vec<_> = line
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.is_empty() {
        bail!("reach line must contain actions");
    }
    let mut node = tree.root;
    for token in tokens {
        if !matches!(token, "X" | "B" | "C" | "F" | "R") {
            bail!("unknown reach action {token}; use X, B, C, F, R");
        }
        let tree::NodeKind::Decision {
            actions, children, ..
        } = &tree.nodes[node].kind
        else {
            bail!("reach line crosses a chance or terminal node at {node}");
        };
        let matches: Vec<_> = actions
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                matches!(
                    (token, a),
                    ("X", tree::Action::Check)
                        | ("B", tree::Action::Bet(_))
                        | ("C", tree::Action::Call)
                        | ("F", tree::Action::Fold)
                        | ("R", tree::Action::Raise(_))
                )
            })
            .collect();
        if matches.len() != 1 {
            bail!(
                "reach action {token} at node {node} has {} matches; exactly one required",
                matches.len()
            );
        }
        node = children[matches[0].0];
    }
    Ok(node)
}

fn snapshot_bytes(
    mut snapshot: solver::ReachSnapshot,
    rules: Option<&sizing::rules::RuleTable>,
) -> Result<Vec<u8>> {
    if let Some(table) = rules {
        // Closures are intentionally not serializable; full table is the authoritative rule.
        snapshot.tree_config.bet_size = SizeRule::Fixed(table.default_fraction);
        let mut value = serde_json::to_value(snapshot)?;
        value["size_rules"] = serde_json::to_value(table)?;
        Ok(serde_json::to_vec_pretty(&value)?)
    } else {
        Ok(serde_json::to_vec_pretty(&snapshot)?)
    }
}

fn run_solve(args: SolveArgs, subgame: bool) -> Result<()> {
    if !subgame && args.state.is_some() {
        bail!("--state is only supported by solve-subgame");
    }
    let size_rules = args.load_size_rules()?.or(if let Some(path) = &args.state {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        value
            .get("size_rules")
            .map(|v| serde_json::from_value::<sizing::rules::RuleTable>(v.clone()))
            .transpose()?
    } else {
        None
    });
    if args.size_rules.is_none()
        && size_rules.is_some()
        && (args.bet.is_some()
            || args.bet_flop.is_some()
            || args.bet_turn.is_some()
            || args.bet_river.is_some())
    {
        bail!("snapshot contains size rules; explicit bet overrides require a replacement --size-rules table");
    }
    // An explicit preset chooses its own sizing; silently restoring the snapshot's
    // rules on top of it would run a hybrid of the two.
    if args.size_rules.is_none() && size_rules.is_some() && args.preset.is_some() {
        bail!("snapshot contains size rules; with an explicit --preset pass --size-rules to choose the sizing");
    }
    if let Some(table) = &size_rules {
        table.validate().map_err(|e| anyhow!(e))?;
    }
    let requests: Vec<ReachRequest> = args
        .reach_requests
        .as_ref()
        .map(|p| -> Result<_> { Ok(serde_json::from_slice(&fs::read(p)?)?) })
        .transpose()?
        .unwrap_or_default();
    // Validate all input/config before creating progress output.
    let mut direct = if args.state.is_none() {
        Some((
            args.game_spec(subgame)?,
            [
                read_range(
                    args.oop_range
                        .clone()
                        .ok_or_else(|| anyhow!("--oop-range is required"))?,
                )?,
                read_range(
                    args.ip_range
                        .clone()
                        .ok_or_else(|| anyhow!("--ip-range is required"))?,
                )?,
            ],
        ))
    } else {
        None
    };
    let mut snapshot = args
        .state
        .as_ref()
        .map(|path| -> Result<solver::ReachSnapshot> {
            let bytes = fs::read(path)
                .with_context(|| format!("cannot read snapshot {}", path.display()))?;
            let mut snapshot: solver::ReachSnapshot = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid snapshot {}", path.display()))?;
            if let Some(card) = args.card {
                snapshot = snapshot.with_card(card).map_err(|e| anyhow!(e))?;
            }
            snapshot.tree_config =
                args.tree_config(snapshot.units_per_bb, Some(&snapshot.tree_config))?;
            Ok(snapshot)
        })
        .transpose()?;
    // Use the same loaded table for the executed tree and provenance.
    if let Some(table) = &size_rules {
        if let Some((spec, _)) = &mut direct {
            spec.tree.bet_size = table.clone().into_size_rule();
        }
        if let Some(snapshot) = &mut snapshot {
            snapshot.tree_config.bet_size = table.clone().into_size_rule();
        }
    }
    let mut progress_file = args
        .progress_jsonl
        .as_ref()
        .map(|path| {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| format!("cannot create progress {}", path.display()))
        })
        .transpose()?;
    let progress = |event: &solver::ProgressEvent| {
        let line = serde_json::to_string(event).map_err(|e| e.to_string())?;
        eprintln!("{line}");
        if let Some(file) = &mut progress_file {
            writeln!(file, "{line}")
                .and_then(|_| file.flush())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    let solution = if let Some(snapshot) = snapshot {
        solver::solve_subgame_with_progress(
            snapshot,
            args.engine_config()?,
            args.stop_conditions(),
            progress,
        )
    } else {
        let (spec, ranges) = direct.expect("validated direct input");
        solve_with_progress(spec, ranges, args.stop_conditions(), progress)
    }
    .map_err(|e| anyhow!(e))?;
    let selected_node = args
        .reach_line
        .as_deref()
        .map(|line| resolve_reach_line(solution.tree(), line))
        .transpose()?
        .or(args.reach_node);
    if let Some(node) = selected_node {
        let mut snapshot = solution.reach_snapshot(node).map_err(|e| anyhow!(e))?;
        if let Some(card) = args.reach_card {
            snapshot = snapshot.with_card(card).map_err(|e| anyhow!(e))?;
        }
        write_atomic(
            args.reach_out
                .as_deref()
                .ok_or_else(|| anyhow!("--reach-out is required"))?,
            &snapshot_bytes(snapshot, size_rules.as_ref())?,
        )?;
    }
    for request in requests {
        let node = resolve_reach_line(solution.tree(), &request.line)?;
        let mut snapshot = solution.reach_snapshot(node).map_err(|e| anyhow!(e))?;
        if let Some(card) = request.card {
            snapshot = snapshot
                .with_card(card.parse().map_err(|e: String| anyhow!(e))?)
                .map_err(|e| anyhow!(e))?;
        }
        write_atomic(
            &request.out,
            &snapshot_bytes(snapshot, size_rules.as_ref())?,
        )?;
    }
    let mut serialized_spec = solution.game_spec.clone();
    if let Some(table) = &size_rules {
        serialized_spec.tree.bet_size = SizeRule::Fixed(table.default_fraction);
    }
    let mut report = solution.report();
    report.game_spec = &serialized_spec;
    if let Some(table) = size_rules {
        let mut value = serde_json::to_value(&report)?;
        value["game_spec"]["size_rules"] = serde_json::to_value(table)?;
        write_atomic(&args.out, &serde_json::to_vec_pretty(&value)?)?;
    } else {
        write_atomic(&args.out, &serde_json::to_vec_pretty(&report)?)?;
    }
    eprintln!(
        "iterations={} exploitability={:.6}% memory={} bytes stop={} output={}",
        report.iterations,
        report.metrics.exploitability_pct,
        report.memory_bytes,
        report.stopped_by,
        args.out.display()
    );
    Ok(())
}
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Solve(args) => run_solve(args, false)?,
        Command::SolveSubgame(args) => run_solve(args, true)?,
        Command::Bench { threads, max_iters } => bench(threads, max_iters)?,
        Command::Compare {
            solution,
            reference_dir,
            out,
        } => {
            let report = compare::run(&solution, &reference_dir)?;
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(path) = out {
                write_atomic(&path, &bytes)?;
            } else {
                println!("{}", String::from_utf8(bytes)?);
            }
        }
    }
    Ok(())
}
#[derive(Serialize)]
struct Benchmark {
    case: &'static str,
    board: String,
    threads: usize,
    iterations: usize,
    arena_nodes: usize,
    memory_bytes: usize,
    setup_seconds: f64,
    solve_seconds: f64,
    seconds_per_iteration: f64,
    terminal_work_seconds: f64,
    terminal_work_over_solve_wall_pct: f64,
    serial_terminal_fraction_pct: f64,
    serial_profile_iterations: usize,
    isomorphism_used: bool,
    merged_chance_children: usize,
    exploitability_pct: f64,
}
fn bench(threads: usize, max_iters: usize) -> Result<()> {
    for (case, board) in [
        ("river", "AhKh2h3h4h"),
        ("turn", "AhKh2h3h"),
        ("flop", "AhKh2h"),
    ] {
        // Deliberately meaningless tiny private-card mixtures; no product data.
        let oop = Range::from_json(r#"{"QhJh":1,"9h8h":1,"7h6h":1}"#).map_err(|e| anyhow!(e))?;
        let ip = Range::from_json(r#"{"Th5h":1,"9h7h":1,"8h6h":1}"#).map_err(|e| anyhow!(e))?;
        let spec = GameSpec {
            board: parse_board(board)?,
            units_per_bb: BB,
            tree: TreeConfig {
                starting_pot: 2 * BB,
                effective_stack: BB,
                oop_can_lead_flop: true,
                bet_size: SizeRule::Fixed(0.5),
                ..TreeConfig::default()
            },
            engine: EngineConfig {
                threads,
                profiling: true,
                suit_isomorphism: true,
                ..EngineConfig::default()
            },
        };
        let stop = StopConditions {
            target_exploitability_pct: 0.0,
            max_iterations: max_iters,
            max_seconds: 300.0,
            metrics_every: 1,
            ..Default::default()
        };
        let solution =
            solve(spec.clone(), [oop.clone(), ip.clone()], stop.clone()).map_err(|e| anyhow!(e))?;
        let report = solution.report();
        // A separate serial pass supplies a meaningful terminal fraction. Adding
        // parallel worker kernel time and dividing by wall time can exceed 100%.
        let (serial_terminal_fraction_pct, serial_profile_iterations) = if threads == 1 {
            (
                report.terminal_seconds / report.solve_seconds.max(f64::MIN_POSITIVE) * 100.0,
                report.iterations,
            )
        } else {
            let mut serial_spec = spec;
            serial_spec.engine.threads = 1;
            let serial = solve(serial_spec, [oop, ip], stop).map_err(|e| anyhow!(e))?;
            let serial_report = serial.report();
            (
                serial_report.terminal_seconds / serial_report.solve_seconds.max(f64::MIN_POSITIVE)
                    * 100.0,
                serial_report.iterations,
            )
        };
        let iteration_total: f64 = report.history.iter().map(|r| r.iteration_seconds).sum();
        println!(
            "{}",
            serde_json::to_string(&Benchmark {
                case,
                board: board.into(),
                threads,
                iterations: report.iterations,
                arena_nodes: report.arena_nodes,
                memory_bytes: report.memory_bytes,
                setup_seconds: report.setup_seconds,
                solve_seconds: report.solve_seconds,
                seconds_per_iteration: iteration_total / report.iterations.max(1) as f64,
                terminal_work_seconds: report.terminal_seconds,
                terminal_work_over_solve_wall_pct: if report.solve_seconds > 0.0 {
                    report.terminal_seconds / report.solve_seconds * 100.0
                } else {
                    0.0
                },
                serial_terminal_fraction_pct,
                serial_profile_iterations,
                isomorphism_used: report.isomorphism_used,
                merged_chance_children: report.merged_chance_children,
                exploitability_pct: report.metrics.exploitability_pct,
            })?
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(command: &str, extra: &[&str]) -> SolveArgs {
        let base = [
            "cli",
            command,
            "--board",
            "QsJh2h",
            "--oop-range",
            "oop.json",
            "--ip-range",
            "ip.json",
            "--out",
            "solution.json",
        ];
        match Cli::try_parse_from(base.into_iter().chain(extra.iter().copied()))
            .unwrap()
            .command
        {
            Command::Solve(args) | Command::SolveSubgame(args) => args,
            _ => panic!("expected solve"),
        }
    }
    fn fractions(rule: &SizeRule) -> [f64; 3] {
        [tree::Street::Flop, tree::Street::Turn, tree::Street::River].map(|street| {
            rule.fraction(&tree::SizeContext {
                board: &[],
                street,
                player: 0,
                first_bet: true,
                after_check: false,
            })
        })
    }
    #[test]
    fn size_rules_conflict_with_every_bet_override() {
        for command in ["solve", "solve-subgame"] {
            for bet in ["--bet", "--bet-flop", "--bet-turn", "--bet-river"] {
                let result = Cli::try_parse_from([
                    "cli",
                    command,
                    "--board",
                    "QsJh2h",
                    "--oop-range",
                    "o",
                    "--ip-range",
                    "i",
                    "--out",
                    "s",
                    "--size-rules",
                    "rules.json",
                    bet,
                    "66",
                ]);
                assert_eq!(
                    result.err().unwrap().kind(),
                    clap::error::ErrorKind::ArgumentConflict
                );
            }
        }
    }
    #[test]
    fn size_rules_cli_context_and_unchanged_default_shape() {
        let path = std::env::temp_dir().join(format!("cli-size-rules-{}.json", std::process::id()));
        let table = sizing::rules::RuleTable {
            version: 1,
            candidates: vec![0.25, 0.5, 1.0],
            default_fraction: 0.66,
            rules: vec![
                sizing::rules::Rule {
                    street: Some(tree::Street::Turn),
                    player: Some(0),
                    after_check: Some(false),
                    fraction: 0.25,
                    ..Default::default()
                },
                sizing::rules::Rule {
                    street: Some(tree::Street::Turn),
                    player: Some(1),
                    after_check: Some(true),
                    fraction: 1.0,
                    ..Default::default()
                },
                sizing::rules::Rule {
                    street: Some(tree::Street::River),
                    fraction: 0.5,
                    ..Default::default()
                },
            ],
        };
        fs::write(&path, serde_json::to_vec(&table).unwrap()).unwrap();
        let configured = args("solve", &["--size-rules", path.to_str().unwrap()]);
        let rule = configured.tree_config(BB, None).unwrap().bet_size;
        let board: Vec<_> = parse_board("QsJh2h3c4d")
            .unwrap()
            .iter()
            .map(|c| c.0)
            .collect();
        for (street, player, after_check, expected) in [
            (tree::Street::Flop, 0, false, 0.66),
            (tree::Street::Turn, 0, false, 0.25),
            (tree::Street::Turn, 1, true, 1.0),
            (tree::Street::Turn, 1, false, 0.66),
            (tree::Street::River, 0, false, 0.5),
        ] {
            let len = match street {
                tree::Street::Flop => 3,
                tree::Street::Turn => 4,
                tree::Street::River => 5,
            };
            assert_eq!(
                rule.fraction(&tree::SizeContext {
                    board: &board[..len],
                    street,
                    player,
                    first_bet: true,
                    after_check
                }),
                expected
            );
        }
        let default_spec = args("solve", &[]).game_spec(false).unwrap();
        assert!(serde_json::to_value(default_spec)
            .unwrap()
            .get("size_rules")
            .is_none());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn reach_line_paths_and_selector_constraints() {
        let config = args("solve", &[]).tree_config(BB, None).unwrap();
        // River keeps this test small; terminal destinations still resolve correctly.
        let tree = tree::GameTree::build(
            parse_board("QsJh2h3c4d")
                .unwrap()
                .iter()
                .map(|c| c.0)
                .collect(),
            config,
        )
        .unwrap();
        assert_eq!(
            resolve_reach_line(&tree, "X,X").unwrap(),
            resolve_reach_line(&tree, "X X").unwrap()
        );
        assert!(resolve_reach_line(&tree, "X,B,C").is_ok());
        for invalid in ["", "Z", "C", "X,X,X"] {
            assert!(resolve_reach_line(&tree, invalid).is_err());
        }
        let parsed = args(
            "solve",
            &[
                "--reach-line",
                "X,X",
                "--reach-out",
                "r.json",
                "--reach-card",
                "3c",
            ],
        );
        assert_eq!(parsed.reach_line.as_deref(), Some("X,X"));
        let request: ReachRequest =
            serde_json::from_str(r#"{"line":"X,X","card":"2c","out":"reach.json"}"#).unwrap();
        assert_eq!(
            request.card.unwrap().parse::<Card>().unwrap().to_string(),
            "2c"
        );
        let mut ambiguous = tree.clone();
        if let tree::NodeKind::Decision {
            actions, children, ..
        } = &mut ambiguous.nodes[tree.root].kind
        {
            let child = children[0];
            actions.push(tree::Action::Check);
            children.push(child);
        }
        assert!(resolve_reach_line(&ambiguous, "X")
            .unwrap_err()
            .to_string()
            .contains("2 matches"));

        assert!(Cli::try_parse_from([
            "cli",
            "solve",
            "--board",
            "QsJh2h",
            "--oop-range",
            "o",
            "--ip-range",
            "i",
            "--out",
            "s",
            "--reach-line",
            "X,X",
            "--reach-node",
            "0",
            "--reach-out",
            "r"
        ])
        .is_err());
    }
    #[test]
    fn scratch_precision_defaults_and_explicit_values_propagate() {
        for command in ["solve", "solve-subgame"] {
            for preset in ["production", "benchmark"] {
                let defaults = args(command, &["--preset", preset]);
                assert!(matches!(
                    defaults.engine_config().unwrap().scratch_precision,
                    ScratchPrecision::F64
                ));
                for (value, expected) in [
                    ("f32", ScratchPrecision::F32),
                    ("f64", ScratchPrecision::F64),
                ] {
                    let explicit =
                        args(command, &["--preset", preset, "--scratch-precision", value]);
                    let actual = explicit.engine_config().unwrap().scratch_precision;
                    assert_eq!(actual, expected);
                }
            }
        }
    }
    #[test]
    fn scratch_precision_rejects_unknown_values() {
        for command in ["solve", "solve-subgame"] {
            let result = Cli::try_parse_from([
                "cli",
                command,
                "--board",
                "QsJh2h",
                "--oop-range",
                "oop.json",
                "--ip-range",
                "ip.json",
                "--out",
                "solution.json",
                "--scratch-precision",
                "f16",
            ]);
            assert!(matches!(
                result.err().unwrap().kind(),
                clap::error::ErrorKind::InvalidValue
            ));
        }
    }
    #[test]
    fn wide_totals_requires_f32_scratch() {
        for command in ["solve", "solve-subgame"] {
            assert!(args(command, &["--wide-totals"]).engine_config().is_err());
            assert!(
                args(command, &["--scratch-precision", "f64", "--wide-totals"])
                    .engine_config()
                    .is_err()
            );
            assert!(matches!(
                args(command, &["--scratch-precision", "f32", "--wide-totals"])
                    .engine_config()
                    .unwrap()
                    .scratch_precision,
                ScratchPrecision::F32WideTotals
            ));
        }
    }
    #[test]
    fn production_and_benchmark_presets() {
        let defaults = args("solve", &[]).game_spec(false).unwrap();
        assert_eq!(defaults.units_per_bb, 1000);
        assert_eq!(defaults.tree.starting_pot, 5500);
        assert_eq!(defaults.tree.effective_stack, 97500);
        assert_eq!(fractions(&defaults.tree.bet_size), [0.33, 0.66, 0.66]);
        assert!(!defaults.tree.oop_can_lead_flop);
        assert!(defaults.tree.allin_merge_enabled && defaults.tree.allow_raises);
        assert_eq!(defaults.tree.allin_merge_ratio, 0.67);
        assert_eq!(defaults.tree.raise_multiplier, 2.5);
        assert_eq!(defaults.tree.rake_rate, 0.05);
        assert_eq!(defaults.tree.rake_cap, 3000);
        assert!(matches!(
            defaults.engine.storage_mode,
            cfr::StorageMode::F32
        ));
        assert_eq!(defaults.engine.dcfr_gamma, 2.0);
        let benchmark = args("solve", &["--preset", "benchmark"])
            .game_spec(false)
            .unwrap();
        assert_eq!(benchmark.units_per_bb, 65536);
        assert_eq!(benchmark.tree.starting_pot, 360448);
        assert_eq!(benchmark.tree.effective_stack, 6389760);
        assert_eq!(fractions(&benchmark.tree.bet_size), [0.5; 3]);
        assert!(benchmark.tree.oop_can_lead_flop);
        assert!(!benchmark.tree.allin_merge_enabled);
        assert_eq!(benchmark.tree.rake_rate, 0.0);
        assert_eq!(benchmark.tree.rake_cap, 0);
        assert!(matches!(
            benchmark.engine.storage_mode,
            cfr::StorageMode::I16
        ));
        assert_eq!(benchmark.engine.dcfr_gamma, 4.0);
    }
    #[test]
    fn explicit_flags_override_presets_and_bet_overrides_all_streets() {
        let config = args(
            "solve",
            &[
                "--preset",
                "benchmark",
                "--pot",
                "10",
                "--stack",
                "20",
                "--units-per-bb",
                "1000",
                "--bet",
                "25",
                "--bet-turn",
                "100",
                "--bet-river",
                "125",
                "--rake",
                "0.05",
                "--rake-cap",
                "3",
                "--allin-merge",
                "--allin-merge-ratio",
                "0.8",
                "--raise-multiplier",
                "3",
                "--no-oop-leads",
                "--storage",
                "f32",
                "--dcfr-gamma",
                "2",
            ],
        )
        .game_spec(false)
        .unwrap();
        assert_eq!(config.tree.starting_pot, 10000);
        assert_eq!(config.tree.effective_stack, 20000);
        assert_eq!(fractions(&config.tree.bet_size), [0.25, 1.0, 1.25]);
        assert_eq!(config.tree.rake_rate, 0.05);
        assert_eq!(config.tree.rake_cap, 3000);
        assert!(config.tree.allin_merge_enabled);
        assert_eq!(config.tree.allin_merge_ratio, 0.8);
        assert!(!config.tree.oop_can_lead_flop);
        assert_eq!(config.tree.raise_multiplier, 3.0);
        assert!(matches!(config.engine.storage_mode, cfr::StorageMode::F32));
        assert_eq!(config.engine.dcfr_gamma, 2.0);
        let old = args(
            "solve",
            &[
                "--bet",
                "50",
                "--no-allin-merge",
                "--allin-merge-ratio",
                "0.5",
                "--oop-leads",
            ],
        );
        let tree = old.game_spec(false).unwrap().tree;
        assert!(!tree.allin_merge_enabled);
        assert!(tree.oop_can_lead_flop);
        assert!(matches!(tree.bet_size, SizeRule::Fixed(0.5)));
        assert!(args("solve", &["--bet-turn", "NaN"])
            .game_spec(false)
            .is_err());
        assert!(args("solve", &["--bet-flop", "0"])
            .game_spec(false)
            .is_err());
    }
    #[test]
    fn production_metrics_and_opt_in_profiling() {
        let defaults = args("solve", &[]);
        assert!(!defaults.profiling && !defaults.no_profiling && !defaults.kernel_diagnostics);
        assert!(!defaults.engine_config().unwrap().profiling);
        let schedule = defaults.stop_conditions().adaptive_metrics.unwrap();
        assert_eq!((schedule.early_every, schedule.near_every), (25, 10));
        assert!(args("solve", &["--preset", "benchmark"])
            .stop_conditions()
            .adaptive_metrics
            .is_none());
        assert!(args("solve", &["--no-adaptive-metrics"])
            .stop_conditions()
            .adaptive_metrics
            .is_none());
        assert_eq!(
            args("solve", &["--metrics-every", "7"])
                .stop_conditions()
                .adaptive_metrics
                .unwrap()
                .near_every,
            7
        );
        assert!(
            args("solve", &["--profiling"])
                .engine_config()
                .unwrap()
                .profiling
        );
        assert_eq!(defaults.parallel_grain, 1);
        let explicit = args(
            "solve",
            &[
                "--no-profiling",
                "--parallel-grain",
                "8",
                "--adaptive-metrics",
                "--metrics-near-every",
                "3",
            ],
        );
        assert!(explicit.no_profiling && explicit.adaptive_metrics);
        assert_eq!(explicit.parallel_grain, 8);
        assert_eq!(explicit.metrics_near_every, Some(3));
        assert!(!explicit.engine_config().unwrap().profiling);
        assert_eq!(
            explicit
                .stop_conditions()
                .adaptive_metrics
                .unwrap()
                .near_every,
            3
        );
    }
    #[test]
    fn subgame_requires_turn_or_river_and_explicit_chip_state() {
        assert!(args("solve-subgame", &["--pot", "5.5", "--stack", "97.5"])
            .game_spec(true)
            .is_err());
        let parsed = Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--board",
            "QsJh2h4c",
            "--pot",
            "5.5",
            "--stack",
            "97.5",
            "--oop-range",
            "oop.json",
            "--ip-range",
            "ip.json",
            "--out",
            "solution.json",
        ])
        .unwrap();
        let Command::SolveSubgame(mut args) = parsed.command else {
            panic!()
        };
        assert_eq!(args.game_spec(true).unwrap().board.len(), 4);
        args.pot = None;
        assert!(args.game_spec(true).is_err());
        assert!(Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--state",
            "reach.json",
            "--card",
            "4c",
            "--out",
            "solution.json"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--state",
            "reach.json",
            "--pot",
            "2",
            "--out",
            "solution.json"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "cli",
            "solve",
            "--board",
            "QsJh2h",
            "--oop-range",
            "oop.json",
            "--ip-range",
            "ip.json",
            "--reach-node",
            "0",
            "--out",
            "solution.json"
        ])
        .is_err());
    }
    #[test]
    fn river_cli_reach_export_and_snapshot_replay() {
        let dir = std::env::temp_dir().join(format!(
            "reyson-cli-m3-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let oop = dir.join("oop.json");
        let ip = dir.join("ip.json");
        let report = dir.join("report.json");
        let reach = dir.join("reach.json");
        let replay = dir.join("replay.json");
        fs::write(&oop, r#"{"QhJh":1,"9h8h":1}"#).unwrap();
        fs::write(&ip, r#"{"Th5h":1,"7h6h":1}"#).unwrap();
        let Command::SolveSubgame(args) = Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--board",
            "AhKh2h3h4h",
            "--pot",
            "2",
            "--stack",
            "1",
            "--oop-range",
            oop.to_str().unwrap(),
            "--ip-range",
            ip.to_str().unwrap(),
            "--threads",
            "1",
            "--max-iters",
            "2",
            "--target",
            "0",
            "--out",
            report.to_str().unwrap(),
            "--reach-node",
            "0",
            "--reach-out",
            reach.to_str().unwrap(),
        ])
        .unwrap()
        .command
        else {
            panic!()
        };
        run_solve(args, true).unwrap();
        let default_report: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
        assert!(default_report["game_spec"].get("size_rules").is_none());
        let snapshot: solver::ReachSnapshot =
            serde_json::from_slice(&fs::read(&reach).unwrap()).unwrap();
        assert_eq!(snapshot.board.len(), 5);
        assert_eq!(snapshot.pot, 2000);
        assert_eq!(snapshot.stacks, [1000; 2]);
        let Command::SolveSubgame(args) = Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--state",
            reach.to_str().unwrap(),
            "--threads",
            "1",
            "--max-iters",
            "2",
            "--target",
            "0",
            "--out",
            replay.to_str().unwrap(),
        ])
        .unwrap()
        .command
        else {
            panic!()
        };
        run_solve(args, true).unwrap();
        let first: serde_json::Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&fs::read(&replay).unwrap()).unwrap();
        assert_eq!(first["game_spec"], second["game_spec"]);
        assert_eq!(first["metrics"], second["metrics"]);
        let rule_path = dir.join("rules.json");
        let ruled_report = dir.join("ruled-report.json");
        let ruled_snapshot = dir.join("ruled-snapshot.json");
        let restored_report = dir.join("restored-report.json");
        let table = sizing::rules::RuleTable {
            version: 1,
            candidates: vec![0.5],
            default_fraction: 0.5,
            rules: vec![],
        };
        fs::write(&rule_path, serde_json::to_vec(&table).unwrap()).unwrap();
        let Command::SolveSubgame(ruled) = Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--state",
            reach.to_str().unwrap(),
            "--size-rules",
            rule_path.to_str().unwrap(),
            "--threads",
            "1",
            "--max-iters",
            "2",
            "--target",
            "0",
            "--reach-node",
            "0",
            "--reach-out",
            ruled_snapshot.to_str().unwrap(),
            "--out",
            ruled_report.to_str().unwrap(),
        ])
        .unwrap()
        .command
        else {
            panic!()
        };
        run_solve(ruled, true).unwrap();
        let actual: serde_json::Value =
            serde_json::from_slice(&fs::read(&ruled_report).unwrap()).unwrap();
        assert_eq!(
            actual["game_spec"]["size_rules"],
            serde_json::to_value(&table).unwrap()
        );
        let Command::SolveSubgame(restored) = Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--state",
            ruled_snapshot.to_str().unwrap(),
            "--threads",
            "1",
            "--max-iters",
            "2",
            "--target",
            "0",
            "--out",
            restored_report.to_str().unwrap(),
        ])
        .unwrap()
        .command
        else {
            panic!()
        };
        run_solve(restored, true).unwrap();
        let restored: serde_json::Value =
            serde_json::from_slice(&fs::read(&restored_report).unwrap()).unwrap();
        assert_eq!(actual["game_spec"], restored["game_spec"]);
        assert_eq!(actual["metrics"], restored["metrics"]);
        // An explicit preset never runs on top of the snapshot's inherited rules.
        let Command::SolveSubgame(mixed) = Cli::try_parse_from([
            "cli",
            "solve-subgame",
            "--state",
            ruled_snapshot.to_str().unwrap(),
            "--preset",
            "benchmark",
            "--out",
            dir.join("mixed-report.json").to_str().unwrap(),
        ])
        .unwrap()
        .command
        else {
            panic!()
        };
        assert!(run_solve(mixed, true).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn board_and_chip_parsing() {
        assert_eq!(parse_board("Ah, Kd 2c").unwrap().len(), 3);
        assert!(parse_board("AhAh2c").is_err());
        assert!(parse_board("🂡Kd2c").is_err());
        assert_eq!(chips(2.5005).unwrap(), 2501);
        assert!(chips(f64::NAN).is_err());
    }
}
