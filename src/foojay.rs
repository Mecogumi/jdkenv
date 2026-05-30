//! Cliente de la foojay Disco API (`https://api.foojay.io/disco/v3.0`) y
//! descarga/extracción de paquetes JDK. Solo deserializamos los campos que usamos.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;

use crate::arch::Arch;

const DISCO_BASE: &str = "https://api.foojay.io/disco/v3.0";

/// Un paquete devuelto por foojay en `result[]`.
#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub distribution: String,
    pub java_version: String,
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

/// Agente ureq con seguimiento de redirects (el `pkg_download_redirect` es un
/// 302 al CDN del distribuidor) y user-agent identificable. Usa rustls (sin
/// OpenSSL) por las features por defecto de ureq.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(10)
        .user_agent(concat!("jdkenv/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Consulta `/packages` con los filtros fijos de jdkenv (JDK, Windows, `.zip`)
/// más `version`/`distribution`/`architecture`.
fn query_packages(
    version: Option<&str>,
    distribution: &str,
    arch: Arch,
    latest: bool,
) -> Result<Vec<Package>> {
    let mut req = agent()
        .get(&format!("{DISCO_BASE}/packages"))
        .query("package_type", "jdk")
        .query("operating_system", "windows")
        .query("architecture", arch.foojay())
        .query("archive_type", "zip")
        .query("distribution", distribution);
    if let Some(v) = version {
        req = req.query("version", v);
    }
    if latest {
        // `available` = el último build disponible por línea de versión.
        req = req.query("latest", "available");
    }

    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            bail!("foojay respondió HTTP {code} al consultar paquetes:\n{body}");
        }
        Err(e) => return Err(e).context("fallo de red consultando la foojay Disco API"),
    };
    let parsed: PackagesResponse = serde_json::from_reader(resp.into_reader())
        .context("no se pudo parsear la respuesta JSON de foojay")?;
    Ok(parsed.result)
}

/// Lista los paquetes disponibles para `list --remote` (último build por línea),
/// ordenados ascendentemente por versión.
pub fn list_remote(distribution: &str, arch: Arch) -> Result<Vec<Package>> {
    let mut pkgs = query_packages(None, distribution, arch, true)?;
    pkgs.sort_by(|a, b| version_key(&a.java_version).cmp(&version_key(&b.java_version)));
    Ok(pkgs)
}

/// Resuelve el paquete a instalar para `version` + `distribution`, eligiendo el
/// build más reciente. Error accionable si la combinación no existe.
pub fn resolve(version: &str, distribution: &str, arch: Arch) -> Result<Package> {
    let pkgs = query_packages(Some(version), distribution, arch, true)?;
    pkgs.into_iter()
        .max_by(|a, b| version_key(&a.java_version).cmp(&version_key(&b.java_version)))
        .ok_or_else(|| {
            anyhow!(
                "no hay build de '{distribution}' {version} para Windows/{} (.zip) en foojay.\n\
                 Revisa versiones con: jdkenv list --remote --distribution {distribution}",
                arch.foojay()
            )
        })
}

/// Descarga el `.zip` del `pkg` y lo extrae a `dest`, dejando `bin\java.exe`
/// directamente bajo `dest` (elimina la carpeta raíz que traen los zips de JDK).
///
/// El staging va en el MISMO volumen que `dest` (dentro de `versions\`) para que
/// el `rename` final sea atómico y no cruce unidades (TEMP podría estar en otra).
pub fn install_package(pkg: &Package, versions_dir: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(versions_dir)
        .with_context(|| format!("no se pudo crear {}", versions_dir.display()))?;

    let stem = dest.file_name().and_then(|s| s.to_str()).unwrap_or("jdk");
    let zip_path = versions_dir.join(format!(".download-{stem}.zip"));
    let stage_dir = versions_dir.join(format!(".stage-{stem}"));
    // Limpia restos de intentos previos.
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&stage_dir);

    let result = (|| -> Result<()> {
        download_to(&pkg.links.pkg_download_redirect, &zip_path)
            .with_context(|| format!("descargando {}", pkg.filename))?;
        extract_zip(&zip_path, &stage_dir).context("extrayendo el archivo JDK")?;

        let java_home = find_java_home(&stage_dir)
            .context("no se encontró bin\\java.exe en el archivo descargado")?;

        if dest.exists() {
            // Propagamos el error real de limpieza; si no, el rename siguiente
            // fallaría con un mensaje confuso ("ya existe" / "acceso denegado")
            // que oculta la causa (p.ej. un java.exe en uso bloqueando la carpeta).
            fs::remove_dir_all(dest)
                .with_context(|| format!("no se pudo limpiar la carpeta previa {}", dest.display()))?;
        }
        fs::rename(&java_home, dest)
            .with_context(|| format!("moviendo {} → {}", java_home.display(), dest.display()))?;
        Ok(())
    })();

    // Limpieza pase lo que pase.
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&stage_dir);
    result
}

/// GET con seguimiento de redirects, volcando el cuerpo a `dest` con barra de
/// progreso si el servidor da `Content-Length`.
fn download_to(url: &str, dest: &Path) -> Result<()> {
    let resp = match agent().get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            bail!("la descarga devolvió HTTP {code} ({})", r.get_url());
        }
        Err(e) => return Err(e).context("fallo de red durante la descarga"),
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
            .expect("plantilla de progreso válida"),
        );
        pb
    } else {
        ProgressBar::new_spinner()
    };

    let mut reader = resp.into_reader();
    let mut file =
        File::create(dest).with_context(|| format!("no se pudo crear {}", dest.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).context("leyendo el cuerpo de la descarga")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("escribiendo {}", dest.display()))?;
        pb.inc(n as u64);
    }
    file.flush()?;
    pb.finish_and_clear();
    Ok(())
}

/// Extrae todas las entradas del zip a `dest`. `enclosed_name()` neutraliza
/// rutas con `..` o absolutas (defensa frente a zip-slip).
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(zip_path)
        .with_context(|| format!("no se pudo abrir {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("archivo .zip inválido o corrupto")?;
    fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            continue; // entrada con ruta peligrosa: la saltamos
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile =
                File::create(&out).with_context(|| format!("escribiendo {}", out.display()))?;
            io::copy(&mut entry, &mut outfile)?;
        }
    }
    Ok(())
}

/// Localiza la carpeta que contiene `bin\java.exe` dentro de `stage`.
/// Casi siempre es la única carpeta raíz del zip; cubrimos también el caso raro
/// sin carpeta raíz.
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
    bail!("estructura de JDK no reconocida en {}", stage.display())
}

/// Componentes numéricos de una versión, para ordenar. Descarta la
/// build-metadata tras `+` (en semver no afecta la precedencia).
fn version_key(v: &str) -> Vec<u64> {
    v.split('+')
        .next()
        .unwrap_or(v)
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}
