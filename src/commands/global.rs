//! `jdkenv global <version> [--distribution <dist>]` — re-points the `current` junction.

use anyhow::{bail, Result};

use crate::paths::Layout;

pub fn run(version: &str, distribution: Option<&str>) -> Result<()> {
    let layout = Layout::resolve()?;
    let matches = layout.find_matching(version, distribution)?;

    match matches.as_slice() {
        // Nothing matches: clear error plus the list of what is installed.
        [] => {
            let by_dist = distribution
                .map(|d| format!(" for distribution '{d}'"))
                .unwrap_or_default();
            bail!(
                "no installed version matches '{version}'{by_dist}.{}",
                installed_hint(&layout)
            );
        }
        // Exactly one match: activate it.
        [jdk] => {
            layout.repoint_current(&jdk.path)?;
            println!("Active version: {}", jdk.dir_name);
            println!("  current → {}", jdk.path.display());
            Ok(())
        }
        // Several match (e.g. two distributions or two builds share the major):
        // refuse and ask the user to disambiguate.
        many => {
            eprintln!(
                "'{version}' is ambiguous — {} installed JDKs match:",
                many.len()
            );
            for jdk in many {
                eprintln!("  {}", jdk.dir_name);
            }
            eprintln!();
            eprintln!("Specify a distribution or an exact version, e.g.:");
            eprintln!("  jdkenv global {version} --distribution <dist>");
            eprintln!(
                "  jdkenv global {}",
                many.last().map(|j| j.dir_name.as_str()).unwrap_or(version)
            );
            bail!("ambiguous version '{version}': specify a distribution or an exact version.");
        }
    }
}

/// Lists installed versions to accompany a "not found" error.
fn installed_hint(layout: &Layout) -> String {
    match layout.installed() {
        Ok(v) if !v.is_empty() => {
            let lines: Vec<String> = v.iter().map(|j| format!("  {}", j.dir_name)).collect();
            format!("\nInstalled:\n{}", lines.join("\n"))
        }
        _ => "\nNo JDKs installed. Install one with: jdkenv install <version> --distribution <dist>"
            .to_string(),
    }
}
