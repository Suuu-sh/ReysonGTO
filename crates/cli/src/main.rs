use anyhow::{anyhow, bail, Context, Result};
use cards::Card;
use cfr::{Algorithm, EngineConfig};
use clap::{Parser, Subcommand, ValueEnum};
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
    Solve {
        #[arg(long)]
        board: String,
        #[arg(long)]
        pot: f64,
        #[arg(long)]
        stack: f64,
        #[arg(long)]
        oop_range: PathBuf,
        #[arg(long)]
        ip_range: PathBuf,
        /// Percentage of the pot, e.g. 50 for a half-pot bet.
        #[arg(long)]
        bet: f64,
        #[arg(long)]
        oop_leads: bool,
        #[arg(long, default_value_t = BB)]
        units_per_bb: i64,
        #[arg(long, default_value_t = 2.5)]
        raise_multiplier: f64,
        #[arg(long, default_value_t = 0.67)]
        allin_merge_ratio: f64,
        #[arg(long)]
        no_allin_merge: bool,
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
        #[arg(long, value_enum, default_value_t = Storage::F32)]
        storage: Storage,
        #[arg(long, default_value_t = 1.5)]
        dcfr_alpha: f64,
        #[arg(long, default_value_t = 0.0)]
        dcfr_beta: f64,
        #[arg(long, default_value_t = 2.0)]
        dcfr_gamma: f64,
        #[arg(long, default_value_t = 0.05)]
        rake: f64,
        #[arg(long, default_value_t = 3.0)]
        rake_cap: f64,
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
    },
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
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Solve {
            board,
            pot,
            stack,
            oop_range,
            ip_range,
            bet,
            oop_leads,
            units_per_bb,
            raise_multiplier,
            allin_merge_ratio,
            no_allin_merge,
            metrics_every,
            adaptive_metrics,
            metrics_early_every,
            metrics_near_every,
            metrics_near_target_multiplier,
            progress_jsonl,
            storage,
            dcfr_alpha,
            dcfr_beta,
            dcfr_gamma,
            rake,
            rake_cap,
            target,
            max_iters,
            max_seconds,
            out,
            threads,
            no_profiling,
            kernel_diagnostics,
            parallel_grain,
            algorithm,
            no_isomorphism,
            memory_mb,
        } => {
            if !bet.is_finite() || bet <= 0.0 {
                bail!("bet percentage must be finite and positive");
            }
            let spec = GameSpec {
                board: parse_board(&board)?,
                units_per_bb,
                tree: TreeConfig {
                    starting_pot: chips_at(pot, units_per_bb)?,
                    effective_stack: chips_at(stack, units_per_bb)?,
                    oop_can_lead_flop: oop_leads,
                    bet_size: SizeRule::Fixed(bet / 100.0),
                    rake_rate: rake,
                    rake_cap: chips_at(rake_cap, units_per_bb)?,
                    raise_multiplier,
                    allin_merge_ratio,
                    allin_merge_enabled: !no_allin_merge,
                    ..TreeConfig::default()
                },
                engine: EngineConfig {
                    threads,
                    parallel_grain,
                    algorithm: match algorithm {
                        Method::Dcfr => Algorithm::Dcfr,
                        Method::CfrPlus => Algorithm::CfrPlus,
                    },
                    suit_isomorphism: !no_isomorphism,
                    memory_limit_bytes: memory_mb
                        .checked_mul(1024 * 1024)
                        .ok_or_else(|| anyhow!("memory limit overflow"))?,
                    profiling: !no_profiling,
                    kernel_diagnostics,
                    storage_mode: match storage {
                        Storage::F32 => cfr::StorageMode::F32,
                        Storage::I16 => cfr::StorageMode::I16,
                    },
                    dcfr_alpha,
                    dcfr_beta,
                    dcfr_gamma,
                },
            };
            let mut progress_file = progress_jsonl
                .map(|path| {
                    fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .with_context(|| format!("cannot create progress {}", path.display()))
                })
                .transpose()?;
            let solution = solve_with_progress(
                spec,
                [read_range(oop_range)?, read_range(ip_range)?],
                StopConditions {
                    target_exploitability_pct: target,
                    max_iterations: max_iters,
                    max_seconds,
                    metrics_every,
                    adaptive_metrics: adaptive_metrics.then_some(AdaptiveMetrics {
                        early_every: metrics_early_every,
                        near_every: metrics_near_every,
                        near_target_multiplier: metrics_near_target_multiplier,
                    }),
                },
                |event| {
                    let line = serde_json::to_string(event).map_err(|e| e.to_string())?;
                    eprintln!("{line}");
                    if let Some(file) = &mut progress_file {
                        writeln!(file, "{line}")
                            .and_then(|_| file.flush())
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                },
            )
            .map_err(|e| anyhow!(e))?;
            let report = solution.report();
            let bytes = serde_json::to_vec_pretty(&report)?;
            write_atomic(&out, &bytes)?;
            eprintln!(
                "iterations={} exploitability={:.6}% memory={} bytes stop={} output={}",
                report.iterations,
                report.metrics.exploitability_pct,
                report.memory_bytes,
                report.stopped_by,
                out.display()
            );
        }
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
    #[test]
    fn solve_performance_options_are_opt_in() {
        let base = [
            "cli",
            "solve",
            "--board",
            "QsJh2h",
            "--pot",
            "5.5",
            "--stack",
            "97.5",
            "--oop-range",
            "oop.json",
            "--ip-range",
            "ip.json",
            "--bet",
            "50",
            "--out",
            "solution.json",
        ];
        let cli = Cli::try_parse_from(base).unwrap();
        let Command::Solve {
            no_profiling,
            parallel_grain,
            adaptive_metrics,
            ..
        } = cli.command
        else {
            panic!("expected solve");
        };
        assert!(!no_profiling);
        assert!(!adaptive_metrics);
        assert_eq!(parallel_grain, 1);
        let cli = Cli::try_parse_from(base.into_iter().chain([
            "--no-profiling",
            "--parallel-grain",
            "8",
            "--adaptive-metrics",
            "--metrics-near-every",
            "3",
        ]))
        .unwrap();
        let Command::Solve {
            no_profiling,
            parallel_grain,
            adaptive_metrics,
            metrics_near_every,
            ..
        } = cli.command
        else {
            panic!("expected solve");
        };
        assert!(no_profiling && adaptive_metrics);
        assert_eq!(parallel_grain, 8);
        assert_eq!(metrics_near_every, 3);
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
