import { useEffect, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import type { AppSettings, OnboardingState } from "../types";
import { useModelPull } from "../useModelPull";
import { OllamaInstall } from "./OllamaInstall";
import { PullProgress } from "./PullProgress";
import { styles } from "./styles";
import { exceedsRam, initialPick, isMissing, ollamaAnswerBase, ramLabel, sameModel } from "./wizardText";

type Props = { state: OnboardingState | null; recheck: () => Promise<OnboardingState | null> };

/** R45.6 local mode: an Ollama answer model sized to this machine's RAM;
 * a BYO OpenAI-compatible endpoint stays under Advanced (same code path). */
export function AnswerModelStep({ state, recheck }: Props) {
  const machine = state?.machine;
  const options = machine?.answer_models ?? [];
  const [pick, setPick] = useState<string>("");
  const [ollamaBase, setOllamaBase] = useState("http://localhost:11434");
  const [current, setCurrent] = useState<{ base: string; model: string } | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const pull = useModelPull();

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        const base = s.ollama_base_url || "http://localhost:11434";
        setOllamaBase(base);
        setCurrent({ base: s.answer_base_url || "", model: s.answer_model || "" });
        const localAnswers = (s.answer_base_url || "").startsWith(base.replace(/\/+$/, ""));
        setPick((p) => p || initialPick(options, localAnswers ? s.answer_model : null, machine?.recommended_answer_model ?? ""));
      })
      .catch(() => {});
    // Options arrive with the first onboarding read; re-run once they do.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.length]);

  const reachable = !!state?.ollama.reachable;
  const missing = state?.ollama.missing_models ?? [];
  const answerBase = ollamaAnswerBase(ollamaBase);
  const usingPick = !!current && current.base === answerBase && sameModel(current.model, pick);
  const ready = usingPick && !!state?.answer_ready && !isMissing(missing, pick);

  async function usePick() {
    if (!pick) return;
    setSaving(true);
    setMsg(null);
    try {
      await api.updateSettings({ answer_base_url: answerBase, answer_model: pick });
      setCurrent({ base: answerBase, model: pick });
      const fresh = await recheck();
      if (fresh && isMissing(fresh.ollama.missing_models, pick)) {
        const done = await pull.startPull([pick]);
        if (done.length > 0) await recheck();
      }
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  const ram = ramLabel(machine?.total_memory_bytes);
  return (
    <div style={styles.body}>
      <div style={styles.note}>
        {ram ? `This computer has ${ram} of memory. ` : "Couldn't read this computer's memory — showing the default. "}
        Bigger models write better answers but need more memory and answer slower. The recommended one leaves room for your other apps.
      </div>
      {options.length > 0 && (
        <div style={{ display: "grid", gap: 6 }} role="radiogroup" aria-label="Answer model">
          {options.map((o) => {
            const tooBig = exceedsRam(o, machine?.total_memory_bytes);
            return (
              <button
                key={o.model}
                role="radio"
                aria-checked={pick === o.model}
                style={{ ...styles.choice, ...(pick === o.model ? styles.choiceActive : {}) }}
                onClick={() => setPick(o.model)}
              >
                <div style={{ display: "flex", justifyContent: "space-between", gap: 8, flexWrap: "wrap" }}>
                  <span style={styles.choiceTitle}>{o.label}</span>
                  {o.recommended && <span style={{ fontSize: 12, color: theme.accentAlt }}>Recommended for this computer</span>}
                </div>
                <div style={{ fontSize: 12, color: tooBig ? theme.amber : theme.textMuted }}>
                  <code>{o.model}</code> · {o.download_gb} GB download
                  {o.min_ram_gb > 0 ? ` · for ${o.min_ram_gb} GB+ RAM` : ""}
                  {tooBig ? " — more than this computer has; expect slow answers" : ""}
                </div>
              </button>
            );
          })}
        </div>
      )}

      <OllamaInstall state={state} recheck={recheck} />

      {reachable && (
        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <button style={{ ...styles.button, ...styles.primary }} disabled={!pick || saving || pull.pulling} onClick={usePick}>
            {pull.pulling ? "Downloading…" : saving ? "Setting up…" : isMissing(missing, pick) || !usingPick ? `Use ${pick} (download if needed)` : `Use ${pick}`}
          </button>
          {ready && (
            <span style={styles.checkRow}>
              <span style={{ ...styles.dot, background: theme.green }} />
              <span style={{ fontSize: 13 }}>Answers: {pick} via Ollama — ready</span>
            </span>
          )}
        </div>
      )}
      {pull.pulling || pull.pullError ? <PullProgress pulls={pull.pulls} models={[pick]} error={pull.pullError} /> : null}
      {msg && <div style={{ fontSize: 12, color: theme.red }}>{msg}</div>}
      {!ready && state?.answer_ready && !usingPick && (
        <div style={styles.hint}>An answer model is already configured ({current?.model}) — keep it, or pick one above.</div>
      )}
      <AdvancedEndpoint ollamaBase={ollamaBase} setOllamaBase={setOllamaBase} recheck={recheck} />
    </div>
  );
}

/** BYO OpenAI-compatible answer endpoint + the Ollama URL — the advanced
 * variant of local mode (R45.2), not a third mode. */
function AdvancedEndpoint({
  ollamaBase,
  setOllamaBase,
  recheck,
}: {
  ollamaBase: string;
  setOllamaBase: (v: string) => void;
  recheck: () => Promise<unknown>;
}) {
  const [open, setOpen] = useState(false);
  const [base, setBase] = useState("");
  const [key, setKey] = useState("");
  const [keyStored, setKeyStored] = useState(false);
  const [model, setModel] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    api
      .getSettings()
      .then((s) => {
        setBase(s.answer_base_url || "");
        setKeyStored(s.answer_api_key === "***set***");
        setModel(s.answer_model || "");
      })
      .catch(() => {});
  }, [open]);

  async function save() {
    setBusy(true);
    setMsg(null);
    try {
      const payload: Partial<AppSettings> = {};
      if (ollamaBase.trim()) payload.ollama_base_url = ollamaBase.trim();
      if (base.trim()) payload.answer_base_url = base.trim();
      if (key.trim()) payload.answer_api_key = key.trim();
      if (model.trim()) payload.answer_model = model.trim();
      await api.updateSettings(payload);
      const t = await api.testConnection("answer");
      setMsg(t.ok ? `Saved. Connected: ${t.message}` : `Saved, but not connected: ${t.message}`);
      await recheck();
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <details open={open} onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary style={{ fontSize: 12, color: theme.textMuted, cursor: "pointer" }}>
        Advanced: another Ollama URL, or an OpenAI-compatible endpoint
      </summary>
      <div style={{ display: "grid", gap: 6, marginTop: 8 }}>
        <label style={styles.label}>Ollama base URL</label>
        <input style={styles.input} value={ollamaBase} onChange={(e) => setOllamaBase(e.target.value)} placeholder="http://localhost:11434" />
        <label style={styles.label}>Answer API base URL</label>
        <input style={styles.input} value={base} onChange={(e) => setBase(e.target.value)} placeholder="https://api.openai.com/v1" />
        <label style={styles.label}>Answer API key — stored in the OS keychain</label>
        <input
          style={styles.input}
          type="password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder={keyStored ? "•••••••• (stored — type to replace, blank keeps it)" : "sk-… (not needed for local Ollama)"}
        />
        <label style={styles.label}>Answer model</label>
        <input style={styles.input} value={model} onChange={(e) => setModel(e.target.value)} placeholder="gpt-4o-mini" />
        <div style={styles.hint}>Cloud endpoints never see Sensitive sources unless the provider is trusted (ANCHOR_TRUSTED_PROVIDERS).</div>
        <div>
          <button style={{ ...styles.button, ...styles.secondary }} disabled={busy} onClick={save}>
            {busy ? "Saving…" : "Save & test"}
          </button>
        </div>
        {msg && <div style={{ fontSize: 12, color: theme.accentAlt }}>{msg}</div>}
      </div>
    </details>
  );
}
