//! `jdkenv install <version> --distribution <dist>`

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
        "Searching {distribution} {version} for Windows/{} on foojay…",
        arch.foojay()
    );
    let pkg = foojay::resolve(version, distribution, arch)?;
    // We use the canonical distribution name and version returned by foojay.
    let dir_name = format!("{}-{}", pkg.distribution, pkg.java_version);
    let dest = layout.versions.join(&dir_name);

    if dest.is_dir() {
        println!("Already installed: {dir_name}");
    } else {
        println!("Downloading {} …", pkg.filename);
        foojay::install_package(&pkg, &layout.versions, &dest)?;
        println!("Installed: {}", dest.display());
    }

    // If it's the FIRST JDK (no `current` junction yet), we activate it automatically.
    if layout.current_target().is_none() {
        layout.repoint_current(&dest)?;
        println!("'{dir_name}' is now the active version (global).");
        hint_setup_if_needed(&layout);
    } else {
        println!("Activate it with:  jdkenv global {}", pkg.java_version);
    }
    Ok(())
}

/// If neither the user PATH nor the system PATH contains `current\bin`, suggests
/// running `setup` (typical case after the first installation).
fn hint_setup_if_needed(layout: &Layout) {
    let current_bin = layout.current_bin();
    // We split the PATH by ';' and compare each entry with same_path (instead
    // of a .contains() over the string, which would give false positives with
    // prefixes like `...\bin` ⊂ `...\bin_extra`).
    let configured = |scope| {
        env_win::read_path(scope)
            .ok()
            .flatten()
            .map(|p| std::env::split_paths(&p).any(|dir| paths::same_path(&dir, &current_bin)))
            .unwrap_or(false)
    };
    if !configured(Scope::User) && !configured(Scope::System) {
        println!("\nIt looks like you haven't run `jdkenv setup` yet.");
        println!("Do it once to register PATH and JAVA_HOME, then open a NEW terminal.");
    }
}
