//! `jdkenv local <version>` — stub.

use anyhow::Result;

pub fn run(_version: &str) -> Result<()> {
    // TODO v2: requiere shims (estilo pyenv) o un hook de shell (estilo jenv)
    // para resolver un archivo `.jdkenv-version` por carpeta. Fuera de alcance v1.
    println!("`jdkenv local` no está implementado todavía (previsto para v2).");
    println!("Por ahora usa `jdkenv global <version>` para cambiar la versión activa.");
    Ok(())
}
