//! `jdkenv uninstall <version>` — deletes the version's folder.

use anyhow::{anyhow, bail, Context, Result};

use crate::paths::{self, Layout};

pub fn run(version: &str) -> Result<()> {
    let layout = Layout::resolve()?;
    let jdk = layout
        .find_installed(version, None)?
        .ok_or_else(|| anyhow!("no installed version matches '{version}'."))?;

    // If it's the active version, don't delete it: the `current` junction would
    // be left dangling and `java`/`JAVA_HOME` would point to nothing.
    let is_active = layout
        .current_target()
        .as_deref()
        .map(|t| paths::same_path(t, &jdk.path))
        .unwrap_or(false);
    if is_active {
        eprintln!("'{}' is the ACTIVE version (current).", jdk.dir_name);
        eprintln!("Switch to another one before uninstalling it:  jdkenv global <other-version>");
        bail!("uninstall was cancelled: the version is in use.");
    }

    std::fs::remove_dir_all(&jdk.path)
        .with_context(|| format!("could not delete {}", jdk.path.display()))?;
    println!("Uninstalled: {}", jdk.dir_name);
    Ok(())
}
