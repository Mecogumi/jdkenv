# jdkenv — one-command installer for Windows.
#
#   irm https://<MY_DOMAIN>/install.ps1 | iex
#
# Downloads the right binary from GitHub Releases, places it in
# %USERPROFILE%\.jdkenv\bin\jdkenv.exe and delegates the PATH/JAVA_HOME registry
# to the binary itself (`jdkenv setup`). This way the registry logic lives in ONE
# single place (the .exe), not duplicated in PowerShell.

$ErrorActionPreference = 'Stop'

# ─── Placeholders: replace them before hosting ───────────────────────────────
$GitHubUser = 'Mecogumi'        # your GitHub user/organization
# ─────────────────────────────────────────────────────────────────────────────

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Fail($msg) { Write-Host "ERROR: $msg" -ForegroundColor Red }

# 1) Detect the process architecture.
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x64' }
    'ARM64' { 'arm64' }
    default {
        Write-Fail "unsupported architecture: '$env:PROCESSOR_ARCHITECTURE' (only x64 / arm64)."
        return
    }
}
Write-Step "Detected architecture: $arch"

# 2) Create %USERPROFILE%\.jdkenv\bin.
$binDir = Join-Path $env:USERPROFILE '.jdkenv\bin'
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

# 3) Download the correct binary from GitHub Releases.
$asset = "jdkenv-$arch.exe"
$url   = "https://github.com/$GitHubUser/jdkenv/releases/latest/download/$asset"
$dest  = Join-Path $binDir 'jdkenv.exe'

Write-Step "Downloading $url"
try {
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
}
catch {
    Write-Fail "could not download '$asset'."
    Write-Fail $_.Exception.Message
    Write-Host "Check that a release exists with the asset '$asset' at:"
    Write-Host "  https://github.com/$GitHubUser/jdkenv/releases/latest"
    return
}
Write-Step "Binary saved to $dest"

# 4) Register PATH and JAVA_HOME by running the binary itself (single source of
#    truth: the .exe knows how to prepend current\bin + its bin and set JAVA_HOME).
Write-Step "Registering the environment (jdkenv setup)…"
& $dest setup
if ($LASTEXITCODE -ne 0) {
    Write-Fail "'jdkenv setup' failed (code $LASTEXITCODE)."
    return
}

# 5) Final instructions.
Write-Host ''
Write-Host 'jdkenv installed successfully.' -ForegroundColor Green
Write-Host 'Open a NEW terminal (so it picks up the PATH) and try:' -ForegroundColor Green
Write-Host '    jdkenv install 21 --distribution corretto'
Write-Host '    jdkenv list 21 --remote'
Write-Host '    jdkenv current'
Write-Host ''
Write-Host 'If another java.exe from the system PATH wins, run:  jdkenv doctor'
