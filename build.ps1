# AnchorCore packaged build (B20). Run from repo root:
#   .\build.ps1
# Produces dist/AnchorCore.exe — a single file for non-developer testers.
# Requires: Python venv deps installed (backend/.venv), Node deps installed.
#
# NOTE: no $ErrorActionPreference = "Stop" here — native tools (npm,
# pyinstaller) write logs to stderr, which PowerShell would treat as
# terminating errors. Invoke-NativeStep checks exit codes instead.
$Root = $PSScriptRoot

function Invoke-NativeStep {
    param([scriptblock]$Step, [string]$What)
    $output = & $Step 2>&1
    if ($LASTEXITCODE -ne 0) {
        $output | ForEach-Object { Write-Host "$_" }
        throw "Failed: $What"
    }
}

# 1. Frontend build (bundled into the exe)
Write-Host "==> Building frontend (npm run build)..."
Push-Location (Join-Path $Root "frontend")
try {
    Invoke-NativeStep { npm run build } "frontend build"
} finally {
    Pop-Location
}

# 2. Backend deps (install if the venv is missing)
$VenvPy = Join-Path $Root "backend\.venv\Scripts\python.exe"
if (-not (Test-Path $VenvPy)) {
    Write-Host "==> Creating backend/.venv ..."
    python -m venv (Join-Path $Root "backend\.venv")
}
if (-not (Test-Path (Join-Path $Root "backend\.venv\Scripts\pyinstaller.exe"))) {
    Write-Host "==> Installing build deps (pyinstaller)..."
    Invoke-NativeStep { & $VenvPy -m pip install pyinstaller } "pyinstaller install"
}

# 3. PyInstaller (use the exe directly — `python -m PyInstaller` writes
# progress to stderr and trips PowerShell's native-command error handling)
Write-Host "==> Building AnchorCore.exe (this takes a minute)..."
$PyInstaller = Join-Path $Root "backend\.venv\Scripts\pyinstaller.exe"
Push-Location $Root
try {
    Invoke-NativeStep { & $PyInstaller --noconfirm packaging.spec } "pyinstaller build"
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "Done: $Root\dist\AnchorCore.exe"
Write-Host "Ship this single file. First run creates ~/.anchorcore data dir,"
Write-Host "starts Ollama if present, and opens http://127.0.0.1:8000"
