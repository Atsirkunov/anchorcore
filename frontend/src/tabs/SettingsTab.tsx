import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { stripPlaceholders } from "../placeholders";
import { theme } from "../theme";
import type { AppSettings } from "../types";

type ProviderId = "local" | "cloud";

function isLocalUrl(url: string): boolean {
  const u = (url || "").trim();
  return u === "" || u.startsWith("http://localhost") || u.startsWith("http://127.0.0.1");
}

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

  const keySet = (key: string) => (settings?.[key as keyof AppSettings] ?? "") === "***set***";

  // each provider block is self-contained: provider select + key + base url + model
  const classifierProvider: ProviderId = isLocalUrl(form?.classifier_base_url ?? "") ? "local" : "cloud";
  const embedderProvider: ProviderId = isLocalUrl(form?.embed_base_url ?? "") ? "local" : "cloud";
  const answerProvider: ProviderId = isLocalUrl(form?.answer_base_url ?? "") ? "local" : "cloud";

  function setClassifierProvider(p: ProviderId) {
    if (!form) return;
    setForm({ ...form, classifier_base_url: p === "local" ? "" : "https://api.openai.com/v1" });
  }
  function setEmbedderProvider(p: ProviderId) {
    if (!form) return;
    setForm({ ...form, embed_base_url: p === "local" ? "" : "https://api.openai.com/v1" });
  }
  function setAnswerProvider(p: ProviderId) {
    if (!form) return;
    setForm({
      ...form,
      answer_base_url: p === "local" ? "http://localhost:11434/v1" : "https://api.openai.com/v1",
      ...(p === "local" ? { answer_model: "llama3.2:3b" } : { answer_model: "gpt-4o-mini" }),
    });
  }

  async function save() {
    if (!form) return;
    setBusy(true);
    setSaved(false);
    setError(null);
    try {
      const payload: Partial<AppSettings> = stripPlaceholders({ ...form });
      // API keys already masked (or blank) are dropped so they never overwrite
      // the stored secret with the literal placeholder.
      for (const k of ["answer_api_key", "classifier_api_key", "embed_api_key"] as const) {
        if (!payload[k]) delete payload[k];
      }
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

  async function testConnection(providerId: "ollama" | "classifier" | "embedder" | "answer") {
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

  const set = (key: keyof AppSettings) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setForm({ ...form!, [key]: e.target.value });

  const providerSelect = (
    value: ProviderId,
    onChange: (p: ProviderId) => void,
    localLabel: string,
    cloudLabel: string
  ) => (
    <select style={styles.input} value={value} onChange={(e) => onChange(e.target.value as ProviderId)}>
      <option value="local">{localLabel}</option>
      <option value="cloud">{cloudLabel}</option>
    </select>
  );

  const keyInput = (key: "answer_api_key" | "classifier_api_key" | "embed_api_key", provider: ProviderId, label: string) => (
    <>
      <label style={styles.label}>{label}</label>
      <input
        style={styles.input}
        type="password"
        value={form![key]}
        onChange={set(key)}
        placeholder={keySet(key) ? "•••••••• (stored — type to replace)" : provider === "local" ? "not needed for local" : "sk-…"}
      />
    </>
  );

  if (!form) {
    return (
      <div>
        <h2>Model settings</h2>
        {error ? (
          <p style={{ color: theme.red }}>Failed to load settings: {error}</p>
        ) : (
          <div>Loading settings…</div>
        )}
      </div>
    );
  }

  return (
    <div>
      <h2>Model settings</h2>
      <div style={{ display: "grid", gap: 8, maxWidth: 560 }}>
        <div style={styles.section}>
          <div style={styles.sectionTitle}>Classification (extracts entities)</div>
          {providerSelect(classifierProvider, setClassifierProvider, "Local Ollama (free, private)", "Cloud OpenAI-compatible")}
          {keyInput("classifier_api_key", classifierProvider, "API key")}
          <label style={styles.label}>Base URL</label>
          <input
            style={styles.input}
            value={form.classifier_base_url || (classifierProvider === "local" ? form.ollama_base_url : "")}
            onChange={set("classifier_base_url")}
            placeholder={classifierProvider === "local" ? "http://localhost:11434 (uses Ollama)" : "https://api.openai.com/v1"}
          />
          <label style={styles.label}>Model</label>
          <input style={styles.input} value={form.classifier_model} onChange={set("classifier_model")} placeholder="llama3.2:3b / gpt-4o-mini" />
          {classifierProvider === "cloud" && (
            <div style={{ fontSize: 12, color: theme.amber, marginTop: 6 }}>
              Cost note: classification calls the model once per document window (a 274-page doc ≈ 160 calls). Cloud classification is token-heavy — reclassify a big source with a cloud model only when needed.
            </div>
          )}
          <button style={styles.button} disabled={testBusy === "classifier"} onClick={() => testConnection("classifier")}>
            {testBusy === "classifier" ? "Testing…" : "Test classifier"}
          </button>
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Embeddings (retrieval)</div>
          {providerSelect(embedderProvider, setEmbedderProvider, "Local Ollama (free, private)", "Cloud OpenAI-compatible")}
          {keyInput("embed_api_key", embedderProvider, "API key")}
          <label style={styles.label}>Base URL</label>
          <input
            style={styles.input}
            value={form.embed_base_url || (embedderProvider === "local" ? form.ollama_base_url : "")}
            onChange={set("embed_base_url")}
            placeholder={embedderProvider === "local" ? "http://localhost:11434 (uses Ollama)" : "https://api.openai.com/v1"}
          />
          <label style={styles.label}>Model</label>
          <input style={styles.input} value={form.embed_model} onChange={set("embed_model")} placeholder="nomic-embed-text / text-embedding-3-small" />
          {embedderProvider === "cloud" && (
            <div style={{ fontSize: 12, color: theme.amber, marginTop: 6 }}>
              Note: changing the embedding model means re-embedding all chunks (reclassify sources or wait for the backfill job).
            </div>
          )}
          <button style={styles.button} disabled={testBusy === "embedder"} onClick={() => testConnection("embedder")}>
            {testBusy === "embedder" ? "Testing…" : "Test embeddings"}
          </button>
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>TOC & Tags — hierarchical retrieval</div>
          <div style={{ fontSize: 12, color: theme.textDim, lineHeight: 1.5 }}>
            Sections are split at headings (<code>§434</code>, <code>4.2.1</code>, <code>Article 12</code>) into a tree (<code>level/path</code>) with per-section summaries. Tags are a dynamic taxonomy — per-section keywords are proposed and reused if cosine &gt; threshold else a new tag is created.
          </div>
          <label style={styles.label}>Tag reuse threshold (cosine 0–1, default 0.82)</label>
          <input
            style={styles.input}
            value={form ? (form as unknown as Record<string, string>)["tag_reuse_threshold"] ?? "0.82" : "0.82"}
            onChange={(e) => setForm({ ...(form as unknown as Record<string, string>), tag_reuse_threshold: e.target.value } as unknown as AppSettings)}
            placeholder="0.82 — higher = stricter reuse, lower = more new tags"
          />
          <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 4 }}>
            Env <code>ANCHOR_TAG_REUSE_THRESHOLD</code> overrides. Lower values create more tags, higher values reuse more aggressively. Effective threshold is read at ingest time.
          </div>
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Answer model (Q&A)</div>
          {providerSelect(answerProvider, setAnswerProvider, "Local Ollama (free, private)", "Cloud OpenAI-compatible")}
          {keyInput("answer_api_key", answerProvider, "API key")}
          <label style={styles.label}>Base URL</label>
          <input
            style={styles.input}
            value={form.answer_base_url}
            onChange={set("answer_base_url")}
            placeholder="http://localhost:11434/v1 / https://api.openai.com/v1"
          />
          <label style={styles.label}>Model</label>
          <input style={styles.input} value={form.answer_model} onChange={set("answer_model")} placeholder="llama3.2:3b / gpt-4o-mini" />
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
          <div style={{ fontSize: 12, color: theme.textDim, marginTop: 4 }}>
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
              background: testResult.ok ? "#1e3a2a" : theme.redBg,
              color: testResult.ok ? "#86efac" : theme.redText,
              fontSize: 13,
            }}
          >
            {testResult.provider}: {testResult.message}
          </div>
        )}

        {error && <p style={{ color: theme.red, margin: 0 }}>{error}</p>}
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button style={{ ...styles.button, background: theme.accent, color: "#fff" }} disabled={busy} onClick={save}>
            {busy ? "Saving…" : "Save"}
          </button>
          {saved && <span style={{ color: theme.green, fontSize: 13 }}>Saved — applied without restart</span>}
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  label: { display: "block", fontSize: 12, color: theme.textMuted, marginBottom: 4, marginTop: 8 },
  input: { width: "100%", padding: "0.5rem 0.75rem", borderRadius: 8, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, boxSizing: "border-box" },
  button: { marginTop: 10, padding: "0.5rem 1rem", borderRadius: 8, border: "none", background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 13 },
  section: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.9rem 1rem" },
  sectionTitle: { fontWeight: 600, fontSize: 14, marginBottom: 4 },
};
