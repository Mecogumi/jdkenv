//! `jdkenv list` (installed) and `jdkenv list --remote` (foojay).

use anyhow::Result;

use crate::arch::Arch;
use crate::foojay;
use crate::paths::{self, Layout};

pub fn run(remote: bool, version: Option<&str>, distribution: Option<&str>) -> Result<()> {
    if remote {
        list_remote(version, distribution)
    } else {
        list_local()
    }
}

fn list_local() -> Result<()> {
    let layout = Layout::resolve()?;
    let installed = layout.installed()?;
    if installed.is_empty() {
        println!("No JDKs installed. Install one with: jdkenv install <version> --distribution <dist>");
        return Ok(());
    }
    let active = layout.current_target();
    println!("Installed JDKs (* = active):");
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

/// Remote listing: every distribution by default (grouped by vendor, one header
/// per vendor with its versions below), optionally narrowed to a single
/// distribution and/or a major version.
fn list_remote(version: Option<&str>, distribution: Option<&str>) -> Result<()> {
    let arch = Arch::detect()?;

    let scope = match (distribution, version) {
        (Some(d), Some(v)) => format!("{d} {v}"),
        (Some(d), None) => d.to_string(),
        (None, Some(v)) => format!("all distributions, version {v}"),
        (None, None) => "all distributions".to_string(),
    };
    println!(
        "Available on foojay for Windows/{} (.zip) — {scope}:",
        arch.foojay()
    );

    let listings = foojay::list_remote(distribution, version, arch)?;
    if listings.iter().all(|l| l.versions.is_empty()) {
        println!(
            "  (none — check the distribution name or version. Distributions use\n\
             underscores, e.g. temurin, corretto, zulu, oracle_open_jdk, sap_machine.)"
        );
        return Ok(());
    }

    for listing in listings {
        if listing.versions.is_empty() {
            continue;
        }
        println!();
        println!("{}", listing.distribution);
        for v in listing.versions {
            println!("  {v}");
        }
    }
    Ok(())
}
