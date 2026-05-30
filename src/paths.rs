//! Layout del árbol `%USERPROFILE%\.jdkenv\` y gestión del junction `current`.
//!
//! El junction `current` es la pieza central del diseño: PATH y JAVA_HOME
//! apuntan SIEMPRE a `current` (nunca a una versión concreta), así que cambiar
//! de JDK es solo re-apuntar el junction, sin tocar el registro ni rehacer el
//! broadcast. Las shells ya abiertas toman la versión nueva al siguiente `java`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

/// Rutas canónicas del árbol `.jdkenv`.
pub struct Layout {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub versions: PathBuf,
    pub current: PathBuf,
}

impl Layout {
    /// Calcula el layout a partir de `%USERPROFILE%`.
    pub fn resolve() -> Result<Self> {
        // USERPROFILE siempre está definido en una sesión interactiva de usuario.
        let profile = std::env::var_os("USERPROFILE")
            .ok_or_else(|| anyhow!("la variable de entorno USERPROFILE no está definida"))?;
        let root = PathBuf::from(profile).join(".jdkenv");
        Ok(Layout {
            bin: root.join("bin"),
            versions: root.join("versions"),
            current: root.join("current"),
            root,
        })
    }

    /// Crea `bin\` y `versions\` (y `root` por extensión). No toca `current`.
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.bin)
            .with_context(|| format!("no se pudo crear {}", self.bin.display()))?;
        fs::create_dir_all(&self.versions)
            .with_context(|| format!("no se pudo crear {}", self.versions.display()))?;
        Ok(())
    }

    /// Ruta `current\bin`, que es lo que se antepone al PATH.
    pub fn current_bin(&self) -> PathBuf {
        self.current.join("bin")
    }

    /// Lista los JDKs instalados bajo `versions\`, ordenados por versión.
    pub fn installed(&self) -> Result<Vec<InstalledJdk>> {
        let mut out = Vec::new();
        if !self.versions.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.versions)
            .with_context(|| format!("no se pudo leer {}", self.versions.display()))?
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

    /// Busca un JDK instalado que coincida con `query` (p.ej. `21`, `21.0.5` o
    /// el nombre de carpeta completo `temurin-21.0.5`), filtrando opcionalmente
    /// por distribución. Si varias coinciden, devuelve la versión más alta.
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

    /// Re-apunta el junction `current` → `target`.
    ///
    /// `junction::delete` borra SOLO el reparse point, no el target (por eso
    /// cambiar de versión nunca borra JDKs). Como el PATH guarda la ruta literal
    /// `...\current\bin`, re-apuntar basta para que todo el sistema vea la
    /// versión nueva sin reiniciar terminales ni rehacer el broadcast.
    pub fn repoint_current(&self, target: &Path) -> Result<()> {
        if !target.is_dir() {
            bail!("el destino del junction no existe: {}", target.display());
        }
        // delete() es no-op si no existe. Si `current` quedó como carpeta/archivo
        // real (estado corrupto), lo retiramos para poder recrear el junction.
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
                "no se pudo crear el junction {} → {}",
                self.current.display(),
                target.display()
            )
        })
    }

    /// Devuelve el target actual del junction `current`, si lo hay.
    pub fn current_target(&self) -> Option<PathBuf> {
        if junction::exists(&self.current).unwrap_or(false) {
            junction::get_target(&self.current).ok()
        } else {
            None
        }
    }
}

/// Un JDK instalado: una carpeta `<dist>-<version>` bajo `versions\`.
#[derive(Debug, Clone)]
pub struct InstalledJdk {
    /// Nombre de carpeta completo, p.ej. `temurin-21.0.5`.
    pub dir_name: String,
    pub distribution: String,
    pub version: String,
    pub path: PathBuf,
}

impl InstalledJdk {
    /// Parsea `temurin-21.0.5` → (`temurin`, `21.0.5`). foojay nombra las
    /// distribuciones con guion BAJO (p.ej. `oracle_open_jdk`, `sap_machine`),
    /// nunca con guion, así que el primer `-` separa siempre distribución de
    /// versión. Ignora carpetas de staging/descarga (empiezan por `.`).
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

    /// Clave numérica para ordenar/comparar versiones (21.0.5 > 21.0.4 > 17.x).
    fn version_key(&self) -> Vec<u64> {
        version_key(&self.version)
    }
}

/// Compara dos rutas de forma robusta en Windows: canonicaliza (resolviendo el
/// prefijo `\\?\` que añade `canonicalize`) y compara sin distinguir mayúsculas.
/// Imprescindible para comparar el target del junction (que suele venir como
/// ruta verbatim) contra una carpeta de `versions\`.
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

/// Extrae los componentes numéricos de una versión para compararla. El `+`
/// separa la build-metadata (en semver NO afecta la precedencia: 21.0.11+10 ==
/// 21.0.11+11), así que la descartamos antes de comparar.
fn version_key(v: &str) -> Vec<u64> {
    v.split('+')
        .next()
        .unwrap_or(v)
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

/// ¿`query` identifica a este JDK? Acepta el nombre de carpeta completo o la
/// versión, ya sea exactos o como prefijo por componentes (`21` ⊑ `21.0.5`,
/// pero `2` NO matchea `21`).
fn version_matches(dir_name: &str, version: &str, query: &str) -> bool {
    matches_componentwise(dir_name, query) || matches_componentwise(version, query)
}

fn matches_componentwise(full: &str, query: &str) -> bool {
    if full == query {
        return true;
    }
    // '.' separa componentes; el query es prefijo si cada componente suyo iguala
    // el correspondiente de `full`. Comparar componentes evita `2` ⊑ `21`.
    let q: Vec<&str> = query.split('.').collect();
    let f: Vec<&str> = full.split('.').collect();
    q.len() <= f.len() && q.iter().zip(&f).all(|(a, b)| a == b)
}
