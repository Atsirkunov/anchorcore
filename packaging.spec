# AnchorCore packaged app spec (B20). Build from repo root:
#   backend\.venv\Scripts\pyinstaller.exe --noconfirm packaging.spec
# Requires: frontend built (npm run build) and venv deps installed.
#
# onedir + console=False → GUI-subsystem app: no terminal window. On Windows
# this produces dist/AnchorCore/ (zip it); on macOS a windowed AnchorCore.app
# bundle that Finder launches without opening Terminal (see run_app.py for the
# windowed-mode stdio guard). Onedir avoids the onefile/.app conflict PyInstaller
# rejects from v7.0.

import sys
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

# sqlite_vec native library — PyInstaller doesn't collect it automatically.
# Windows ships .dll, macOS ships .dylib.
_sqlite_vec_dir = Path(sqlite_vec.__file__).resolve().parent
_sqlite_vec_lib = list(_sqlite_vec_dir.glob("*.dll")) + list(_sqlite_vec_dir.glob("*.dylib"))
if not _sqlite_vec_lib:
    raise SystemExit("sqlite_vec native library not found")
binaries = [(str(lib), "sqlite_vec") for lib in _sqlite_vec_lib]

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

# onedir: EXE is just the bootloader; binaries/datas ship next to it in the
# COLLECT folder (_internal on Windows / Contents/Frameworks on macOS).
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="AnchorCore",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    console=False,  # GUI subsystem — no terminal window (launcher redirects stdio)
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=None,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name="AnchorCore",
)

# macOS: wrap the onedir folder in a proper .app bundle so Finder launches it
# without opening Terminal. Skipped on Windows (BUNDLE is macOS-only).
if sys.platform == "darwin":
    app = BUNDLE(
        coll,
        name="AnchorCore.app",
        icon=None,
        bundle_identifier="com.anchorcore.app",
    )
