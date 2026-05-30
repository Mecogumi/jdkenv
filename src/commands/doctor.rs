//! `jdkenv doctor` — environment diagnostics.
//!
//! Detects the painful Windows case: another `java.exe` with priority in the
//! PATH (especially Oracle's `javapath` in the SYSTEM PATH, which beats the
//! user PATH), and validates junction + JAVA_HOME.

use std::path::Path;

use anyhow::Result;

use crate::env_win::{self, Scope};
use crate::paths::{self, Layout};

pub fn run() -> Result<()> {
    let layout = Layout::resolve()?;
    let mut problems = 0;

    println!("jdkenv doctor");
    println!("=============");
    println!("root: {}", layout.root.display());

    // 1) Junction `current`.
    match layout.current_target() {
        Some(t) if t.is_dir() => println!("[ok] current → {}", t.display()),
        Some(t) => {
            println!("[!!] 'current' points to a nonexistent folder: {}", t.display());
            println!("     Fix it with:  jdkenv global <version>");
            problems += 1;
        }
        None => {
            println!("[!!] no active version (junction 'current' missing).");
            println!("     Install one (jdkenv install <v> --distribution <dist>) or activate it (jdkenv global <v>).");
            problems += 1;
        }
    }

    // 2) PATH (user or system) contains `current\bin`. We compare entry by
    //    entry with same_path (not a .contains() over the string: it would give
    //    false positives with prefixes like `...\bin` ⊂ `...\bin_extra`).
    let current_bin = layout.current_bin();
    let user_path = env_win::read_path(Scope::User).ok().flatten().unwrap_or_default();
    let system_path = env_win::read_path(Scope::System).ok().flatten().unwrap_or_default();
    let in_user = std::env::split_paths(&user_path).any(|dir| paths::same_path(&dir, &current_bin));
    let in_system = std::env::split_paths(&system_path).any(|dir| paths::same_path(&dir, &current_bin));
    if in_user || in_system {
        let scope_label = if in_system { "system" } else { "user" };
        println!("[ok] the {scope_label} PATH contains current\\bin");
    } else {
        println!("[!!] the PATH does not contain {}", layout.current_bin().display());
        println!("     Run:  jdkenv setup");
        problems += 1;
    }

    // 3) JAVA_HOME points to `current`.
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
            println!("     Expected: {}", layout.current.display());
            println!("     Run:  jdkenv setup");
            problems += 1;
        }
        None => {
            println!("[!!] JAVA_HOME is not defined.   Run:  jdkenv setup");
            problems += 1;
        }
    }

    // 4) Does another java.exe win in this process's effective PATH?
    detect_shadowing_java(&layout, &system_path, &mut problems);

    println!();
    if problems == 0 {
        println!("Everything OK. ✔");
    } else {
        println!("{problems} problem(s). Check the suggestions above.");
    }
    Ok(())
}

/// Walks this process's effective PATH (already expanded) in order and compares
/// the FIRST `java.exe` that appears against jdkenv's.
fn detect_shadowing_java(layout: &Layout, system_path_raw: &str, problems: &mut i32) {
    let our_bin = lower_path(&layout.current_bin());
    let path = std::env::var_os("PATH").unwrap_or_default();

    let first_java = std::env::split_paths(&path).find(|dir| dir.join("java.exe").is_file());

    match first_java {
        None => println!("[ok] there is no earlier java.exe in the PATH"),
        Some(dir) => {
            let dir_l = dir.to_string_lossy().trim_end_matches('\\').to_lowercase();
            if dir_l == our_bin.trim_end_matches('\\') {
                println!("[ok] the first java.exe in the PATH is jdkenv's");
            } else {
                println!("[!!] another java.exe wins in the PATH: {}", dir.display());
                *problems += 1;
                let is_oracle_javapath =
                    dir_l.contains("oracle\\java\\javapath") || system_path_raw.to_lowercase().contains("javapath");
                if is_oracle_javapath {
                    println!("     → it's Oracle's 'javapath', usually in the SYSTEM PATH.");
                    println!("       The system PATH beats the user one, so run:  jdkenv setup --system");
                } else {
                    println!("       Prepend jdkenv with:  jdkenv setup");
                    println!("       (or  jdkenv setup --system  if that java.exe is in the system PATH)");
                }
                println!("     Note: Maven/Gradle prioritize JAVA_HOME over the PATH; with JAVA_HOME");
                println!("           set correctly many workflows already work even if the PATH order isn't perfect.");
            }
        }
    }
}

fn lower_path(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}
