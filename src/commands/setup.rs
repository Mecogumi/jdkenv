//! `jdkenv setup [--system]` — registers PATH and JAVA_HOME (idempotent).

use anyhow::{Context, Result};

use crate::env_win::{self, Scope};
use crate::paths::{self, Layout};

/// - Without `--system`: edits the USER PATH (HKCU). No UAC, covers most cases.
/// - With `--system`: edits the SYSTEM PATH (HKLM). Since the effective PATH is
///   system-first, this is the only thing that beats a JDK already present in the
///   system PATH (e.g. Oracle's `javapath`). Requires elevation: if we don't have
///   it, we relaunch the process with UAC and the same arguments.
/// - With `--undo`: reverts the above (removes jdkenv entries from the PATH and
///   deletes `JAVA_HOME` if it points to jdkenv). Does not delete JDKs or the junction.
pub fn run(system: bool, undo: bool) -> Result<()> {
    let layout = Layout::resolve()?;

    // --undo doesn't need to create directories or copy the exe.
    if undo {
        if system && !env_win::is_elevated() {
            println!("`setup --system --undo` requires administrator; requesting elevation (UAC)…");
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
        println!("`setup --system` requires administrator; requesting elevation (UAC)…");
        let code = env_win::relaunch_elevated(&["setup".to_string(), "--system".to_string()])?;
        // The elevated process already did the work; we propagate its exit code.
        std::process::exit(code);
    }

    let scope = if system { Scope::System } else { Scope::User };
    apply(&layout, scope)
}

fn apply(layout: &Layout, scope: Scope) -> Result<()> {
    let current_bin = layout.current_bin().to_string_lossy().into_owned();
    let own_bin = layout.bin.to_string_lossy().into_owned();
    let java_home = layout.current.to_string_lossy().into_owned();

    // Priority: first `current\bin` (so the active `java` wins), then
    // our own `bin` (so `jdkenv` is available).
    let path_changed = env_win::prepend_path(scope, &[&current_bin, &own_bin])?;
    let jh_changed = env_win::set_java_home(scope, &java_home)?;

    let scope_name = match scope {
        Scope::User => "user",
        Scope::System => "system",
    };
    println!("{scope_name} environment configured:");
    println!("  PATH (prepend) {current_bin}");
    println!("  PATH (prepend) {own_bin}");
    println!("  JAVA_HOME = {java_home}");
    install_shell_function();

    if path_changed || jh_changed {
        // Notify new shells; the ones already open will see `current\bin`
        // because it's a literal path of the junction.
        env_win::broadcast_env_change();
        println!("\nDone. Open a NEW terminal for the changes to take effect.");
    } else {
        println!("\nEverything was already configured (no changes).");
    }
    println!("Try:  jdkenv install 21 --distribution temurin");
    Ok(())
}

/// Inverse of [`apply`]: removes jdkenv entries from the PATH and deletes
/// `JAVA_HOME` (only if it points to jdkenv). Does NOT touch the installed JDKs or
/// the `current` junction — that is removed by deleting the `.jdkenv` folder.
fn undo_apply(layout: &Layout, scope: Scope) -> Result<()> {
    let current_bin = layout.current_bin().to_string_lossy().into_owned();
    let own_bin = layout.bin.to_string_lossy().into_owned();
    let java_home = layout.current.to_string_lossy().into_owned();

    let path_changed = env_win::remove_from_path(scope, &[&current_bin, &own_bin])?;
    let jh = env_win::clear_java_home_if(scope, &java_home)?;
    let jh_removed = matches!(jh, env_win::JavaHomeUndo::Removed);
    uninstall_shell_function();

    let scope_name = match scope {
        Scope::User => "user",
        Scope::System => "system",
    };
    println!("Undoing the jdkenv registration in the {scope_name} environment:");
    if path_changed {
        println!("  PATH: removed jdkenv entries (current\\bin and bin)");
    } else {
        println!("  PATH: there were no jdkenv entries to remove");
    }
    match jh {
        env_win::JavaHomeUndo::Removed => println!("  JAVA_HOME: removed"),
        env_win::JavaHomeUndo::NotSet => println!("  JAVA_HOME: was not set"),
        env_win::JavaHomeUndo::KeptForeign(v) => {
            println!("  JAVA_HOME: kept (points to '{v}', not to jdkenv)")
        }
    }

    if path_changed || jh_removed {
        env_win::broadcast_env_change();
        println!("\nDone. Open a NEW terminal for the changes to take effect.");
    } else {
        println!("\nThere was nothing to undo (no changes).");
    }
    println!(
        "Note: this does NOT delete the installed JDKs or the junction. To remove\n\
         everything, delete the %USERPROFILE%\\.jdkenv folder."
    );
    Ok(())
}

/// Copies this executable to `bin\jdkenv.exe` if it's not already running from
/// there. The bootstrap (install.ps1) places it, but `setup` must be idempotent
/// and work even if the binary is launched from another folder.
fn install_self(layout: &Layout) -> Result<()> {
    let exe = std::env::current_exe().context("could not get the executable path")?;
    let dest = layout.bin.join("jdkenv.exe");
    if paths::same_path(&exe, &dest) {
        return Ok(());
    }
    std::fs::create_dir_all(&layout.bin).ok();
    std::fs::copy(&exe, &dest)
        .with_context(|| format!("could not copy the executable to {}", dest.display()))?;
    Ok(())
}

fn install_shell_function() {
    let script = "$ErrorActionPreference = 'Stop'\n\
$line = 'Invoke-Expression (& \"$env:USERPROFILE\\.jdkenv\\bin\\jdkenv.exe\" init | Out-String)'\n\
$p = $PROFILE.CurrentUserAllHosts\n\
$dir = Split-Path -Parent $p\n\
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }\n\
$has = (Test-Path $p) -and (Select-String -Path $p -SimpleMatch 'jdkenv.exe\" init' -Quiet)\n\
if (-not $has) { Add-Content -Path $p -Value $line; Write-Output ('added -> ' + $p) } else { Write-Output ('already present -> ' + $p) }\n";
    run_in_profiles(script, "could not register the `set` wrapper in the PowerShell profile");
}

fn uninstall_shell_function() {
    let script = "$ErrorActionPreference = 'Stop'\n\
$p = $PROFILE.CurrentUserAllHosts\n\
if (Test-Path $p) { (Get-Content $p) | Where-Object { $_ -notmatch 'jdkenv\\.exe\" init' } | Set-Content -Path $p; Write-Output ('cleaned -> ' + $p) } else { Write-Output 'no profile' }\n";
    run_in_profiles(script, "could not remove the `set` wrapper from the PowerShell profile");
}

fn run_in_profiles(script: &str, on_fail: &str) {
    let mut tmp = std::env::temp_dir();
    tmp.push("jdkenv-profile.ps1");
    if std::fs::write(&tmp, script).is_err() {
        println!("  {on_fail}");
        return;
    }
    let mut any = false;
    for host in ["pwsh", "powershell"] {
        let out = std::process::Command::new(host)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&tmp)
            .output();
        if let Ok(out) = out
            && out.status.success()
        {
            let msg = String::from_utf8_lossy(&out.stdout);
            let msg = msg.trim();
            if !msg.is_empty() {
                println!("  shell wrapper ({host}): {msg}");
                any = true;
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    if !any {
        println!("  {on_fail}");
    }
}
