# AnchorCore — Backend Port to Rust (evaluation + plan, B31)

> **Status: backlog — workspace `rust/` created.** The question is real — *"would Rust make this
> easier to run and faster?"* — and the answer is more honest than either a blanket
> "yes" or "no". This doc captures the reasoning and a concrete, incremental plan.
> Companion: [product-plan.md](./product-plan.md#b31-backend-port-to-rust-p3deferred) + `rust/BACKLOG.md:1` + `rust/README.md:1`.

---

## 1. The two claims, separated

### "Rust is easier for people to run" — TRUE, and this is the strongest argument

The pain today is not the Python language — it is **Python packaging**:

| Pain | Root cause | Rust |
|---|---|---|
| Release crash (v1.0.1): `enable_load_extension` missing | CI's CPython (toolcache & python.org) compiles `sqlite3` without loadable-extension support; PyInstaller bundles that build | impossible — sqlite-vec linked statically at build time |
| sqlite_vec native lib not found by PyInstaller | the `.dll`/`.dylib` has to be hand-collected in `packaging.spec` | linked in, never "collected" |
| macOS must be built *on* a Mac | PyInstaller doesn't cross-compile | cargo cross-compiles win/mac/linux from one runner |
| venv, `backend/.venv`, pip, Python 3.12 requirement | Python runtime + dependency tree | one statically-linked binary |
| Slow cold start | interpreter + import storm | near-instant |

A single static Rust binary with the React UI embedded is the cleanest distribution
story available. That part of the thesis is solid.

### "Rust is faster" — MOSTLY FALSE for the user-visible path

The dominant cost is **LLM inference** — classifying 16k-char windows and generating
answers — and that runs in **Ollama / a cloud API regardless of backend language**.
Rust does not make one model call faster.

Rust would speed up the CPU-bound slice:
- retrieval (`_vector_search` does a **full-table scan + Python cosine** — genuinely slow
  on large corpora),
- chunking / text processing / parsing.

But that slice is small relative to model latency. **Faster retrieval is achievable
in Python for a day of work** (sqlite-vec `vec0` index, see §4) — without a rewrite.

### The real blocker for non-technical people is NOT the language

A non-techie cannot install Ollama + pull models any more easily than they can install
a Python runtime. The largest distribution gap in the app is **the Ollama prerequisite**
— which is *language-independent*. That fix (bundled llama.cpp inference, first-run
wizard) delivers more adoption than a rewrite ever will.

---

## 2. Ease of support: Python vs Rust, honestly

| Dimension | Python (today) | Rust |
|---|---|---|
| **Talent pool** | Huge; a solo maintainer is never alone on the internet | Smaller; domain code written by the same person stays with them |
| **Iteration speed** | Fast — edit + reload, no compile | Slower — compile cycle, borrow checker friction on domain logic |
| **Runtime failure surface** | Packaging is a *constant tax*: PyInstaller, native libs, Python version drift, PEP-668 | Tiny: no runtime to get wrong |
| **Release confidence** | We've already shipped **two packaging bugs** (`.dylib` glob; toolcache Python) | binary either builds or it doesn't |
| **Debugging prod** | Stack traces everywhere, easy to inspect | harder to introspect, but fewer failure modes |
| **Compile times** | n/a | minutes per change — real cost for a solo dev |
| **Long-term support** | The Python toolchain will keep evolving under you | a frozen binary keeps working forever |

**Net for a single-maintainer product:** Rust trades iteration speed for runtime
reliability. Since the failures that actually *reach testers* today are packaging
failures (not logic bugs), Rust removes exactly the failures that hurt the product's
reputation. That is a fair trade — **provided the port is incremental** (§5), so you
don't lose iteration speed across the whole backlog while it happens.

---

## 3. What would move, what would stay

**Stays (unchanged, reused as-is):**
- React/Vite frontend — talks to the API; the backend port is invisible to it
- SQLite database + schema + Alembic migration history — a Rust binary reads the same file
- Ollama / BYO LLM integration (HTTP) — identical calls from Rust (`reqwest`)

**Moves to Rust (axum + rusqlite + sqlite-vec crate):**
- REST API layer (drop-in replacement: same routes, same JSON, same error shapes)
- Ingestion pipeline: folder watch (`notify`), extraction, hash dedup, chunking
- Classification orchestration (still calls Ollama/cloud)
- Retrieval: RRF fusion, FTS5 (rusqlite supports it), cosine or `vec0` index
- Job manager, settings service, secret store (OS keychain via `keyring`-equivalent, e.g. `security` CLI or the `keyring` crate)

**Drop in the move:** SQLAlchemy, FastAPI, uvicorn, PyInstaller, the venv — the entire
Python runtime surface.

---

## 4. Cheaper wins first (any language)

1. **Bundled inference (llama.cpp / llamafile)** — already in `packaging.md` open work.
   Kills the Ollama prerequisite → the *actual* "works for non-techies" fix. ~2–3 days.
2. **`vec0` index for retrieval** — `_vector_search` scans every chunk and computes
   cosine in Python. sqlite-vec (already bundled) has a `vec0` virtual table; using it
   is ~10–100x faster retrieval, ~1 day, zero rewrite. (Partial progress: B18's IDF gate
   already skips low-signal chunks from the scan.)
3. **First-run wizard (B8)** — guided Ollama install / model pull, so the remaining
   prerequisite is a click-through, not an unknown.

Do these first. If testers are happy after 1+3, the Rust port loses most of its urgency.

---

## 5. The plan (incremental — never a big-bang rewrite)

**Golden rule: the REST API + SQLite schema are the contract.** Both sides must pass
the same conformance tests (the existing `tests/` suite becomes it). Until the port is
complete, the Python backend stays the shipped artifact and the Rust side is developed
behind `ANCHOR_PORT` on a dev box.

### Phase 0 — decide (0 days, gate on §4 outcomes)
If bundled inference + vec0 make testers happy and releases stop breaking, **don't port**.
Revisit only if packaging bugs or retrieval scale force the issue.

### Phase 1 — perf spike in Python (1–2 days, regardless)
Land the `vec0` retrieval index (§4.2) and re-run the live probes from B12.1. Record
retrieval latency on the real corpus. This is the benchmark the Rust side must beat.

### Phase 2 — Rust skeleton behind the same API (1–2 weeks)
- axum app serving the same routes from the same SQLite file (read-only first)
- migration-history awareness so the Rust binary never writes a schema the Python side
  doesn't recognize (reuse Alembic-created tables as-is)
- CI job builds the Rust binary + runs the **shared conformance tests** against it

### Phase 3 — hot path in Rust (1 week)
Port retrieval + answer orchestration only. Python still does ingestion/classification;
Rust answers queries. Both read the same DB. This is where the user-visible perf gain
(if any) shows up and where a rollback is trivial (point the API back at Python).

### Phase 4 — full port (2–3 weeks)
Port connectors, ingestion, jobs, settings, secrets. Python backend retires. The 82
tests now run green against the Rust binary. Frontend unchanged.

**Total single-dev estimate: ~5–8 weeks end-to-end; ~3–4 weeks to a genuine
drop-in answer path (Phase 3).** The React UI and the backlog never stall because the
contract keeps both worlds working in parallel.

---

## 6. When the port is actually worth it

| Signal | Action |
|---|---|
| Testers can't install Ollama | bundled inference **first** (language-independent) |
| Release keeps breaking on packaging (2 incidents already) | port sooner — this is Rust's biggest win |
| Retrieval latency on real corpora matters to users | vec0 first; Rust only if still insufficient |
| Team/agent access (B14) drives traffic | Rust's concurrency + memory safety help |

---

*Workspace: `rust/` (Cargo workspace, `rust/AGENTS.md`, `rust/BACKLOG.md`). AI agents: pick one `R*.*` task, branch `rust/R1.1`.*

*Companion docs: [architecture.md](./architecture.md), [packaging.md](./packaging.md), [product-plan.md](./product-plan.md), [rust/BACKLOG.md](../rust/BACKLOG.md)*
