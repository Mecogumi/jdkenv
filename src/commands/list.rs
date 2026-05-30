//! `jdkenv list` (instaladas) y `jdkenv list --remote` (foojay).

use anyhow::Result;

use crate::arch::Arch;
use crate::foojay;
use crate::paths::{self, Layout};

pub fn run(remote: bool, distribution: &str) -> Result<()> {
    if remote {
        list_remote(distribution)
    } else {
        list_local()
    }
}

fn list_local() -> Result<()> {
    let layout = Layout::resolve()?;
    let installed = layout.installed()?;
    if installed.is_empty() {
        println!("No hay JDKs instalados. Instala uno con: jdkenv install <version>");
        return Ok(());
    }
    let active = layout.current_target();
    println!("JDKs instalados (* = activo):");
    for jdk in installed {
        let is_active = active
            .as_deref()
            .map(|t| paths::same_path(t, &jdk.path))
            .unwrap_or(false);
        let marker = if is_active { '*' } else { ' ' };
        println!("{marker} {}", jdk.dir_name);
    }
    Ok(())
}

fn list_remote(distribution: &str) -> Result<()> {
    let arch = Arch::detect()?;
    println!(
        "Versiones de '{distribution}' disponibles para Windows/{} (.zip):",
        arch.foojay()
    );
    let pkgs = foojay::list_remote(distribution, arch)?;
    if pkgs.is_empty() {
        println!("  (ninguna — ¿distribución válida? prueba: temurin, corretto, zulu, oracle_open_jdk, …)");
        return Ok(());
    }
    for pkg in pkgs {
        println!("  {} {}", pkg.distribution, pkg.java_version);
    }
    Ok(())
}
