import { useEffect, useState } from "react";
import { api } from "./api";
import type { Citation, OnboardingState } from "./types";

const STEPS = ["Setup checks", "Connect a source", "Ask a question"];

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function platformHints(): { brew: string | null; installer: string | null } {
  const os = (navigator.platform || "").toLowerCase();
  if (os.includes("mac")) return { brew: "brew install ollama", installer: "https://ollama.com/download" };
  if (os.includes("win")) return { brew: null, installer: "https://ollama.com/download" };
  return { brew: "curl -fsSL https://ollama.com/install.sh | sh", installer: "https://ollama.com/download" };
}

export function OnboardingWizard({ onClose }: { onClose: () => void }) {
  const [state, setState] = useState<OnboardingState | null>(null);
  const [step, setStep] = useState(0);
  const [folderPath, setFolderPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ total: number; processed: number } | null>(null);
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<{ text: string; citations: Citation[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const hints = platformHints();

  useEffect(() => {
    api.onboarding().then(setState).catch((e) => setError(String(e)));
  }, []);

  async function recheck() {
    setError(null);
    setState(await api.onboarding());
  }

  async function runSync(path: string, name: string) {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      const source = await api.createSource({ connector: "folder", name, config: { path } });
      const job = await api.syncSource(source.id);
      for (;;) {
        await delay(900);
        const j = await api.getJob(job.id);
        setProgress({ total: j.total, processed: j.processed });
        if (j.status !== "running") {
          if (j.status === "failed") throw new Error(j.error || "sync failed");
          break;
        }
      }
      setStep(2);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function connectSample() {
    if (state?.sample.path) await runSync(state.sample.path, "Sample company memory");
  }

  async function connectFolder() {
    if (folderPath.trim()) await runSync(folderPath.trim(), "My folder");
  }

  async function askSample() {
    if (!question.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.ask(question);
      setAnswer({ text: res.answer, citations: res.citations });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={styles.backdrop}>
      <div style={styles.card}>
        <div style={styles.header}>
          <h2 style={{ margin: 0, fontSize: 20 }}>Welcome to AnchorCore</h2>
          <div style={{ fontSize: 13, color: "#9ca3af", marginTop: 2 }}>
            Connect a folder, and AnchorCore turns it into searchable company memory.
          </div>
        </div>

        <div style={styles.steps}>
          {STEPS.map((label, i) => (
            <span key={label} style={{ ...styles.step, ...(i === step ? styles.stepActive : {}), ...(i < step ? styles.stepDone : {}) }}>
              {i + 1}. {label}
            </span>
          ))}
        </div>

        {error && <p style={{ color: "#f87171" }}>{error}</p>}

        {step === 0 && (
          <div style={styles.body}>
            <div style={styles.checkRow}>
              <span style={{ ...styles.dot, background: state?.ollama.reachable ? "#4ade80" : "#f87171" }} />
              <span>
                Ollama {state?.ollama.reachable ? "is running" : "is not reachable"} ({state?.ollama.base_url || "http://localhost:11434"})
              </span>
            </div>
            {state && !state.ollama.reachable && (
              <div style={styles.note}>
                <div>Install Ollama, then start it and it will be detected here:</div>
                {hints.brew && <pre style={styles.code}>{hints.brew}</pre>}
                {hints.installer && (
                  <a href={hints.installer} target="_blank" rel="noreferrer" style={{ color: "#7dd3fc" }}>
                    Download Ollama ↗
                  </a>
                )}
              </div>
            )}
            {state?.ollama.reachable && state.ollama.missing_models.length > 0 && (
              <div style={styles.note}>
                <div>Models are missing — pull them:</div>
                <pre style={styles.code}>ollama pull {state.ollama.missing_models.join(" && ollama pull ")}</pre>
              </div>
            )}
            {state?.ollama.reachable && state.ollama.missing_models.length === 0 && (
              <div style={{ ...styles.checkRow, marginTop: 8 }}>
                <span style={{ ...styles.dot, background: "#4ade80" }} />
                <span>Classifier + embeddings models ready</span>
              </div>
            )}
            {state?.answer_provider === "missing" && (
              <div style={styles.note}>
                No answer model configured yet — you can set one later in Settings (or point answers at Ollama). Questions still work locally.
              </div>
            )}
            <div style={styles.actions}>
              <button style={styles.button} onClick={recheck}>Re-check</button>
              <button style={{ ...styles.button, background: "#6366f1", color: "#fff" }} onClick={() => setStep(1)}>
                Continue →
              </button>
            </div>
          </div>
        )}

        {step === 1 && (
          <div style={styles.body}>
            <div style={styles.note}>
              AnchorCore ingests a folder of notes/docs (Markdown, txt, PDF). You can also use the bundled sample corpus to try it.
            </div>
            {state?.sample.available && (
              <button style={{ ...styles.button, background: "#1e3a5f", color: "#7dd3fc" }} disabled={busy} onClick={connectSample}>
                {busy ? "Syncing sample…" : "Try the sample corpus"}
              </button>
            )}
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <input
                style={styles.input}
                placeholder="Or enter a folder path, e.g. /Users/me/Documents/notes"
                value={folderPath}
                onChange={(e) => setFolderPath(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && connectFolder()}
              />
              <button style={{ ...styles.button, background: "#6366f1", color: "#fff" }} disabled={busy} onClick={connectFolder}>
                Connect
              </button>
            </div>
            {busy && progress && (
              <div style={{ marginTop: 10, fontSize: 13, color: "#7dd3fc" }}>
                {progress.total > 0 ? `Ingesting… ${progress.processed}/${progress.total}` : "Ingesting…"}
              </div>
            )}
            {!busy && (
              <div style={styles.actions}>
                <button style={styles.button} onClick={() => setStep(0)}>← Back</button>
              </div>
            )}
          </div>
        )}

        {step === 2 && (
          <div style={styles.body}>
            <div style={styles.note}>Ask a question about what you just connected — the answer cites its sources.</div>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                style={styles.input}
                placeholder="e.g. What was decided about billing, and why?"
                value={question}
                onChange={(e) => setQuestion(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && askSample()}
              />
              <button style={{ ...styles.button, background: "#6366f1", color: "#fff" }} disabled={busy} onClick={askSample}>
                {busy ? "Asking…" : "Ask"}
              </button>
            </div>
            {answer && (
              <div style={styles.answerBox}>
                <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{answer.text}</div>
                {answer.citations.length > 0 && (
                  <div style={{ marginTop: 10, fontSize: 12, color: "#9ca3af" }}>
                    Sources:
                    {answer.citations.map((c, i) => (
                      <div key={i} style={{ marginTop: 4 }}>
                        [{c.kind}] {c.summary} — {c.source_ref}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
            <div style={styles.actions}>
              <button style={styles.button} onClick={() => setStep(1)}>← Back</button>
              <button style={{ ...styles.button, background: "#2b3240" }} onClick={onClose}>Done — open AnchorCore</button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  backdrop: {
    position: "fixed",
    inset: 0,
    background: "rgba(7, 9, 12, 0.82)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 50,
    padding: "1rem",
  },
  card: {
    background: "#171a21",
    border: "1px solid #2d333b",
    borderRadius: 12,
    padding: "1.4rem 1.6rem",
    maxWidth: 640,
    width: "100%",
    boxShadow: "0 12px 40px rgba(0,0,0,0.5)",
  },
  header: { marginBottom: 14 },
  steps: { display: "flex", gap: 12, fontSize: 12, color: "#6b7280", marginBottom: 14, flexWrap: "wrap" },
  step: { padding: "0.2rem 0.6rem", borderRadius: 999, background: "#14171d" },
  stepActive: { background: "#1e3a5f", color: "#7dd3fc" },
  stepDone: { color: "#4ade80" },
  body: { display: "grid", gap: 10 },
  checkRow: { display: "flex", alignItems: "center", gap: 8, fontSize: 14 },
  dot: { width: 10, height: 10, borderRadius: "50%", display: "inline-block", flexShrink: 0 },
  note: {
    background: "#14171d",
    border: "1px solid #2d333b",
    borderRadius: 8,
    padding: "0.6rem 0.8rem",
    fontSize: 13,
    color: "#d1d5db",
    lineHeight: 1.5,
  },
  code: {
    background: "#0f1115",
    border: "1px solid #2d333b",
    borderRadius: 6,
    padding: "0.5rem 0.7rem",
    fontSize: 12,
    overflowX: "auto",
    margin: "0.4rem 0",
  },
  input: {
    flex: 1,
    padding: "0.55rem 0.75rem",
    borderRadius: 8,
    border: "1px solid #2d333b",
    background: "#0f1115",
    color: "#e6e8eb",
  },
  button: {
    padding: "0.5rem 1rem",
    borderRadius: 8,
    border: "none",
    background: "#2b3240",
    color: "#e6e8eb",
    cursor: "pointer",
    fontSize: 13,
  },
  actions: { display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 6 },
  answerBox: {
    background: "#14171d",
    border: "1px solid #2d333b",
    borderRadius: 8,
    padding: "0.7rem 0.9rem",
    fontSize: 14,
    maxHeight: 260,
    overflowY: "auto",
  },
};
