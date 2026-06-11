//! `jdkenv set <version> [--distribution <dist>]` — switch the JDK for THIS
//! terminal session only.
//!
//! A child process can't change its parent shell's environment, so `set` prints
//! the assignments and you eval them in the current shell:
//!
//!     jdkenv set 21 | iex            # PowerShell
//!     jdkenv set 21 --cmd > "%TEMP%\jdkset.bat" & call "%TEMP%\jdkset.bat"   # cmd
//!
//! It resolves the version exactly like `global` (same matching + ambiguity),
//! but instead of re-pointing the junction it prepends the chosen version's
//! `bin` to the session `PATH` and points `JAVA_HOME` at it — gone when the
//! terminal closes.

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{bail, Result};

use crate::paths::{self, Layout};

pub fn run(version: &str, distribution: Option<&str>, cmd: bool) -> Result<()> {
    let layout = Layout::resolve()?;
    let matches = layout.find_matching(version, distribution)?;

    let jdk = match matches.as_slice() {
        [] => bail!(
            "no installed version matches '{version}'.{}",
            installed_hint(&layout)
        ),
        [only] => only,
        many => {
            // Same disambiguation contract as `global`.
            eprintln!(
                "'{version}' is ambiguous — {} installed JDKs match:",
                many.len()
            );
            for j in many {
                eprintln!("  {}", j.dir_name);
            }
            eprintln!();
            eprintln!("Specify a distribution or an exact version, e.g.:");
            eprintln!("  jdkenv set {version} --distribution <dist>");
            eprintln!(
                "  jdkenv set {}",
                many.last().map(|j| j.dir_name.as_str()).unwrap_or(version)
            );
            bail!("ambiguous version '{version}': specify a distribution or an exact version.");
        }
    };

    let java_home = jdk.path.clone();
    let bin = java_home.join("bin");

    if std::io::stdout().is_terminal() {
        let pipe = if cmd {
            format!(
                "jdkenv set {version} --cmd > \"%TEMP%\\jdkset.bat\" & call \"%TEMP%\\jdkset.bat\""
            )
        } else {
            format!("jdkenv set {version} | iex")
        };
        eprintln!(
            "{} matched, but nothing changed yet: `set` has to be eval'd by your shell.",
            jdk.dir_name
        );
        eprintln!("Run it so the current terminal applies it:");
        eprintln!("  {pipe}");
        return Ok(());
    }

    let new_path = session_path_with(&layout, &bin);
    let jh = java_home.to_string_lossy();

    // stdout: the eval-able assignments (consumed by `| iex`). Keep each on a
    // single complete line so Invoke-Expression runs them independently.
    if cmd {
        println!("set \"JAVA_HOME={jh}\"");
        println!("set \"PATH={new_path}\"");
    } else {
        println!("$env:JAVA_HOME = '{}'", ps_quote(&jh));
        println!("$env:Path = '{}'", ps_quote(&new_path));
    }

    // stderr: human feedback — kept off stdout so it doesn't pollute the eval.
    eprintln!(
        "{} is now active in THIS terminal only (use `jdkenv global` for a persistent switch).",
        jdk.dir_name
    );
    Ok(())
}

/// Builds the session `PATH`: the chosen version's `bin` first, then the current
/// session `PATH` minus any previously `set` version `bin` (so repeated `set`s
/// don't pile up). `…\current\bin` and everything else are kept untouched, so
/// the specific version wins over the junction just for this session.
fn session_path_with(layout: &Layout, new_bin: &Path) -> String {
    let versions_dir = layout.versions.to_string_lossy().to_lowercase();
    let current = std::env::var_os("PATH").unwrap_or_default();

    let mut parts: Vec<std::path::PathBuf> = vec![new_bin.to_path_buf()];
    for p in std::env::split_paths(&current) {
        let under_versions = p
            .to_string_lossy()
            .to_lowercase()
            .starts_with(&versions_dir);
        if !under_versions && !paths::same_path(&p, new_bin) {
            parts.push(p);
        }
    }
    std::env::join_paths(parts)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| new_bin.to_string_lossy().into_owned())
}

/// Escapes a value for a PowerShell single-quoted string.
fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

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
