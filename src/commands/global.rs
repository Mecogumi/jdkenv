//! `jdkenv global <version>` — re-apunta el junction `current`.

use anyhow::{anyhow, Result};

use crate::paths::Layout;

pub fn run(version: &str) -> Result<()> {
    let layout = Layout::resolve()?;
    let jdk = layout
        .find_installed(version, None)?
        .ok_or_else(|| anyhow!("no hay ninguna versión instalada que coincida con '{version}'.{}", installed_hint(&layout)))?;

    layout.repoint_current(&jdk.path)?;
    println!("Versión activa: {}", jdk.dir_name);
    println!("  current → {}", jdk.path.display());
    Ok(())
}

/// Lista las versiones instaladas para acompañar un error de "no encontrada".
fn installed_hint(layout: &Layout) -> String {
    match layout.installed() {
        Ok(v) if !v.is_empty() => {
            let lines: Vec<String> = v.iter().map(|j| format!("  {}", j.dir_name)).collect();
            format!("\nInstaladas:\n{}", lines.join("\n"))
        }
        _ => "\nNo hay JDKs instalados. Instala uno con: jdkenv install <version>".to_string(),
    }
}
