//! `jdkenv doctor` — diagnóstico del entorno.
//!
//! Detecta el caso doloroso de Windows: otro `java.exe` con prioridad en el
//! PATH (sobre todo el `javapath` de Oracle en el PATH de SISTEMA, que vence al
//! PATH de usuario), y valida junction + JAVA_HOME.

use std::path::Path;

use anyhow::Result;

use crate::env_win::{self, Scope};
use crate::paths::Layout;

pub fn run() -> Result<()> {
    let layout = Layout::resolve()?;
    let mut problems = 0;

    println!("jdkenv doctor");
    println!("=============");
    println!("raíz: {}", layout.root.display());

    // 1) Junction `current`.
    match layout.current_target() {
        Some(t) if t.is_dir() => println!("[ok] current → {}", t.display()),
        Some(t) => {
            println!("[!!] 'current' apunta a una carpeta inexistente: {}", t.display());
            println!("     Arréglalo con:  jdkenv global <version>");
            problems += 1;
        }
        None => {
            println!("[!!] no hay versión activa (junction 'current' ausente).");
            println!("     Instala una (jdkenv install <v>) o actívala (jdkenv global <v>).");
            problems += 1;
        }
    }

    // 2) PATH (usuario o sistema) contiene `current\bin`.
    let want_bin = lower_path(&layout.current_bin());
    let user_path = env_win::read_path(Scope::User).ok().flatten().unwrap_or_default();
    let system_path = env_win::read_path(Scope::System).ok().flatten().unwrap_or_default();
    let in_user = user_path.to_lowercase().contains(&want_bin);
    let in_system = system_path.to_lowercase().contains(&want_bin);
    if in_user || in_system {
        let dónde = if in_system { "sistema" } else { "usuario" };
        println!("[ok] el PATH de {dónde} contiene current\\bin");
    } else {
        println!("[!!] el PATH no contiene {}", layout.current_bin().display());
        println!("     Ejecuta:  jdkenv setup");
        problems += 1;
    }

    // 3) JAVA_HOME apunta a `current`.
    let want_home = lower_path(&layout.current);
    let java_home = env_win::read_java_home(Scope::User)
        .ok()
        .flatten()
        .or_else(|| env_win::read_java_home(Scope::System).ok().flatten());
    match java_home {
        Some(v) if v.trim_end_matches('\\').to_lowercase() == want_home => {
            println!("[ok] JAVA_HOME = {v}");
        }
        Some(v) => {
            println!("[!!] JAVA_HOME = {v}");
            println!("     Se esperaba: {}", layout.current.display());
            println!("     Ejecuta:  jdkenv setup");
            problems += 1;
        }
        None => {
            println!("[!!] JAVA_HOME no está definido.   Ejecuta:  jdkenv setup");
            problems += 1;
        }
    }

    // 4) ¿Otro java.exe gana en el PATH efectivo de este proceso?
    detect_shadowing_java(&layout, &system_path, &mut problems);

    println!();
    if problems == 0 {
        println!("Todo correcto. ✔");
    } else {
        println!("{problems} problema(s). Revisa las sugerencias de arriba.");
    }
    Ok(())
}

/// Recorre el PATH efectivo (ya expandido) de este proceso en orden y compara el
/// PRIMER `java.exe` que aparece contra el de jdkenv.
fn detect_shadowing_java(layout: &Layout, system_path_raw: &str, problems: &mut i32) {
    let our_bin = lower_path(&layout.current_bin());
    let path = std::env::var_os("PATH").unwrap_or_default();

    let first_java = std::env::split_paths(&path).find(|dir| dir.join("java.exe").is_file());

    match first_java {
        None => println!("[ok] no hay ningún java.exe previo en el PATH"),
        Some(dir) => {
            let dir_l = dir.to_string_lossy().trim_end_matches('\\').to_lowercase();
            if dir_l == our_bin.trim_end_matches('\\') {
                println!("[ok] el primer java.exe del PATH es el de jdkenv");
            } else {
                println!("[!!] otro java.exe gana en el PATH: {}", dir.display());
                *problems += 1;
                let is_oracle_javapath =
                    dir_l.contains("oracle\\java\\javapath") || system_path_raw.to_lowercase().contains("javapath");
                if is_oracle_javapath {
                    println!("     → es el 'javapath' de Oracle, normalmente en el PATH de SISTEMA.");
                    println!("       El PATH de sistema vence al de usuario, así que ejecuta:  jdkenv setup --system");
                } else {
                    println!("       Antepón jdkenv con:  jdkenv setup");
                    println!("       (o  jdkenv setup --system  si ese java.exe está en el PATH de sistema)");
                }
                println!("     Nota: Maven/Gradle priorizan JAVA_HOME sobre el PATH; con JAVA_HOME bien");
                println!("           seteado muchos flujos ya funcionan aunque el orden del PATH no sea perfecto.");
            }
        }
    }
}

fn lower_path(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}
