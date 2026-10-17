//! Reproducible local preflop tools; no product ranges are embedded.
use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use preflop::{
    cfr::{PreflopSolver, RealizationConfig},
    config::PreflopConfig,
    equity::{class_index, EquityTable, ThreeWayEquityTable},
    tree::PreflopTree,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Parser)]
#[command(
    name = "preflop",
    about = "Clean-room preflop tree, equity and multiway DCFR"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Count the full public betting tree (no private-card chance expansion).
    TreeStats {
        #[arg(long)]
        config: PathBuf,
        /// Start at BTN with all other seats except BB already folded.
        #[arg(long)]
        heads_up: bool,
        #[arg(long)]
        push_fold: bool,
    },
    /// Monte Carlo each unordered class matchup; reverse is its complement.
    EquityBuild {
        #[arg(long, default_value_t = 10000)]
        samples_per_pair: u64,
        #[arg(long, default_value_t = 20261015)]
        seed: u64,
        #[arg(long, default_value_t = 8)]
        threads: usize,
        #[arg(long, default_value = "data/equity/hu169.json")]
        out: PathBuf,
    },
    /// Sample every unordered three-class matchup with independent deterministic seeds.
    #[command(name = "equity-build-3way")]
    EquityBuild3way {
        #[arg(long)]
        samples: u64,
        #[arg(long)]
        seed: u64,
        #[arg(long, default_value_t = 4)]
        threads: usize,
        #[arg(long)]
        out: PathBuf,
    },
    /// Validate a saved table and print three familiar matchup diagnostics.
    EquityCheck {
        #[arg(long)]
        table: PathBuf,
    },
    /// Solve the configured seats using saved equity and explicit realization.
    Solve {
        #[arg(long)]
        heads_up: bool,
        #[arg(long)]
        equity_3way: Option<PathBuf>,
        /// Omit per-node class arrays to keep large-tree smoke reports bounded.
        #[arg(long)]
        summary_only: bool,
        /// Use the frozen M5 betting tree for historical two-player regression.
        #[arg(long)]
        legacy_hu: bool,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        equity: PathBuf,
        #[arg(long)]
        realization: PathBuf,
        #[arg(long)]
        push_fold: bool,
        #[arg(long, default_value_t = 2000)]
        max_iters: u64,
        #[arg(long, default_value_t = 300.0)]
        max_seconds: f64,
        #[arg(long, default_value_t = 50)]
        metrics_every: u64,
        /// Stop when SUM of all unilateral BR gains is below this many BB.
        #[arg(long, default_value_t = 0.001)]
        target_bb: f64,
        #[arg(long)]
        out: PathBuf,
    },
}
fn read_config(path: &Path, heads_up: bool) -> Result<PreflopConfig> {
    let config = PreflopConfig::from_path(path).map_err(anyhow::Error::msg)?;
    if heads_up {
        config.heads_up_btn_bb().map_err(anyhow::Error::msg)
    } else {
        Ok(config)
    }
}
fn build_tree(config: PreflopConfig, push_fold: bool) -> Result<PreflopTree> {
    if push_fold {
        PreflopTree::push_fold(config)
    } else {
        PreflopTree::build(config)
    }
    .map_err(anyhow::Error::msg)
}
fn save_report(path: &Path, report: &Value) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, serde_json::to_vec_pretty(report)?)?;
    fs::rename(&temporary, path).with_context(|| format!("cannot save {}", path.display()))
}
fn compact_report(report: &Value) -> Value {
    let fields = [
        "iterations",
        "training_seconds",
        "report_seconds",
        "profile_ev_bb",
        "best_response_ev_bb",
        "best_response_gain_bb",
        "nash_conv_bb",
        "half_gap_bb",
        "summary",
        "stop_reason",
        "wall_seconds",
        "target_nash_conv_bb",
        "tree_stats",
    ];
    Value::Object(
        fields
            .into_iter()
            .filter_map(|key| report.get(key).map(|value| (key.to_owned(), value.clone())))
            .collect(),
    )
}
fn validate_solve_limits(
    max_iters: u64,
    max_seconds: f64,
    metrics_every: u64,
    target_bb: f64,
) -> Result<()> {
    if max_iters == 0
        || metrics_every == 0
        || !max_seconds.is_finite()
        || max_seconds <= 0.0
        || !target_bb.is_finite()
        || target_bb < 0.0
    {
        bail!("iterations/cadence/time must be positive and target BB finite and nonnegative");
    }
    Ok(())
}
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::TreeStats {
            config,
            heads_up,
            push_fold,
        } => {
            let tree = build_tree(read_config(&config, heads_up)?, push_fold)?;
            println!("{}", serde_json::to_string_pretty(&tree.stats())?);
        }
        Command::EquityBuild {
            samples_per_pair,
            seed,
            threads,
            out,
        } => {
            if threads == 0 {
                bail!("threads must be positive");
            }
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()?;
            eprintln!("Generating 169x169 table: {samples_per_pair} samples/unordered matchup, {threads} threads; no concurrent solve.");
            let started = Instant::now();
            let table = pool
                .install(|| EquityTable::generate(samples_per_pair, seed))
                .map_err(anyhow::Error::msg)?;
            table.save(&out).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "out": out, "wall_seconds": started.elapsed().as_secs_f64(),
                    "threads": threads, "metadata": table.metadata,
                }))?
            );
        }
        Command::EquityBuild3way {
            samples,
            seed,
            threads,
            out,
        } => {
            if threads == 0 {
                bail!("threads must be positive");
            }
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()?;
            let started = Instant::now();
            let table = pool
                .install(|| ThreeWayEquityTable::generate(samples, seed))
                .map_err(anyhow::Error::msg)?;
            table.save(&out).map_err(anyhow::Error::msg)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "out": out, "wall_seconds": started.elapsed().as_secs_f64(),
                    "threads": threads, "metadata": table.metadata,
                }))?
            );
        }
        Command::EquityCheck { table } => {
            let table = EquityTable::load(table).map_err(anyhow::Error::msg)?;
            let known = [("AA", "KK"), ("AKs", "22"), ("AKo", "QQ")].map(|(a,b)| {
                json!({"a":a,"b":b,"equity_a":table.get(class_index(a).unwrap(),class_index(b).unwrap())})
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"metadata":table.metadata,"matchups":known}))?
            );
        }
        Command::Solve {
            heads_up,
            equity_3way,
            summary_only,
            legacy_hu,
            config,
            equity,
            realization,
            push_fold,
            max_iters,
            max_seconds,
            metrics_every,
            target_bb,
            out,
        } => {
            validate_solve_limits(max_iters, max_seconds, metrics_every, target_bb)?;
            let started = Instant::now();
            let config = read_config(&config, heads_up)?;
            if legacy_hu && (config.players != 2 || push_fold) {
                bail!("--legacy-hu requires two seats and cannot be combined with --push-fold");
            }
            let tree = if legacy_hu {
                PreflopTree::build_legacy(config).map_err(anyhow::Error::msg)?
            } else {
                build_tree(config, push_fold)?
            };
            let stats = serde_json::to_value(tree.stats())?;
            let table = EquityTable::load(equity).map_err(anyhow::Error::msg)?;
            let realization =
                RealizationConfig::from_path(realization).map_err(anyhow::Error::msg)?;
            let three_way = equity_3way
                .map(ThreeWayEquityTable::load)
                .transpose()
                .map_err(anyhow::Error::msg)?;
            let mut solver = PreflopSolver::new_with_three_way(tree, table, realization, three_way)
                .map_err(anyhow::Error::msg)?;
            let mut checkpoints = Vec::new();
            let initial = serde_json::to_value(if summary_only {
                solver.report_summary()
            } else {
                solver.report()
            })?;
            eprintln!("{}", compact_report(&initial));
            checkpoints.push(compact_report(&initial));
            for iteration in 1..=max_iters {
                solver.step();
                let timed_out = started.elapsed().as_secs_f64() >= max_seconds;
                if iteration % metrics_every == 0 || iteration == max_iters || timed_out {
                    let mut report = serde_json::to_value(if summary_only {
                        solver.report_summary()
                    } else {
                        solver.report()
                    })?;
                    let gap = report["nash_conv_bb"]
                        .as_f64()
                        .ok_or_else(|| anyhow!("missing NashConv"))?;
                    let compact = compact_report(&report);
                    eprintln!("{compact}");
                    checkpoints.push(compact);
                    let stop_reason = if gap <= target_bb {
                        "target_bb"
                    } else if timed_out {
                        "max_seconds"
                    } else if iteration == max_iters {
                        "max_iters"
                    } else {
                        "running"
                    };
                    report["checkpoints"] = json!(checkpoints);
                    report["tree_stats"] = stats.clone();
                    report["betting_tree"] = json!(if legacy_hu {
                        "m5_legacy_hu"
                    } else if push_fold {
                        "push_fold"
                    } else {
                        "p4"
                    });
                    report["wall_seconds"] = json!(started.elapsed().as_secs_f64());
                    report["stop_reason"] = json!(stop_reason);
                    report["target_nash_conv_bb"] = json!(target_bb);
                    save_report(&out, &report)?;
                    if stop_reason != "running" {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&compact_report(&report))?
                        );
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn tree_stats_parses_heads_up_and_push_fold() {
        let cli = Cli::try_parse_from([
            "preflop",
            "tree-stats",
            "--config",
            "external.json",
            "--heads-up",
            "--push-fold",
        ])
        .unwrap();
        match cli.command {
            Command::TreeStats {
                config,
                heads_up,
                push_fold,
            } => {
                assert_eq!(config, PathBuf::from("external.json"));
                assert!(heads_up && push_fold);
            }
            _ => panic!("expected tree-stats"),
        }
    }

    #[test]
    fn equity_build_parses_defaults_and_overrides() {
        let defaults = Cli::try_parse_from(["preflop", "equity-build"]).unwrap();
        match defaults.command {
            Command::EquityBuild {
                samples_per_pair,
                seed,
                threads,
                out,
            } => {
                assert_eq!((samples_per_pair, seed, threads), (10_000, 20_261_015, 8));
                assert_eq!(out, PathBuf::from("data/equity/hu169.json"));
            }
            _ => panic!("expected equity-build"),
        }
        let custom = Cli::try_parse_from([
            "preflop",
            "equity-build",
            "--samples-per-pair",
            "20000",
            "--seed",
            "7",
            "--threads",
            "2",
            "--out",
            "custom.json",
        ])
        .unwrap();
        match custom.command {
            Command::EquityBuild {
                samples_per_pair,
                seed,
                threads,
                out,
            } => {
                assert_eq!((samples_per_pair, seed, threads), (20_000, 7, 2));
                assert_eq!(out, PathBuf::from("custom.json"));
            }
            _ => panic!("expected equity-build"),
        }
    }

    #[test]
    fn solve_parses_push_fold_and_validates_finite_limits() {
        let cli = Cli::try_parse_from([
            "preflop",
            "solve",
            "--config",
            "external.json",
            "--equity",
            "hu169.json",
            "--realization",
            "realization.json",
            "--push-fold",
            "--max-iters",
            "20",
            "--max-seconds",
            "1.5",
            "--metrics-every",
            "5",
            "--target-bb",
            "0",
            "--out",
            "report.json",
        ])
        .unwrap();
        match cli.command {
            Command::Solve {
                config,
                equity,
                realization,
                push_fold,
                max_iters,
                max_seconds,
                metrics_every,
                target_bb,
                out,
                ..
            } => {
                assert_eq!(config, PathBuf::from("external.json"));
                assert_eq!(equity, PathBuf::from("hu169.json"));
                assert_eq!(realization, PathBuf::from("realization.json"));
                assert_eq!(out, PathBuf::from("report.json"));
                assert!(push_fold);
                assert_eq!(
                    (max_iters, max_seconds, metrics_every, target_bb),
                    (20, 1.5, 5, 0.0)
                );
                validate_solve_limits(max_iters, max_seconds, metrics_every, target_bb).unwrap();
            }
            _ => panic!("expected solve"),
        }
        for limits in [
            (0, 1.0, 1, 0.0),
            (1, 1.0, 0, 0.0),
            (1, 0.0, 1, 0.0),
            (1, -1.0, 1, 0.0),
            (1, f64::NAN, 1, 0.0),
            (1, f64::INFINITY, 1, 0.0),
            (1, 1.0, 1, -0.1),
            (1, 1.0, 1, f64::NAN),
            (1, 1.0, 1, f64::INFINITY),
        ] {
            assert!(validate_solve_limits(limits.0, limits.1, limits.2, limits.3).is_err());
        }
    }

    #[test]
    fn three_way_build_and_multiway_solve_flags_parse() {
        let cli = Cli::try_parse_from([
            "preflop",
            "equity-build-3way",
            "--samples",
            "2",
            "--seed",
            "7",
            "--out",
            "three.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::EquityBuild3way {
                samples: 2,
                seed: 7,
                threads: 4,
                ..
            }
        ));
        let cli = Cli::try_parse_from([
            "preflop",
            "solve",
            "--config",
            "six.json",
            "--equity",
            "hu.json",
            "--equity-3way",
            "three.json",
            "--realization",
            "realization.json",
            "--out",
            "report.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Solve {
                heads_up: false,
                equity_3way: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn missing_required_file_arguments_are_parse_errors() {
        for args in [
            vec!["preflop", "tree-stats"],
            vec!["preflop", "equity-check"],
            vec!["preflop", "solve"],
            vec![
                "preflop",
                "solve",
                "--config",
                "external.json",
                "--equity",
                "hu169.json",
                "--out",
                "report.json",
            ],
        ] {
            let error = Cli::try_parse_from(args)
                .err()
                .expect("required argument must fail");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
        }
    }
}
