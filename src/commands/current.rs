//! `jdkenv current` (alias `which`) — muestra la versión activa.

use anyhow::Result;

use crate::paths::Layout;

pub fn run() -> Result<()> {
    let layout = Layout::resolve()?;
    match layout.current_target() {
        Some(target) => {
            let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            println!("{name}");
            println!("  current → {}", target.display());
            if !target.is_dir() {
                println!("  (¡atención! el destino no existe — ejecuta `jdkenv global <v>`)");
            }
        }
        None => {
            println!("No hay versión activa.");
            println!("Instala una (jdkenv install <v>) o actívala (jdkenv global <v>).");
        }
    }
    Ok(())
}
