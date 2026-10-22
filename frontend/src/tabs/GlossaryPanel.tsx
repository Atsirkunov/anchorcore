import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { theme, radius } from "../theme";
import type { GlossaryEntry, ItemMeta } from "../types";

/**
 * R45.13: the people-owned glossary. Replaces "Words to confirm": nothing is
 * proposed, so nothing waits for review. People type entries here or mark a
 * whole file (CSV, Markdown table, `CODE - meaning` lines) as a glossary.
 * Every entry is active at once and teaches search that the short form means
 * the long one.
 */
export function GlossaryPanel() {
  const [entries, setEntries] = useState<GlossaryEntry[]>([]);
  const [open, setOpen] = useState(false);
  const [term, setTerm] = useState("");
  const [expansion, setExpansion] = useState("");
  const [fileQuery, setFileQuery] = useState("");
  const [files, setFiles] = useState<ItemMeta[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .glossary()
      .then(setEntries)
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(() => refresh(), [refresh]);

  async function run(work: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await work();
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const add = () =>
    run(async () => {
      await api.addGlossaryEntry(term, expansion);
      setTerm("");
      setExpansion("");
    });

  const findFiles = () =>
    run(async () => {
      const list = await api.listItems({ path_prefix: fileQuery.trim(), limit: 10 });
      setFiles(list.items);
    });

  const toggleFile = (item: ItemMeta) =>
    run(async () => {
      const updated = await api.setItemGlossary(item.id, !item.is_glossary);
      setFiles((fs) => fs.map((f) => (f.id === item.id ? { ...f, is_glossary: updated.is_glossary } : f)));
    });

  const manual = entries.filter((e) => e.origin === "manual");
  const fromFiles = entries.filter((e) => e.origin === "glossary");

  return (
    <section style={styles.card}>
      <button style={styles.header} onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        Glossary ({entries.length}) {open ? "▾" : "▸"}
      </button>
      <div style={styles.hint}>
        Terms your team uses, so search finds “KYC” when a document says “know your customer”. Entries
        you add work immediately; nothing is guessed for you to approve.
      </div>
      {open && (
        <div style={{ display: "grid", gap: 10, marginTop: 8 }}>
          {error && <div style={{ color: theme.red, fontSize: 12 }}>{error}</div>}
          <div style={styles.row}>
            <input style={styles.input} placeholder="Short form (e.g. KYC)" value={term} onChange={(e) => setTerm(e.target.value)} />
            <input
              style={{ ...styles.input, flex: 1 }}
              placeholder="Meaning (e.g. know your customer)"
              value={expansion}
              onChange={(e) => setExpansion(e.target.value)}
            />
            <button style={styles.button} disabled={busy || !term.trim() || !expansion.trim()} onClick={add}>
              Add
            </button>
          </div>
          {manual.length > 0 && (
            <ul style={styles.list}>
              {manual.map((e) => (
                <li key={e.id} style={styles.entry}>
                  <span>
                    <b>{e.term}</b> = {e.expansion}
                  </span>
                  <button style={styles.link} disabled={busy} onClick={() => run(() => api.deleteGlossaryEntry(e.id))}>
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div style={styles.subhead}>Use a file as a glossary</div>
          <div style={styles.row}>
            <input
              style={{ ...styles.input, flex: 1 }}
              placeholder="File path starts with… (e.g. docs/glossary)"
              value={fileQuery}
              onChange={(e) => setFileQuery(e.target.value)}
            />
            <button style={styles.button} disabled={busy || !fileQuery.trim()} onClick={findFiles}>
              Find
            </button>
          </div>
          {files.length > 0 && (
            <ul style={styles.list}>
              {files.map((f) => (
                <li key={f.id} style={styles.entry}>
                  <span>{f.path}</span>
                  <button style={styles.button} disabled={busy} onClick={() => toggleFile(f)}>
                    {f.is_glossary ? "Stop using as glossary" : "Use as glossary"}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {fromFiles.length > 0 && (
            <div style={styles.hint}>
              {fromFiles.length} entries come from marked files ({[...new Set(fromFiles.map((e) => e.path ?? ""))].join(", ")}).
            </div>
          )}
        </div>
      )}
    </section>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.7rem 0.9rem", marginBottom: 12 },
  header: { background: "none", border: "none", color: theme.text, fontWeight: 700, fontSize: 13, padding: 0, cursor: "pointer" },
  hint: { fontSize: 12, color: theme.textDim, marginTop: 2 },
  subhead: { fontSize: 12, fontWeight: 700, color: theme.textMuted },
  row: { display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" },
  input: { padding: "0.3rem 0.5rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgElevated, color: theme.text, fontSize: 12 },
  button: { padding: "0.3rem 0.7rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgHover, color: theme.text, cursor: "pointer", fontSize: 12 },
  link: { background: "none", border: "none", color: theme.accentAlt, cursor: "pointer", fontSize: 12, padding: 0, textDecoration: "underline" },
  list: { listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 4 },
  entry: { display: "flex", justifyContent: "space-between", gap: 8, fontSize: 12, color: theme.text },
};
