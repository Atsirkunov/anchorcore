import { useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import type { OnboardingState } from "../types";
import { styles } from "./styles";

function platformHints(): { brew: string | null; installer: string } {
  const os = (navigator.platform || "").toLowerCase();
  if (os.includes("mac")) return { brew: "brew install ollama", installer: "https://ollama.com/download" };
  if (os.includes("win")) return { brew: null, installer: "https://ollama.com/download" };
  return { brew: "curl -fsSL https://ollama.com/install.sh | sh", installer: "https://ollama.com/download" };
}

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

/** Ollama status row; when it isn't reachable: Start (if installed) or install hints. */
export function OllamaInstall({ state, recheck }: { state: OnboardingState | null; recheck: () => Promise<unknown> }) {
  const [starting, setStarting] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const hints = platformHints();
  const reachable = !!state?.ollama.reachable;

  async function start() {
    setStarting(true);
    setMsg(null);
    try {
      const res = await api.startOllama();
      if (res.ok) {
        setMsg(res.already_running ? "Ollama already running — re-checking…" : "Ollama started — re-checking…");
        await delay(1200);
        await recheck();
      } else {
        setMsg(res.error ?? "Could not start Ollama — install it first (see below).");
      }
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }

  return (
    <>
      <div style={styles.checkRow}>
        <span style={{ ...styles.dot, background: reachable ? theme.green : theme.red }} />
        <span style={{ fontSize: 13 }}>
          Ollama {reachable ? "is running" : "is not reachable"} ({state?.ollama.base_url || "http://localhost:11434"})
        </span>
      </div>
      {state && !reachable && (
        <div style={styles.note}>
          <div>Local answers run in Ollama (free). Start it if it&apos;s installed:</div>
          <div style={{ display: "flex", gap: 8, marginTop: 6, alignItems: "center" }}>
            <button style={{ ...styles.button, ...styles.primary }} disabled={starting} onClick={start}>
              {starting ? "Starting…" : "Start Ollama"}
            </button>
            <button style={styles.button} onClick={() => recheck()}>
              Re-check
            </button>
          </div>
          {msg && <div style={{ fontSize: 12, marginTop: 6 }}>{msg}</div>}
          <div style={{ marginTop: 8 }}>Don&apos;t have it yet?</div>
          {hints.brew && <pre style={styles.code}>{hints.brew}</pre>}
          <a href={hints.installer} target="_blank" rel="noreferrer" style={{ color: theme.accentAlt }}>
            Download Ollama ↗
          </a>
        </div>
      )}
    </>
  );
}
