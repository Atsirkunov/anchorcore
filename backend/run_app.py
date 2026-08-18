"""Packaged app entry point: boots the FastAPI app in-process with uvicorn.

Used by the PyInstaller build (B20). The launcher sets ANCHOR_DATA_DIR and
auto-starts the local Ollama server (models are pulled at first use via the
Settings tab / sync flow).

The build is `console=False` (no terminal window), so in windowed mode
PyInstaller leaves sys.stdout/sys.stderr unset — redirect them to devnull
before importing anything that prints or logs, or the app crashes on startup.
"""

import os
import subprocess
import sys
import threading
import time
import urllib.request
import webbrowser
from pathlib import Path


def _guard_windowed_stdio() -> None:
    """Windowed (console=False) builds have no stdout/stderr — point them at
    devnull so print()/logging don't crash. Real logs still go to the file
    handler set up in app.main (ANCHOR_DATA_DIR/anchorcore.log)."""
    if getattr(sys, "frozen", False):
        for name in ("stdout", "stderr"):
            if getattr(sys, name, None) is None:
                setattr(sys, name, open(os.devnull, "w", encoding="utf-8"))


def _ollama_up() -> bool:
    try:
        with urllib.request.urlopen("http://127.0.0.1:11434/api/tags", timeout=2):
            return True
    except Exception:  # noqa: BLE001
        return False


def _start_ollama() -> None:
    """Best-effort: launch the local Ollama server if it isn't running."""
    if _ollama_up():
        return
    candidates = [
        os.environ.get("OLLAMA_BIN"),
        r"%LOCALAPPDATA%\Programs\Ollama\ollama.exe",
        "/usr/local/bin/ollama",
        "/opt/homebrew/bin/ollama",
    ]
    for raw in candidates:
        if not raw:
            continue
        path = Path(os.path.expandvars(raw))
        if path.exists():
            try:
                if sys.platform == "win32":
                    subprocess.Popen(
                        [str(path), "serve"],
                        creationflags=subprocess.CREATE_NO_WINDOW,
                    )
                else:
                    subprocess.Popen([str(path), "serve"], start_new_session=True)
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    if _ollama_up():
                        return
                    time.sleep(0.5)
            except Exception:  # noqa: BLE001
                pass
            return


def _browser_launcher(url: str) -> None:
    # poll until the server is actually listening (migrations can take a few
    # seconds on first boot) — avoids Safari "can't connect" race from fixed sleep
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(f"{url}/health", timeout=1):
                break
        except Exception:
            pass
        try:
            with urllib.request.urlopen(url, timeout=1):
                break
        except Exception:
            pass
        time.sleep(0.4)
    try:
        webbrowser.open(url)
    except Exception:  # noqa: BLE001
        pass


def main() -> None:
    _guard_windowed_stdio()
    if getattr(sys, "frozen", False):
        os.environ.setdefault("ANCHOR_DATA_DIR", str(Path.home() / ".anchorcore"))

    _start_ollama()

    import uvicorn

    from app.main import app  # noqa: F401  (imports side effects: routes, mount)

    port = int(os.environ.get("ANCHOR_PORT", "8000"))
    url = f"http://127.0.0.1:{port}"
    if os.environ.get("ANCHOR_OPEN_BROWSER", "1") == "1":
        threading.Thread(target=_browser_launcher, args=(url,), daemon=True).start()
    print(f"AnchorCore starting at {url} (Ctrl+C to stop)")
    uvicorn.run(app, host="127.0.0.1", port=port)


if __name__ == "__main__":
    main()
