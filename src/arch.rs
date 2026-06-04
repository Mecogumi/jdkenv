//! Runtime architecture detection and mapping to the foojay Disco API parameter.

use anyhow::{bail, Result};

/// CPU architecture supported by jdkenv on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Aarch64,
}

impl Arch {
    /// Detects the architecture of the running binary.
    ///
    /// We use `std::env::consts::ARCH` (the process arch, not the OS arch) on
    /// purpose: we want a JDK that runs natively alongside this
    /// `jdkenv.exe`. On a Windows ARM64 running a jdkenv x64 under
    /// emulation, this reports x64 and an x64 JDK is installed — which is
    /// consistent with the rest of the PATH inherited by the shells.
    pub fn detect() -> Result<Self> {
        match std::env::consts::ARCH {
            "x86_64" => Ok(Arch::X64),
            "aarch64" => Ok(Arch::Aarch64),
            other => bail!(
                "unsupported architecture: '{other}' (jdkenv supports x86_64 and aarch64 on Windows)"
            ),
        }
    }

    /// Value of the `architecture` parameter expected by foojay.
    pub fn foojay(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Aarch64 => "aarch64",
        }
    }

    /// Suffix of the GitHub release asset for this arch: `jdkenv-<suffix>.exe`
    /// (matches the names produced by the release workflow and install.ps1).
    pub fn release_asset(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Aarch64 => "arm64",
        }
    }
}
