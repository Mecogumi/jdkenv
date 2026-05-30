//! foojay Disco API client (`https://api.foojay.io/disco/v3.0`) and
//! download/extraction of JDK packages. We only deserialize the fields we use.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;

use crate::arch::Arch;

const DISCO_BASE: &str = "https://api.foojay.io/disco/v3.0";

/// A package returned by foojay in `result[]`.
#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub distribution: String,
    pub java_version: String,
    /// Vendor-specific version string (e.g. Corretto `21.0.5.11.1`). Used as the
    /// dedup key for the remote listing. Absent/null for some distributions.
    #[serde(default)]
    pub distribution_version: Option<String>,
    pub filename: String,
    pub links: Links,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Links {
    pub pkg_download_redirect: String,
}

#[derive(Debug, Deserialize)]
struct PackagesResponse {
    result: Vec<Package>,
}

/// ureq agent with redirect following (`pkg_download_redirect` is a
/// 302 to the distributor's CDN) and an identifiable user-agent. Uses rustls (no
/// OpenSSL) via ureq's default features.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(10)
        .user_agent(concat!("jdkenv/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Queries `/packages` with jdkenv's fixed filters (JDK, Windows, `.zip`)
/// plus `version`/`distribution`/`architecture`.
fn query_packages(
    version: Option<&str>,
    distribution: Option<&str>,
    arch: Arch,
    latest: bool,
) -> Result<Vec<Package>> {
    let mut req = agent()
        .get(&format!("{DISCO_BASE}/packages"))
        .query("package_type", "jdk")
        .query("operating_system", "windows")
        .query("architecture", arch.foojay())
        .query("archive_type", "zip");
    // Omitting `distribution` makes foojay return every distribution.
    if let Some(d) = distribution {
        req = req.query("distribution", d);
    }
    if let Some(v) = version {
        req = req.query("version", v);
    }
    if latest {
        // `available` = the latest available build per version line.
        req = req.query("latest", "available");
    }

    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            bail!("foojay responded HTTP {code} while querying packages:\n{body}");
        }
        Err(e) => return Err(e).context("network failure querying the foojay Disco API"),
    };
    let parsed: PackagesResponse = serde_json::from_reader(resp.into_reader())
        .context("could not parse foojay's JSON response")?;
    Ok(parsed.result)
}

/// One distribution's available versions in the remote listing: sorted ascending
/// and deduplicated by `distribution_version`.
pub struct RemoteListing {
    pub distribution: String,
    pub versions: Vec<String>,
}

/// Lists packages available on foojay, optionally narrowed by `distribution`
/// and/or major `version`. With `distribution = None`, the `distribution`
/// parameter is omitted from the query so foojay returns every distribution.
/// Results are grouped by distribution, deduplicated by `distribution_version`,
/// and each group is sorted ascending by version.
pub fn list_remote(
    distribution: Option<&str>,
    version: Option<&str>,
    arch: Arch,
) -> Result<Vec<RemoteListing>> {
    let pkgs = query_packages(version, distribution, arch, true)?;

    // Group by distribution (BTreeMap → vendors come out alphabetically).
    let mut groups: BTreeMap<String, Vec<Package>> = BTreeMap::new();
    for pkg in pkgs {
        groups.entry(pkg.distribution.clone()).or_default().push(pkg);
    }

    let mut out = Vec::new();
    for (distribution, mut packages) in groups {
        // Dedup by distribution_version (fall back to java_version when absent).
        let mut seen = HashSet::new();
        packages.retain(|p| {
            let key = p
                .distribution_version
                .clone()
                .unwrap_or_else(|| p.java_version.clone());
            seen.insert(key)
        });
        // Sort ascending by the canonical java version.
        packages.sort_by(|a, b| version_key(&a.java_version).cmp(&version_key(&b.java_version)));
        // Display java_version, collapsing exact duplicates (kept adjacent by the
        // sort) that distinct distribution_versions can otherwise produce.
        let mut versions: Vec<String> = Vec::new();
        for p in packages {
            if versions.last() != Some(&p.java_version) {
                versions.push(p.java_version);
            }
        }
        out.push(RemoteListing {
            distribution,
            versions,
        });
    }
    Ok(out)
}

