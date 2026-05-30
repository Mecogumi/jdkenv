//! `jdkenv global <version>` — re-points the `current` junction.

use anyhow::{anyhow, Result};

use crate::paths::Layout;

pub fn run(version: &str) -> Result<()> {
    let layout = Layout::resolve()?;
    let jdk = layout
        .find_installed(version, None)?
        .ok_or_else(|| anyhow!("no installed version matches '{version}'.{}", installed_hint(&layout)))?;

    layout.repoint_current(&jdk.path)?;
    println!("Active version: {}", jdk.dir_name);
    println!("  current → {}", jdk.path.display());
    Ok(())
}

/// Lists the installed versions to accompany a "not found" error.
fn installed_hint(layout: &Layout) -> String {
    match layout.installed() {
        Ok(v) if !v.is_empty() => {
            let lines: Vec<String> = v.iter().map(|j| format!("  {}", j.dir_name)).collect();
            format!("\nInstalled:\n{}", lines.join("\n"))
        }
        _ => "\nNo JDKs installed. Install one with: jdkenv install <version>".to_string(),
    }
}
