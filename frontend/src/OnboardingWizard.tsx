import { useEffect, useState } from "react";
import { api } from "./api";
import { theme } from "./theme";
import type { Citation, OnboardingState } from "./types";

const STEPS = ["Setup checks", "Configure model", "Connect a source", "Ask a question"];

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
  // Configure model (new step 1)
  const [ollamaBaseUrl, setOllamaBaseUrl] = useState("http://localhost:11434");
  const [answerBaseUrl, setAnswerBaseUrl] = useState("");
  const [answerApiKey, setAnswerApiKey] = useState("");
  const [answerModel, setAnswerModel] = useState("");
  const [savingConfig, setSavingConfig] = useState(false);
  const [configMsg, setConfigMsg] = useState<string | null>(null);

  useEffect(() => {
    api.onboarding().then(setState).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (step === 1) {
      api.getSettings().then((s) => {
        setOllamaBaseUrl((s as unknown as Record<string, string>).ollama_base_url || "http://localhost:11434");
        setAnswerBaseUrl((s as unknown as Record<string, string>).answer_base_url || "");
        const k = (s as unknown as Record<string, string>).answer_api_key || "";
        setAnswerApiKey(k === "***set***" ? "" : k);
        setAnswerModel((s as unknown as Record<string, string>).answer_model || "");
      }).catch(() => {});
    }
  }, [step]);

  async function saveConfig() {
    setSavingConfig(true);
    setConfigMsg(null);
    try {
      const payload: Record<string, string> = {};
      if (ollamaBaseUrl.trim()) payload.ollama_base_url = ollamaBaseUrl.trim();
      if (answerBaseUrl.trim()) payload.answer_base_url = answerBaseUrl.trim();
      if (answerApiKey.trim()) payload.answer_api_key = answerApiKey.trim();
      if (answerModel.trim()) payload.answer_model = answerModel.trim();
      await api.updateSettings(payload as unknown as Parameters<typeof api.updateSettings>[0]);
      setConfigMsg("Saved — re-checking…");
      await recheck();
      setTimeout(() => setStep(2), 600);
    } catch (e) {
      setConfigMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingConfig(false);
    }
  }

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
      setStep(3);
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
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 12 }}>
            <div>
              <h2 style={{ margin: 0, fontSize: 20 }}>Welcome to AnchorCore</h2>
              <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 2 }}>
                Connect a folder, and AnchorCore turns it into searchable company memory.
              </div>
            </div>
            <button
              onClick={onClose}
              title="Skip the wizard — you can connect a source later in Sources"
              style={{ ...styles.button, background: "transparent", border: `1px solid ${theme.border}`, color: theme.textMuted, flexShrink: 0 }}
            >
              Skip →
            </button>
          </div>
        </div>

        <div style={styles.steps}>
          {STEPS.map((label, i) => (
            <span key={label} style={{ ...styles.step, ...(i === step ? styles.stepActive : {}), ...(i < step ? styles.stepDone : {}) }}>
              {i + 1}. {label}
            </span>
          ))}
        </div>

        {error && <p style={{ color: theme.red }}>{error}</p>}

        {step === 0 && (
          <div style={styles.body}>
            <div style={styles.checkRow}>
              <span style={{ ...styles.dot, background: state?.ollama.reachable ? theme.green : theme.red }} />
              <span>
                Ollama {state?.ollama.reachable ? "is running" : "is not reachable"} ({state?.ollama.base_url || "http://localhost:11434"})
              </span>
            </div>
            {state && !state.ollama.reachable && (
              <div style={styles.note}>
                <div>Install Ollama, then start it and it will be detected here:</div>
                {hints.brew && <pre style={styles.code}>{hints.brew}</pre>}
                {hints.installer && (
                  <a href={hints.installer} target="_blank" rel="noreferrer" style={{ color: theme.accentAlt }}>
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
                <span style={{ ...styles.dot, background: theme.green }} />
                <span>Classifier + embeddings models ready</span>
              </div>
            )}
            {state?.answer_provider === "missing" && (
              <div style={styles.note}>
                No answer model configured yet — you can set one later in Settings (or point answers at Ollama). Questions still work locally.
              </div>
            )}
            <div style={styles.actions}>
              <button style={{ ...styles.button, background: "transparent", border: `1px solid ${theme.border}`, color: theme.textMuted }} onClick={onClose}>
                Skip for now
              </button>
              <button style={styles.button} onClick={recheck}>Re-check</button>
              <button style={{ ...styles.button, background: theme.accent, color: "#fff" }} onClick={() => setStep(1)}>
                Continue →
              </button>
            </div>
          </div>
        )}

        {step === 1 && (
          <div style={styles.body}>
            <div style={styles.note}>
              Configure where AnchorCore gets its models. Ollama is local and free; or point answers at any OpenAI-compatible API. You can change this anytime in Settings.
            </div>
            <div style={{ display: "grid", gap: 8 }}>
              <label style={{ fontSize: 13, color: theme.textMuted }}>Ollama base URL (local LLM)</label>
              <input style={styles.input} value={ollamaBaseUrl} onChange={(e) => setOllamaBaseUrl(e.target.value)} placeholder="http://localhost:11434" />
              <label style={{ fontSize: 13, color: theme.textMuted }}>Answer API base URL (cloud, e.g. https://api.openai.com/v1 — leave empty to use Ollama)</label>
              <input style={styles.input} value={answerBaseUrl} onChange={(e) => setAnswerBaseUrl(e.target.value)} placeholder="https://api.openai.com/v1 or empty for Ollama" />
              <label style={{ fontSize: 13, color: theme.textMuted }}>Answer API key (if cloud) — stored in OS keychain</label>
              <input style={styles.input} type="password" value={answerApiKey} onChange={(e) => setAnswerApiKey(e.target.value)} placeholder="sk-..." />
              <label style={{ fontSize: 13, color: theme.textMuted }}>Answer model</label>
              <input style={styles.input} value={answerModel} onChange={(e) => setAnswerModel(e.target.value)} placeholder="gpt-4o-mini or llama3.2:3b" />
            </div>
            {configMsg && <div style={{ fontSize: 12, color: theme.accentAlt }}>{configMsg}</div>}
            <div style={styles.actions}>
              <button style={styles.button} onClick={() => setStep(0)}>← Back</button>
              <button style={{ ...styles.button, background: "transparent", border: `1px solid ${theme.border}`, color: theme.textMuted }} onClick={() => setStep(2)}>Skip</button>
              <button style={{ ...styles.button, background: theme.accent, color: "#fff" }} disabled={savingConfig} onClick={saveConfig}>{savingConfig ? "Saving…" : "Save & continue →"}</button>
            </div>
            <div style={{ fontSize: 12, color: theme.textDim, marginTop: 6 }}>
              Ollama: <a href="https://ollama.com/download" target="_blank" rel="noreferrer" style={{ color: theme.accentAlt }}>Download</a> then `ollama pull llama3.2:3b && ollama pull nomic-embed-text` — or set a cloud key above.
            </div>
          </div>
        )}

        {step === 2 && (
          <div style={styles.body}>
            <div style={styles.note}>
              AnchorCore ingests a folder of notes/docs (Markdown, txt, PDF). You can also use the bundled sample corpus to try it.
            </div>
            {state?.sample.available && (
              <button style={{ ...styles.button, background: theme.blueBg, color: theme.accentAlt }} disabled={busy} onClick={connectSample}>
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
              <button style={{ ...styles.button, background: theme.accent, color: "#fff" }} disabled={busy} onClick={connectFolder}>
                Connect
              </button>
            </div>
            {busy && progress && (
              <div style={{ marginTop: 10, fontSize: 13, color: theme.accentAlt }}>
                {progress.total > 0 ? `Ingesting… ${progress.processed}/${progress.total}` : "Ingesting…"}
              </div>
            )}
            {!busy && (
              <div style={styles.actions}>
                <button style={styles.button} onClick={() => setStep(1)}>← Back</button>
              </div>
            )}
          </div>
        )}

        {step === 3 && (
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
              <button style={{ ...styles.button, background: theme.accent, color: "#fff" }} disabled={busy} onClick={askSample}>
                {busy ? "Asking…" : "Ask"}
              </button>
            </div>
            {answer && (
              <div style={styles.answerBox}>
                <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{answer.text}</div>
                {answer.citations.length > 0 && (
                  <div style={{ marginTop: 10, fontSize: 12, color: theme.textMuted }}>
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
              <button style={styles.button} onClick={() => setStep(2)}>← Back</button>
              <button style={{ ...styles.button, background: theme.buttonBg }} onClick={onClose}>Done — open AnchorCore</button>
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
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    borderRadius: 12,
    padding: "1.4rem 1.6rem",
    maxWidth: 640,
    width: "100%",
    boxShadow: "0 12px 40px rgba(0,0,0,0.5)",
  },
  header: { marginBottom: 14 },
  steps: { display: "flex", gap: 12, fontSize: 12, color: theme.textDim, marginBottom: 14, flexWrap: "wrap" },
  step: { padding: "0.2rem 0.6rem", borderRadius: 999, background: theme.bgElevated },
  stepActive: { background: theme.blueBg, color: theme.accentAlt },
  stepDone: { color: theme.green },
  body: { display: "grid", gap: 10 },
  checkRow: { display: "flex", alignItems: "center", gap: 8, fontSize: 14 },
  dot: { width: 10, height: 10, borderRadius: "50%", display: "inline-block", flexShrink: 0 },
  note: {
    background: theme.bgElevated,
    border: `1px solid ${theme.border}`,
    borderRadius: 8,
    padding: "0.6rem 0.8rem",
    fontSize: 13,
    color: "#d1d5db",
    lineHeight: 1.5,
  },
  code: {
    background: theme.bg,
    border: `1px solid ${theme.border}`,
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
    border: `1px solid ${theme.border}`,
    background: theme.bg,
    color: theme.text,
  },
  button: {
    padding: "0.5rem 1rem",
    borderRadius: 8,
    border: "none",
    background: theme.buttonBg,
    color: theme.text,
    cursor: "pointer",
    fontSize: 13,
  },
  actions: { display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 6 },
  answerBox: {
    background: theme.bgElevated,
    border: `1px solid ${theme.border}`,
    borderRadius: 8,
    padding: "0.7rem 0.9rem",
    fontSize: 14,
    maxHeight: 260,
    overflowY: "auto",
  },
};
