//! `sizing select` writes the representative and check flops; `sizing flops` lists all 1,755 with features.
use clap::{Parser, Subcommand};
use sizing::flops::{canonical_flops, select_flops};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Stratified training and validation flops as JSON.
    Select {
        #[arg(long, default_value_t = 200)]
        training: usize,
        #[arg(long, default_value_t = 80)]
        validation: usize,
        #[arg(long, default_value_t = 20261015)]
        seed: u64,
    },
    /// Every canonical flop with its weight and features as JSON.
    Flops,
    /// Run the sequential M4 comparison runner (remaining arguments are passed through).
    #[command(disable_help_flag = true)]
    CompareFlops {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Select {
            training,
            validation,
            seed,
        } => {
            let s = select_flops(training, validation, seed);
            let text = |v: &[sizing::flops::CanonicalFlop]| {
                v.iter().map(|f| serde_json::json!({ "flop": f.text(), "weight": f.weight, "features": f.features })).collect::<Vec<_>>()
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "seed": seed, "training": text(&s.training), "validation": text(&s.validation) })
                )?
            );
        }
        Command::CompareFlops { args } => {
            let executable = std::env::current_exe()?;
            let solver = executable.with_file_name("cli");
            let script =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/m4_trial.py");
            let status = std::process::Command::new("python3")
                .arg(script)
                .arg("--binary")
                .arg(solver)
                .arg("--sizing-binary")
                .arg(executable)
                .args(args)
                .status()?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Command::Flops => {
            let all: Vec<_> = canonical_flops().iter().map(|f| serde_json::json!({ "flop": f.text(), "weight": f.weight, "features": f.features })).collect();
            println!("{}", serde_json::to_string_pretty(&all)?);
        }
    }
    Ok(())
}
