//! `jdkenv install <version> [--distribution <dist>]`

use anyhow::Result;

use crate::arch::Arch;
use crate::env_win::{self, Scope};
use crate::foojay;
use crate::paths::{self, Layout};

pub fn run(version: &str, distribution: &str) -> Result<()> {
    let layout = Layout::resolve()?;
    layout.ensure_dirs()?;
    let arch = Arch::detect()?;

    println!(
        "Buscando {distribution} {version} para Windows/{} en foojay…",
        arch.foojay()
    );
    let pkg = foojay::resolve(version, distribution, arch)?;
    // Usamos el nombre de distribución y la versión canónicos que devuelve foojay.
    let dir_name = format!("{}-{}", pkg.distribution, pkg.java_version);
    let dest = layout.versions.join(&dir_name);

    if dest.is_dir() {
        println!("Ya estaba instalado: {dir_name}");
    } else {
        println!("Descargando {} …", pkg.filename);
        foojay::install_package(&pkg, &layout.versions, &dest)?;
        println!("Instalado: {}", dest.display());
    }

    // Si es el PRIMER JDK (aún no hay junction `current`), lo activamos solo.
    if layout.current_target().is_none() {
        layout.repoint_current(&dest)?;
        println!("'{dir_name}' es ahora la versión activa (global).");
        hint_setup_if_needed(&layout);
    } else {
        println!("Actívala con:  jdkenv global {}", pkg.java_version);
    }
    Ok(())
}

/// Si ni el PATH de usuario ni el de sistema contienen `current\bin`, sugiere
/// ejecutar `setup` (caso típico tras la primera instalación).
fn hint_setup_if_needed(layout: &Layout) {
    let want = layout.current_bin().to_string_lossy().to_lowercase();
    let configured = |scope| {
        env_win::read_path(scope)
            .ok()
            .flatten()
            .map(|p| p.to_lowercase().contains(&want))
            .unwrap_or(false)
    };
    if !configured(Scope::User) && !configured(Scope::System) {
        println!("\nParece que aún no ejecutaste `jdkenv setup`.");
        println!("Hazlo una vez para registrar PATH y JAVA_HOME, luego abre una terminal nueva.");
    }
}
