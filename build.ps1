# AnchorCore packaged build. Run from repo root:
#   .\build.ps1
# Produces dist/AnchorCore-windows.zip — anchorcore.exe + anchorcore-mcp.exe
# (Rust release build, frontend/dist embedded) for non-developer testers.
# Same layout as the CI asset (release.yml build-rust-windows).
# Requires: Rust stable (cargo on PATH), Node deps installed.
# (Legacy Python PyInstaller path retired — backend/ is deprecated.)
#
# NOTE: no $ErrorActionPreference = "Stop" here — native tools (npm,
# cargo) write logs to stderr, which PowerShell would treat as
# terminating errors. Invoke-NativeStep checks exit codes instead.
$Root = $PSScriptRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo not found on PATH — install Rust stable (https://rustup.rs) first."
}

function Invoke-NativeStep {
    param([scriptblock]$Step, [string]$What)
    $output = & $Step 2>&1
    if ($LASTEXITCODE -ne 0) {
        $output | ForEach-Object { Write-Host "$_" }
        throw "Failed: $What"
    }
}

# 0. Kill a running app so the exe files aren't locked (WinError 5 otherwise).
Get-Process -Name "anchorcore" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

# 1. Frontend build (embedded into the Rust binary via include_dir!)
Write-Host "==> Building frontend (npm run build)..."
Push-Location (Join-Path $Root "frontend")
try {
    Invoke-NativeStep { npm run build } "frontend build"
} finally {
    Pop-Location
}

# 2. Rust release build (statically-linked, UI embedded)
Write-Host "==> Building Rust anchorcore (cargo build --release)..."
Push-Location (Join-Path $Root "rust")
try {
    Invoke-NativeStep { cargo build --release } "cargo build --release"
} finally {
    Pop-Location
}

# 3. Zip the binaries for distribution
Write-Host "==> Zipping anchorcore.exe + anchorcore-mcp.exe..."
$DistDir = Join-Path $Root "dist"
New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
$ZipPath = Join-Path $DistDir "AnchorCore-windows.zip"
Remove-Item $ZipPath -ErrorAction SilentlyContinue
Copy-Item (Join-Path $Root "rust\target\release\anchorcore.exe") (Join-Path $DistDir "anchorcore.exe") -Force
Copy-Item (Join-Path $Root "rust\target\release\anchorcore-mcp.exe") (Join-Path $DistDir "anchorcore-mcp.exe") -Force
Push-Location $DistDir
try {
    Compress-Archive -Path anchorcore.exe,anchorcore-mcp.exe -DestinationPath AnchorCore-windows.zip -Force
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "Done: $Root\dist\AnchorCore-windows.zip"
Write-Host "Unzip it, then run anchorcore.exe. First run creates ~/.anchorcore"
Write-Host "data dir, starts Ollama if present, and opens http://127.0.0.1:8000"
