# jdkenv — instalador de un comando para Windows.
#
#   irm https://<MI_DOMINIO>/install.ps1 | iex
#
# Descarga el binario adecuado desde GitHub Releases, lo coloca en
# %USERPROFILE%\.jdkenv\bin\jdkenv.exe y delega el registro de PATH/JAVA_HOME en
# el propio binario (`jdkenv setup`). Así la lógica de registro vive en UN solo
# sitio (el .exe), no duplicada en PowerShell.

$ErrorActionPreference = 'Stop'

# ─── Placeholders: reemplázalos antes de hospedar ────────────────────────────
$GitHubUser = '<USER>'        # tu usuario/organización de GitHub
# ─────────────────────────────────────────────────────────────────────────────

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Fail($msg) { Write-Host "ERROR: $msg" -ForegroundColor Red }

# 1) Detectar la arquitectura del proceso.
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x64' }
    'ARM64' { 'arm64' }
    default {
        Write-Fail "arquitectura no soportada: '$env:PROCESSOR_ARCHITECTURE' (solo x64 / arm64)."
        return
    }
}
Write-Step "Arquitectura detectada: $arch"

# 2) Crear %USERPROFILE%\.jdkenv\bin.
$binDir = Join-Path $env:USERPROFILE '.jdkenv\bin'
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

# 3) Descargar el binario correcto desde GitHub Releases.
$asset = "jdkenv-$arch.exe"
$url   = "https://github.com/$GitHubUser/jdkenv/releases/latest/download/$asset"
$dest  = Join-Path $binDir 'jdkenv.exe'

Write-Step "Descargando $url"
try {
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
}
catch {
    Write-Fail "no se pudo descargar '$asset'."
    Write-Fail $_.Exception.Message
    Write-Host "Verifica que exista una release con el asset '$asset' en:"
    Write-Host "  https://github.com/$GitHubUser/jdkenv/releases/latest"
    return
}
Write-Step "Binario guardado en $dest"

# 4) Registrar PATH y JAVA_HOME ejecutando el propio binario (única fuente de
#    verdad: el .exe sabe anteponer current\bin + su bin y setear JAVA_HOME).
Write-Step "Registrando el entorno (jdkenv setup)…"
& $dest setup
if ($LASTEXITCODE -ne 0) {
    Write-Fail "'jdkenv setup' falló (código $LASTEXITCODE)."
    return
}

# 5) Instrucciones finales.
Write-Host ''
Write-Host 'jdkenv instalado correctamente.' -ForegroundColor Green
Write-Host 'Abre una terminal NUEVA (para que tome el PATH) y prueba:' -ForegroundColor Green
Write-Host '    jdkenv install 21'
Write-Host '    jdkenv current'
Write-Host ''
Write-Host 'Si otro java.exe del PATH de sistema te gana, ejecuta:  jdkenv doctor'
