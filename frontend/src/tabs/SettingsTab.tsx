import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { stripPlaceholders } from "../placeholders";
import { theme, radius } from "../theme";
import type { AppSettings, AskResponse, EmbedProvider, LlmMode } from "../types";
import { BuiltinEmbedder } from "../BuiltinEmbedder";
import { SensitiveSharingCard } from "./SensitiveSharingCard";

type ProbeResult = { status: "pass" | "fail" | "inconclusive"; message: string };

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
  const [probeBusy, setProbeBusy] = useState(false);
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
  const [savedCount, setSavedCount] = useState(0);

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
  const embedderProvider: ProviderId = isLocalUrl(form?.embed_base_url ?? "") ? "local" : "cloud";
  const answerProvider: ProviderId = isLocalUrl(form?.answer_base_url ?? "") ? "local" : "cloud";

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
      for (const k of ["answer_api_key", "embed_api_key"] as const) {
        if (!payload[k]) delete payload[k];
      }
      const updated = await api.updateSettings(payload);
      setSettings(updated);
      setForm({ ...updated });
      setSaved(true);
      setSavedCount((n) => n + 1);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function testConnection(providerId: "ollama" | "embedder" | "answer") {
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

  // R23.4: the one-flow BYOM check — a real ask whose mode must be
  // "composed". Save first (the probe reads the saved endpoint), then test
  // the connection, then probe: only composed means answers really work.
  async function runComposeProbe() {
    setProbeBusy(true);
    setProbeResult(null);
    setError(null);
    try {
      const raw = await api.ask("Summarize what this memory contains in one sentence.", [], undefined, undefined, false);
      const res = raw as AskResponse & { warning?: string | null };
      if (res.mode === "composed") {
        setProbeResult({ status: "pass", message: "Composed answer confirmed (mode: composed) — your answer model is working." });
      } else if (res.refusal) {
        setProbeResult({ status: "fail", message: res.warning ?? "Probe refused: relevant knowledge is gated from this provider." });
      } else if ((res.citations?.length ?? 0) === 0) {
        setProbeResult({
          status: "inconclusive",
          message: "No matching content to compose from — sync a source first, then probe again. (An empty memory reports mode: none even with a working model.)",
        });
      } else {
        setProbeResult({ status: "fail", message: res.warning ?? `No answer model ran (mode: ${res.mode ?? "unknown"}). Save settings, then probe again.` });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setProbeBusy(false);
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

  const keyInput = (key: "answer_api_key" | "embed_api_key", provider: ProviderId, label: string) => (
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
          <div style={styles.sectionTitle}>Embeddings (retrieval)</div>
          <label style={styles.label}>Engine</label>
          <select
            style={styles.input}
            value={form.embed_provider ?? "auto"}
            onChange={(e) => setForm({ ...form, embed_provider: e.target.value as EmbedProvider })}
          >
            <option value="auto">Automatic — Ollama/endpoint below when it serves, else the built-in model</option>
            <option value="builtin">Built-in model (in the app, no Ollama)</option>
            <option value="ollama">Ollama or a cloud endpoint (below)</option>
          </select>
          <BuiltinEmbedder />
          {form.embed_provider !== "builtin" && (
            <>
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
            </>
          )}
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
          <div style={styles.sectionTitle}>Updates</div>
          <div style={{ fontSize: 12, color: theme.textDim, lineHeight: 1.5 }}>
            AnchorCore never checks for new releases on its own. The System tab has a manual <b>Check for
            updates</b> button; turn on the opt-in below if you want one check when the app starts.
          </div>
          <label style={{ display: "flex", gap: 8, alignItems: "flex-start", marginTop: 10, fontSize: 13, cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={["1", "true", "yes", "on"].includes(
                ((form as unknown as Record<string, string>)["update_check_enabled"] ?? "").toLowerCase(),
              )}
              onChange={(e) =>
                setForm({
                  ...(form as unknown as Record<string, string>),
                  update_check_enabled: e.target.checked ? "1" : "0",
                } as unknown as AppSettings)
              }
            />
            <span>
              Check for new releases once at startup
              <span style={{ display: "block", fontSize: 11, color: theme.textMuted }}>
                One request to github.com at launch — a plain fetch of the public release info; nothing about you
                is sent. Off unless you turn it on.
              </span>
            </span>
          </label>
        </div>

        <div style={styles.section}>
          <div style={styles.sectionTitle}>Who writes the answers?</div>
          <select
            style={styles.input}
            value={form?.llm_mode === "agent" ? "agent" : "local"}
            onChange={(e) => form && setForm({ ...form, llm_mode: e.target.value as LlmMode })}
          >
            <option value="agent">My AI agent (MCP or the ask-docs skill)</option>
            <option value="local">A local model on this computer</option>
          </select>
          <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 4, lineHeight: 1.5 }}>
            Agent: AnchorCore runs no chat model for answers — <code>ask</code> returns ranked, cited passages
            (<code>mode: agent_compose</code>) and your agent composes. Local: the answer model below writes them.
            Switching needs no re-ingest.
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
          <div style={{ fontSize: 12, color: theme.textDim, marginTop: 4 }}>
            Local-only? Set the Ollama URL explicitly here — answers never fall back to Ollama on their own (embeddings do).
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <button style={styles.button} disabled={testBusy === "answer"} onClick={() => testConnection("answer")}>
              {testBusy === "answer" ? "Testing…" : "Test answer model"}
            </button>
            <button
              style={styles.button}
              disabled={probeBusy}
              onClick={runComposeProbe}
              title="Save first, then probe: asks a real question and requires a composed answer (mode: composed)"
            >
              {probeBusy ? "Probing…" : "Run compose probe"}
            </button>
          </div>
          <div style={{ fontSize: 12, color: theme.textDim, marginTop: 4 }}>
            One flow: pick local Ollama or your own endpoint → Save → Test → Probe. Only a probe reporting mode: composed means answers really work.
          </div>
          {probeResult && (
            <div
              style={{
                marginTop: 8,
                padding: "0.5rem 0.75rem",
                borderRadius: radius,
                background: probeResult.status === "pass" ? theme.greenBg : probeResult.status === "fail" ? theme.redBg : theme.amberBg,
                color: probeResult.status === "pass" ? theme.greenText : probeResult.status === "fail" ? theme.redText : theme.amber,
                fontSize: 13,
              }}
            >
              {probeResult.message}
            </div>
          )}
        </div>

        <SensitiveSharingCard refreshKey={savedCount} />

        {testResult && (
          <div
            style={{
              padding: "0.5rem 0.75rem",
              borderRadius: radius,
              background: testResult.ok ? theme.greenBg : theme.redBg,
              color: testResult.ok ? theme.greenText : theme.redText,
              fontSize: 13,
            }}
          >
            {testResult.provider}: {testResult.message}
          </div>
        )}

        {error && <p style={{ color: theme.red, margin: 0 }}>{error}</p>}
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button style={{ ...styles.button, background: theme.accent, color: theme.onAccent }} disabled={busy} onClick={save}>
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
  input: { width: "100%", padding: "0.5rem 0.75rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, boxSizing: "border-box" },
  button: { marginTop: 10, padding: "0.5rem 1rem", borderRadius: radius, border: "none", background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 13 },
  section: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.9rem 1rem" },
  sectionTitle: { fontWeight: 600, fontSize: 14, marginBottom: 4 },
};
