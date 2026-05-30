//! jdkenv — gestor de versiones de JDK para Windows (nativo, sin WSL/Git Bash).
//!
//! Modelo de diseño: un directory **junction** estable `%USERPROFILE%\.jdkenv\
//! current` actúa como nivel de indirección. PATH y JAVA_HOME apuntan a `current`
//! una sola vez (en `setup`); cambiar de versión solo re-apunta el junction.

mod arch;
mod commands;
mod env_win;
mod foojay;
mod paths;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// Gestor de versiones de JDK para Windows.
#[derive(Parser)]
#[command(name = "jdkenv", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Descarga e instala un JDK desde foojay (.zip).
    Install {
        /// Versión a instalar (p.ej. `21`, `17.0.13`).
        version: String,
        /// Distribución: temurin (por defecto), corretto, zulu, …
        #[arg(long, default_value = "temurin")]
        distribution: String,
    },
    /// Activa una versión instalada (re-apunta el junction `current`).
    Global {
        /// Versión instalada a activar (p.ej. `21` o `21.0.5`).
        version: String,
    },
    /// Lista versiones instaladas; con `--remote`, las disponibles en foojay.
    List {
        /// Lista versiones remotas (foojay) en lugar de las instaladas.
        #[arg(long)]
        remote: bool,
        /// Distribución para `--remote`.
        #[arg(long, default_value = "temurin")]
        distribution: String,
    },
    /// Desinstala una versión (borra su carpeta). Avisa si está en uso.
    Uninstall {
        /// Versión instalada a borrar.
        version: String,
    },
    /// Muestra la versión activa y a qué carpeta apunta `current`.
    #[command(alias = "which")]
    Current,
    /// Registra PATH y JAVA_HOME (idempotente). `--system` usa HKLM; `--undo` revierte.
    Setup {
        /// Edita el PATH de SISTEMA (HKLM) en lugar del de usuario. Requiere admin.
        #[arg(long)]
        system: bool,
        /// Revierte el registro de jdkenv (quita PATH y JAVA_HOME). No borra JDKs.
        #[arg(long)]
        undo: bool,
    },
    /// Diagnostica el entorno (junction, PATH, JAVA_HOME, java.exe en conflicto).
    Doctor,
    /// [v2 — no implementado] Fija la versión por carpeta (`.jdkenv-version`).
    Local {
        /// Versión para esta carpeta.
        version: String,
    },
}

fn main() {
    if let Err(e) = run() {
        // `{e:#}` imprime toda la cadena de contexto de anyhow.
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
        Command::Global { version } => commands::global::run(&version),
        Command::List {
            remote,
            distribution,
        } => commands::list::run(remote, &distribution),
        Command::Uninstall { version } => commands::uninstall::run(&version),
        Command::Current => commands::current::run(),
        Command::Setup { system, undo } => commands::setup::run(system, undo),
        Command::Doctor => commands::doctor::run(),
        Command::Local { version } => commands::local::run(&version),
    }
}
