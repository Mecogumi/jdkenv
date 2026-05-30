//! `jdkenv current` (alias `which`) — shows the active version.

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
                println!("  (warning! the target does not exist — run `jdkenv global <v>`)");
            }
        }
        None => {
            println!("No active version.");
            println!("Install one (jdkenv install <v>) or activate it (jdkenv global <v>).");
        }
    }
    Ok(())
}
