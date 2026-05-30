//! `jdkenv local <version>` — stub.

use anyhow::Result;

pub fn run(_version: &str) -> Result<()> {
    // TODO v2: requires shims (pyenv style) or a shell hook (jenv style)
    // to resolve a per-folder `.jdkenv-version` file. Out of scope for v1.
    println!("`jdkenv local` is not implemented yet (planned for v2).");
    println!("For now use `jdkenv global <version>` to change the active version.");
    Ok(())
}
