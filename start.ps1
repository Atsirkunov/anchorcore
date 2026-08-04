# AnchorCore single run entry point (Windows).
#   .\start.ps1          boots the backend (serves the built UI at http://localhost:8000)
#   .\start.ps1 -Dev     also starts the Vite dev server in a new window (http://localhost:5173)
#   .\start.ps1 -NoOllama   skips auto-starting the local Ollama server
#
# - Kills orphaned processes still holding the port
# - Starts Ollama if it isn't running (skippable)
# - Creates the venv + .env on first run
# - Applies Alembic migrations (stamps legacy DBs, then upgrades)
# - The app prints its effective config banner at boot
#
# NOTE: no $ErrorActionPreference = "Stop" here — native tools (python/uvicorn)
# write logs to stderr, which PowerShell would treat as terminating errors.
param([switch]$Dev, [switch]$NoOllama)

# Runs a native command quietly; on failure prints its output and throws.
# (Without this, python's stderr log lines render as scary red error records.)
function Invoke-NativeStep {
    param([scriptblock]$Step, [string]$What)
    $output = & $Step 2>&1
    if ($LASTEXITCODE -ne 0) {
        $output | ForEach-Object { Write-Host "$_" }
        throw "Failed: $What"
    }
}

function Test-OllamaUp {
    # 127.0.0.1, not localhost: on some systems localhost resolves to ::1
    # first and PowerShell won't fall back to IPv4 while Ollama binds IPv4 only.
    try {
        return (Invoke-WebRequest -Uri "http://127.0.0.1:11434/api/tags" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200
    } catch {
        return $false
    }
}

$Root = $PSScriptRoot
$Backend = Join-Path $Root "backend"
$VenvDir = Join-Path $Backend ".venv"
$VenvPy = Join-Path $VenvDir "Scripts\python.exe"
$Port = 8000

# 1. Kill orphans on the port (stale processes with old code or locked SQLite).
$orphans = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
foreach ($o in $orphans) {
    Write-Host "Killing orphan on port $Port (PID $($o.OwningProcess))..."
    Stop-Process -Id $o.OwningProcess -Force -ErrorAction SilentlyContinue
}

# 2. Environment: venv + .env on first run.
if (-not (Test-Path $VenvPy)) {
    Write-Host "Creating backend/.venv ..."
    python -m venv $VenvDir
    if ($LASTEXITCODE -ne 0) { throw "Failed to create venv" }
    Invoke-NativeStep { & $VenvPy -m pip install --upgrade pip } "pip upgrade"
    Invoke-NativeStep { & $VenvPy -m pip install -r (Join-Path $Backend "requirements.txt") } "pip install backend requirements"
}
if (-not (Test-Path (Join-Path $Backend ".env"))) {
    Copy-Item (Join-Path $Backend ".env.example") (Join-Path $Backend ".env")
    Write-Host "Created backend/.env from .env.example - review it (set ANCHOR_ANSWER_API_KEY or point ANCHOR_ANSWER_BASE_URL at Ollama)."
}

# 3. Ollama: start the local model server if it isn't already running.
if (-not $NoOllama -and -not (Test-OllamaUp)) {
    Write-Host "Ollama is not running - starting it..."
    $ollama = Get-Command ollama -ErrorAction SilentlyContinue
    if ($ollama) {
        Start-Process -FilePath $ollama.Source -ArgumentList "serve" -WindowStyle Hidden
    } elseif (Test-Path (Join-Path $env:LOCALAPPDATA "Programs\Ollama\ollama.exe")) {
        Start-Process -FilePath (Join-Path $env:LOCALAPPDATA "Programs\Ollama\ollama.exe") -ArgumentList "serve" -WindowStyle Hidden
    } else {
        Write-Host "WARNING: ollama not found on PATH - install from https://ollama.com or add it to PATH."
    }
    $deadline = (Get-Date).AddSeconds(15)
    while (-not (Test-OllamaUp) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
}
if (Test-OllamaUp) {
    Write-Host "Ollama: up (http://localhost:11434)"
} else {
    Write-Host "WARNING: Ollama still not reachable - classification will fall back to rules and answers will be limited."
}

# 3b. Pull missing models (configured in backend/.env; downloads only what's absent).
if (-not $NoOllama -and (Test-OllamaUp)) {
    $ollamaCmd = Get-Command ollama -ErrorAction SilentlyContinue
    if ($ollamaCmd) {
        $configured = & $VenvPy -c "from app.config import settings; print(settings.classifier_model); print(settings.embed_model)" 2>$null
        $have = & $ollamaCmd.Source list 2>$null | Select-Object -Skip 1 | ForEach-Object { (($_ -split "\s+")[0] -split ":")[0] }
        foreach ($model in $configured) {
            $short = ($model -split ":")[0]
            if ($short -notin $have) {
                Write-Host "Pulling Ollama model $model (first run only)..."
                & $ollamaCmd.Source pull $model
                if ($LASTEXITCODE -ne 0) {
                    Write-Host "WARNING: failed to pull $model - run 'ollama pull $model' manually."
                }
            }
        }
    }
}

# 4. Migrations (stamps legacy DBs, then upgrades; quiet when up to date).
Push-Location $Backend
try {
    Invoke-NativeStep { & $VenvPy -c "from app.main import _run_migrations; _run_migrations()" } "Alembic migrations"
} finally {
    Pop-Location
}

# 5. Boot.
Write-Host "Starting AnchorCore backend on http://localhost:$Port (Ctrl+C to stop)..."
if ($Dev) {
    Start-Process powershell -ArgumentList "-NoExit", "-Command", "Set-Location '$Root\frontend'; npm run dev"
}
Push-Location $Backend
try {
    & $VenvPy -m uvicorn app.main:app --host 127.0.0.1 --port $Port
} finally {
    Pop-Location
}
