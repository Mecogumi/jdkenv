//! jdkenv — JDK version manager for Windows (native, no WSL/Git Bash).
//!
//! Design model: a stable directory **junction** `%USERPROFILE%\.jdkenv\
//! current` acts as a level of indirection. PATH and JAVA_HOME point to `current`
//! only once (in `setup`); switching version only re-points the junction.

mod arch;
mod commands;
mod env_win;
mod foojay;
mod paths;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// JDK version manager for Windows.
#[derive(Parser)]
#[command(name = "jdkenv", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download and install a JDK from foojay (.zip).
    Install {
        /// Version to install (e.g. `21`, `17.0.13`).
        version: String,
        /// Distribution to install from. Required — there is no default vendor
        /// (e.g. temurin, corretto, zulu, oracle_open_jdk, …).
        #[arg(long)]
        distribution: String,
    },
    /// Activate an installed version (re-points the `current` junction).
    Global {
        /// Installed version to activate (e.g. `21` or `21.0.5`). A bare major
        /// works when unambiguous; otherwise jdkenv asks you to disambiguate.
        version: String,
        /// Disambiguate when several installed JDKs share the major version.
        #[arg(long)]
        distribution: Option<String>,
    },
    /// List installed versions; with `--remote`, those available on foojay.
    List {
        /// With `--remote`: filter by major version across all distributions
        /// (e.g. `list --remote 21`).
        version: Option<String>,
        /// List remote versions (foojay) instead of the installed ones.
        #[arg(long)]
        remote: bool,
        /// With `--remote`: restrict to a single distribution (optional). When
        /// omitted, every distribution is listed, grouped by vendor.
        #[arg(long)]
        distribution: Option<String>,
    },
    /// Uninstall a version (deletes its folder). Warns if it is in use.
    Uninstall {
        /// Installed version to delete.
        version: String,
    },
    /// Show the active version and which folder `current` points to.
    #[command(alias = "which")]
    Current,
    /// Register PATH and JAVA_HOME (idempotent). `--system` uses HKLM; `--undo` reverts.
    Setup {
        /// Edit the SYSTEM PATH (HKLM) instead of the user one. Requires admin.
        #[arg(long)]
        system: bool,
        /// Revert the jdkenv registry (removes PATH and JAVA_HOME). Does not delete JDKs.
        #[arg(long)]
        undo: bool,
    },
    /// Diagnose the environment (junction, PATH, JAVA_HOME, conflicting java.exe).
    Doctor,
    /// [v2 — not implemented] Pin the version per folder (`.jdkenv-version`).
    Local {
        /// Version for this folder.
        version: String,
    },
}

fn main() {
    if let Err(e) = run() {
        // `{e:#}` prints the entire anyhow context chain.
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Install {
            version,
            distribution,
        } => commands::install::run(&version, &distribution),
        Command::Global {
            version,
            distribution,
        } => commands::global::run(&version, distribution.as_deref()),
        Command::List {
            version,
            remote,
            distribution,
        } => commands::list::run(remote, version.as_deref(), distribution.as_deref()),
        Command::Uninstall { version } => commands::uninstall::run(&version),
        Command::Current => commands::current::run(),
        Command::Setup { system, undo } => commands::setup::run(system, undo),
        Command::Doctor => commands::doctor::run(),
        Command::Local { version } => commands::local::run(&version),
    }
}
