use anyhow::{anyhow, bail, Context, Result};
use cards::Card;
use cfr::{Algorithm, EngineConfig};
use clap::{Parser, Subcommand, ValueEnum};
use ranges::Range;
use serde::Serialize;
use solver::{solve, GameSpec, StopConditions};
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
        #[arg(long, value_enum, default_value_t = Method::Dcfr)]
        algorithm: Method,
        #[arg(long)]
        no_isomorphism: bool,
        /// Conservative memory budget in MiB; also bounds arena construction.
        #[arg(long, default_value_t = 8192)]
        memory_mb: usize,
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
fn chips(bb: f64) -> Result<i64> {
    if !bb.is_finite() || bb < 0.0 || bb * BB as f64 >= i64::MAX as f64 {
        bail!("BB amount must be finite, nonnegative and representable");
    }
    Ok((bb * BB as f64).round() as i64)
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
            rake,
            rake_cap,
            target,
            max_iters,
            max_seconds,
            out,
            threads,
            algorithm,
            no_isomorphism,
            memory_mb,
        } => {
            if !bet.is_finite() || bet <= 0.0 {
                bail!("bet percentage must be finite and positive");
            }
            let spec = GameSpec {
                board: parse_board(&board)?,
                tree: TreeConfig {
                    starting_pot: chips(pot)?,
                    effective_stack: chips(stack)?,
                    oop_can_lead_flop: oop_leads,
                    bet_size: SizeRule::Fixed(bet / 100.0),
                    rake_rate: rake,
                    rake_cap: chips(rake_cap)?,
                    ..TreeConfig::default()
                },
                engine: EngineConfig {
                    threads,
                    algorithm: match algorithm {
                        Method::Dcfr => Algorithm::Dcfr,
                        Method::CfrPlus => Algorithm::CfrPlus,
                    },
                    suit_isomorphism: !no_isomorphism,
                    memory_limit_bytes: memory_mb
                        .checked_mul(1024 * 1024)
                        .ok_or_else(|| anyhow!("memory limit overflow"))?,
                    profiling: true,
                },
            };
            let solution = solve(
                spec,
                [read_range(oop_range)?, read_range(ip_range)?],
                StopConditions {
                    target_exploitability_pct: target,
                    max_iterations: max_iters,
                    max_seconds,
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
    fn board_and_chip_parsing() {
        assert_eq!(parse_board("Ah, Kd 2c").unwrap().len(), 3);
        assert!(parse_board("AhAh2c").is_err());
        assert!(parse_board("🂡Kd2c").is_err());
        assert_eq!(chips(2.5005).unwrap(), 2501);
        assert!(chips(f64::NAN).is_err());
    }
}
