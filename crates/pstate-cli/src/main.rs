//! Local debug/verification CLI for `cpu-power-hal` — no Kubernetes
//! required. Meant to be run (or `scp`'d) directly on a node to sanity
//! check backend detection and apply/read behavior against real hardware,
//! since CI can never have the real NUCs/Framework Desktop/QEMU VM.

use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use cpu_power_hal::{PowerProfile, RootedSysfs, SysfsIo, detect_backend, discover_policies};

#[derive(Parser)]
#[command(
    name = "pstate-cli",
    version,
    about = "Probe, apply, and read CPU power profiles via cpu-power-hal"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Detect the CPU power backend on this host and print its capabilities.
    Probe,
    /// Apply a power profile and/or turbo state on this host.
    Apply {
        /// One of: default, performance, balance_performance, balance_power, power
        #[arg(long)]
        profile: Option<String>,
        /// One of: enabled, disabled
        #[arg(long)]
        turbo: Option<String>,
    },
    /// Print the currently applied profile, if determinable.
    Read,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let io: Arc<dyn SysfsIo> = Arc::new(RootedSysfs::host());

    match cli.command {
        Command::Probe => probe(io),
        Command::Apply { profile, turbo } => apply(io, profile, turbo),
        Command::Read => read(io),
    }
}

fn probe(io: Arc<dyn SysfsIo>) -> ExitCode {
    let policies = match discover_policies(io.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error discovering cpufreq policies: {e}");
            return ExitCode::FAILURE;
        }
    };

    if policies.is_empty() {
        println!("no cpufreq policies found under /sys/devices/system/cpu/cpufreq");
        println!("this host will use the Unsupported backend (all operations are no-ops)");
    } else {
        println!(
            "found {} cpufreq polic{}",
            policies.len(),
            if policies.len() == 1 { "y" } else { "ies" }
        );
        for policy in &policies {
            print!(
                "  policy{}: driver={} epp={} governors=[{}]",
                policy.id,
                policy.scaling_driver,
                policy.has_epp,
                policy.available_governors.join(", "),
            );
            if policy.has_epp {
                println!(" epp_values=[{}]", policy.available_epp.join(", "));
            } else {
                println!();
            }
        }
    }

    let backend = detect_backend(io);
    println!(
        "\ndetected backend: {} (supported={})",
        backend.kind(),
        backend.is_supported()
    );
    match backend.current() {
        Ok(Some(profile)) => println!("current profile: {profile}"),
        Ok(None) => println!("current profile: <not determinable on this backend>"),
        Err(e) => println!("current profile: error reading it ({e})"),
    }

    ExitCode::SUCCESS
}

fn apply(io: Arc<dyn SysfsIo>, profile: Option<String>, turbo: Option<String>) -> ExitCode {
    if profile.is_none() && turbo.is_none() {
        eprintln!("nothing to do: pass --profile and/or --turbo");
        return ExitCode::FAILURE;
    }

    let backend = detect_backend(io);
    let mut ok = true;

    if let Some(raw) = &profile {
        match raw.parse::<PowerProfile>() {
            Ok(p) => match backend.apply(p) {
                Ok(()) => println!("applied profile: {p}"),
                Err(e) => {
                    eprintln!("failed to apply profile {raw}: {e}");
                    ok = false;
                }
            },
            Err(e) => {
                eprintln!("invalid profile '{raw}': {e}");
                ok = false;
            }
        }
    }

    if let Some(raw) = &turbo {
        match raw.as_str() {
            "enabled" | "disabled" => {
                let enabled = raw == "enabled";
                match backend.set_turbo(enabled) {
                    Ok(()) => println!("applied turbo: {raw}"),
                    Err(e) => {
                        eprintln!("failed to apply turbo: {e}");
                        ok = false;
                    }
                }
            }
            other => {
                eprintln!("invalid turbo value '{other}': expected 'enabled' or 'disabled'");
                ok = false;
            }
        }
    }

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn read(io: Arc<dyn SysfsIo>) -> ExitCode {
    let backend = detect_backend(io);
    println!("backend: {}", backend.kind());
    match backend.current() {
        Ok(Some(profile)) => {
            println!("profile: {profile}");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("profile: <not determinable on this backend>");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error reading current profile: {e}");
            ExitCode::FAILURE
        }
    }
}
