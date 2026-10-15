use anyhow::{anyhow, bail, Context, Result};
use cards::Card;
use cfr::{Algorithm, EngineConfig};
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
    /// Opt in to sparse early exact metrics, then denser checks near the target.
    #[arg(long)]
    adaptive_metrics: bool,
    #[arg(long, default_value_t = 25, requires = "adaptive_metrics")]
    metrics_early_every: usize,
    #[arg(long, default_value_t = 5, requires = "adaptive_metrics")]
    metrics_near_every: usize,
    /// Switch permanently to near cadence at target times this multiplier.
    #[arg(long, default_value_t = 2.0, requires = "adaptive_metrics")]
    metrics_near_target_multiplier: f64,
    /// Flush setup and fresh metric checkpoint JSONL while solving.
    #[arg(long)]
    progress_jsonl: Option<PathBuf>,
    /// Export average-policy own-reach ranges and exact node chip state.
    #[arg(long, requires = "reach_out")]
    reach_node: Option<usize>,
    #[arg(long, requires = "reach_node")]
    reach_out: Option<PathBuf>,
    /// Optionally deal one card before exporting the snapshot.
    #[arg(long, requires = "reach_node")]
    reach_card: Option<Card>,
    #[arg(long, value_enum)]
    storage: Option<Storage>,
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
    /// Disable fine-grained worker timers (the selected mode is recorded in engine metadata).
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
        Ok(config)
    }
    fn engine_config(&self) -> Result<EngineConfig> {
        let benchmark = self.preset == Some(Preset::Benchmark);
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
            profiling: !self.no_profiling,
            kernel_diagnostics: self.kernel_diagnostics,
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
            adaptive_metrics: self.adaptive_metrics.then_some(AdaptiveMetrics {
                early_every: self.metrics_early_every,
                near_every: self.metrics_near_every,
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
fn run_solve(args: SolveArgs, subgame: bool) -> Result<()> {
    if !subgame && args.state.is_some() {
        bail!("--state is only supported by solve-subgame");
    }
    // Validate all input/config before creating progress output.
    let direct = if args.state.is_none() {
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
    let snapshot = args
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
    if let Some(node) = args.reach_node {
        let mut snapshot = solution.reach_snapshot(node).map_err(|e| anyhow!(e))?;
        if let Some(card) = args.reach_card {
            snapshot = snapshot.with_card(card).map_err(|e| anyhow!(e))?;
        }
        write_atomic(
            args.reach_out
                .as_deref()
                .ok_or_else(|| anyhow!("--reach-out is required"))?,
            &serde_json::to_vec_pretty(&snapshot)?,
        )?;
    }
    let report = solution.report();
    write_atomic(&args.out, &serde_json::to_vec_pretty(&report)?)?;
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
    fn solve_performance_options_remain_opt_in() {
        let defaults = args("solve", &[]);
        assert!(
            !defaults.no_profiling && !defaults.adaptive_metrics && !defaults.kernel_diagnostics
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
        assert_eq!(explicit.metrics_near_every, 3);
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
