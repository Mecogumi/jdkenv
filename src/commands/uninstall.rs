//! `jdkenv uninstall <version>` — borra la carpeta de la versión.

use anyhow::{anyhow, bail, Context, Result};

use crate::paths::{self, Layout};

pub fn run(version: &str) -> Result<()> {
    let layout = Layout::resolve()?;
    let jdk = layout
        .find_installed(version, None)?
        .ok_or_else(|| anyhow!("no hay ninguna versión instalada que coincida con '{version}'."))?;

    // Si es la versión activa, no la borramos: el junction `current` quedaría
    // colgando y `java`/`JAVA_HOME` apuntarían a la nada.
    let is_active = layout
        .current_target()
        .as_deref()
        .map(|t| paths::same_path(t, &jdk.path))
        .unwrap_or(false);
    if is_active {
        eprintln!("'{}' es la versión ACTIVA (current).", jdk.dir_name);
        eprintln!("Cambia a otra antes de desinstalarla:  jdkenv global <otra-version>");
        bail!("desinstalación cancelada: la versión está en uso.");
    }

    std::fs::remove_dir_all(&jdk.path)
        .with_context(|| format!("no se pudo borrar {}", jdk.path.display()))?;
    println!("Desinstalada: {}", jdk.dir_name);
    Ok(())
}
