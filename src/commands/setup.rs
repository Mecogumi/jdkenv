//! `jdkenv setup [--system]` — registra PATH y JAVA_HOME (idempotente).

use anyhow::{Context, Result};

use crate::env_win::{self, Scope};
use crate::paths::{self, Layout};

/// - Sin `--system`: edita el PATH de USUARIO (HKCU). Sin UAC, cubre la mayoría.
/// - Con `--system`: edita el PATH de SISTEMA (HKLM). Como el PATH efectivo es
///   sistema-primero, esto es lo único que vence a un JDK ya presente en el PATH
///   de sistema (p.ej. el `javapath` de Oracle). Requiere elevación: si no la
///   tenemos, relanzamos el proceso con UAC y los mismos argumentos.
/// - Con `--undo`: revierte lo anterior (quita del PATH las entradas de jdkenv y
///   borra `JAVA_HOME` si apunta a jdkenv). No borra JDKs ni el junction.
pub fn run(system: bool, undo: bool) -> Result<()> {
    let layout = Layout::resolve()?;

    // --undo no necesita crear directorios ni copiar el exe.
    if undo {
        if system && !env_win::is_elevated() {
            println!("`setup --system --undo` requiere administrador; solicitando elevación (UAC)…");
            let code = env_win::relaunch_elevated(&[
                "setup".to_string(),
                "--system".to_string(),
                "--undo".to_string(),
            ])?;
            std::process::exit(code);
        }
        let scope = if system { Scope::System } else { Scope::User };
        return undo_apply(&layout, scope);
    }

    layout.ensure_dirs()?;
    install_self(&layout)?;

    if system && !env_win::is_elevated() {
        println!("`setup --system` requiere administrador; solicitando elevación (UAC)…");
        let code = env_win::relaunch_elevated(&["setup".to_string(), "--system".to_string()])?;
        // El proceso elevado ya hizo el trabajo; propagamos su código de salida.
        std::process::exit(code);
    }

    let scope = if system { Scope::System } else { Scope::User };
    apply(&layout, scope)
}

fn apply(layout: &Layout, scope: Scope) -> Result<()> {
    let current_bin = layout.current_bin().to_string_lossy().into_owned();
    let own_bin = layout.bin.to_string_lossy().into_owned();
    let java_home = layout.current.to_string_lossy().into_owned();

    // Prioridad: primero `current\bin` (para que gane el `java` activo), luego
    // nuestro propio `bin` (para que `jdkenv` esté disponible).
    let path_changed = env_win::prepend_path(scope, &[&current_bin, &own_bin])?;
    let jh_changed = env_win::set_java_home(scope, &java_home)?;

    let scope_name = match scope {
        Scope::User => "usuario",
        Scope::System => "sistema",
    };
    println!("Entorno de {scope_name} configurado:");
    println!("  PATH (anteponer) {current_bin}");
    println!("  PATH (anteponer) {own_bin}");
    println!("  JAVA_HOME = {java_home}");

    if path_changed || jh_changed {
        // Notifica a las shells nuevas; las ya abiertas verán `current\bin` por
        // ser una ruta literal del junction.
        env_win::broadcast_env_change();
        println!("\nListo. Abre una terminal NUEVA para que tome los cambios.");
    } else {
        println!("\nYa estaba todo configurado (sin cambios).");
    }
    println!("Prueba:  jdkenv install 21");
    Ok(())
}

/// Inverso de [`apply`]: quita del PATH las entradas de jdkenv y borra
/// `JAVA_HOME` (solo si apunta a jdkenv). NO toca los JDKs instalados ni el
/// junction `current` — eso se elimina borrando la carpeta `.jdkenv`.
fn undo_apply(layout: &Layout, scope: Scope) -> Result<()> {
    let current_bin = layout.current_bin().to_string_lossy().into_owned();
    let own_bin = layout.bin.to_string_lossy().into_owned();
    let java_home = layout.current.to_string_lossy().into_owned();

    let path_changed = env_win::remove_from_path(scope, &[&current_bin, &own_bin])?;
    let jh = env_win::clear_java_home_if(scope, &java_home)?;
    let jh_removed = matches!(jh, env_win::JavaHomeUndo::Removed);

    let scope_name = match scope {
        Scope::User => "usuario",
        Scope::System => "sistema",
    };
    println!("Deshaciendo el registro de jdkenv en el entorno de {scope_name}:");
    if path_changed {
        println!("  PATH: eliminadas las entradas de jdkenv (current\\bin y bin)");
    } else {
        println!("  PATH: no había entradas de jdkenv que quitar");
    }
    match jh {
        env_win::JavaHomeUndo::Removed => println!("  JAVA_HOME: eliminado"),
        env_win::JavaHomeUndo::NotSet => println!("  JAVA_HOME: no estaba definido"),
        env_win::JavaHomeUndo::KeptForeign(v) => {
            println!("  JAVA_HOME: conservado (apunta a '{v}', no a jdkenv)")
        }
    }

    if path_changed || jh_removed {
        env_win::broadcast_env_change();
        println!("\nListo. Abre una terminal NUEVA para que tome los cambios.");
    } else {
        println!("\nNo había nada que deshacer (sin cambios).");
    }
    println!(
        "Nota: esto NO borra los JDKs instalados ni el junction. Para eliminarlo\n\
         todo, borra la carpeta %USERPROFILE%\\.jdkenv."
    );
    Ok(())
}

/// Copia este ejecutable a `bin\jdkenv.exe` si no se está corriendo ya desde
/// allí. El bootstrap (install.ps1) lo coloca, pero `setup` debe ser idempotente
/// y funcionar aunque se lance el binario desde otra carpeta.
fn install_self(layout: &Layout) -> Result<()> {
    let exe = std::env::current_exe().context("no se pudo obtener la ruta del ejecutable")?;
    let dest = layout.bin.join("jdkenv.exe");
    if paths::same_path(&exe, &dest) {
        return Ok(());
    }
    std::fs::create_dir_all(&layout.bin).ok();
    std::fs::copy(&exe, &dest)
        .with_context(|| format!("no se pudo copiar el ejecutable a {}", dest.display()))?;
    Ok(())
}
