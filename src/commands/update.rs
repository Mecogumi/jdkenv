//! `jdkenv update` — self-update the jdkenv binary from GitHub Releases.
//!
//! Asks the GitHub API for the latest release tag, compares it with the running
//! version, and (if newer, or `--force`) downloads the matching `jdkenv-<arch>.exe`
//! and replaces the canonical binary at `%USERPROFILE%\.jdkenv\bin\jdkenv.exe`.
//!
//! Windows won't let you overwrite a running `.exe`, but it DOES let you rename
//! it — the classic self-update trick: move the old binary aside to a `.old`
//! backup, drop the new one in its place. The running process keeps executing
//! from the renamed file; the next invocation picks up the new version.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::arch::Arch;
use crate::foojay;
use crate::paths::Layout;

// Source of releases. Kept in sync with install.ps1's `$GitHubUser`.
const GITHUB_OWNER: &str = "Mecogumi";
const GITHUB_REPO: &str = "jdkenv";

/// The single field we read from the GitHub "latest release" response.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

pub fn run(force: bool) -> Result<()> {
    let arch = Arch::detect()?;
    let layout = Layout::resolve()?;
    layout.ensure_dirs()?;
    // A leftover backup from a previous self-update is no longer locked now (its
    // process exited), so clean it up.
    let _ = std::fs::remove_file(backup_path(&layout));

    let current = env!("CARGO_PKG_VERSION");
    println!("jdkenv {current} — checking GitHub for the latest release…");
    let latest_tag = latest_release_tag()?;
    let latest = latest_tag.trim_start_matches('v');

    if !is_newer(latest, current) && !force {
        println!("Already up to date (latest release is {latest}).");
        return Ok(());
    }

    if is_newer(latest, current) {
        println!("Updating {current} → {latest} …");
    } else {
        println!("Reinstalling {latest} (forced)…");
    }

    let asset = format!("jdkenv-{}.exe", arch.release_asset());
    let url = format!(
        "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/{latest_tag}/{asset}"
    );

    // Stage the download inside bin\ (same volume → the final rename is atomic and
    // never crosses drives).
    let staged = layout.bin.join(".jdkenv-update.exe");
    let _ = std::fs::remove_file(&staged);
    println!("Downloading {asset} …");
    foojay::download_to(&url, &staged).with_context(|| format!("downloading {asset}"))?;

    let target = target_path(&layout);
    replace_binary(&staged, &target)?;

    println!("Updated to {latest}: {}", target.display());
    println!("Re-run `jdkenv` (this terminal already loaded the old binary).");
    Ok(())
}

/// Canonical binary on PATH that `update` replaces, regardless of where THIS
/// process is running from.
fn target_path(layout: &Layout) -> PathBuf {
    layout.bin.join("jdkenv.exe")
}

/// Backup name the old binary is moved aside to during the swap.
fn backup_path(layout: &Layout) -> PathBuf {
    layout.bin.join("jdkenv.exe.old")
}

/// Queries `https://api.github.com/repos/<owner>/<repo>/releases/latest` and
/// returns its `tag_name` (e.g. `v0.2.1`).
fn latest_release_tag() -> Result<String> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    let resp = match foojay::http_agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => bail!(
            "no published releases at github.com/{GITHUB_OWNER}/{GITHUB_REPO} \
             (nothing to update to yet)."
        ),
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            bail!("the GitHub API returned HTTP {code}:\n{body}");
        }
        Err(e) => return Err(e).context("network failure querying the GitHub API"),
    };
    let release: Release = serde_json::from_reader(resp.into_reader())
        .context("could not parse the GitHub API response")?;
    Ok(release.tag_name)
}

/// Swaps `staged` in for `target`. Moves an existing `target` aside to its `.old`
/// backup first (a running `.exe` can be renamed but not overwritten), then
/// renames the new binary into place. The `.old` is cleaned up on the next run.
fn replace_binary(staged: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        let backup = target.with_file_name("jdkenv.exe.old");
        // Best effort: may still be locked if it's somehow in use; the rename
        // below will then surface a clear error.
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup).with_context(|| {
            format!(
                "could not move the current binary aside ({} → {}). Is another \
                 jdkenv running?",
                target.display(),
                backup.display()
            )
        })?;
    }
    std::fs::rename(staged, target)
        .with_context(|| format!("could not install the new binary to {}", target.display()))?;
    Ok(())
}

/// Is `latest` a strictly newer version than `current`? Compares numeric
/// components left to right (`0.2.1` > `0.2.0`), so an equal version is NOT newer.
fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn parse_version(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}
