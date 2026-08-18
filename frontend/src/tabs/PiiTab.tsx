import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { theme, commonStyles } from "../theme";
import type { PiiChunk, PiiConfig } from "../types";

/** B30: PII configuration + review. Configure what counts as PII (built-in
 * categories + custom filter words), then review chunks the scan flagged and
 * confirm/override their PII-sensitive classification. */
export function PiiTab() {
  const [config, setConfig] = useState<PiiConfig | null>(null);
  const [review, setReview] = useState<PiiChunk[]>([]);
  const [newWord, setNewWord] = useState("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.piiConfig().then(setConfig).catch((e) => setError(String(e)));
    api.piiReview({ only_flagged: false }).then(setReview).catch(() => {});
  }, []);

  useEffect(() => refresh(), [refresh]);

  function toggleCategory(id: string) {
    if (!config) return;
    setConfig({
      ...config,
      categories: config.categories.map((c) => (c.id === id ? { ...c, enabled: !c.enabled } : c)),
    });
  }

  async function saveConfig() {
    if (!config) return;
    setBusy("config");
    setError(null);
    setSaved(false);
    try {
      const updated = await api.updatePiiConfig({
        custom_words: config.custom_words,
        disabled_categories: config.categories.filter((c) => !c.enabled).map((c) => c.id),
      });
      setConfig(updated);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  function addWord() {
    const word = newWord.trim();
    if (!config || !word) return;
    if (config.custom_words.includes(word)) return;
    setConfig({ ...config, custom_words: [...config.custom_words, word] });
    setNewWord("");
  }

  function removeWord(word: string) {
    if (!config) return;
    setConfig({ ...config, custom_words: config.custom_words.filter((w) => w !== word) });
  }

  async function decide(chunk: PiiChunk, is_pii: boolean) {
    setBusy(`chunk-${chunk.chunk_id}`);
    setError(null);
    try {
      await api.decidePii(chunk.chunk_id, is_pii);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  if (!config) {
    return (
      <div>
        <h2>PII configuration</h2>
        {error ? <p style={{ color: theme.red }}>Failed to load: {error}</p> : <p>Loading…</p>}
      </div>
    );
  }

  return (
    <div>
      <h2>PII configuration</h2>
      {error && <p style={{ color: theme.red }}>{error}</p>}

      <div style={{ ...styles.section, marginBottom: 16 }}>
        <div style={styles.sectionTitle}>
          Custom filter words
          <span style={{ fontSize: 12, color: theme.textDim, fontWeight: 400, marginLeft: 8 }}>
            any chunk containing these is flagged PII-sensitive
          </span>
        </div>
        <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
          <input
            style={commonStyles.input}
            placeholder="e.g. sam rivers, national insurance, passport scan"
            value={newWord}
            onChange={(e) => setNewWord(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addWord()}
          />
          <button onClick={addWord} style={commonStyles.button}>
            Add
          </button>
        </div>
        {config.custom_words.length === 0 ? (
          <div style={{ fontSize: 12, color: theme.textDim }}>No custom words yet.</div>
        ) : (
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
            {config.custom_words.map((w) => (
              <span key={w} style={styles.wordChip}>
                {w}
                <button onClick={() => removeWord(w)} style={styles.chipRemove} title="remove">
                  ×
                </button>
              </span>
            ))}
          </div>
        )}
      </div>

      <div style={{ ...styles.section, marginBottom: 16 }}>
        <div style={styles.sectionTitle}>
          Recognised PII categories
          <span style={{ fontSize: 12, color: theme.textDim, fontWeight: 400, marginLeft: 8 }}>
            toggle to enable/disable detection
          </span>
        </div>
        <div style={{ display: "grid", gap: 6 }}>
          {config.categories.map((c) => (
            <label key={c.id} style={styles.categoryRow}>
              <input type="checkbox" checked={c.enabled} onChange={() => toggleCategory(c.id)} />
              <div style={{ flex: 1 }}>
                <div style={{ fontWeight: 600, fontSize: 13 }}>{c.label}</div>
                <div style={{ fontSize: 12, color: theme.textDim }}>{c.description}</div>
                {c.field_names.length > 0 && (
                  <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 2 }}>
                    fields: {c.field_names.join(", ")}
                  </div>
                )}
              </div>
            </label>
          ))}
        </div>
      </div>

      <div style={{ display: "flex", gap: 8, marginBottom: 24 }}>
        <button onClick={saveConfig} disabled={busy === "config"} style={{ ...commonStyles.button, background: theme.accent, color: "#fff" }}>
          {busy === "config" ? "Saving…" : "Save PII config"}
        </button>
        {saved && <span style={{ color: theme.green, alignSelf: "center", fontSize: 13 }}>Saved</span>}
      </div>

      <h3 style={{ marginBottom: 8 }}>Review — chunks matching PII</h3>
      {review.length === 0 ? (
        <p style={{ color: theme.textDim }}>No chunks matched PII yet.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 8 }}>
          {review.map((r) => (
            <li key={r.chunk_id} style={styles.reviewCard}>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                <span
                  style={{
                    fontSize: 11,
                    background: r.is_pii ? theme.redBg : "#1e2430",
                    color: r.is_pii ? theme.redText : theme.textMuted,
                    padding: "0.1rem 0.5rem",
                    borderRadius: 999,
                    fontWeight: 700,
                  }}
                >
                  {r.is_pii ? "PII-SENSITIVE" : "flagged"}
                </span>
                {r.source_name && <span style={{ fontSize: 12, color: theme.textMuted }}>{r.source_name}</span>}
                {r.source_label !== "internal" && (
                  <span style={{ fontSize: 12, color: theme.amber }}>label: {r.source_label}</span>
                )}
                <span style={{ fontSize: 12, color: theme.textDim }}>#{r.chunk_id} · {r.kind}</span>
              </div>
              {r.categories.length > 0 && (
                <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 6 }}>
                  {r.categories.map((m) => (
                    <span
                      key={m.category}
                      style={{
                        fontSize: 11,
                        background: m.strong ? theme.redBg : "#1e2430",
                        color: m.strong ? theme.redText : theme.textMuted,
                        padding: "0.1rem 0.5rem",
                        borderRadius: 999,
                      }}
                      title={m.match ? `matched: ${m.match}` : m.label}
                    >
                      {m.label}
                      {m.match ? `: ${m.match}` : ""}
                    </span>
                  ))}
                </div>
              )}
              <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 6, whiteSpace: "pre-wrap" }}>{r.snippet}</div>
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button
                  onClick={() => decide(r, true)}
                  disabled={busy === `chunk-${r.chunk_id}`}
                  style={{ ...commonStyles.button, background: theme.redBg, color: theme.redText, fontSize: 12 }}
                >
                  Mark PII
                </button>
                <button
                  onClick={() => decide(r, false)}
                  disabled={busy === `chunk-${r.chunk_id}`}
                  style={{ ...commonStyles.button, fontSize: 12 }}
                >
                  Not PII
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  section: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.9rem 1rem" },
  sectionTitle: { fontWeight: 600, marginBottom: 8 },
  wordChip: {
    display: "inline-flex",
    alignItems: "center",
    gap: 6,
    background: theme.bgHover,
    border: `1px solid ${theme.border}`,
    borderRadius: 999,
    padding: "0.2rem 0.6rem",
    fontSize: 13,
  },
  chipRemove: { background: "none", border: "none", color: theme.redText, cursor: "pointer", fontSize: 14, lineHeight: 1 },
  categoryRow: { display: "flex", gap: 8, alignItems: "flex-start", cursor: "pointer", fontSize: 13 },
  reviewCard: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.7rem 0.9rem" },
};
