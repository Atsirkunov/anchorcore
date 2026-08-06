# AnchorCore packaged app spec (B20). Build from repo root:
#   backend\.venv\Scripts\pyinstaller.exe --noconfirm packaging.spec
# Requires: frontend built (npm run build) and venv deps installed.

from pathlib import Path

import sqlite_vec  # noqa: E402  (spec runs inside the build venv)

ROOT = Path(SPECPATH).resolve()

datas = []
# frontend dist mounted at runtime under sys._MEIPASS/frontend_dist
dist_dir = ROOT / "frontend" / "dist"
if dist_dir.exists():
    datas.append((str(dist_dir), "frontend_dist"))
else:
    raise SystemExit("frontend/dist missing — run 'npm run build' first")

# alembic migration scripts — needed at startup by app.main._run_migrations
alembic_dir = ROOT / "backend" / "alembic"
if alembic_dir.exists():
    datas.append((str(alembic_dir), "alembic"))
datas.append((str(ROOT / "backend" / "alembic.ini"), "."))

# sqlite_vec native DLL — PyInstaller doesn't collect it automatically
_sqlite_vec_dir = Path(sqlite_vec.__file__).resolve().parent
binaries = [(str(dll), "sqlite_vec") for dll in _sqlite_vec_dir.glob("*.dll")]

a = Analysis(
    [str(ROOT / "backend" / "run_app.py")],
    pathex=[str(ROOT / "backend")],
    binaries=binaries,
    datas=datas,
    hiddenimports=[
        "app.main",
        "app.routers.settings",
        "app.routers.system",
        "app.routers.entities",
        "app.routers.qa",
        "app.routers.sources",
        "sqlite_vec",
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="AnchorCore",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,  # keep console for logs in v1; set False when UI is polished
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=None,
)