/// Resolves the package to install for `version` + `distribution`, picking the
/// most recent build. Actionable error if the combination does not exist.
pub fn resolve(version: &str, distribution: &str, arch: Arch) -> Result<Package> {
    let pkgs = query_packages(Some(version), Some(distribution), arch, true)?;
    pkgs.into_iter()
        .max_by(|a, b| version_key(&a.java_version).cmp(&version_key(&b.java_version)))
        .ok_or_else(|| {
            anyhow!(
                "no '{distribution}' {version} build for Windows/{} (.zip) on foojay.\n\
                 Check versions with: jdkenv list --remote --distribution {distribution}",
                arch.foojay()
            )
        })
}

/// Downloads `pkg`'s `.zip` and extracts it to `dest`, leaving `bin\java.exe`
/// directly under `dest` (removes the root folder that JDK zips ship with).
///
/// Staging goes on the SAME volume as `dest` (inside `versions\`) so the final
/// `rename` is atomic and does not cross drives (TEMP could be on another).
pub fn install_package(pkg: &Package, versions_dir: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(versions_dir)
        .with_context(|| format!("could not create {}", versions_dir.display()))?;

    let stem = dest.file_name().and_then(|s| s.to_str()).unwrap_or("jdk");
    let zip_path = versions_dir.join(format!(".download-{stem}.zip"));
    let stage_dir = versions_dir.join(format!(".stage-{stem}"));
    // Clean up leftovers from previous attempts.
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&stage_dir);

    let result = (|| -> Result<()> {
        download_to(&pkg.links.pkg_download_redirect, &zip_path)
            .with_context(|| format!("downloading {}", pkg.filename))?;
        extract_zip(&zip_path, &stage_dir).context("extracting the JDK archive")?;

        let java_home = find_java_home(&stage_dir)
            .context("bin\\java.exe not found in the downloaded archive")?;

        if dest.exists() {
            // Propagate the real cleanup error; otherwise the following rename
            // would fail with a confusing message ("already exists" / "access denied")
            // that hides the cause (e.g. a java.exe in use locking the folder).
            fs::remove_dir_all(dest)
                .with_context(|| format!("could not clean up the previous folder {}", dest.display()))?;
        }
        fs::rename(&java_home, dest)
            .with_context(|| format!("moving {} → {}", java_home.display(), dest.display()))?;
        Ok(())
    })();

    // Clean up no matter what.
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&stage_dir);
    result
}

/// GET with redirect following, dumping the body to `dest` with a progress
/// bar if the server provides `Content-Length`.
fn download_to(url: &str, dest: &Path) -> Result<()> {
    let resp = match agent().get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            bail!("the download returned HTTP {code} ({})", r.get_url());
        }
        Err(e) => return Err(e).context("network failure during the download"),
    };
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let pb = if total > 0 {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::with_template(
                "  {bar:40.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})",
            )
            .expect("valid progress template"),
        );
        pb
    } else {
        ProgressBar::new_spinner()
    };

    let mut reader = resp.into_reader();
    let mut file =
        File::create(dest).with_context(|| format!("could not create {}", dest.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).context("reading the download body")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("writing {}", dest.display()))?;
        pb.inc(n as u64);
    }
    file.flush()?;
    pb.finish_and_clear();
    Ok(())
}

/// Extracts all zip entries to `dest`. `enclosed_name()` neutralizes
/// paths with `..` or absolute paths (defense against zip-slip).
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(zip_path)
        .with_context(|| format!("could not open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("invalid or corrupt .zip archive")?;
    fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            continue; // entry with a dangerous path: skip it
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile =
                File::create(&out).with_context(|| format!("writing {}", out.display()))?;
            io::copy(&mut entry, &mut outfile)?;
        }
    }
    Ok(())
}

/// Locates the folder containing `bin\java.exe` within `stage`.
/// It's almost always the zip's single root folder; we also cover the rare case
/// without a root folder.
fn find_java_home(stage: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(stage)? {
        let dir = entry?.path();
        if dir.join("bin").join("java.exe").is_file() {
            return Ok(dir);
        }
    }
    if stage.join("bin").join("java.exe").is_file() {
        return Ok(stage.to_path_buf());
    }
    bail!("unrecognized JDK structure in {}", stage.display())
}

/// Numeric components of a version, for sorting. Discards the
/// build metadata after `+` (in semver it does not affect precedence).
fn version_key(v: &str) -> Vec<u64> {
    v.split('+')
        .next()
        .unwrap_or(v)
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}
