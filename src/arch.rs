//! Detección de arquitectura en runtime y mapeo al parámetro de la foojay Disco API.

use anyhow::{bail, Result};

/// Arquitectura de CPU soportada por jdkenv en Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Aarch64,
}

impl Arch {
    /// Detecta la arquitectura del binario en ejecución.
    ///
    /// Usamos `std::env::consts::ARCH` (la arch del proceso, no del SO) a
    /// propósito: queremos un JDK que corra nativamente junto a este
    /// `jdkenv.exe`. En un Windows ARM64 ejecutando un jdkenv x64 bajo
    /// emulación, esto reporta x64 y se instala un JDK x64 — que es lo
    /// coherente con el resto del PATH heredado por las shells.
    pub fn detect() -> Result<Self> {
        match std::env::consts::ARCH {
            "x86_64" => Ok(Arch::X64),
            "aarch64" => Ok(Arch::Aarch64),
            other => bail!(
                "arquitectura no soportada: '{other}' (jdkenv soporta x86_64 y aarch64 en Windows)"
            ),
        }
    }

    /// Valor del parámetro `architecture` que espera foojay.
    pub fn foojay(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Aarch64 => "aarch64",
        }
    }
}
