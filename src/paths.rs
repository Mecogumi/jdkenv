//! Layout of the `%USERPROFILE%\.jdkenv\` tree and management of the `current` junction.
//!
//! The `current` junction is the centerpiece of the design: PATH and JAVA_HOME
//! ALWAYS point to `current` (never to a specific version), so switching
//! JDKs is just re-pointing the junction, without touching the registry or redoing the
//! broadcast. Shells already open pick up the new version at the next `java`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

/// Canonical paths of the `.jdkenv` tree.
pub struct Layout {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub versions: PathBuf,
    pub current: PathBuf,
}

impl Layout {
    /// Computes the layout from `%USERPROFILE%`.
    pub fn resolve() -> Result<Self> {
        // USERPROFILE is always defined in an interactive user session.
        let profile = std::env::var_os("USERPROFILE")
            .ok_or_else(|| anyhow!("the USERPROFILE environment variable is not defined"))?;
        let root = PathBuf::from(profile).join(".jdkenv");
        Ok(Layout {
            bin: root.join("bin"),
            versions: root.join("versions"),
            current: root.join("current"),
            root,
        })
    }

    /// Creates `bin\` and `versions\` (and `root` by extension). Does not touch `current`.
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.bin)
            .with_context(|| format!("could not create {}", self.bin.display()))?;
        fs::create_dir_all(&self.versions)
            .with_context(|| format!("could not create {}", self.versions.display()))?;
        Ok(())
    }

    /// `current\bin` path, which is what gets prepended to PATH.
    pub fn current_bin(&self) -> PathBuf {
        self.current.join("bin")
    }

    /// Lists the JDKs installed under `versions\`, sorted by version.
    pub fn installed(&self) -> Result<Vec<InstalledJdk>> {
        let mut out = Vec::new();
        if !self.versions.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.versions)
            .with_context(|| format!("could not read {}", self.versions.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(jdk) = InstalledJdk::from_dir(entry.path())
            {
                out.push(jdk);
            }
        }
        out.sort_by(|a, b| {
            a.distribution
                .cmp(&b.distribution)
                .then_with(|| a.version_key().cmp(&b.version_key()))
        });
        Ok(out)
    }

    /// Searches for an installed JDK matching `query` (e.g. `21`, `21.0.5` or
    /// the full folder name `temurin-21.0.5`), optionally filtering
    /// by distribution. If several match, returns the highest version.
    pub fn find_installed(
        &self,
        query: &str,
        distribution: Option<&str>,
    ) -> Result<Option<InstalledJdk>> {
        let best = self
            .installed()?
            .into_iter()
            .filter(|j| distribution.is_none_or(|d| j.distribution == d))
            .filter(|j| version_matches(&j.dir_name, &j.version, query))
            .max_by(|a, b| a.version_key().cmp(&b.version_key()));
        Ok(best)
    }

    /// Returns ALL installed JDKs matching `query` (and optional `distribution`),
    /// sorted ascending by version. `global` uses this to detect ambiguity — e.g.
    /// two distributions (or two builds) sharing the same major version.
    pub fn find_matching(
        &self,
        query: &str,
        distribution: Option<&str>,
    ) -> Result<Vec<InstalledJdk>> {
        let mut matches: Vec<InstalledJdk> = self
            .installed()?
            .into_iter()
            .filter(|j| distribution.is_none_or(|d| j.distribution == d))
            .filter(|j| version_matches(&j.dir_name, &j.version, query))
            .collect();
        matches.sort_by_key(|a| a.version_key());
        Ok(matches)
    }

    /// Re-points the `current` junction → `target`.
    ///
    /// `junction::delete` removes ONLY the reparse point, not the target (that's why
    /// switching versions never deletes JDKs). Since PATH stores the literal path
    /// `...\current\bin`, re-pointing is enough for the whole system to see the
    /// new version without restarting terminals or redoing the broadcast.
    pub fn repoint_current(&self, target: &Path) -> Result<()> {
        if !target.is_dir() {
            bail!("the junction target does not exist: {}", target.display());
        }
        // delete() is a no-op if it doesn't exist. If `current` ended up as a real
        // folder/file (corrupt state), we remove it to be able to recreate the junction.
        let _ = junction::delete(&self.current);
        if self.current.exists() {
            if self.current.is_dir() {
                let _ = fs::remove_dir_all(&self.current);
            } else {
                let _ = fs::remove_file(&self.current);
            }
        }
        junction::create(target, &self.current).with_context(|| {
            format!(
                "could not create the junction {} → {}",
                self.current.display(),
                target.display()
            )
        })
    }

    /// Returns the current target of the `current` junction, if any.
    pub fn current_target(&self) -> Option<PathBuf> {
        if junction::exists(&self.current).unwrap_or(false) {
            junction::get_target(&self.current).ok()
        } else {
            None
        }
    }
}

/// An installed JDK: a `<dist>-<version>` folder under `versions\`.
#[derive(Debug, Clone)]
pub struct InstalledJdk {
    /// Full folder name, e.g. `temurin-21.0.5`.
    pub dir_name: String,
    pub distribution: String,
    pub version: String,
    pub path: PathBuf,
}

impl InstalledJdk {
    /// Parses `temurin-21.0.5` → (`temurin`, `21.0.5`). foojay names
    /// distributions with UNDERSCORES (e.g. `oracle_open_jdk`, `sap_machine`),
    /// never with a hyphen, so the first `-` always separates distribution from
    /// version. Ignores staging/download folders (they start with `.`).
    fn from_dir(path: PathBuf) -> Option<Self> {
        let dir_name = path.file_name()?.to_str()?.to_string();
        if dir_name.starts_with('.') {
            return None;
        }
        let (distribution, version) = dir_name.split_once('-')?;
        if distribution.is_empty() || version.is_empty() {
            return None;
        }
        Some(InstalledJdk {
            distribution: distribution.to_string(),
            version: version.to_string(),
            path,
            dir_name,
        })
    }

    /// Numeric key for sorting/comparing versions (21.0.5 > 21.0.4 > 17.x).
    fn version_key(&self) -> Vec<u64> {
        version_key(&self.version)
    }
}

/// Compares two paths robustly on Windows: canonicalizes (resolving the
/// `\\?\` prefix that `canonicalize` adds) and compares case-insensitively.
/// Essential for comparing the junction target (which usually comes as a
/// verbatim path) against a `versions\` folder.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Extracts the numeric components of a version to compare it. The `+`
/// separates the build metadata (in semver it does NOT affect precedence: 21.0.11+10 ==
/// 21.0.11+11), so we discard it before comparing.
fn version_key(v: &str) -> Vec<u64> {
    v.split('+')
        .next()
        .unwrap_or(v)
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

/// Does `query` identify this JDK? Accepts the full folder name or the
/// version, either exact or as a component-wise prefix (`21` ⊑ `21.0.5`,
/// but `2` does NOT match `21`).
fn version_matches(dir_name: &str, version: &str, query: &str) -> bool {
    matches_componentwise(dir_name, query) || matches_componentwise(version, query)
}

fn matches_componentwise(full: &str, query: &str) -> bool {
    if full == query {
        return true;
    }
    // '.' and '+' separate components; the query is a prefix if each of its
    // components equals the corresponding one in `full`. Splitting on '+' lets
    // `21.0.11` match the stored `21.0.11+10`. Comparing components avoids `2` ⊑ `21`.
    let q: Vec<&str> = query.split(['.', '+']).collect();
    let f: Vec<&str> = full.split(['.', '+']).collect();
    q.len() <= f.len() && q.iter().zip(&f).all(|(a, b)| a == b)
}
