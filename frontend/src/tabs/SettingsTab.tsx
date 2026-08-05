import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { AppSettings } from "../types";

const PROVIDERS = [
  { id: "ollama", label: "Ollama (local, offline-first)" },
  { id: "openai", label: "OpenAI-compatible cloud" },
  { id: "custom", label: "Custom OpenAI-compatible" },
];

export function SettingsTab() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [form, setForm] = useState<AppSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{ provider: string; ok: boolean; message: string } | null>(null);
  const [testBusy, setTestBusy] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setForm({ ...s });
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

  const provider = () => {
    const base = form?.answer_base_url ?? "";
    if (base.startsWith("http://localhost") || base.startsWith("http://127.0.0.1")) return "ollama";
    if (base.includes("openai.com")) return "openai";
    return "custom";
  };

  const isOllama = provider() === "ollama";
  const apiKeySet = (settings?.answer_api_key ?? "") === "***set***";

  async function save() {
    if (!form) return;
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      const payload: Partial<AppSettings> = { ...form };
      if (!payload.answer_api_key) delete payload.answer_api_key;
      const updated = await api.updateSettings(payload);
      setSettings(updated);
      setForm({ ...updated });
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function testConnection(providerId: "ollama" | "answer") {
    setTestBusy(providerId);
    setTestResult(null);
    setError(null);
    try {
      const r = await api.testConnection(providerId);
      setTestResult({ provider: providerId, ok: r.ok, message: r.message });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTestBusy(null);
    }
  }

  function applyPreset(providerId: string) {
    if (!form) return;
    const next = { ...form };
    if (providerId === "ollama") {
      next.ollama_base_url = "http://localhost:11434";
      next.classifier_model = "llama3.2:3b";
      next.embed_model = "nomic-embed-text";
      next.answer_base_url = "http://localhost:11434/v1";
      next.answer_model = "llama3.2:3b";
    } else if (providerId === "openai") {
      next.answer_base_url = "https://api.openai.com/v1";
      next.answer_model = "gpt-4o-mini";
    }
    setForm(next);
  }

  if (!form) {
    return (
      <div>
        <h2>Model settings</h2>
        {error ? (
          <p style={{ color: "#f87171" }}>Failed to load settings: {error}</p>
        ) : (
          <div>Loading settings…</div>
        )}
      </div>
    );
  }

  const set = (key: keyof AppSettings) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setForm({ ...form, [key]: e.target.value });

  return (
    <div>
      <h2>Model settings</h2>
      <div style={{ display: "grid", gap: 8, maxWidth: 560 }}>
        <div>
          <label style={styles.label}>Provider preset</label>
          <select value={provider()} onChange={(e) => applyPreset(e.target.value)} style={styles.input}>
            {PROVIDERS.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Ollama (classification + embeddings)</div>
          <label style={styles.label}>Base URL</label>
          <input style={styles.input} value={form.ollama_base_url} onChange={set("ollama_base_url")} placeholder="http://localhost:11434" />
          <div style={{ display: "flex", gap: 8 }}>
            <div style={{ flex: 1 }}>
              <label style={styles.label}>Classifier model</label>
              <input style={styles.input} value={form.classifier_model} onChange={set("classifier_model")} placeholder="llama3.2:3b" />
            </div>
            <div style={{ flex: 1 }}>
              <label style={styles.label}>Embed model</label>
              <input style={styles.input} value={form.embed_model} onChange={set("embed_model")} placeholder="nomic-embed-text" />
            </div>
          </div>
          <button style={styles.button} disabled={testBusy === "ollama"} onClick={() => testConnection("ollama")}>
            {testBusy === "ollama" ? "Testing…" : "Test Ollama"}
          </button>
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Answer model</div>
          {!isOllama && (
            <>
              <label style={styles.label}>API key</label>
              <input
                style={styles.input}
                type="password"
                value={form.answer_api_key}
                onChange={set("answer_api_key")}
                placeholder={apiKeySet ? "•••••••• (stored — type to replace)" : "sk-…"}
              />
            </>
          )}
          <label style={styles.label}>Base URL</label>
          <input style={styles.input} value={form.answer_base_url} onChange={set("answer_base_url")} placeholder="https://api.openai.com/v1" />
          <label style={styles.label}>Model</label>
          <input style={styles.input} value={form.answer_model} onChange={set("answer_model")} placeholder="gpt-4o-mini" />
          <label style={styles.label}>Reasoning effort (for reasoning-capable models)</label>
          <select
            style={styles.input}
            value={form.answer_reasoning_effort || "none"}
            onChange={(e) => setForm({ ...form, answer_reasoning_effort: e.target.value })}
          >
            <option value="none">none — fast, no reasoning (default)</option>
            <option value="low">low</option>
            <option value="medium">medium</option>
            <option value="high">high</option>
          </select>
          <div style={{ fontSize: 12, color: "#6b7280", marginTop: 4 }}>
            Cloud (o-series, gpt-5): sends <code>reasoning_effort</code>. Local Ollama: enables thinking on reasoning models (deepseek-r1, qwen3, llama3.3-thinking).
          </div>
          <button style={styles.button} disabled={testBusy === "answer"} onClick={() => testConnection("answer")}>
            {testBusy === "answer" ? "Testing…" : "Test answer model"}
          </button>
        </div>

        {testResult && (
          <div
            style={{
              padding: "0.5rem 0.75rem",
              borderRadius: 6,
              background: testResult.ok ? "#1e3a2a" : "#3a1d1d",
              color: testResult.ok ? "#86efac" : "#fca5a5",
              fontSize: 13,
            }}
          >
            {testResult.provider}: {testResult.message}
          </div>
        )}

        {error && <p style={{ color: "#f87171", margin: 0 }}>{error}</p>}
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button style={{ ...styles.button, background: "#6366f1", color: "#fff" }} disabled={busy} onClick={save}>
            {busy ? "Saving…" : "Save"}
          </button>
          {saved && <span style={{ color: "#4ade80", fontSize: 13 }}>Saved — applied without restart</span>}
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  label: { display: "block", fontSize: 12, color: "#9ca3af", marginBottom: 4, marginTop: 8 },
  input: { width: "100%", padding: "0.5rem 0.75rem", borderRadius: 8, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb", boxSizing: "border-box" },
  button: { marginTop: 10, padding: "0.5rem 1rem", borderRadius: 8, border: "none", background: "#2b3240", color: "#e6e8eb", cursor: "pointer", fontSize: 13 },
  section: { background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.9rem 1rem" },
  sectionTitle: { fontWeight: 600, fontSize: 14, marginBottom: 4 },
};
